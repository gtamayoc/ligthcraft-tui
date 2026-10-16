#![allow(dead_code, unused_imports, unused_variables)]

//! High-fidelity in-memory Mock JSON-RPC 2.0 LightCraft MCP server.
//! Provides deterministic, standalone offline testing of the MCP client without an external daemon.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Represents a standard JSON-RPC 2.0 request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// Represents a standard JSON-RPC 2.0 response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 Error object conforming to spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;

    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: Self::METHOD_NOT_FOUND,
            message: format!("Method not found: {}", method),
            data: None,
        }
    }

    pub fn invalid_params(msg: &str) -> Self {
        Self {
            code: Self::INVALID_PARAMS,
            message: msg.to_string(),
            data: None,
        }
    }
}

/// Simulated state of the LightCraft photography engine.
#[derive(Debug, Clone)]
pub struct LightCraftEngineState {
    pub photos: HashMap<String, Value>,
    pub develop_settings: HashMap<String, HashMap<String, f64>>,
    pub virtual_copies: HashMap<String, String>, // copy_id -> parent_id
    pub history_log: Vec<String>,
    pub simulated_fault: Option<SimulatedFault>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimulatedFault {
    Disconnect,
    MalformedResponse,
    Timeout,
    ErrorResponse(i32, String),
}

/// Thread-safe in-memory mock MCP server.
#[derive(Clone)]
pub struct MockMcpServer {
    state: Arc<Mutex<LightCraftEngineState>>,
    request_history: Arc<Mutex<Vec<JsonRpcRequest>>>,
}

impl Default for MockMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl MockMcpServer {
    pub fn new() -> Self {
        let mut photos = HashMap::new();
        let mut develop_settings = HashMap::new();

        // Seed default photo entries matching ./DRAW/
        let cr3_files = [
            ("IMG_0588.CR3", "p_0588", 26527598),
            ("IMG_0589.CR3", "p_0589", 28369647),
            ("IMG_0590.CR3", "p_0590", 27704456),
            ("IMG_0969.CR3", "p_0969", 26614589),
            ("IMG_0981.CR3", "p_0981", 29087028),
        ];

        for (filename, id, size) in cr3_files {
            photos.insert(
                id.to_string(),
                json!({
                    "id": id,
                    "filename": filename,
                    "path": format!("C:\\discolocal\\PROYECTOS\\PYTHON\\LIGTHCRAFT\\0001\\DRAW\\{}", filename),
                    "size": size,
                    "width": 6000,
                    "height": 4000,
                    "format": "CR3",
                    "orientation": 1
                }),
            );

            let mut dev = HashMap::new();
            dev.insert("light.exposure".to_string(), 0.0);
            dev.insert("light.contrast".to_string(), 0.0);
            dev.insert("light.highlights".to_string(), 0.0);
            dev.insert("light.shadows".to_string(), 0.0);
            dev.insert("light.whites".to_string(), 0.0);
            dev.insert("light.blacks".to_string(), 0.0);
            dev.insert("wb.temp".to_string(), 5500.0);
            dev.insert("wb.tint".to_string(), 0.0);
            develop_settings.insert(id.to_string(), dev);
        }

        Self {
            state: Arc::new(Mutex::new(LightCraftEngineState {
                photos,
                develop_settings,
                virtual_copies: HashMap::new(),
                history_log: Vec::new(),
                simulated_fault: None,
            })),
            request_history: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Set simulated fault condition for negative/edge-case testing.
    pub fn set_fault(&self, fault: Option<SimulatedFault>) {
        let mut st = self.state.lock().unwrap();
        st.simulated_fault = fault;
    }

    /// Retrieve list of all requests dispatched to this mock server.
    pub fn get_requests(&self) -> Vec<JsonRpcRequest> {
        self.request_history.lock().unwrap().clone()
    }

    /// Clear recorded request history.
    pub fn clear_history(&self) {
        self.request_history.lock().unwrap().clear();
    }

    /// Process a raw JSON-RPC 2.0 string request and return the string response.
    pub fn handle_raw(&self, line: &str) -> String {
        let req: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                let err_resp = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: None,
                    result: None,
                    error: Some(JsonRpcError {
                        code: JsonRpcError::PARSE_ERROR,
                        message: format!("Parse error: {}", e),
                        data: None,
                    }),
                };
                return serde_json::to_string(&err_resp).unwrap();
            }
        };

        let resp = self.handle_request(req);
        serde_json::to_string(&resp).unwrap()
    }

    /// Handle structured JSON-RPC 2.0 request according to LightCraft MCP specifications.
    pub fn handle_request(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        self.request_history.lock().unwrap().push(req.clone());

        // Check for active fault injection
        {
            let st = self.state.lock().unwrap();
            if let Some(ref fault) = st.simulated_fault {
                match fault {
                    SimulatedFault::Disconnect => {
                        return JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: req.id,
                            result: None,
                            error: Some(JsonRpcError {
                                code: -32000,
                                message: "Connection reset by peer (simulated fault)".to_string(),
                                data: None,
                            }),
                        };
                    }
                    SimulatedFault::ErrorResponse(code, msg) => {
                        return JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: req.id,
                            result: None,
                            error: Some(JsonRpcError {
                                code: *code,
                                message: msg.clone(),
                                data: None,
                            }),
                        };
                    }
                    SimulatedFault::MalformedResponse => {
                        // Return response missing jsonrpc field or corrupt
                        return JsonRpcResponse {
                            jsonrpc: "INVALID_PROTOCOL".to_string(),
                            id: None,
                            result: None,
                            error: None,
                        };
                    }
                    SimulatedFault::Timeout => {
                        // Handled in async callers
                    }
                }
            }
        }

        let method = req.method.as_str();
        match method {
            "initialize" => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: req.id,
                result: Some(json!({
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {
                        "name": "lightcraft",
                        "title": "LightCraft",
                        "version": "0.8.0"
                    },
                    "capabilities": {
                        "tools": {
                            "listChanged": false
                        }
                    }
                })),
                error: None,
            },

            "notifications/initialized" | "ping" => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: req.id,
                result: Some(json!({})),
                error: None,
            },

            "tools/list" => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: req.id,
                result: Some(json!({
                    "tools": [
                        {
                            "name": "set_develop",
                            "description": "Adjust develop parameters (exposure, contrast, WB, etc.) for a photo",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "photoId": { "type": "string" },
                                    "values": { "type": "object" }
                                },
                                "required": ["photoId", "values"]
                            }
                        },
                        {
                            "name": "get_develop",
                            "description": "Retrieve current develop parameters for a photo",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "photoId": { "type": "string" }
                                },
                                "required": ["photoId"]
                            }
                        },
                        {
                            "name": "list_controls",
                            "description": "List all available develop controls, categories, and range constraints",
                            "inputSchema": { "type": "object" }
                        },
                        {
                            "name": "cmd_presets_list",
                            "description": "List all native LightCraft presets (built-in and user)",
                            "inputSchema": { "type": "object" }
                        },
                        {
                            "name": "apply_preset",
                            "description": "Apply a named preset to a photo with intensity scaling",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "photoId": { "type": "string" },
                                    "preset": { "type": "string" },
                                    "amount": { "type": "number", "minimum": 0, "maximum": 200 }
                                },
                                "required": ["photoId", "preset"]
                            }
                        },
                        {
                            "name": "render_preview",
                            "description": "Render a photographic preview at specified maximum dimension",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "photoId": { "type": "string" },
                                    "format": { "type": "string", "enum": ["jpeg", "png", "webp"] },
                                    "maxSide": { "type": "integer", "minimum": 64, "maximum": 8192 }
                                },
                                "required": ["photoId"]
                            }
                        },
                        {
                            "name": "export",
                            "description": "Render and export photo to disk with custom dimensions, format, and collision rules",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "photoId": { "type": "string" },
                                    "dir": { "type": "string" },
                                    "format": { "type": "string", "enum": ["jpeg", "png", "tiff", "webp", "avif"] },
                                    "quality": { "type": "integer", "minimum": 1, "maximum": 100 },
                                    "conflict": { "type": "string", "enum": ["unique", "overwrite", "skip"] },
                                    "naming": { "type": "string" }
                                },
                                "required": ["photoId", "dir", "format"]
                            }
                        },
                        {
                            "name": "cmd_photo_virtualCopy",
                            "description": "Create a virtual copy of a photo for non-destructive variation branches",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "photoId": { "type": "string" },
                                    "name": { "type": "string" }
                                },
                                "required": ["photoId"]
                            }
                        },
                        {
                            "name": "query_photos",
                            "description": "Query photographic catalog records",
                            "inputSchema": { "type": "object" }
                        },
                        {
                            "name": "import",
                            "description": "Import files or directories into catalog non-destructively",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "paths": { "type": "array", "items": { "type": "string" } },
                                    "mode": { "type": "string", "enum": ["add", "copy", "move"] }
                                },
                                "required": ["paths"]
                            }
                        }
                    ]
                })),
                error: None,
            },

            "list_controls" => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: req.id,
                result: Some(json!({
                    "controls": [
                        { "id": "light.exposure", "category": "Basic", "min": -5.0, "max": 5.0, "default": 0.0, "step": 0.05 },
                        { "id": "light.contrast", "category": "Basic", "min": -100.0, "max": 100.0, "default": 0.0, "step": 1.0 },
                        { "id": "light.highlights", "category": "Basic", "min": -100.0, "max": 100.0, "default": 0.0, "step": 1.0 },
                        { "id": "light.shadows", "category": "Basic", "min": -100.0, "max": 100.0, "default": 0.0, "step": 1.0 },
                        { "id": "light.whites", "category": "Basic", "min": -100.0, "max": 100.0, "default": 0.0, "step": 1.0 },
                        { "id": "light.blacks", "category": "Basic", "min": -100.0, "max": 100.0, "default": 0.0, "step": 1.0 },
                        { "id": "wb.temp", "category": "WhiteBalance", "min": 2000.0, "max": 50000.0, "default": 5500.0, "step": 50.0 },
                        { "id": "wb.tint", "category": "WhiteBalance", "min": -150.0, "max": 150.0, "default": 0.0, "step": 1.0 },
                        { "id": "effects.clarity", "category": "Presence", "min": -100.0, "max": 100.0, "default": 0.0, "step": 1.0 },
                        { "id": "effects.dehaze", "category": "Presence", "min": -100.0, "max": 100.0, "default": 0.0, "step": 1.0 },
                        { "id": "color.vibrance", "category": "Presence", "min": -100.0, "max": 100.0, "default": 0.0, "step": 1.0 },
                        { "id": "color.saturation", "category": "Presence", "min": -100.0, "max": 100.0, "default": 0.0, "step": 1.0 }
                    ]
                })),
                error: None,
            },

            "cmd_presets_list" => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: req.id,
                result: Some(json!([
                    { "id": "lc.crisp-landscape", "name": "Crisp Landscape", "group": "Landscape", "builtin": true },
                    { "id": "lc.cinematic", "name": "Cinematic", "group": "Film", "builtin": true },
                    { "id": "lc.bw-high-contrast", "name": "B&W High Contrast", "group": "B&W", "builtin": true },
                    { "id": "lc.bw-soft", "name": "B&W Soft", "group": "B&W", "builtin": true },
                    { "id": "lc.teal-orange", "name": "Teal & Orange", "group": "Color", "builtin": true },
                    { "id": "lc.golden-hour", "name": "Golden Hour", "group": "Color", "builtin": true },
                    { "id": "lc.warm-glow", "name": "Warm Glow", "group": "Portrait", "builtin": true },
                    { "id": "lc.moody", "name": "Moody Shadows", "group": "Film", "builtin": true },
                    { "id": "lc.faded-matte", "name": "Faded Matte", "group": "Film", "builtin": true }
                ])),
                error: None,
            },

            "query_photos" => {
                let st = self.state.lock().unwrap();
                let photos: Vec<Value> = st.photos.values().cloned().collect();
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: req.id,
                    result: Some(json!({ "photos": photos, "total": photos.len() })),
                    error: None,
                }
            }

            "get_develop" => {
                let photo_id = req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("photoId"))
                    .and_then(|v| v.as_str());

                match photo_id {
                    Some(id) => {
                        let st = self.state.lock().unwrap();
                        if let Some(dev) = st.develop_settings.get(id) {
                            JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: req.id,
                                result: Some(json!({ "values": dev })),
                                error: None,
                            }
                        } else {
                            JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: req.id,
                                result: None,
                                error: Some(JsonRpcError::invalid_params(&format!(
                                    "Photo ID not found: {}",
                                    id
                                ))),
                            }
                        }
                    }
                    None => JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError::invalid_params("Missing photoId")),
                    },
                }
            }

            "set_develop" => {
                let params = req.params.as_ref();
                let photo_id = params
                    .and_then(|p| p.get("photoId"))
                    .and_then(|v| v.as_str());
                let values = params
                    .and_then(|p| p.get("values"))
                    .and_then(|v| v.as_object());

                if let (Some(id), Some(vals)) = (photo_id, values) {
                    let mut st = self.state.lock().unwrap();
                    if let Some(dev) = st.develop_settings.get_mut(id) {
                        for (k, v) in vals {
                            if let Some(num) = v.as_f64() {
                                dev.insert(k.clone(), num);
                            }
                        }
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: req.id,
                            result: Some(json!({ "success": true })),
                            error: None,
                        }
                    } else {
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: req.id,
                            result: None,
                            error: Some(JsonRpcError::invalid_params(&format!(
                                "Photo ID not found: {}",
                                id
                            ))),
                        }
                    }
                } else {
                    JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError::invalid_params(
                            "set_develop requires photoId and values object",
                        )),
                    }
                }
            }

            "apply_preset" => {
                let params = req.params.as_ref();
                let photo_id = params
                    .and_then(|p| p.get("photoId"))
                    .and_then(|v| v.as_str());
                let preset = params
                    .and_then(|p| p.get("preset"))
                    .and_then(|v| v.as_str());
                let amount = params
                    .and_then(|p| p.get("amount"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(100.0);

                if let (Some(id), Some(preset_id)) = (photo_id, preset) {
                    let mut st = self.state.lock().unwrap();
                    if st.photos.contains_key(id) {
                        let factor = amount / 100.0;
                        if let Some(dev) = st.develop_settings.get_mut(id) {
                            if preset_id == "lc.cinematic" {
                                dev.insert("light.contrast".to_string(), 25.0 * factor);
                                dev.insert("light.shadows".to_string(), 15.0 * factor);
                                dev.insert("wb.temp".to_string(), 5200.0);
                            } else if preset_id == "lc.bw-high-contrast" {
                                dev.insert("color.saturation".to_string(), -100.0);
                                dev.insert("light.contrast".to_string(), 45.0 * factor);
                            } else if preset_id == "lc.teal-orange" {
                                dev.insert("light.contrast".to_string(), 20.0 * factor);
                                dev.insert("color.vibrance".to_string(), 30.0 * factor);
                            }
                        }
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: req.id,
                            result: Some(json!({ "success": true, "preset": preset_id, "amount": amount })),
                            error: None,
                        }
                    } else {
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: req.id,
                            result: None,
                            error: Some(JsonRpcError::invalid_params(&format!(
                                "Photo ID not found: {}",
                                id
                            ))),
                        }
                    }
                } else {
                    JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError::invalid_params(
                            "apply_preset requires photoId and preset",
                        )),
                    }
                }
            }

            "render_preview" => {
                let photo_id = req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("photoId"))
                    .and_then(|v| v.as_str());
                let max_side = req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("maxSide"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(512);

                if let Some(id) = photo_id {
                    let st = self.state.lock().unwrap();
                    if st.photos.contains_key(id) {
                        // 1x1 base64 transparent PNG
                        let mock_png_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: req.id,
                            result: Some(json!({
                                "photoId": id,
                                "format": "png",
                                "width": max_side,
                                "height": (max_side as f64 * 0.6667) as u64,
                                "base64": mock_png_base64
                            })),
                            error: None,
                        }
                    } else {
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: req.id,
                            result: None,
                            error: Some(JsonRpcError::invalid_params("Photo not found")),
                        }
                    }
                } else {
                    JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError::invalid_params("Missing photoId")),
                    }
                }
            }

            "cmd_photo_virtualCopy" => {
                let photo_id = req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("photoId"))
                    .and_then(|v| v.as_str());

                if let Some(id) = photo_id {
                    let mut st = self.state.lock().unwrap();
                    if st.photos.contains_key(id) {
                        let copy_id = format!("{}_vc_{}", id, st.virtual_copies.len() + 1);
                        st.virtual_copies.insert(copy_id.clone(), id.to_string());
                        let parent_dev = st.develop_settings.get(id).cloned().unwrap_or_default();
                        st.develop_settings.insert(copy_id.clone(), parent_dev);
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: req.id,
                            result: Some(json!({ "virtualCopyId": copy_id })),
                            error: None,
                        }
                    } else {
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: req.id,
                            result: None,
                            error: Some(JsonRpcError::invalid_params("Photo not found")),
                        }
                    }
                } else {
                    JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError::invalid_params("Missing photoId")),
                    }
                }
            }

            "export" => {
                let params = req.params.as_ref();
                let photo_id = params
                    .and_then(|p| p.get("photoId"))
                    .and_then(|v| v.as_str());
                let dir = params
                    .and_then(|p| p.get("dir"))
                    .and_then(|v| v.as_str());
                let format = params
                    .and_then(|p| p.get("format"))
                    .and_then(|v| v.as_str());
                let conflict = params
                    .and_then(|p| p.get("conflict"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unique");

                if let (Some(id), Some(target_dir), Some(fmt)) = (photo_id, dir, format) {
                    let st = self.state.lock().unwrap();
                    if !st.photos.contains_key(id) {
                        return JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: req.id,
                            result: None,
                            error: Some(JsonRpcError::invalid_params("Photo not found")),
                        };
                    }

                    // Enforce conflict option check
                    if !["unique", "overwrite", "skip"].contains(&conflict) {
                        return JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: req.id,
                            result: None,
                            error: Some(JsonRpcError::invalid_params(&format!(
                                "Invalid conflict option: {}",
                                conflict
                            ))),
                        };
                    }

                    let output_file = format!("{}/{}_exported.{}", target_dir, id, fmt);
                    JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: req.id,
                        result: Some(json!({
                            "success": true,
                            "path": output_file,
                            "format": fmt,
                            "conflictPolicy": conflict
                        })),
                        error: None,
                    }
                } else {
                    JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError::invalid_params("export requires photoId, dir, and format")),
                    }
                }
            }

            unknown => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: req.id,
                result: None,
                error: Some(JsonRpcError::method_not_found(unknown)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_mcp_initialize() {
        let server = MockMcpServer::new();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "initialize".to_string(),
            params: None,
        };
        let resp = server.handle_request(req);
        assert_eq!(resp.id, Some(json!(1)));
        assert!(resp.error.is_none());
        let res = resp.result.unwrap();
        assert_eq!(res["serverInfo"]["name"], "lightcraft");
        assert_eq!(res["serverInfo"]["version"], "0.8.0");
    }

    #[test]
    fn test_mock_mcp_unknown_method() {
        let server = MockMcpServer::new();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(42)),
            method: "non_existent_method".to_string(),
            params: None,
        };
        let resp = server.handle_request(req);
        assert_eq!(resp.id, Some(json!(42)));
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, JsonRpcError::METHOD_NOT_FOUND);
    }

    #[test]
    fn test_mock_mcp_raw_json_handling() {
        let server = MockMcpServer::new();
        let raw = r#"{"jsonrpc":"2.0","id":100,"method":"cmd_presets_list"}"#;
        let resp_str = server.handle_raw(raw);
        let resp: JsonRpcResponse = serde_json::from_str(&resp_str).unwrap();
        assert_eq!(resp.id, Some(json!(100)));
        assert!(resp.result.is_some());
    }
}
