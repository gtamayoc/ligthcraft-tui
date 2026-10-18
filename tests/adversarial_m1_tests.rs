#![allow(dead_code)]

//! Tier 2 Adversarial Stress Test Suite for Milestone M1
//! Empirically tests JSON-RPC 2.0 stdio client, protocol parser, and error resilience.

mod common;

#[path = "../src/error.rs"]
pub mod error;
#[path = "../src/mcp/mod.rs"]
pub mod mcp;

use std::path::PathBuf;
use std::time::Duration;

use error::McpError;
use mcp::client::{ClientMode, McpClient, McpClientConfig};
use mcp::protocol::{
    error_codes, JsonRpcError, JsonRpcId, JsonRpcMessage, JsonRpcResponse, ToolCallResult,
};
use serde_json::{json, Value};

/// Helper to ensure the adversarial test server executable is built in target/debug
fn ensure_adversarial_server_built() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let exe_path = manifest_dir
        .join("target")
        .join("debug")
        .join("adversarial_server.exe");

    if !exe_path.exists() {
        let src_path = manifest_dir
            .join("tests")
            .join("adversarial_helper")
            .join("main.rs");
        let status = std::process::Command::new("rustc")
            .arg(src_path)
            .arg("-o")
            .arg(&exe_path)
            .status()
            .expect("Failed to execute rustc to build adversarial mock server");
        assert!(status.success(), "rustc compilation failed");
    }
    exe_path
}

/// Helper to create client config pointing to adversarial child process with a specific scenario
fn make_adversarial_config(scenario: &str, timeout_secs: u64) -> McpClientConfig {
    let exe = ensure_adversarial_server_built();
    McpClientConfig {
        executable_path: Some(exe),
        extra_args: vec!["--scenario".to_string(), scenario.to_string()],
        mode: ClientMode::Live,
        request_timeout: Duration::from_secs(timeout_secs),
        auto_reconnect: false, // disable auto reconnect for isolated test determinism
        max_reconnect_attempts: 0,
        reconnect_backoff: Duration::from_millis(50),
        max_line_length: 32 * 1024 * 1024,
        source_directories: vec![mcp::integrity::find_draw_dir()],
    }
}

// ============================================================================
// 1. MALFORMED JSON RESPONSES
// ============================================================================

#[test]
fn test_adv_01_protocol_parser_syntax_errors() {
    let invalid_json_samples = [
        "",
        "   ",
        "<<<NOT_JSON_AT_ALL>>>",
        r#"{"jsonrpc": "2.0", incomplete"#,
        r#"{"jsonrpc": "2.0", "id": 1, "result": { "unclosed": "brace" "#,
        "12345",
        "[1, 2, 3]",
        "null",
        "true",
    ];

    for &sample in &invalid_json_samples {
        let parse_result = serde_json::from_str::<JsonRpcMessage>(sample);
        assert!(
            parse_result.is_err(),
            "Parser should fail gracefully on malformed JSON: {:?}",
            sample
        );
    }
}

#[test]
fn test_adv_02_protocol_parser_missing_or_invalid_fields() {
    // Missing 'jsonrpc' field
    let missing_jsonrpc = r#"{"id":1,"result":{"status":"ok"}}"#;
    let res = serde_json::from_str::<JsonRpcMessage>(missing_jsonrpc);
    assert!(
        res.is_err(),
        "Response missing jsonrpc version must be rejected"
    );

    // Empty response (neither result nor error)
    let empty_resp = JsonRpcResponse::<Value> {
        jsonrpc: "2.0".to_string(),
        id: Some(JsonRpcId::Number(1)),
        result: None,
        error: None,
    };
    let into_res = empty_resp.into_result();
    assert!(into_res.is_err(), "Empty response must yield an error");
    if let Err(e) = into_res {
        assert_eq!(e.code, error_codes::INTERNAL_ERROR);
        assert!(e.message.contains("Empty JSON-RPC response"));
    }

    // Both result and error present -> error must take precedence
    let dual_resp = JsonRpcResponse::<Value> {
        jsonrpc: "2.0".to_string(),
        id: Some(JsonRpcId::Number(2)),
        result: Some(json!({"data": 123})),
        error: Some(JsonRpcError {
            code: -32000,
            message: "Conflict error".to_string(),
            data: None,
        }),
    };
    let dual_res = dual_resp.into_result();
    assert!(dual_res.is_err(), "Error must take precedence over result");
    assert_eq!(dual_res.unwrap_err().code, -32000);
}

#[tokio::test]
async fn test_adv_03_stdio_transport_malformed_syntax_resilience() {
    // Spawns live adversarial server that sends:
    // 1. Raw non-JSON bytes
    // 2. Truncated JSON
    // 3. Response missing ID
    // 4. Response with bad version
    // 5. Normal valid response for request 3
    let config = make_adversarial_config("malformed_syntax", 10);
    let client = McpClient::spawn(config)
        .await
        .expect("Handshake should succeed");
    assert!(client.is_alive().await);

    // Send request 2 - the server will emit corrupted lines followed by bad version response
    // Wait, request 2 receives the bad version response (which has id 2)
    let req2_res: Result<Value, McpError> =
        client.send_raw_request("query_photos", None::<()>).await;
    assert!(
        req2_res.is_ok(),
        "Client parser should accept string jsonrpc version without panicking"
    );
    let val2 = req2_res.unwrap();
    assert_eq!(val2["status"], "bad_version_accepted");

    // Send request 3 - verifies client connection is completely healthy and operational
    let req3_res: Result<Value, McpError> =
        client.send_raw_request("get_develop", None::<()>).await;
    assert!(
        req3_res.is_ok(),
        "Client should remain operational after encountering malformed lines"
    );
    let val3 = req3_res.unwrap();
    assert_eq!(val3["status"], "recovered_ok");

    let _ = client.shutdown().await;
}

// ============================================================================
// 2. SERVER ERROR PAYLOADS
// ============================================================================

#[test]
fn test_adv_04_protocol_parser_server_error_structures() {
    // Standard error with data
    let json_str = r#"{
        "jsonrpc": "2.0",
        "id": 42,
        "error": {
            "code": -32601,
            "message": "Method not found",
            "data": { "method": "nonexistent", "hint": "check spelling" }
        }
    }"#;

    let msg: JsonRpcMessage = serde_json::from_str(json_str).expect("Valid JSON-RPC error");
    match msg {
        JsonRpcMessage::Response(resp) => {
            assert_eq!(resp.id, Some(JsonRpcId::Number(42)));
            let err = resp.error.expect("Expected error object");
            assert_eq!(err.code, -32601);
            assert_eq!(err.message, "Method not found");
            assert!(err.data.is_some());
            assert_eq!(err.data.unwrap()["hint"], "check spelling");
        }
        _ => panic!("Expected Response variant"),
    }
}

#[tokio::test]
async fn test_adv_05_stdio_transport_server_error_payloads() {
    let config = make_adversarial_config("server_errors", 10);
    let client = McpClient::spawn(config)
        .await
        .expect("Handshake should succeed");

    // 1. Request 2: Method Not Found (-32601)
    let res1: Result<Value, McpError> = client
        .send_raw_request("unregistered_tool", None::<()>)
        .await;
    match res1 {
        Err(McpError::JsonRpcError {
            code,
            message,
            data,
        }) => {
            assert_eq!(code, -32601);
            assert_eq!(message, "Method not found");
            assert!(data.is_some());
        }
        other => panic!("Expected McpError::JsonRpcError, got: {:?}", other),
    }

    // 2. Request 3: Tool Execution Failure (isError: true)
    let res2: Result<Value, McpError> = client.call_tool("heavy_operation", json!({})).await;
    match res2 {
        Err(McpError::ToolError { tool, message }) => {
            assert_eq!(tool, "heavy_operation");
            assert!(message.contains("Out of memory"));
        }
        other => panic!("Expected McpError::ToolError, got: {:?}", other),
    }

    // 3. Request 4: Custom Error (-32000)
    let res3: Result<Value, McpError> = client
        .send_raw_request("pipeline_execute", None::<()>)
        .await;
    match res3 {
        Err(McpError::JsonRpcError { code, message, .. }) => {
            assert_eq!(code, -32000);
            assert!(message.contains("Custom LightCraft pipeline error"));
        }
        other => panic!("Expected McpError::JsonRpcError, got: {:?}", other),
    }

    let _ = client.shutdown().await;
}

// ============================================================================
// 3. OUT-OF-ORDER MATCHING ACROSS CONCURRENT REQUESTS
// ============================================================================

#[tokio::test]
async fn test_adv_06_stdio_transport_out_of_order_concurrent() {
    let config = make_adversarial_config("out_of_order", 15);
    let client = McpClient::spawn(config)
        .await
        .expect("Handshake should succeed");

    // Launch 10 concurrent requests
    let mut handles = Vec::new();
    for i in 0..10 {
        let client_clone = client.clone();
        let handle = tokio::spawn(async move {
            let res: Result<Value, McpError> = client_clone
                .send_raw_request("concurrent_op", Some(json!({ "client_index": i })))
                .await;
            (i, res)
        });
        handles.push(handle);
    }

    // Await all 10 concurrent requests
    for handle in handles {
        let (index, res) = handle.await.expect("Task join failed");
        assert!(
            res.is_ok(),
            "Request index {} failed: {:?}",
            index,
            res.err()
        );
        let val = res.unwrap();
        // Server echoed back `{"order_received": id}`
        assert!(val.get("order_received").is_some());
    }

    let _ = client.shutdown().await;
}

// ============================================================================
// 4. SERVER PROCESS TERMINATION / CLOSED PIPE
// ============================================================================

#[tokio::test]
async fn test_adv_07_stdio_transport_server_crash_in_flight() {
    let start_time = std::time::Instant::now();
    // Configure timeout to 10s, but crash should unblock immediately
    let config = make_adversarial_config("server_crash", 10);
    let client = McpClient::spawn(config)
        .await
        .expect("Handshake should succeed");

    // Request 2 causes immediate exit(42) in child process
    let res: Result<Value, McpError> = client.send_raw_request("trigger_crash", None::<()>).await;

    let elapsed = start_time.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "Client must unblock immediately on EOF rather than waiting for timeout. Elapsed: {:?}",
        elapsed
    );

    // Verify error is StdoutEof
    match res {
        Err(McpError::StdoutEof) => {
            // Expected
        }
        Err(e) => {
            // Also acceptable if channel dropped
            assert!(
                matches!(
                    e,
                    McpError::StdoutEof
                        | McpError::ResponseChannelDropped { .. }
                        | McpError::StdinClosed
                ),
                "Expected EOF or pipe closure error, got: {:?}",
                e
            );
        }
        Ok(_) => panic!("Request should not succeed when server crashed"),
    }

    // Verify connection status reflects disconnection
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !client.is_alive().await,
        "Client must report disconnected after server crash"
    );

    // Attempting a subsequent request should fail cleanly without hanging
    let post_crash_res: Result<Value, McpError> =
        client.send_raw_request("post_crash_op", None::<()>).await;
    assert!(
        post_crash_res.is_err(),
        "Post-crash request must fail cleanly"
    );
}

#[tokio::test]
async fn test_adv_08_handshake_failure_handling() {
    // 1. Handshake malformed JSON
    let bad_json_config = make_adversarial_config("handshake_malformed", 5);
    let spawn1 = McpClient::spawn(bad_json_config).await;
    assert!(
        spawn1.is_err(),
        "Handshake with corrupt JSON must fail spawn"
    );

    // 2. Handshake server error code
    let error_config = make_adversarial_config("handshake_error", 5);
    let spawn2 = McpClient::spawn(error_config).await;
    assert!(
        spawn2.is_err(),
        "Handshake with server error must fail spawn"
    );
}

// ============================================================================
// 5. LARGE PAYLOADS (5 MB & OVERSIZED)
// ============================================================================

#[tokio::test]
async fn test_adv_09_stdio_transport_large_payload_5mb() {
    let config = make_adversarial_config("large_payload", 15);
    let client = McpClient::spawn(config)
        .await
        .expect("Handshake should succeed");

    let raw: Result<ToolCallResult, McpError> =
        client.call_tool_raw("render_preview", json!({})).await;
    assert!(
        raw.is_ok(),
        "5 MB payload should be read without error: {:?}",
        raw.err()
    );

    let tool_res = raw.unwrap();
    let (data, mime) = tool_res
        .image_content()
        .expect("Image content should be present");
    assert_eq!(mime, "image/jpeg");
    assert_eq!(
        data.len(),
        5 * 1024 * 1024,
        "Image base64 data must match 5 MB"
    );

    let _ = client.shutdown().await;
}

#[tokio::test]
async fn test_adv_10_stdio_transport_oversized_payload_rejection() {
    // Server sends 33 MB line, exceeding default 32 MB line limit
    let config = make_adversarial_config("oversized_payload", 10);
    let client = McpClient::spawn(config)
        .await
        .expect("Handshake should succeed");

    let res: Result<Value, McpError> = client.send_raw_request("get_giant_data", None::<()>).await;
    // Client should encounter line length exceeded, drop the stream and unblock
    assert!(
        res.is_err(),
        "Oversized payload exceeding line buffer must fail gracefully"
    );

    // Client must not crash or hang
    let _ = client.shutdown().await;
}

// ============================================================================
// 6. WINDOWS STDERR BUFFER SATURATION DEADLOCK RESILIENCE
// ============================================================================

#[tokio::test]
async fn test_adv_11_stdio_transport_stderr_flood_no_deadlock() {
    // Server floods 100 KB to stderr before answering (exceeding 4 KB Windows buffer)
    let config = make_adversarial_config("stderr_flood", 10);
    let client = McpClient::spawn(config)
        .await
        .expect("Handshake should succeed");

    let res: Result<Value, McpError> = client.send_raw_request("flood_op", None::<()>).await;
    assert!(
        res.is_ok(),
        "Client must not deadlock when child process floods stderr: {:?}",
        res.err()
    );
    assert_eq!(res.unwrap()["flood_passed"], true);

    let _ = client.shutdown().await;
}
