#![allow(dead_code)]

//! Tier 1 Integration Tests: LightCraft MCP Protocol, Handshake, Schema Discovery & Immutability.
//! Covers M1 features: F-MCP-01, F-MCP-02, F-SEC-01, F-RNF-04.

mod common;

#[path = "../src/error.rs"]
pub mod error;
#[path = "../src/mcp/mod.rs"]
pub mod mcp;

use common::mock_mcp::{JsonRpcError, JsonRpcRequest};
use common::TestFixture;
use mcp::client::McpClient;
use mcp::integrity::{find_draw_dir, IntegrityEngine};
use mcp::tools::*;
use serde_json::json;

#[test]
fn test_mcp_01_handshake_lifecycle() {
    let fixture = TestFixture::new("mcp_handshake");
    let server = &fixture.mock_server;

    // Send initialize request
    let init_req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: "initialize".to_string(),
        params: Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "lightcraft-tui",
                "version": "0.1.0"
            }
        })),
    };

    let resp = server.handle_request(init_req);
    assert_eq!(resp.id, Some(json!(1)), "Response ID must match request ID");
    assert!(resp.error.is_none(), "Handshake must not return error");

    let res = resp
        .result
        .expect("Expected result payload in handshake response");
    assert_eq!(
        res["protocolVersion"], "2024-11-05",
        "Protocol version must match LightCraft 2024-11-05"
    );
    assert_eq!(
        res["serverInfo"]["name"], "lightcraft",
        "Server name must be lightcraft"
    );
    assert_eq!(
        res["serverInfo"]["version"], "0.8.0",
        "Server version must match 0.8.0"
    );

    // Send notifications/initialized
    let notify_req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: None,
        method: "notifications/initialized".to_string(),
        params: None,
    };
    let notify_resp = server.handle_request(notify_req);
    assert!(notify_resp.error.is_none());

    // Verify source DRAW files remained 100% immutable
    fixture.assert_integrity();
}

#[test]
fn test_mcp_02_tool_discovery_and_registration() {
    let fixture = TestFixture::new("tool_discovery");
    let server = &fixture.mock_server;

    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(2)),
        method: "tools/list".to_string(),
        params: None,
    };

    let resp = server.handle_request(req);
    assert_eq!(resp.id, Some(json!(2)));
    let result = resp.result.expect("Expected tools/list result");
    let tools = result["tools"].as_array().expect("Tools must be an array");

    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

    // Assert all critical photographic tools are present
    let mandatory_tools = [
        "set_develop",
        "get_develop",
        "list_controls",
        "apply_preset",
        "cmd_presets_list",
        "render_preview",
        "export",
        "cmd_photo_virtualCopy",
        "query_photos",
        "import",
    ];

    for &tool in &mandatory_tools {
        assert!(
            tool_names.contains(&tool),
            "Mandatory LightCraft MCP tool '{}' was not discovered in catalog",
            tool
        );
    }

    fixture.assert_integrity();
}

#[test]
fn test_mcp_03_develop_controls_schema_validation() {
    let fixture = TestFixture::new("develop_controls");
    let server = &fixture.mock_server;

    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(3)),
        method: "list_controls".to_string(),
        params: None,
    };

    let resp = server.handle_request(req);
    assert_eq!(resp.id, Some(json!(3)));
    let result = resp.result.expect("Expected list_controls result");
    let controls = result["controls"]
        .as_array()
        .expect("Controls must be an array");

    // Verify Exposure control bounds
    let exposure = controls
        .iter()
        .find(|c| c["id"] == "light.exposure")
        .expect("light.exposure control must exist");
    assert_eq!(exposure["min"], -5.0);
    assert_eq!(exposure["max"], 5.0);
    assert_eq!(exposure["default"], 0.0);

    // Verify Contrast control bounds
    let contrast = controls
        .iter()
        .find(|c| c["id"] == "light.contrast")
        .expect("light.contrast control must exist");
    assert_eq!(contrast["min"], -100.0);
    assert_eq!(contrast["max"], 100.0);

    // Verify White Balance temperature bounds
    let wb_temp = controls
        .iter()
        .find(|c| c["id"] == "wb.temp")
        .expect("wb.temp control must exist");
    assert_eq!(wb_temp["min"], 2000.0);
    assert_eq!(wb_temp["max"], 50000.0);

    fixture.assert_integrity();
}

#[test]
fn test_mcp_04_native_presets_enumeration() {
    let fixture = TestFixture::new("native_presets");
    let server = &fixture.mock_server;

    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(4)),
        method: "cmd_presets_list".to_string(),
        params: None,
    };

    let resp = server.handle_request(req);
    assert_eq!(resp.id, Some(json!(4)));
    let presets = resp
        .result
        .expect("Expected presets result")
        .as_array()
        .expect("Presets must be an array")
        .clone();

    assert!(
        presets.len() >= 5,
        "Server must provide at least 5 built-in aesthetic presets"
    );

    let preset_ids: Vec<&str> = presets.iter().filter_map(|p| p["id"].as_str()).collect();

    assert!(preset_ids.contains(&"lc.crisp-landscape"));
    assert!(preset_ids.contains(&"lc.cinematic"));
    assert!(preset_ids.contains(&"lc.bw-high-contrast"));
    assert!(preset_ids.contains(&"lc.teal-orange"));
    assert!(preset_ids.contains(&"lc.golden-hour"));

    // Verify built-in flag on all native presets
    for p in &presets {
        assert_eq!(
            p["builtin"], true,
            "Native preset {} must have builtin: true",
            p["id"]
        );
    }

    fixture.assert_integrity();
}

#[test]
fn test_mcp_05_develop_immutability_and_state_isolation() {
    let fixture = TestFixture::new("state_isolation");
    let server = &fixture.mock_server;

    // 1. Query baseline develop settings for IMG_0588.CR3 (id: p_0588)
    let get_req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(501)),
        method: "get_develop".to_string(),
        params: Some(json!({ "photoId": "p_0588" })),
    };
    let get_resp = server.handle_request(get_req);
    let baseline = get_resp.result.unwrap()["values"].clone();
    assert_eq!(baseline["light.exposure"], 0.0);

    // 2. Create virtual copy for variant testing
    let vc_req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(502)),
        method: "cmd_photo_virtualCopy".to_string(),
        params: Some(json!({ "photoId": "p_0588" })),
    };
    let vc_resp = server.handle_request(vc_req);
    let vc_id = vc_resp.result.unwrap()["virtualCopyId"]
        .as_str()
        .unwrap()
        .to_string();

    // 3. Mutate virtual copy settings aggressively
    let set_vc_req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(503)),
        method: "set_develop".to_string(),
        params: Some(json!({
            "photoId": vc_id,
            "values": {
                "light.exposure": 2.5,
                "light.contrast": 50.0
            }
        })),
    };
    let set_vc_resp = server.handle_request(set_vc_req);
    assert!(set_vc_resp.error.is_none());

    // 4. Query parent photo baseline again to assert zero contamination
    let get_parent_req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(504)),
        method: "get_develop".to_string(),
        params: Some(json!({ "photoId": "p_0588" })),
    };
    let get_parent_resp = server.handle_request(get_parent_req);
    let parent_settings = get_parent_resp.result.unwrap()["values"].clone();
    assert_eq!(
        parent_settings["light.exposure"], 0.0,
        "Parent photo develop settings must remain completely unchanged"
    );

    // 5. Assert source CR3 files on disk maintain 100% cryptographic SHA-256 identity
    fixture.assert_integrity();
}

#[test]
fn test_mcp_06_export_schema_and_collision_policies() {
    let fixture = TestFixture::new("export_schema");
    let server = &fixture.mock_server;

    // Test valid export with unique conflict policy
    let valid_export = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(601)),
        method: "export".to_string(),
        params: Some(json!({
            "photoId": "p_0588",
            "dir": fixture.output_dir.to_str().unwrap(),
            "format": "jpeg",
            "quality": 90,
            "conflict": "unique"
        })),
    };
    let resp = server.handle_request(valid_export);
    assert!(resp.error.is_none(), "Valid export request must succeed");
    assert_eq!(resp.result.unwrap()["conflictPolicy"], "unique");

    // Test invalid conflict policy
    let invalid_export = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(602)),
        method: "export".to_string(),
        params: Some(json!({
            "photoId": "p_0588",
            "dir": fixture.output_dir.to_str().unwrap(),
            "format": "jpeg",
            "conflict": "invalid_policy"
        })),
    };
    let err_resp = server.handle_request(invalid_export);
    assert!(
        err_resp.error.is_some(),
        "Invalid conflict policy must be rejected"
    );
    assert_eq!(err_resp.error.unwrap().code, JsonRpcError::INVALID_PARAMS);

    fixture.assert_integrity();
}

#[test]
fn test_mcp_07_adversarial_error_handling() {
    let fixture = TestFixture::new("adversarial_errors");
    let server = &fixture.mock_server;

    // 1. Unregistered method -> METHOD_NOT_FOUND (-32601)
    let unknown_req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(701)),
        method: "cmd_nonexistent_action".to_string(),
        params: None,
    };
    let resp1 = server.handle_request(unknown_req);
    assert_eq!(resp1.error.unwrap().code, JsonRpcError::METHOD_NOT_FOUND);

    // 2. Corrupted JSON -> PARSE_ERROR (-32700)
    let raw_err = server.handle_raw("<<<NOT_JSON_DATA>>>");
    let resp2: common::mock_mcp::JsonRpcResponse = serde_json::from_str(&raw_err).unwrap();
    assert_eq!(resp2.error.unwrap().code, JsonRpcError::PARSE_ERROR);

    // 3. Missing mandatory parameter -> INVALID_PARAMS (-32602)
    let bad_param_req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(703)),
        method: "get_develop".to_string(),
        params: Some(json!({})), // Missing photoId
    };
    let resp3 = server.handle_request(bad_param_req);
    assert_eq!(resp3.error.unwrap().code, JsonRpcError::INVALID_PARAMS);

    fixture.assert_integrity();
}

#[tokio::test]
async fn test_mcp_08_client_async_connect_and_handshake() {
    let client = McpClient::connect_mock()
        .await
        .expect("Client mock connect failed");
    assert!(
        client.is_alive().await,
        "Client must report alive after handshake"
    );

    let server_info = client
        .server_info()
        .await
        .expect("ServerInfo must be present");
    assert_eq!(server_info.name, "lightcraft");
    assert_eq!(server_info.version, "0.8.0");

    let tools = client.list_tools().await.expect("Failed to list tools");
    assert!(tools.contains(&"import".to_string()));
    assert!(tools.contains(&"export".to_string()));
    assert!(tools.contains(&"render_preview".to_string()));
    assert!(tools.contains(&"set_develop".to_string()));
    assert!(tools.contains(&"get_develop".to_string()));

    client.shutdown().await.expect("Shutdown failed");
    assert!(!client.is_alive().await);
}

#[tokio::test]
async fn test_mcp_09_client_import_and_catalog_query() {
    let client = McpClient::connect_mock()
        .await
        .expect("Client mock connect failed");

    // Test import
    let import_res = client
        .import(vec!["DRAW/IMG_0588.CR3".to_string()])
        .await
        .expect("Import failed");
    assert_eq!(import_res.imported_count, 5);
    assert_eq!(import_res.photos, vec![1, 2, 3, 4, 5]);

    // Test query_photos
    let photos = client
        .query_photos(QueryPhotosParams::default())
        .await
        .expect("query_photos failed");
    assert_eq!(photos.len(), 5);
    assert_eq!(photos[0].file_name, "IMG_0588.CR3");
    assert_eq!(photos[0].format, "CR3");

    client.shutdown().await.expect("Shutdown failed");
}

#[tokio::test]
async fn test_mcp_10_client_develop_and_virtual_copy() {
    let client = McpClient::connect_mock()
        .await
        .expect("Client mock connect failed");

    // Test get_develop
    let dev = client
        .get_develop(Some(1))
        .await
        .expect("get_develop failed");
    assert_eq!(dev.treatment, "color");
    assert_eq!(dev.light.exposure, 0.0);

    // Test create_virtual_copy
    let copy_id = client
        .create_virtual_copy(1, Some("Variant Cinema"))
        .await
        .expect("create_virtual_copy failed");
    assert_eq!(copy_id, 101);

    // Test set_develop
    let mut values = std::collections::HashMap::new();
    values.insert("light.exposure".to_string(), 1.5);
    let set_res = client
        .set_develop(SetDevelopParams {
            id: Some(copy_id),
            values: Some(values),
            ..Default::default()
        })
        .await
        .expect("set_develop failed");
    assert!(set_res.ok);

    // Test apply_preset
    client
        .apply_preset("lc.cinematic", Some(100.0), Some(vec![copy_id]))
        .await
        .expect("apply_preset failed");

    client.shutdown().await.expect("Shutdown failed");
}

#[tokio::test]
async fn test_mcp_11_client_render_preview_and_export() {
    let client = McpClient::connect_mock()
        .await
        .expect("Client mock connect failed");

    // Test render_preview
    let preview = client
        .render_preview(Some(1), Some(1024), Some(ImageFormat::Jpeg))
        .await
        .expect("render_preview failed");
    assert_eq!(preview.format, ImageFormat::Jpeg);
    assert!(!preview.data_base64.is_empty());
    let raw_bytes = preview.decode_bytes().expect("Base64 decode failed");
    assert!(
        !raw_bytes.is_empty(),
        "Decoded image bytes must not be empty"
    );

    // Test export
    let exp = client
        .export(ExportParams {
            id: Some(1),
            format: Some(ExportFormat::Jpeg),
            quality: Some(90),
            conflict: Some(ConflictPolicy::Unique),
            ..Default::default()
        })
        .await
        .expect("export failed");
    assert_eq!(exp.bytes, 1024);
    assert!(exp.success.unwrap_or(true));

    client.shutdown().await.expect("Shutdown failed");
}

#[test]
fn test_mcp_12_integrity_engine_raw_immutability_guard() {
    let draw_dir = find_draw_dir();
    assert!(draw_dir.exists(), "DRAW folder must exist");

    let engine = IntegrityEngine::new(vec![draw_dir.clone()], true);

    // Pre-flight manifest capture
    let manifest = engine
        .capture_preflight()
        .expect("capture_preflight failed");
    assert!(
        !manifest.records.is_empty(),
        "Manifest must capture present CR3 files"
    );

    // Immutability verification
    engine
        .verify_manifest(&manifest)
        .expect("verify_manifest must pass");

    // Safe mode add passes
    assert!(engine.validate_import_mode(None).is_ok());
    assert!(engine.validate_import_mode(Some("add")).is_ok());

    // Prohibited move mode rejected
    let engine_for_prohibited = IntegrityEngine::new(vec![draw_dir.clone()], true);
    assert!(engine_for_prohibited
        .validate_import_mode(Some("move"))
        .is_err());
    assert!(engine_for_prohibited.is_poisoned());

    // Prohibited export into DRAW rejected
    let engine_for_dest = IntegrityEngine::new(vec![draw_dir.clone()], true);
    let inside_draw = draw_dir.join("polluted_output.jpg");
    assert!(engine_for_dest
        .validate_destination_path(&inside_draw)
        .is_err());
    assert!(engine_for_dest.is_poisoned());
}
