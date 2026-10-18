//! Domain error definitions for LightCraft MCP client and TUI application.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use thiserror::Error;

/// MCP protocol, transport, and child process execution errors.
#[derive(Debug, Error)]
pub enum McpError {
    #[error("Failed to spawn LightCraft executable at '{path}': {source}")]
    SpawnFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("LightCraft process terminated unexpectedly with exit code: {exit_code:?}")]
    ProcessTerminated { exit_code: Option<i32> },

    #[error("LightCraft server stdin pipe was closed")]
    StdinClosed,

    #[error("LightCraft server stdout EOF reached")]
    StdoutEof,

    #[error("JSON-RPC error [{code}]: {message}")]
    JsonRpcError {
        code: i64,
        message: String,
        data: Option<serde_json::Value>,
    },

    #[error("MCP tool '{tool}' failed: {message}")]
    ToolError { tool: String, message: String },

    #[error("MCP request {id} timed out after {timeout:?}")]
    Timeout { id: u64, timeout: Duration },

    #[error("Response channel dropped before receiving response for request ID {id}")]
    ResponseChannelDropped { id: u64 },

    #[error("Invalid JSON-RPC protocol framing: {0}")]
    ProtocolViolation(String),

    #[error("MCP handshake failed: {0}")]
    HandshakeFailed(String),

    #[error("MCP client is disconnected; cannot dispatch request")]
    NotConnected,

    #[error("Max reconnect attempts ({max_attempts}) reached without success")]
    MaxReconnectAttemptsReached { max_attempts: usize },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("I/O error during MCP transport: {0}")]
    Io(#[from] std::io::Error),

    #[error("Integrity violation: {0}")]
    IntegrityViolation(#[from] IntegrityViolationError),
}

impl McpError {
    /// Convenience constructor for integrity violation errors.
    pub fn integrity(err: IntegrityViolationError) -> Self {
        McpError::IntegrityViolation(err)
    }
}

/// Non-destructive source safety and cryptographic integrity violations.
#[derive(Debug, Error)]
pub enum IntegrityViolationError {
    #[error("CRITICAL SAFETY VIOLATION: Source file '{path}' SHA-256 hash mismatch! Expected: {expected}, Actual: {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },

    #[error("CRITICAL SAFETY VIOLATION: Source file '{path}' size changed from {expected_bytes} to {actual_bytes} bytes")]
    SizeMismatch {
        path: PathBuf,
        expected_bytes: u64,
        actual_bytes: u64,
    },

    #[error("CRITICAL SAFETY VIOLATION: Source file '{path}' modified timestamp changed! Expected: {expected:?}, Actual: {actual:?}")]
    TimestampMismatch {
        path: PathBuf,
        expected: SystemTime,
        actual: SystemTime,
    },

    #[error("Prohibited import mode '{mode}': Only 'add' mode is allowed to ensure non-destructive safety")]
    ProhibitedImportMode { mode: String },

    #[error("Export destination '{target}' conflicts with source RAW directory '{source_dir}'; direct export into RAW folder is forbidden")]
    ExportDestinationConflict {
        target: PathBuf,
        source_dir: PathBuf,
    },

    #[error("Destination conflict: target export path '{path}' resolves inside source directory '{source_dir}'")]
    SourceDirectoryContamination { path: PathBuf, source_dir: PathBuf },

    #[error("Export destination '{target}' contains prohibited path traversal ('..')")]
    PathTraversal { target: PathBuf },

    #[error("Prohibited target extension '{extension}': Exporting to source RAW formats (.CR3) is strictly forbidden")]
    ProhibitedTargetExtension { path: PathBuf, extension: String },

    #[error("Source directory contamination detected: stray file '{stray_path}' created in source folder")]
    DirectoryContaminated { stray_path: PathBuf },

    #[error("Source file not found for integrity validation: '{0}'")]
    FileNotFound(PathBuf),

    #[error("MCP session poisoned due to previous integrity failure")]
    SessionPoisoned,

    #[error("Integrity check I/O failure: {0}")]
    Io(#[from] std::io::Error),
}

/// Serialization and deserialization errors across JSON, TOML, and Base64.
#[derive(Debug, Error)]
pub enum SerializationError {
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML deserialization error: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("TOML serialization error: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("Base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
}

/// Storage, persistence, and preset lifecycle errors.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Storage I/O error at '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Storage serialization error: {0}")]
    Serialization(#[from] SerializationError),

    #[error("Preset '{0}' was not found")]
    NotFound(String),

    #[error("Preset '{0}' already exists")]
    AlreadyExists(String),

    #[error("Invalid preset format at '{path}': {reason}")]
    InvalidFormat { path: PathBuf, reason: String },

    #[error("Permission denied for preset operation: {0}")]
    PermissionDenied(String),

    #[error("Corrupted preset file at '{path}': {reason}")]
    CorruptedFile { path: PathBuf, reason: String },

    #[error("Security violation: path traversal detected in identifier '{0}'")]
    PathTraversal(String),

    #[error("Invalid preset identifier '{0}': identifier must be non-empty (1..64 chars) containing only ASCII alphanumeric, '-', or '_'")]
    InvalidIdentifier(String),
}

impl StorageError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        StorageError::Io {
            path: path.into(),
            source,
        }
    }

    pub fn invalid_format(path: impl Into<PathBuf>, reason: impl Into<String>) -> Self {
        StorageError::InvalidFormat {
            path: path.into(),
            reason: reason.into(),
        }
    }

    pub fn corrupted(path: impl Into<PathBuf>, reason: impl Into<String>) -> Self {
        StorageError::CorruptedFile {
            path: path.into(),
            reason: reason.into(),
        }
    }
}

/// Top-level application error integrating all domain failure modes.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("MCP Subsystem Error: {0}")]
    Mcp(#[from] McpError),

    #[error("Integrity Violation: {0}")]
    Integrity(#[from] IntegrityViolationError),

    #[error("Storage Error: {0}")]
    Storage(#[from] StorageError),

    #[error("Serialization Error: {0}")]
    Serialization(#[from] SerializationError),

    #[error("Filesystem I/O Error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration Error: {0}")]
    Config(String),

    #[error("Image Processing Error: {0}")]
    Image(String),

    #[error("Batch Queue Error: {0}")]
    Queue(String),
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;
