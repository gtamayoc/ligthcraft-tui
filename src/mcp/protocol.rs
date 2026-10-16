//! JSON-RPC 2.0 and MCP protocol message specifications.
//!
//! Conforms to Model Context Protocol (MCP) version "2024-11-05" over newline-delimited stdio.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Standard JSON-RPC 2.0 version string.
pub const JSONRPC_VERSION: &str = "2.0";

/// Current MCP protocol version supported by LightCraft 0.8.0.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// JSON-RPC 2.0 Request ID, supporting integers, strings, or null.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(i64),
    String(String),
    Null,
}

impl JsonRpcId {
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            JsonRpcId::Number(n) if *n >= 0 => Some(*n as u64),
            JsonRpcId::String(s) => s.parse::<u64>().ok(),
            _ => None,
        }
    }
}

impl From<i64> for JsonRpcId {
    fn from(val: i64) -> Self {
        JsonRpcId::Number(val)
    }
}

impl From<i32> for JsonRpcId {
    fn from(val: i32) -> Self {
        JsonRpcId::Number(val as i64)
    }
}

impl From<u32> for JsonRpcId {
    fn from(val: u32) -> Self {
        JsonRpcId::Number(val as i64)
    }
}

impl From<u64> for JsonRpcId {
    fn from(val: u64) -> Self {
        JsonRpcId::Number(val as i64)
    }
}

impl From<usize> for JsonRpcId {
    fn from(val: usize) -> Self {
        JsonRpcId::Number(val as i64)
    }
}

impl From<&str> for JsonRpcId {
    fn from(val: &str) -> Self {
        JsonRpcId::String(val.to_string())
    }
}

impl From<String> for JsonRpcId {
    fn from(val: String) -> Self {
        JsonRpcId::String(val)
    }
}

/// JSON-RPC 2.0 Request object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest<P = serde_json::Value> {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<P>,
}

impl<P> JsonRpcRequest<P> {
    pub fn new(id: impl Into<JsonRpcId>, method: impl Into<String>, params: Option<P>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: id.into(),
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 Notification object (no ID, expects no response).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcNotification<P = serde_json::Value> {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<P>,
}

impl<P> JsonRpcNotification<P> {
    pub fn new(method: impl Into<String>, params: Option<P>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            method: method.into(),
            params,
        }
    }
}

/// Standard JSON-RPC 2.0 Error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for JsonRpcError {}

/// Standard JSON-RPC 2.0 error codes.
pub mod error_codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
}

/// JSON-RPC 2.0 Response object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcResponse<R = serde_json::Value> {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<JsonRpcId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<R>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl<R> JsonRpcResponse<R> {
    /// Helper to convert the response into a standard Result.
    pub fn into_result(self) -> Result<R, JsonRpcError> {
        if let Some(err) = self.error {
            Err(err)
        } else if let Some(res) = self.result {
            Ok(res)
        } else {
            Err(JsonRpcError {
                code: error_codes::INTERNAL_ERROR,
                message: "Empty JSON-RPC response (no result or error)".to_string(),
                data: None,
            })
        }
    }
}

/// Incoming JSON-RPC 2.0 message envelope for multiplexed stdout decoding.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Notification(JsonRpcNotification),
    Response(JsonRpcResponse),
}

impl Serialize for JsonRpcMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            JsonRpcMessage::Request(r) => r.serialize(serializer),
            JsonRpcMessage::Notification(n) => n.serialize(serializer),
            JsonRpcMessage::Response(resp) => resp.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for JsonRpcMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let val = serde_json::Value::deserialize(deserializer)?;
        if val.get("method").is_some() {
            if val.get("id").is_some() && !val["id"].is_null() {
                serde_json::from_value(val)
                    .map(JsonRpcMessage::Request)
                    .map_err(serde::de::Error::custom)
            } else {
                serde_json::from_value(val)
                    .map(JsonRpcMessage::Notification)
                    .map_err(serde::de::Error::custom)
            }
        } else {
            serde_json::from_value(val)
                .map(JsonRpcMessage::Response)
                .map_err(serde::de::Error::custom)
        }
    }
}

// ============================================================================
// MCP Handshake Schemas (Method: "initialize")
// ============================================================================

/// Parameters sent by client in the "initialize" request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: ClientInfo,
}

impl Default for InitializeParams {
    fn default() -> Self {
        Self {
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            capabilities: ClientCapabilities::default(),
            client_info: ClientInfo {
                name: "lightcraft-tui".to_string(),
                version: "0.1.0".to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roots: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

/// Server response returned from "initialize".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: ServerInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    #[serde(default)]
    pub tools: ToolsCapability,
    #[serde(default)]
    pub resources: ResourcesCapability,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCapability {
    #[serde(default)]
    pub list_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcesCapability {
    #[serde(default)]
    pub list_changed: bool,
    #[serde(default)]
    pub subscribe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    pub version: String,
}

// ============================================================================
// MCP Tool Dispatch Framing (Method: "tools/call")
// ============================================================================

/// Parameters for "tools/call" method.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
}

/// Generic MCP tool call execution result returned in `result` of JSON-RPC response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResult {
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub content: Vec<McpContentItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<serde_json::Value>,
}

/// Content payload item inside tool response content array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpContentItem {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        #[serde(rename = "mimeType")]
        mime_type: String,
        data: String,
    },
    #[serde(rename = "resource")]
    Resource { resource: serde_json::Value },
}

impl ToolCallResult {
    /// Extracts primary text message if available.
    pub fn text_content(&self) -> Option<&str> {
        self.content.iter().find_map(|item| match item {
            McpContentItem::Text { text } => Some(text.as_str()),
            _ => None,
        })
    }

    /// Extracts base64 image data and mime type if available.
    pub fn image_content(&self) -> Option<(&str, &str)> {
        self.content.iter().find_map(|item| match item {
            McpContentItem::Image { mime_type, data } => Some((data.as_str(), mime_type.as_str())),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsonrpc_request_serialization() {
        let req = JsonRpcRequest::new(1, "tools/list", None::<()>);
        let serialized = serde_json::to_string(&req).unwrap();
        assert!(serialized.contains("\"jsonrpc\":\"2.0\""));
        assert!(serialized.contains("\"id\":1"));
        assert!(serialized.contains("\"method\":\"tools/list\""));
    }

    #[test]
    fn test_jsonrpc_response_deserialization() {
        let json_str = r#"{"jsonrpc":"2.0","id":1,"result":{"status":"ok"}}"#;
        let msg: JsonRpcMessage = serde_json::from_str(json_str).unwrap();
        match msg {
            JsonRpcMessage::Response(resp) => {
                assert_eq!(resp.id, Some(JsonRpcId::Number(1)));
                assert_eq!(resp.result.unwrap()["status"], "ok");
            }
            _ => panic!("Expected Response variant"),
        }
    }

    #[test]
    fn test_tool_call_result_text_and_image() {
        let res = ToolCallResult {
            is_error: false,
            content: vec![
                McpContentItem::Text {
                    text: "hello world".to_string(),
                },
                McpContentItem::Image {
                    mime_type: "image/jpeg".to_string(),
                    data: "base64data".to_string(),
                },
            ],
            structured_content: None,
        };
        assert_eq!(res.text_content(), Some("hello world"));
        assert_eq!(res.image_content(), Some(("base64data", "image/jpeg")));
    }
}
