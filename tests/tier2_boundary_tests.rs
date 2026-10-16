#![allow(dead_code)]

//! Tier 2 Boundary & Adversarial Tests: Non-Destructive Safety, SHA-256 Tamper Detection,
//! Import Mode Validation, and Destination Path Sandboxing.

mod common;

#[path = "../src/error.rs"]
pub mod error;
#[path = "../src/mcp/mod.rs"]
pub mod mcp;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::tempdir;

use error::{IntegrityViolationError, McpError};
use mcp::client::McpClient;
use mcp::integrity::{find_draw_dir, verify_cr3_baseline, IntegrityEngine};
use mcp::tools::{ExportParams, ImportMode, ImportParams};

/// Helper to create a dummy test file with known contents in a directory
fn create_sample_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
    let path = dir.join(name);
    let mut file = File::create(&path).expect("failed to create sample file");
    file.write_all(content).expect("failed to write sample content");
    file.flush().expect("failed to flush sample file");
    path
}

// ============================================================================
// Group 1: Authoritative Baseline Hash Verification
// ============================================================================

#[test]
fn test_cr3_baseline_hashes_all_match() {
    let draw_dir = find_draw_dir();
    assert!(draw_dir.exists(), "DRAW directory must exist");

    let results = verify_cr3_baseline(&draw_dir).expect("Baseline verification must succeed");
    assert_eq!(results.len(), 5, "Expected 5 Canon CR3 baseline files");

    for r in results {
        assert!(
            r.is_valid,
            "File {} failed baseline hash check. Expected {}, got {}",
            r.filename, r.expected_sha256, r.actual_sha256
        );
        assert_eq!(
            r.actual_size, r.expected_size,
            "File {} size mismatch. Expected {}, got {}",
            r.filename, r.expected_size, r.actual_size
        );
    }
}

// ============================================================================
// Group 2: Tamper Detection (Bit-flip, Truncation, Timestamp modification)
// ============================================================================

#[test]
fn test_tamper_detection_single_bit_flip() {
    let temp = tempdir().expect("tempdir");
    let file_path = create_sample_file(temp.path(), "sample.bin", b"ABCDEF0123456789ImmutableSourceData");

    let engine = IntegrityEngine::new(vec![temp.path().to_path_buf()], true);
    let manifest = engine.capture_preflight().expect("capture preflight");
    assert_eq!(manifest.records.len(), 1);

    // Initial state verified
    engine.verify_manifest(&manifest).expect("clean verify");

    let canonical = file_path.canonicalize().unwrap_or(file_path.clone());
    let initial_time = manifest.records.get(&canonical).unwrap().modified_time;

    // Introduce single-bit flip at byte index 5, restoring initial timestamp to test SHA-256 detection
    {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&file_path)
            .expect("open for tamper");
        file.seek(SeekFrom::Start(5)).unwrap();
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte).unwrap();
        byte[0] ^= 0x01; // flip lowest bit
        file.seek(SeekFrom::Start(5)).unwrap();
        file.write_all(&byte).unwrap();
        file.flush().unwrap();
        file.set_modified(initial_time).expect("restore timestamp");
    }

    // verify_manifest MUST detect the bit-flip via SHA-256 mismatch
    let result = engine.verify_manifest(&manifest);
    assert!(result.is_err(), "Engine must detect single-bit flip");
    match result.unwrap_err() {
        IntegrityViolationError::HashMismatch { path, expected, actual } => {
            assert_eq!(path, file_path.canonicalize().unwrap_or(file_path));
            assert_ne!(expected, actual);
        }
        other => panic!("Expected HashMismatch, got {:?}", other),
    }
    assert!(engine.is_poisoned(), "Engine must be poisoned after tamper detection");
}

#[test]
fn test_tamper_detection_truncated_file() {
    let temp = tempdir().expect("tempdir");
    let file_path = create_sample_file(temp.path(), "sample.bin", b"012345678901234567890123456789");

    let engine = IntegrityEngine::new(vec![temp.path().to_path_buf()], true);
    let manifest = engine.capture_preflight().expect("capture preflight");

    // Truncate file from 30 bytes to 20 bytes
    {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&file_path)
            .expect("open for truncate");
        file.set_len(20).expect("truncate to 20 bytes");
    }

    // verify_manifest MUST detect truncation via size mismatch
    let result = engine.verify_manifest(&manifest);
    assert!(result.is_err(), "Engine must detect file truncation");
    match result.unwrap_err() {
        IntegrityViolationError::SizeMismatch { expected_bytes, actual_bytes, .. } => {
            assert_eq!(expected_bytes, 30);
            assert_eq!(actual_bytes, 20);
        }
        other => panic!("Expected SizeMismatch, got {:?}", other),
    }
    assert!(engine.is_poisoned(), "Engine must be poisoned after truncation");
}

#[test]
fn test_tamper_detection_timestamp_modification_stress() {
    let temp = tempdir().expect("tempdir");
    let file_path = create_sample_file(temp.path(), "sample.bin", b"ConstantContentData123456789");

    let engine = IntegrityEngine::new(vec![temp.path().to_path_buf()], true);
    let manifest = engine.capture_preflight().expect("capture preflight");

    // Retrieve initial recorded modified time
    let canonical = file_path.canonicalize().unwrap_or(file_path.clone());
    let initial_time = manifest.records.get(&canonical).unwrap().modified_time;

    // Mutate the file's LastWriteTime without changing bytes or length
    // Using std::fs::File::set_modified (Rust 1.75+)
    let past_time = initial_time - Duration::from_secs(3600); // 1 hour earlier
    {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&file_path)
            .expect("open to set modified");
        file.set_modified(past_time).expect("set_modified");
    }

    let new_meta = std::fs::metadata(&file_path).expect("metadata");
    assert_ne!(
        new_meta.modified().unwrap(),
        initial_time,
        "Timestamp was successfully changed on disk"
    );

    // Stress test: Does verify_manifest check modified_time?
    let result = engine.verify_manifest(&manifest);
    // Contract requirement: IntegrityVerifier must detect modified timestamps
    assert!(
        result.is_err(),
        "CRITICAL DEFECT: verify_manifest failed to detect modified timestamp on file '{}'!",
        file_path.display()
    );
    match result.unwrap_err() {
        IntegrityViolationError::TimestampMismatch { path, expected, actual } => {
            assert_eq!(path, canonical);
            assert_eq!(expected, initial_time);
            assert_eq!(actual, new_meta.modified().unwrap());
        }
        other => panic!("Expected TimestampMismatch, got {:?}", other),
    }
    assert!(engine.is_poisoned(), "Engine must be poisoned after timestamp tamper detection");
}

// ============================================================================
// Group 3: Import Mode Validation
// ============================================================================

#[test]
fn test_integrity_engine_import_mode_validation() {
    let draw_dir = find_draw_dir();
    let engine = IntegrityEngine::new(vec![draw_dir.clone()], true);

    // Safe mode "add"
    assert!(engine.validate_import_mode(None).is_ok());
    assert!(engine.validate_import_mode(Some("add")).is_ok());
    assert!(!engine.is_poisoned());

    // Prohibited mode "move"
    let engine_move = IntegrityEngine::new(vec![draw_dir.clone()], true);
    let res_move = engine_move.validate_import_mode(Some("move"));
    assert!(res_move.is_err(), "Mode 'move' must be rejected");
    assert!(engine_move.is_poisoned(), "Engine must be poisoned after 'move'");

    // Prohibited mode "copy"
    let engine_copy = IntegrityEngine::new(vec![draw_dir.clone()], true);
    let res_copy = engine_copy.validate_import_mode(Some("copy"));
    assert!(res_copy.is_err(), "Mode 'copy' must be rejected");
    assert!(engine_copy.is_poisoned(), "Engine must be poisoned after 'copy'");

    // Arbitrary invalid mode
    let engine_junk = IntegrityEngine::new(vec![draw_dir.clone()], true);
    assert!(engine_junk.validate_import_mode(Some("sync")).is_err());
}

#[tokio::test]
async fn test_mcp_client_import_mode_move_rejection_at_client_boundary() {
    let client = McpClient::connect_mock().await.expect("connect mock");

    // Scenario A: Client high-level import API defaults to "add"
    let paths = vec!["DRAW/IMG_0588.CR3".to_string()];
    let res = client.import(paths).await;
    assert!(res.is_ok(), "High-level import with add should succeed");

    // Scenario B: Calling call_tool with ImportMode::Move or ImportMode::Copy
    let move_params = ImportParams {
        paths: vec!["DRAW/IMG_0588.CR3".to_string()],
        mode: ImportMode::Move,
        album: None,
        destination: None,
        on_deleted: None,
        organize: None,
        rename: None,
    };

    // Contract requirement: attempts to invoke import with mode 'move' must be strictly rejected BEFORE hitting MCP server
    let call_res: Result<serde_json::Value, McpError> = client.call_tool("import", move_params).await;
    assert!(
        call_res.is_err(),
        "CRITICAL DEFECT: McpClient dispatched 'import' with mode: 'move' without pre-flight validation!"
    );

    client.shutdown().await.expect("shutdown");
}

// ============================================================================
// Group 4: Output Path Sandboxing
// ============================================================================

#[test]
fn test_destination_path_sandboxing_draw_subpaths() {
    let draw_dir = find_draw_dir();
    let engine = IntegrityEngine::new(vec![draw_dir.clone()], true);

    // 1. Direct file inside DRAW
    let target1 = draw_dir.join("exported_variant.jpg");
    assert!(engine.validate_destination_path(&target1).is_err());

    // 2. Subdirectory inside DRAW
    let target2 = draw_dir.join("exports").join("variant.jpg");
    assert!(engine.validate_destination_path(&target2).is_err());

    // 3. Dot-dot traversal attempting to escape into DRAW
    let target3 = draw_dir.parent().unwrap().join("DRAW").join("test.png");
    assert!(engine.validate_destination_path(&target3).is_err());

    // 4. Valid target outside DRAW
    let valid_target = draw_dir.parent().unwrap().join("target").join("test_exports").join("variant.jpg");
    assert!(engine.validate_destination_path(&valid_target).is_ok());
}

#[tokio::test]
async fn test_mcp_client_export_destination_into_draw_boundary() {
    let client = McpClient::connect_mock().await.expect("connect mock");
    let draw_dir = find_draw_dir();

    // Contract requirement: export destination targeting DRAW/ must be rejected or redirected
    let export_target = draw_dir.join("polluting_export.jpg");
    let export_params = ExportParams {
        id: Some(1),
        path: Some(export_target.to_string_lossy().to_string()),
        dir: Some(draw_dir.to_string_lossy().to_string()),
        format: Some(mcp::tools::ExportFormat::Jpeg),
        ..Default::default()
    };

    let export_res = client.export(export_params).await;
    assert!(
        export_res.is_err(),
        "CRITICAL DEFECT: McpClient::export allowed destination targeting DRAW/ directory without rejection or redirection!"
    );

    client.shutdown().await.expect("shutdown");
}
