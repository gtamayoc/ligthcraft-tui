//! LightCraft Model Context Protocol (MCP) subsystem.
//!
//! Provides the async JSON-RPC 2.0 stdio client, typed tool wrappers, protocol schemas,
//! and cryptographic SHA-256 integrity verifier.

pub mod client;
pub mod integrity;
pub mod protocol;
pub mod tools;

pub use client::{ClientMode, ConnectionStatus, McpClient, McpClientConfig};
pub use integrity::{
    compute_file_sha256, find_draw_dir, verify_cr3_baseline, BaselineEntry, IntegrityEngine,
    IntegritySessionGuard, SourceFileRecord, SourceManifest, VerificationResult, CR3_BASELINE,
};
pub use protocol::{
    InitializeParams, InitializeResult, JsonRpcError, JsonRpcId, JsonRpcMessage,
    JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, ServerInfo, ToolCallParams,
    ToolCallResult, MCP_PROTOCOL_VERSION,
};
pub use tools::*;
