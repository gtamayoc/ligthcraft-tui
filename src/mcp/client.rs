//! Asynchronous LightCraft MCP Client with Tokio stdio framing and supervisor loop.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex, Notify, RwLock};
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

use crate::error::{IntegrityViolationError, McpError};
use crate::mcp::integrity::{find_draw_dir, IntegrityEngine};
use crate::mcp::protocol::{
    error_codes, InitializeParams, InitializeResult, JsonRpcError, JsonRpcId, JsonRpcMessage,
    JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, ServerInfo, ToolCallParams,
    ToolCallResult, MCP_PROTOCOL_VERSION,
};
use crate::mcp::tools::*;

/// Connection status of the MCP client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Handshaking,
    Connected,
    ShuttingDown,
}

/// Operational mode: Live child process or Mock in-memory server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMode {
    Live,
    Mock,
}

/// Configuration settings for launching and maintaining the MCP client.
#[derive(Debug, Clone)]
pub struct McpClientConfig {
    /// Explicit executable path (defaults to auto-detection).
    pub executable_path: Option<PathBuf>,
    /// Additional arguments passed to lightcraft-cli (e.g. `["--library", "..."]`).
    pub extra_args: Vec<String>,
    /// Operational mode.
    pub mode: ClientMode,
    /// Per-request timeout duration (default: 30 seconds).
    pub request_timeout: Duration,
    /// Automatically attempt reconnection if child crashes.
    pub auto_reconnect: bool,
    /// Maximum reconnection attempts.
    pub max_reconnect_attempts: usize,
    /// Initial backoff delay between reconnect attempts.
    pub reconnect_backoff: Duration,
    /// Max line buffer for LinesCodec (default: 32 MB for image previews).
    pub max_line_length: usize,
    /// Protected source directories for non-destructive integrity enforcement and destination sandboxing.
    pub source_directories: Vec<PathBuf>,
}

impl Default for McpClientConfig {
    fn default() -> Self {
        Self {
            executable_path: None,
            extra_args: Vec::new(),
            mode: ClientMode::Live,
            request_timeout: Duration::from_secs(30),
            auto_reconnect: true,
            max_reconnect_attempts: 5,
            reconnect_backoff: Duration::from_millis(500),
            max_line_length: 32 * 1024 * 1024, // 32 Megabytes
            source_directories: vec![find_draw_dir()],
        }
    }
}

impl McpClientConfig {
    /// Create configuration targeting in-memory Mock mode for tests or offline execution.
    pub fn mock() -> Self {
        Self {
            mode: ClientMode::Mock,
            ..Default::default()
        }
    }

    /// Auto-detect LightCraft CLI executable on the system.
    pub fn resolve_executable(&self) -> Result<PathBuf, McpError> {
        if let Some(ref path) = self.executable_path {
            if path.exists() {
                return Ok(path.clone());
            }
        }

        // 1. Check environment variable LIGHTCRAFT_BIN or LIGHTCRAFT_CLI_PATH
        if let Ok(env_path) = std::env::var("LIGHTCRAFT_BIN") {
            let p = PathBuf::from(env_path);
            if p.exists() {
                return Ok(p);
            }
        }
        if let Ok(env_path) = std::env::var("LIGHTCRAFT_CLI_PATH") {
            let p = PathBuf::from(env_path);
            if p.exists() {
                return Ok(p);
            }
        }

        // 2. Standard Windows installation paths
        let standard_windows_paths = [
            PathBuf::from(r"C:\Program Files\LightCraft\lightcraft-cli.exe"),
            PathBuf::from(r"C:\Program Files (x86)\LightCraft\lightcraft-cli.exe"),
        ];

        for path in &standard_windows_paths {
            if path.exists() {
                return Ok(path.clone());
            }
        }

        // 3. Fallback: Search current working directory
        let relative_path = PathBuf::from("lightcraft-cli.exe");
        if relative_path.exists() {
            return Ok(relative_path);
        }

        Err(McpError::SpawnFailed {
            path: PathBuf::from("lightcraft-cli.exe"),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "LightCraft CLI executable not found in standard paths or LIGHTCRAFT_BIN",
            ),
        })
    }
}

type PendingSender = oneshot::Sender<Result<JsonRpcResponse, McpError>>;
type PendingRegistry = Arc<Mutex<HashMap<u64, PendingSender>>>;

struct McpClientInner {
    config: McpClientConfig,
    status: Arc<RwLock<ConnectionStatus>>,
    next_request_id: AtomicU64,
    pending: PendingRegistry,
    writer_tx: mpsc::Sender<String>,
    server_info: Arc<RwLock<Option<ServerInfo>>>,
    shutdown_flag: Arc<AtomicBool>,
    reconnect_notify: Arc<Notify>,
    pub(crate) integrity: Arc<IntegrityEngine>,
}

/// Asynchronous handle to the LightCraft MCP Client.
/// Cheaply cloneable (backed by `Arc`).
#[derive(Clone)]
pub struct McpClient {
    inner: Arc<McpClientInner>,
}

impl McpClient {
    /// Spawn a new MCP client session, completing the initialize handshake.
    pub async fn spawn(config: McpClientConfig) -> Result<Self, McpError> {
        let (writer_tx, writer_rx) = mpsc::channel::<String>(256);
        let pending: PendingRegistry = Arc::new(Mutex::new(HashMap::new()));
        let status = Arc::new(RwLock::new(ConnectionStatus::Connecting));
        let server_info = Arc::new(RwLock::new(None));
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let reconnect_notify = Arc::new(Notify::new());

        let source_dirs = if config.source_directories.is_empty() {
            vec![find_draw_dir()]
        } else {
            config.source_directories.clone()
        };
        let integrity = Arc::new(IntegrityEngine::new(source_dirs, true));

        let inner = Arc::new(McpClientInner {
            config: config.clone(),
            status: status.clone(),
            next_request_id: AtomicU64::new(1),
            pending: pending.clone(),
            writer_tx,
            server_info: server_info.clone(),
            shutdown_flag: shutdown_flag.clone(),
            reconnect_notify: reconnect_notify.clone(),
            integrity,
        });

        let client = Self { inner };

        if config.mode == ClientMode::Mock {
            tracing::info!("Initializing MCP client in MOCK mode");
            client.start_mock_transport(writer_rx).await?;
        } else {
            let exe_path = config.resolve_executable()?;
            tracing::info!("Spawning LightCraft child process: {:?}", exe_path);
            client.spawn_process_loop(exe_path, writer_rx).await?;
        }

        // Execute MCP initialize handshake
        client.perform_handshake().await?;

        Ok(client)
    }

    /// Convenience constructor that connects in mock mode.
    pub async fn connect_mock() -> Result<Self, McpError> {
        Self::spawn(McpClientConfig::mock()).await
    }

    /// Convenience constructor attempting live child process first, falling back to mock.
    pub async fn connect_auto() -> Result<Self, McpError> {
        let config = McpClientConfig::default();
        if config.resolve_executable().is_ok() {
            match Self::spawn(config).await {
                Ok(client) => return Ok(client),
                Err(e) => {
                    tracing::warn!(
                        "Failed to spawn live LightCraft daemon ({}), falling back to mock",
                        e
                    );
                }
            }
        }
        Self::connect_mock().await
    }

    /// Access the underlying cryptographic integrity engine.
    pub fn integrity(&self) -> Arc<IntegrityEngine> {
        self.inner.integrity.clone()
    }

    /// Spawns the child process and launches dedicated reader, writer, and stderr consumer tasks.
    async fn spawn_process_loop(
        &self,
        exe_path: PathBuf,
        mut writer_rx: mpsc::Receiver<String>,
    ) -> Result<(), McpError> {
        let mut cmd = Command::new(&exe_path);
        cmd.arg("mcp")
            .args(&self.inner.config.extra_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW (0x08000000) prevents unexpected console window popups on Windows
            cmd.creation_flags(0x08000000);
        }

        let mut child: Child = cmd.spawn().map_err(|e| McpError::SpawnFailed {
            path: exe_path.clone(),
            source: e,
        })?;

        let stdin = child.stdin.take().ok_or(McpError::StdinClosed)?;
        let stdout = child.stdout.take().ok_or(McpError::StdoutEof)?;
        let stderr = child.stderr.take().ok_or(McpError::StdoutEof)?;

        // Configure LinesCodec with 32MB max buffer for image previews
        let mut framed_writer = FramedWrite::new(stdin, LinesCodec::new());
        let mut framed_reader = FramedRead::new(
            stdout,
            LinesCodec::new_with_max_length(self.inner.config.max_line_length),
        );

        let pending = self.inner.pending.clone();
        let status = self.inner.status.clone();
        let shutdown_flag = self.inner.shutdown_flag.clone();
        let reconnect_notify = self.inner.reconnect_notify.clone();

        // Task 1: Stdin Writer Task
        tokio::spawn(async move {
            while let Some(msg) = writer_rx.recv().await {
                if let Err(e) = framed_writer.send(msg).await {
                    tracing::error!("Failed to write to child stdin: {}", e);
                    break;
                }
            }
            tracing::debug!("McpClient stdin writer task terminated");
        });

        // Task 2: Stderr Consumer Task (CRITICAL: prevents Windows pipe buffer deadlock)
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                tracing::warn!(target: "lightcraft_stderr", "{}", line);
            }
            tracing::debug!("McpClient stderr consumer task terminated");
        });

        // Task 3: Stdout Reader & Demultiplexer Task
        tokio::spawn(async move {
            while let Some(line_res) = framed_reader.next().await {
                match line_res {
                    Ok(line) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        match serde_json::from_str::<JsonRpcMessage>(trimmed) {
                            Ok(JsonRpcMessage::Response(resp)) => {
                                if let Some(id) = resp.id.as_ref().and_then(|id| id.as_u64()) {
                                    let mut p = pending.lock().await;
                                    if let Some(tx) = p.remove(&id) {
                                        let _ = tx.send(Ok(resp));
                                    } else {
                                        tracing::warn!(
                                            "Received response for untracked or timed-out ID: {}",
                                            id
                                        );
                                    }
                                }
                            }
                            Ok(JsonRpcMessage::Notification(notif)) => {
                                tracing::debug!("Received server notification: {}", notif.method);
                            }
                            Ok(JsonRpcMessage::Request(req)) => {
                                tracing::warn!(
                                    "Ignoring unexpected server-to-client request: {}",
                                    req.method
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Failed to parse incoming JSON-RPC line: {}. Raw: {}",
                                    e,
                                    trimmed
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Error reading line from LightCraft stdout: {}", e);
                        break;
                    }
                }
            }

            tracing::warn!("LightCraft stdout reached EOF");

            // Drain all pending request channels with EOF error
            let mut p = pending.lock().await;
            for (_id, sender) in p.drain() {
                let _ = sender.send(Err(McpError::StdoutEof));
            }

            *status.write().await = ConnectionStatus::Disconnected;

            if !shutdown_flag.load(Ordering::SeqCst) {
                reconnect_notify.notify_one();
            }
        });

        // Task 4: Subprocess Child Watchdog Task
        tokio::spawn(async move {
            match child.wait().await {
                Ok(status) => {
                    tracing::info!("LightCraft child process exited with status: {:?}", status);
                }
                Err(e) => {
                    tracing::error!("Error waiting on LightCraft child process: {}", e);
                }
            }
        });

        Ok(())
    }

    /// Perform the two-stage MCP initialize handshake.
    async fn perform_handshake(&self) -> Result<(), McpError> {
        *self.inner.status.write().await = ConnectionStatus::Handshaking;

        let init_params = InitializeParams {
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            capabilities: Default::default(),
            client_info: crate::mcp::protocol::ClientInfo {
                name: "lightcraft-tui".to_string(),
                version: "0.1.0".to_string(),
            },
        };

        // 1. Send "initialize" request
        let resp: InitializeResult = self
            .send_raw_request("initialize", Some(init_params))
            .await?;

        if resp.protocol_version != MCP_PROTOCOL_VERSION {
            tracing::warn!(
                "MCP protocol version mismatch: expected {}, got {}",
                MCP_PROTOCOL_VERSION,
                resp.protocol_version
            );
        }

        *self.inner.server_info.write().await = Some(resp.server_info);

        // 2. Send "notifications/initialized"
        let notif = JsonRpcNotification::new("notifications/initialized", None::<()>);
        let notif_line = serde_json::to_string(&notif)?;
        self.inner
            .writer_tx
            .send(notif_line)
            .await
            .map_err(|_| McpError::StdinClosed)?;

        *self.inner.status.write().await = ConnectionStatus::Connected;
        tracing::info!("MCP handshake completed successfully. Server connected.");

        Ok(())
    }

    /// Supervised reconnection loop with exponential backoff.
    pub async fn start_supervisor_loop(self) {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            while !inner.shutdown_flag.load(Ordering::SeqCst) {
                inner.reconnect_notify.notified().await;

                if inner.shutdown_flag.load(Ordering::SeqCst) || !inner.config.auto_reconnect {
                    break;
                }

                tracing::warn!("MCP client disconnected; initiating reconnect loop...");
                let mut attempts = 0;
                let mut backoff = inner.config.reconnect_backoff;

                while attempts < inner.config.max_reconnect_attempts {
                    if inner.shutdown_flag.load(Ordering::SeqCst) {
                        return;
                    }

                    attempts += 1;
                    tracing::info!(
                        "Reconnection attempt {}/{}",
                        attempts,
                        inner.config.max_reconnect_attempts
                    );
                    tokio::time::sleep(backoff).await;

                    // Re-spawn process and run handshake
                    let client = McpClient {
                        inner: inner.clone(),
                    };
                    if let Ok(exe_path) = inner.config.resolve_executable() {
                        let (_tx, rx) = mpsc::channel(256);
                        if client.spawn_process_loop(exe_path, rx).await.is_ok() {
                            if client.perform_handshake().await.is_ok() {
                                tracing::info!(
                                    "Successfully reconnected to LightCraft MCP server!"
                                );
                                break;
                            }
                        }
                    }

                    backoff = std::cmp::min(backoff * 2, Duration::from_secs(10));
                }
            }
        });
    }

    /// Dispatches an MCP tool call with strongly-typed parameter and return structures.
    pub async fn call_tool<P: Serialize, R: DeserializeOwned>(
        &self,
        name: &str,
        arguments: P,
    ) -> Result<R, McpError> {
        let args_val = serde_json::to_value(arguments)?;
        let result: ToolCallResult = self.call_tool_raw(name, args_val).await?;

        if result.is_error {
            let msg = result
                .text_content()
                .unwrap_or("Unknown tool execution error")
                .to_string();
            return Err(McpError::ToolError {
                tool: name.to_string(),
                message: msg,
            });
        }

        // 1. If structuredContent is present, deserialize from it
        if let Some(ref structured) = result.structured_content {
            if let Ok(val) = serde_json::from_value::<R>(structured.clone()) {
                return Ok(val);
            }
        }

        // 2. Try deserializing from primary text content
        if let Some(text) = result.text_content() {
            if let Ok(val) = serde_json::from_str::<R>(text) {
                return Ok(val);
            }
        }

        // 3. Try deserializing from the full ToolCallResult representation
        let val = serde_json::to_value(&result)?;
        serde_json::from_value(val).map_err(McpError::from)
    }

    /// Dispatches an MCP tool returning raw `ToolCallResult`.
    pub async fn call_tool_raw(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolCallResult, McpError> {
        self.preflight_validate_tool(name, &arguments)?;

        let tool_params = ToolCallParams {
            name: name.to_string(),
            arguments: Some(arguments),
        };

        self.send_raw_request("tools/call", Some(tool_params)).await
    }

    /// Pre-flight safety check executed before any tool is dispatched to the server.
    /// Intercepts operations that could mutate or endanger source files.
    pub fn preflight_validate_tool(
        &self,
        name: &str,
        arguments: &serde_json::Value,
    ) -> Result<(), McpError> {
        if self.inner.integrity.is_poisoned() {
            return Err(IntegrityViolationError::SessionPoisoned.into());
        }

        match name {
            "export" => {
                if let Some(path_str) = arguments.get("path").and_then(|v| v.as_str()) {
                    if !path_str.trim().is_empty() {
                        self.inner
                            .integrity
                            .validate_destination_path(std::path::Path::new(path_str))?;
                    }
                }
                let dir_str = arguments.get("dir").and_then(|v| v.as_str());
                let sub_str = arguments
                    .get("subfolder")
                    .or_else(|| arguments.get("subFolder"))
                    .or_else(|| arguments.get("sub_folder"))
                    .and_then(|v| v.as_str());

                if let Some(dir) = dir_str {
                    if !dir.trim().is_empty() {
                        let dir_path = std::path::PathBuf::from(dir);
                        let full = if let Some(sub) = sub_str {
                            if !sub.trim().is_empty() {
                                dir_path.join(sub)
                            } else {
                                dir_path
                            }
                        } else {
                            dir_path
                        };
                        self.inner.integrity.validate_destination_path(&full)?;
                    }
                } else if let Some(sub) = sub_str {
                    if !sub.trim().is_empty() {
                        self.inner
                            .integrity
                            .validate_destination_path(std::path::Path::new(sub))?;
                    }
                }
            }
            "render_photo" => {
                if let Some(path_str) = arguments.get("path").and_then(|v| v.as_str()) {
                    if !path_str.trim().is_empty() {
                        self.inner
                            .integrity
                            .validate_destination_path(std::path::Path::new(path_str))?;
                    }
                }
            }
            "import" | "cmd_library_import" => {
                if let Some(obj) = arguments.as_object() {
                    if let Some(mode_val) = obj.get("mode") {
                        if !mode_val.is_null() {
                            if let Some(mode_str) = mode_val.as_str() {
                                self.inner.integrity.validate_import_mode(Some(mode_str))?;
                            } else {
                                self.inner.integrity.poison();
                                return Err(IntegrityViolationError::ProhibitedImportMode {
                                    mode: mode_val.to_string(),
                                }
                                .into());
                            }
                        } else {
                            self.inner.integrity.validate_import_mode(None)?;
                        }
                    } else {
                        self.inner.integrity.validate_import_mode(None)?;
                    }
                } else {
                    self.inner.integrity.validate_import_mode(None)?;
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Validates destination paths in ExportParams to ensure no output is written to source directories.
    pub fn validate_export_params(&self, params: &ExportParams) -> Result<(), McpError> {
        if let Some(ref path_str) = params.path {
            if !path_str.trim().is_empty() {
                self.inner
                    .integrity
                    .validate_destination_path(std::path::Path::new(path_str))?;
            }
        }
        if let Some(ref dir_str) = params.dir {
            if !dir_str.trim().is_empty() {
                let dir_path = std::path::PathBuf::from(dir_str);
                let full = if let Some(ref sub) = params.subfolder {
                    if !sub.trim().is_empty() {
                        dir_path.join(sub)
                    } else {
                        dir_path
                    }
                } else {
                    dir_path
                };
                self.inner.integrity.validate_destination_path(&full)?;
            }
        } else if let Some(ref sub) = params.subfolder {
            if !sub.trim().is_empty() {
                self.inner
                    .integrity
                    .validate_destination_path(std::path::Path::new(sub))?;
            }
        }
        Ok(())
    }

    /// Validates destination path in RenderPhotoParams.
    pub fn validate_render_photo_params(&self, params: &RenderPhotoParams) -> Result<(), McpError> {
        if let Some(ref path_str) = params.path {
            if !path_str.trim().is_empty() {
                self.inner
                    .integrity
                    .validate_destination_path(std::path::Path::new(path_str))?;
            }
        }
        Ok(())
    }

    /// Dispatches a low-level JSON-RPC request and awaits its matching response.
    pub async fn send_raw_request<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: Option<P>,
    ) -> Result<R, McpError> {
        let id = self.inner.next_request_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest::new(id, method, params);
        let req_line = serde_json::to_string(&req)?;

        let (resp_tx, resp_rx) = oneshot::channel();

        // Register in-flight request before sending
        {
            let mut pending = self.inner.pending.lock().await;
            pending.insert(id, resp_tx);
        }

        // Send line across channel to writer task
        if self.inner.writer_tx.send(req_line).await.is_err() {
            let mut pending = self.inner.pending.lock().await;
            pending.remove(&id);
            return Err(McpError::StdinClosed);
        }

        // Await response bounded by timeout
        let timeout_duration = self.inner.config.request_timeout;
        let response_res = tokio::time::timeout(timeout_duration, resp_rx)
            .await
            .map_err(|_| {
                tokio::spawn({
                    let pending = self.inner.pending.clone();
                    async move {
                        let mut p = pending.lock().await;
                        p.remove(&id);
                    }
                });
                McpError::Timeout {
                    id,
                    timeout: timeout_duration,
                }
            })?;

        let rpc_response = response_res.map_err(|_| McpError::ResponseChannelDropped { id })??;

        if let Some(err) = rpc_response.error {
            return Err(McpError::JsonRpcError {
                code: err.code,
                message: err.message,
                data: err.data,
            });
        }

        if let Some(result_val) = rpc_response.result {
            serde_json::from_value(result_val).map_err(McpError::from)
        } else {
            Err(McpError::ProtocolViolation(
                "Response contained neither result nor error".to_string(),
            ))
        }
    }

    /// Check if connection is currently active and healthy.
    pub async fn is_alive(&self) -> bool {
        *self.inner.status.read().await == ConnectionStatus::Connected
    }

    /// Returns server metadata obtained during handshake.
    pub async fn server_info(&self) -> Option<ServerInfo> {
        self.inner.server_info.read().await.clone()
    }

    /// Gracefully shut down the client and child process.
    pub async fn shutdown(&self) -> Result<(), McpError> {
        self.inner.shutdown_flag.store(true, Ordering::SeqCst);
        *self.inner.status.write().await = ConnectionStatus::ShuttingDown;

        // Drain pending channels
        let mut pending = self.inner.pending.lock().await;
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err(McpError::StdinClosed));
        }

        *self.inner.status.write().await = ConnectionStatus::Disconnected;
        tracing::info!("MCP client shut down cleanly");
        Ok(())
    }

    // ========================================================================
    // High-Level Photographic Tool Dispatch API
    // ========================================================================

    /// Import files or folders non-destructively (`mode: "add"`).
    pub async fn import(&self, paths: Vec<String>) -> Result<ImportResponse, McpError> {
        let params = ImportParams::new_add(paths);
        let raw: ImportRawResult = self.call_tool("import", params).await?;
        Ok(raw.into())
    }

    /// Query photos from catalog.
    pub async fn query_photos(
        &self,
        params: QueryPhotosParams,
    ) -> Result<Vec<PhotoInfo>, McpError> {
        let res: QueryPhotosResult = self.call_tool("query_photos", params).await?;
        Ok(res.photos)
    }

    /// Retrieve full develop settings for a photo.
    pub async fn get_develop(&self, id: Option<i64>) -> Result<DevelopSettings, McpError> {
        let params = GetDevelopParams {
            id,
            photo_id: id.map(|i| format!("p_{:04}", i)),
        };
        self.call_tool("get_develop", params).await
    }

    /// Update develop settings or sliders.
    pub async fn set_develop(
        &self,
        params: SetDevelopParams,
    ) -> Result<SetDevelopResult, McpError> {
        self.call_tool("set_develop", params).await
    }

    /// Apply a preset with intensity scaling.
    pub async fn apply_preset(
        &self,
        preset: &str,
        amount: Option<f64>,
        ids: Option<Vec<i64>>,
    ) -> Result<(), McpError> {
        let params = ApplyPresetParams {
            preset: preset.to_string(),
            amount,
            ids,
            photo_id: None,
        };
        let _: serde_json::Value = self.call_tool("apply_preset", params).await?;
        Ok(())
    }

    /// Create a virtual copy of a photo.
    pub async fn create_virtual_copy(&self, id: i64, name: Option<&str>) -> Result<i64, McpError> {
        let params = VirtualCopyParams {
            ids: Some(vec![id]),
            photo_id: Some(format!("p_{:04}", id)),
            name: name.map(String::from),
        };
        let raw: VirtualCopyRawResult = self.call_tool("cmd_photo_virtualCopy", params).await?;
        raw.primary_id().ok_or_else(|| McpError::ToolError {
            tool: "cmd_photo_virtualCopy".to_string(),
            message: "No virtual copy ID returned".to_string(),
        })
    }

    /// Render a preview for display in terminal UI.
    pub async fn render_preview(
        &self,
        id: Option<i64>,
        max_side: Option<u32>,
        format: Option<ImageFormat>,
    ) -> Result<RenderPreviewResult, McpError> {
        let params = RenderPreviewParams {
            id,
            photo_id: id.map(|i| format!("p_{:04}", i)),
            max_side,
            format,
        };
        let raw: ToolCallResult = self
            .call_tool_raw("render_preview", serde_json::to_value(params)?)
            .await?;

        let (data_base64, mime) = raw.image_content().unwrap_or(("", "image/jpeg"));
        let fmt = if mime.contains("png") {
            ImageFormat::Png
        } else {
            ImageFormat::Jpeg
        };

        let meta: RenderMeta = raw
            .text_content()
            .and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or(RenderMeta {
                id,
                width: max_side.unwrap_or(1024),
                height: max_side.unwrap_or(768),
            });

        Ok(RenderPreviewResult {
            id: meta.id.or(id),
            width: meta.width,
            height: meta.height,
            format: fmt,
            data_base64: data_base64.to_string(),
        })
    }

    /// Render and/or save a photo.
    pub async fn render_photo(
        &self,
        params: RenderPhotoParams,
    ) -> Result<RenderPhotoResult, McpError> {
        self.validate_render_photo_params(&params)?;

        let raw: ToolCallResult = self
            .call_tool_raw("render_photo", serde_json::to_value(&params)?)
            .await?;
        let (data_base64, mime) = raw.image_content().unwrap_or(("", "image/jpeg"));
        let fmt = if mime.contains("png") {
            ImageFormat::Png
        } else {
            ImageFormat::Jpeg
        };
        Ok(RenderPhotoResult {
            id: params.id,
            width: params.size.unwrap_or(1920),
            height: params.size.unwrap_or(1080),
            format: fmt,
            data_base64: data_base64.to_string(),
            saved_path: params.path,
        })
    }

    /// Export photo(s) to disk.
    pub async fn export(&self, params: ExportParams) -> Result<ExportResult, McpError> {
        self.validate_export_params(&params)?;
        self.call_tool("export", params).await
    }

    /// Enumerate all available native presets.
    pub async fn list_presets(&self) -> Result<Vec<McpPresetInfo>, McpError> {
        self.call_tool("cmd_presets_list", serde_json::json!({}))
            .await
    }

    /// Enumerate all color profiles.
    pub async fn list_profiles(&self) -> Result<Vec<McpProfileInfo>, McpError> {
        self.call_tool("cmd_profiles_list", serde_json::json!({}))
            .await
    }

    /// Enumerate all develop slider definitions.
    pub async fn list_controls(&self) -> Result<Vec<DevelopControlInfo>, McpError> {
        let res: serde_json::Value = self
            .call_tool("list_controls", serde_json::json!({}))
            .await?;
        if let Some(controls_array) = res.get("controls").and_then(|c| c.as_array()) {
            serde_json::from_value(serde_json::Value::Array(controls_array.clone()))
                .map_err(McpError::from)
        } else if let Ok(controls) = serde_json::from_value::<Vec<DevelopControlInfo>>(res.clone())
        {
            Ok(controls)
        } else {
            Ok(Vec::new())
        }
    }

    /// List all discovered MCP tools from the server.
    pub async fn list_tools(&self) -> Result<Vec<String>, McpError> {
        let res: serde_json::Value = self.send_raw_request("tools/list", None::<()>).await?;
        if let Some(tools_array) = res.get("tools").and_then(|t| t.as_array()) {
            let names = tools_array
                .iter()
                .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect();
            Ok(names)
        } else {
            Ok(Vec::new())
        }
    }

    // ========================================================================
    // In-Memory Offline Mock Transport
    // ========================================================================

    async fn start_mock_transport(
        &self,
        mut writer_rx: mpsc::Receiver<String>,
    ) -> Result<(), McpError> {
        let pending = self.inner.pending.clone();
        let status = self.inner.status.clone();

        tokio::spawn(async move {
            while let Some(line) = writer_rx.recv().await {
                if let Ok(JsonRpcMessage::Request(req)) =
                    serde_json::from_str::<JsonRpcMessage>(&line)
                {
                    let req_id = match req.id {
                        JsonRpcId::Number(n) => n as u64,
                        _ => 0,
                    };

                    let response = match req.method.as_str() {
                        "initialize" => JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: Some(req.id),
                            result: Some(serde_json::json!({
                                "protocolVersion": "2024-11-05",
                                "capabilities": {
                                    "resources": { "listChanged": false, "subscribe": false },
                                    "tools": { "listChanged": false }
                                },
                                "serverInfo": {
                                    "name": "lightcraft",
                                    "title": "LightCraft Mock Server",
                                    "version": "0.8.0"
                                }
                            })),
                            error: None,
                        },
                        "tools/list" => JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: Some(req.id),
                            result: Some(serde_json::json!({
                                "tools": [
                                    { "name": "import", "description": "Import files" },
                                    { "name": "query_photos", "description": "Query catalog" },
                                    { "name": "get_develop", "description": "Get develop settings" },
                                    { "name": "set_develop", "description": "Set develop settings" },
                                    { "name": "apply_preset", "description": "Apply preset" },
                                    { "name": "cmd_photo_virtualCopy", "description": "Create virtual copy" },
                                    { "name": "render_preview", "description": "Render image preview" },
                                    { "name": "render_photo", "description": "Render photo" },
                                    { "name": "export", "description": "Export variants" },
                                    { "name": "cmd_presets_list", "description": "List native presets" },
                                    { "name": "cmd_profiles_list", "description": "List color profiles" },
                                    { "name": "list_controls", "description": "List develop controls" }
                                ]
                            })),
                            error: None,
                        },
                        "tools/call" => {
                            let tool_name = req
                                .params
                                .as_ref()
                                .and_then(|p| p.get("name"))
                                .and_then(|n| n.as_str())
                                .unwrap_or("");

                            let result_data = match tool_name {
                                "import" => serde_json::json!({
                                    "isError": false,
                                    "structuredContent": {
                                        "duplicates": [],
                                        "failed": [],
                                        "files": 5,
                                        "imported": [1, 2, 3, 4, 5],
                                        "scanned": 5,
                                        "sidecars": 0
                                    }
                                }),
                                "query_photos" => serde_json::json!({
                                    "isError": false,
                                    "structuredContent": {
                                        "photos": [
                                            { "id": 1, "fileName": "IMG_0588.CR3", "width": 6000, "height": 4000, "edited": false, "format": "CR3", "kind": "raw", "flag": "none", "rating": 0, "title": "" },
                                            { "id": 2, "fileName": "IMG_0589.CR3", "width": 6000, "height": 4000, "edited": false, "format": "CR3", "kind": "raw", "flag": "none", "rating": 0, "title": "" },
                                            { "id": 3, "fileName": "IMG_0590.CR3", "width": 6000, "height": 4000, "edited": false, "format": "CR3", "kind": "raw", "flag": "none", "rating": 0, "title": "" },
                                            { "id": 4, "fileName": "IMG_0969.CR3", "width": 6000, "height": 4000, "edited": false, "format": "CR3", "kind": "raw", "flag": "none", "rating": 0, "title": "" },
                                            { "id": 5, "fileName": "IMG_0981.CR3", "width": 6000, "height": 4000, "edited": false, "format": "CR3", "kind": "raw", "flag": "none", "rating": 0, "title": "" }
                                        ],
                                        "total": 5
                                    }
                                }),
                                "get_develop" => serde_json::json!({
                                    "isError": false,
                                    "structuredContent": {
                                        "treatment": "color",
                                        "process": 1,
                                        "version": 1,
                                        "orientation": "normal",
                                        "profile": { "id": "adobe.color", "amount": 100.0 },
                                        "wb": { "mode": "as_shot", "temp": 5500.0, "tint": 0.0 },
                                        "light": { "exposure": 0.0, "contrast": 0.0, "highlights": 0.0, "shadows": 0.0, "whites": 0.0, "blacks": 0.0 },
                                        "color": { "saturation": 0.0, "vibrance": 0.0 },
                                        "effects": { "texture": 0.0, "clarity": 0.0, "dehaze": 0.0 },
                                        "curve": { "master": [], "red": [], "green": [], "blue": [], "highlights": 0.0, "lights": 0.0, "darks": 0.0, "shadows": 0.0, "refine_saturation": 0.0, "split_highlights": 0.0, "split_mid": 0.0, "split_shadows": 0.0 },
                                        "mixer": { "red": { "hue": 0.0, "sat": 0.0, "lum": 0.0 }, "orange": { "hue": 0.0, "sat": 0.0, "lum": 0.0 }, "yellow": { "hue": 0.0, "sat": 0.0, "lum": 0.0 }, "green": { "hue": 0.0, "sat": 0.0, "lum": 0.0 }, "aqua": { "hue": 0.0, "sat": 0.0, "lum": 0.0 }, "blue": { "hue": 0.0, "sat": 0.0, "lum": 0.0 }, "purple": { "hue": 0.0, "sat": 0.0, "lum": 0.0 }, "magenta": { "hue": 0.0, "sat": 0.0, "lum": 0.0 } },
                                        "bw_mix": { "red": 0.0, "orange": 0.0, "yellow": 0.0, "green": 0.0, "aqua": 0.0, "blue": 0.0, "purple": 0.0, "magenta": 0.0 },
                                        "grading": { "shadows": { "hue": 0.0, "sat": 0.0, "lum": 0.0 }, "midtones": { "hue": 0.0, "sat": 0.0, "lum": 0.0 }, "highlights": { "hue": 0.0, "sat": 0.0, "lum": 0.0 }, "global": { "hue": 0.0, "sat": 0.0, "lum": 0.0 }, "blending": 50.0, "balance": 0.0 },
                                        "detail": { "sharpen_amount": 40.0, "sharpen_radius": 1.0, "sharpen_detail": 25.0, "sharpen_masking": 0.0, "nr_luminance": 0.0, "nr_detail": 50.0, "nr_contrast": 0.0, "nr_color": 25.0, "nr_color_detail": 50.0, "nr_color_smoothness": 50.0 },
                                        "grain": { "amount": 0.0, "size": 25.0, "roughness": 50.0, "seed": 0 },
                                        "vignette": { "amount": 0.0, "midpoint": 50.0, "roundness": 0.0, "feather": 50.0, "highlights": 0.0, "style": "paint_overlay" },
                                        "optics": { "distortion": 0.0, "vignetting": 0.0, "vignetting_midpoint": 50.0, "remove_ca": true, "ca_red": 0.0, "ca_blue": 0.0, "lens_profile": true, "profile_distortion": 100.0, "profile_vignetting": 100.0, "defringe_purple_amount": 0.0, "defringe_purple_hue_lo": 30.0, "defringe_purple_hue_hi": 70.0, "defringe_green_amount": 0.0, "defringe_green_hue_lo": 40.0, "defringe_green_hue_hi": 60.0 },
                                        "geometry": { "vertical": 0.0, "horizontal": 0.0, "rotate": 0.0, "scale": 100.0, "aspect": 0.0, "offset_x": 0.0, "offset_y": 0.0, "upright": "off", "constrain_crop": false, "guides": [] },
                                        "crop": { "geometry": { "angle": 0.0, "rect": { "x0": 0.0, "y0": 0.0, "x1": 1.0, "y1": 1.0 } }, "flip_h": false, "flip_v": false },
                                        "calibration": { "shadows_tint": 0.0, "red_hue": 0.0, "red_sat": 0.0, "green_hue": 0.0, "green_sat": 0.0, "blue_hue": 0.0, "blue_sat": 0.0 }
                                    }
                                }),
                                "set_develop" => serde_json::json!({
                                    "isError": false,
                                    "structuredContent": { "ok": true, "controls": [] }
                                }),
                                "apply_preset" => serde_json::json!({
                                    "isError": false,
                                    "content": [{ "type": "text", "text": "ok" }]
                                }),
                                "cmd_photo_virtualCopy" => serde_json::json!({
                                    "isError": false,
                                    "structuredContent": { "ids": [101], "virtualCopyId": "101" }
                                }),
                                "render_preview" => serde_json::json!({
                                    "isError": false,
                                    "content": [
                                        {
                                            "type": "image",
                                            "mimeType": "image/jpeg",
                                            "data": "/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////wgALCAABAAEBAREA/8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABPxA="
                                        },
                                        {
                                            "type": "text",
                                            "text": "{\"height\":100,\"id\":1,\"width\":100}"
                                        }
                                    ]
                                }),
                                "render_photo" => serde_json::json!({
                                    "isError": false,
                                    "content": [
                                        {
                                            "type": "image",
                                            "mimeType": "image/jpeg",
                                            "data": "/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////wgALCAABAAEBAREA/8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABPxA="
                                        }
                                    ]
                                }),
                                "export" => serde_json::json!({
                                    "isError": false,
                                    "structuredContent": {
                                        "bytes": 1024,
                                        "height": 1080,
                                        "width": 1920,
                                        "path": "test_export.jpg",
                                        "format": "jpeg",
                                        "conflictPolicy": "unique",
                                        "success": true,
                                        "files": []
                                    }
                                }),
                                "cmd_presets_list" => serde_json::json!({
                                    "isError": false,
                                    "structuredContent": [
                                        { "id": "lc.crisp-landscape", "name": "Crisp Landscape", "group": "Landscape", "builtin": true, "favorite": false },
                                        { "id": "lc.cinematic", "name": "Cinematic", "group": "Film", "builtin": true, "favorite": false },
                                        { "id": "lc.bw-high-contrast", "name": "B&W High Contrast", "group": "B&W", "builtin": true, "favorite": false },
                                        { "id": "lc.teal-orange", "name": "Teal & Orange", "group": "Color", "builtin": true, "favorite": true },
                                        { "id": "lc.golden-hour", "name": "Golden Hour", "group": "Landscape", "builtin": true, "favorite": false }
                                    ]
                                }),
                                "cmd_profiles_list" => serde_json::json!({
                                    "isError": false,
                                    "structuredContent": [
                                        { "id": "adobe.color", "name": "Adobe Color", "group": "Adobe Raw", "favorite": true },
                                        { "id": "adobe.standard", "name": "Adobe Standard", "group": "Adobe Raw", "favorite": false }
                                    ]
                                }),
                                "list_controls" => serde_json::json!({
                                    "isError": false,
                                    "structuredContent": {
                                        "controls": [
                                            { "id": "light.exposure", "label": "Exposure", "section": "light", "min": -5.0, "max": 5.0, "default": 0.0, "value": 0.0, "step": 0.05, "decimals": 2 },
                                            { "id": "light.contrast", "label": "Contrast", "section": "light", "min": -100.0, "max": 100.0, "default": 0.0, "value": 0.0, "step": 1.0, "decimals": 0 },
                                            { "id": "wb.temp", "label": "Temperature", "section": "wb", "min": 2000.0, "max": 50000.0, "default": 5500.0, "value": 5500.0, "step": 50.0, "decimals": 0 }
                                        ]
                                    }
                                }),
                                _ => serde_json::json!({
                                    "isError": true,
                                    "content": [{ "type": "text", "text": format!("Unknown mock tool: {}", tool_name) }]
                                }),
                            };

                            JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: Some(req.id),
                                result: Some(result_data),
                                error: None,
                            }
                        }
                        _ => JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: Some(req.id),
                            result: None,
                            error: Some(JsonRpcError {
                                code: error_codes::METHOD_NOT_FOUND,
                                message: format!("Method not found: {}", req.method),
                                data: None,
                            }),
                        },
                    };

                    let mut p = pending.lock().await;
                    if let Some(sender) = p.remove(&req_id) {
                        let _ = sender.send(Ok(response));
                    }
                }
            }
        });

        *status.write().await = ConnectionStatus::Connected;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_client_lifecycle_and_handshake() {
        let client = McpClient::connect_mock().await.expect("Mock spawn failed");
        assert!(client.is_alive().await);

        let info = client.server_info().await.expect("Server info missing");
        assert_eq!(info.name, "lightcraft");
        assert_eq!(info.version, "0.8.0");

        let tools = client.list_tools().await.expect("Tools list failed");
        assert!(tools.contains(&"import".to_string()));
        assert!(tools.contains(&"export".to_string()));
        assert!(tools.contains(&"render_preview".to_string()));

        client.shutdown().await.expect("Shutdown failed");
        assert!(!client.is_alive().await);
    }

    #[tokio::test]
    async fn test_mock_client_tool_calls() {
        let client = McpClient::connect_mock().await.expect("Mock spawn failed");

        // 1. query_photos
        let photos = client
            .query_photos(QueryPhotosParams::default())
            .await
            .expect("query_photos failed");
        assert_eq!(photos.len(), 5);
        assert_eq!(photos[0].file_name, "IMG_0588.CR3");

        // 2. list_presets
        let presets = client.list_presets().await.expect("list_presets failed");
        assert!(presets.len() >= 5);
        assert_eq!(presets[0].id, "lc.crisp-landscape");

        // 3. render_preview
        let preview = client
            .render_preview(Some(1), Some(1024), Some(ImageFormat::Jpeg))
            .await
            .expect("render_preview failed");
        assert!(!preview.data_base64.is_empty());
        let bytes = preview.decode_bytes().expect("Base64 decode failed");
        assert!(!bytes.is_empty());

        // 4. export
        let exp = client
            .export(ExportParams {
                format: Some(ExportFormat::Jpeg),
                ..Default::default()
            })
            .await
            .expect("export failed");
        assert_eq!(exp.bytes, 1024);
    }
}
