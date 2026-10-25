#![allow(dead_code, unused_imports)]

//! Milestone 4 Challenger 1: Adversarial Streaming Pipeline & Deduplication Stress Suite
//!
//! Authoritative source: `PROJECT.md` & `tasks/prd-sd-ingest-hub.md` (US-006, FR-8, FR-9, FR-11).
//!
//! Objectives:
//! 1. Multi-megabyte streaming & memory bounding (1 byte, 1MB, 1MB+1B, 5MB, 10MB, 25MB).
//!    Verify in-flight SHA-256 matches independent computation bit-for-bit.
//! 2. Chunk size invariance (1KB, 64KB, 512KB, 1MB, 2MB).
//! 3. Single-pass verification: trace monotonic chunk-by-chunk progress, zero SD writes.
//! 4. Deduplication accuracy across all scenarios:
//!    - Scenario A: Target does not exist -> transferred.
//!    - Scenario B: Target exists with matching size and matching SHA-256 -> skipped (0 disk writes).
//!    - Scenario C: Target exists with matching size but corrupt/different hash -> transferred & overwritten.
//!    - Scenario D: Target exists with differing size -> transferred & overwritten.
//! 5. Manifest schema, field types, counts, and byte sum precision.
//! 6. Responsive cancellation during multi-megabyte transfer ensuring clean .tmp file deletion.
//! 7. Zero-byte file transfer and deduplication handling.
//! 8. Windows Toast notification formatting and non-fatal error resilience.

mod common;

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::SystemTime;

use chrono::Utc;
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use tokio::sync::mpsc;

use common::exif_gen::ExifPhotoBuilder;
use common::sd_fixture::SyntheticSdCard;

use lightcraft_tui::ingest::pipeline::{
    compute_file_sha256, compute_file_sha256_async, IngestCancellationToken, IngestManifest,
    IngestPipeline, IngestProgress, IngestSessionManifest, IngestStatus, IngestSummary,
    ManifestEntry, DEFAULT_CHUNK_SIZE,
};
use lightcraft_tui::ingest::registry::CameraProfile;
use lightcraft_tui::ingest::scanner::SdCardVolume;
use lightcraft_tui::ingest::toast::MockToastNotifier;
use lightcraft_tui::ingest::IngestError;

/// Helper: convert SyntheticSdCard to production SdCardVolume.
fn to_real_volume(sd: &SyntheticSdCard) -> SdCardVolume {
    let v = sd.to_volume();
    SdCardVolume {
        drive_letter: v.drive_letter,
        volume_label: v.volume_label,
        file_system: v.file_system,
        total_bytes: v.total_bytes,
        free_bytes: v.free_bytes,
        has_dcim: v.has_dcim,
        camera_folders: v.camera_folders,
    }
}

/// Helper: generate deterministic non-trivial pseudo-random binary payload.
fn generate_deterministic_payload(size: usize, seed: u8) -> Vec<u8> {
    let mut payload = Vec::with_capacity(size);
    let mut state = seed as u32;
    for i in 0..size {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        let byte = ((state >> 16) ^ (i as u32)) as u8;
        payload.push(byte);
    }
    payload
}

/// Helper: compute SHA-256 of raw bytes in memory.
fn compute_memory_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

// ============================================================================
// 1. Multi-Megabyte Streaming & Bit-for-Bit In-Flight SHA-256 (up to 25 MB)
// ============================================================================

#[tokio::test]
async fn test_adv_m4_streaming_scaling_multimegabyte_files() {
    let sd = SyntheticSdCard::new("CANON_EOS").expect("synthetic SD");
    let camera_folder = sd.add_camera_folder("100CANON").expect("add folder");

    // File sizes to test:
    // 1 byte, exactly 1 MB chunk, 1 MB + 1 byte, 5 MB, 10 MB, 25 MB.
    let test_cases: Vec<(&'static str, usize, u8)> = vec![
        ("TINY_1B.JPG", 1, 0x11),
        ("EXACT_1MB.CR3", 1024 * 1024, 0x22),
        ("PLUS_1B.NEF", 1024 * 1024 + 1, 0x33),
        ("STREAM_5MB.ARW", 5 * 1024 * 1024, 0x44),
        ("STREAM_10MB.CR2", 10 * 1024 * 1024, 0x55),
        ("STREAM_25MB.CR3", 25 * 1024 * 1024, 0x66),
    ];

    let mut expected_hashes: HashMap<String, (usize, String)> = HashMap::new();
    let mut total_expected_bytes = 0u64;

    for (name, size, seed) in &test_cases {
        let payload = generate_deterministic_payload(*size, *seed);
        let hash = compute_memory_sha256(&payload);
        expected_hashes.insert(name.to_string(), (*size, hash));
        total_expected_bytes += *size as u64;

        let photo_path = camera_folder.join(name);
        fs::write(&photo_path, &payload).expect("write photo");
    }

    // Baseline snapshot to enforce 100% Read-Only SD Protocol
    let baseline_tree = sd.compute_sha256_tree().expect("baseline tree");

    let target_dir = tempdir().expect("target tempdir");
    let profile = CameraProfile {
        camera_id: "CANON_R6_STRESS_01".to_string(),
        display_name: "Canon EOS R6 Stress".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let mock_toast = Arc::new(MockToastNotifier::new());
    let pipeline = IngestPipeline::new(Some(mock_toast.clone()));

    let summary = pipeline
        .run_ingest_simple(&to_real_volume(&sd), &profile)
        .await
        .expect("pipeline execution failed");

    // 1. Summary metrics validation
    assert_eq!(summary.transferred_files, test_cases.len());
    assert_eq!(summary.skipped_duplicates, 0);
    assert_eq!(summary.total_bytes, total_expected_bytes);
    assert!(summary.manifest_path.exists());

    // 2. Read and verify session manifest
    let manifest_str = fs::read_to_string(&summary.manifest_path).expect("read manifest");
    let manifest: IngestSessionManifest =
        serde_json::from_str(&manifest_str).expect("deserialize manifest");

    assert_eq!(manifest.total_transferred, test_cases.len());
    assert_eq!(manifest.total_skipped, 0);
    assert_eq!(manifest.total_bytes, total_expected_bytes);
    assert_eq!(manifest.files.len(), test_cases.len());

    // 3. Bit-for-bit file integrity and in-flight SHA-256 match
    for entry in &manifest.files {
        let filename = Path::new(&entry.relative_path)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();

        let (expected_size, expected_hash) = expected_hashes
            .get(filename)
            .unwrap_or_else(|| panic!("Unexpected manifest entry: {}", filename));

        assert_eq!(
            entry.file_size, *expected_size as u64,
            "Manifest file size mismatch for {}",
            filename
        );
        assert_eq!(
            &entry.sha256, expected_hash,
            "In-flight SHA-256 failed to match independent hash bit-for-bit for {}",
            filename
        );
        assert!(!entry.skipped_duplicate);

        // Verify physical destination file
        let dest_file_path = target_dir.path().join(&entry.relative_path);
        assert!(dest_file_path.exists(), "Target file missing at {:?}", dest_file_path);

        let dest_metadata = fs::metadata(&dest_file_path).expect("dest stat");
        assert_eq!(dest_metadata.len(), *expected_size as u64);

        let disk_hash = compute_file_sha256(&dest_file_path).expect("dest sha256");
        assert_eq!(
            &disk_hash, expected_hash,
            "Destination disk file hash mismatch for {}",
            filename
        );
    }

    // 4. Invariant: Source SD card must be 100% UNTOUCHED
    sd.assert_unmodified(&baseline_tree);
}

// ============================================================================
// 2. Chunk Size Invariance (1KB to 2MB) on Multi-MB Payload
// ============================================================================

#[tokio::test]
async fn test_adv_m4_chunk_size_invariance_and_boundary_conditions() {
    let payload_size = 4 * 1024 * 1024 + 37; // 4 MB + 37 bytes (unaligned)
    let payload = generate_deterministic_payload(payload_size, 0x88);
    let expected_hash = compute_memory_sha256(&payload);

    let chunk_sizes = [
        1024,             // 1 KB
        64 * 1024,        // 64 KB
        512 * 1024,       // 512 KB
        1024 * 1024,      // 1 MB (default)
        2 * 1024 * 1024,  // 2 MB
    ];

    for chunk_size in chunk_sizes {
        let sd = SyntheticSdCard::new("CHUNK_TEST").expect("synthetic SD");
        let folder = sd.add_camera_folder("100CANON").expect("folder");
        let photo_path = folder.join("UNALIGNED.CR3");
        fs::write(&photo_path, &payload).expect("write");

        let target_dir = tempdir().expect("target");
        let profile = CameraProfile {
            camera_id: "CHUNK_CAM".to_string(),
            display_name: "Chunk Camera".to_string(),
            target_directory: target_dir.path().to_path_buf(),
            known_card_capacities: vec![64 * 1024 * 1024 * 1024],
            last_ingest_timestamp: None,
            total_photos_ingested: 0,
        };

        let pipeline = IngestPipeline::new(None).with_chunk_size(chunk_size);
        let summary = pipeline
            .run_ingest_simple(&to_real_volume(&sd), &profile)
            .await
            .unwrap_or_else(|e| panic!("Chunk size {} failed: {}", chunk_size, e));

        assert_eq!(summary.transferred_files, 1);
        assert_eq!(summary.total_bytes, payload_size as u64);

        let manifest_str = fs::read_to_string(&summary.manifest_path).unwrap();
        let manifest: IngestSessionManifest = serde_json::from_str(&manifest_str).unwrap();
        assert_eq!(
            manifest.files[0].sha256, expected_hash,
            "Chunk size {} produced incorrect in-flight hash",
            chunk_size
        );
    }
}

// ============================================================================
// 3. Single-Pass Transfer Verification (Source File Monotonic Progress Stream)
// ============================================================================

#[tokio::test]
async fn test_adv_m4_single_pass_source_read_verification() {
    let sd = SyntheticSdCard::new("PROGRESS_SD").expect("synthetic SD");
    let folder = sd.add_camera_folder("100CANON").expect("folder");

    let file_size = 5 * 1024 * 1024; // 5 MB
    let payload = generate_deterministic_payload(file_size, 0x99);
    let expected_hash = compute_memory_sha256(&payload);
    let photo_path = folder.join("STREAM_5M.JPG");
    fs::write(&photo_path, &payload).expect("write photo");

    let baseline_tree = sd.compute_sha256_tree().expect("baseline");

    let target_dir = tempdir().expect("target");
    let profile = CameraProfile {
        camera_id: "SINGLE_PASS_CAM".to_string(),
        display_name: "Single Pass Cam".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![64 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let (progress_tx, mut progress_rx) = mpsc::channel::<IngestProgress>(100);
    let monitor_handle = tokio::spawn(async move {
        let mut events = Vec::new();
        while let Some(evt) = progress_rx.recv().await {
            events.push(evt);
        }
        events
    });

    let pipeline = IngestPipeline::new(None);
    let summary = pipeline
        .run_ingest(&to_real_volume(&sd), &profile, Some(progress_tx))
        .await
        .expect("transfer failed");

    let events = monitor_handle.await.expect("monitor task panicked");

    // Verify progress events structure
    assert!(!events.is_empty(), "Progress events must not be empty");
    assert_eq!(events[0].status, IngestStatus::Starting);

    // Filter transferring events
    let transfer_events: Vec<&IngestProgress> = events
        .iter()
        .filter(|e| e.status == IngestStatus::Transferring)
        .collect();

    // Exactly 5 chunks for 5 MB with 1 MB default chunk size
    assert_eq!(
        transfer_events.len(),
        5,
        "Expected 5 chunk progress events for 5 MB file in 1 MB chunks"
    );

    // Verify strictly monotonic byte progression
    let expected_steps = [
        1024 * 1024,
        2 * 1024 * 1024,
        3 * 1024 * 1024,
        4 * 1024 * 1024,
        5 * 1024 * 1024,
    ];
    for (i, evt) in transfer_events.iter().enumerate() {
        assert_eq!(
            evt.current_file_bytes, expected_steps[i] as u64,
            "Transferring event {} had unexpected byte count",
            i
        );
        assert_eq!(evt.current_file_total_bytes, file_size as u64);
    }

    // Verify terminal events
    let file_completed_events: Vec<&IngestProgress> = events
        .iter()
        .filter(|e| e.status == IngestStatus::FileCompleted)
        .collect();
    assert_eq!(file_completed_events.len(), 1);
    assert_eq!(file_completed_events[0].current_file_bytes, file_size as u64);

    assert_eq!(events.last().unwrap().status, IngestStatus::OverallCompleted);

    // Destination file exists and matches hash
    let manifest_str = fs::read_to_string(&summary.manifest_path).unwrap();
    let manifest: IngestSessionManifest = serde_json::from_str(&manifest_str).unwrap();
    assert_eq!(manifest.files[0].sha256, expected_hash);

    // Source SD card strictly unmodified
    sd.assert_unmodified(&baseline_tree);
}

// ============================================================================
// 4. Deduplication Accuracy: Scenarios A, B, C, and D
// ============================================================================

#[tokio::test]
async fn test_adv_m4_deduplication_scenarios_a_b_c_exhaustive() {
    let sd = SyntheticSdCard::new("DEDUP_SD").expect("synthetic SD");
    let folder = sd.add_camera_folder("100CANON").expect("folder");

    let file_a_size = 2 * 1024 * 1024; // 2 MB
    let payload_a = generate_deterministic_payload(file_a_size, 0xAA);
    let hash_a = compute_memory_sha256(&payload_a);
    let photo_a = folder.join("PHOTO_A.JPG");
    fs::write(&photo_a, &payload_a).expect("write photo A");

    let file_c_size = 3 * 1024 * 1024; // 3 MB
    let payload_c = generate_deterministic_payload(file_c_size, 0xCC);
    let hash_c = compute_memory_sha256(&payload_c);
    let photo_c = folder.join("PHOTO_C.JPG");
    fs::write(&photo_c, &payload_c).expect("write photo C");

    let target_dir = tempdir().expect("target");
    let profile = CameraProfile {
        camera_id: "DEDUP_CAM".to_string(),
        display_name: "Dedup Camera".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let pipeline = IngestPipeline::new(None);

    // ------------------------------------------------------------------------
    // SCENARIO A: Target file does not exist -> transferred.
    // ------------------------------------------------------------------------
    let summary_a = pipeline
        .run_ingest_simple(&to_real_volume(&sd), &profile)
        .await
        .expect("Scenario A failed");

    assert_eq!(summary_a.transferred_files, 2);
    assert_eq!(summary_a.skipped_duplicates, 0);
    assert_eq!(summary_a.total_bytes, (file_a_size + file_c_size) as u64);

    let manifest_a_str = fs::read_to_string(&summary_a.manifest_path).unwrap();
    let manifest_a: IngestSessionManifest = serde_json::from_str(&manifest_a_str).unwrap();
    assert_eq!(manifest_a.files.len(), 2);
    for file in &manifest_a.files {
        assert!(!file.skipped_duplicate);
        assert_eq!(file.status.as_deref(), Some("transferred"));
    }

    // Locate PHOTO_A and PHOTO_C on destination disk
    let dest_photo_a = target_dir
        .path()
        .join(&manifest_a.files[0].relative_path);
    let dest_photo_c = target_dir
        .path()
        .join(&manifest_a.files[1].relative_path);

    assert!(dest_photo_a.exists());
    assert!(dest_photo_c.exists());

    // Record mtime of destination files before Run 2
    let meta_a_before = fs::metadata(&dest_photo_a).expect("stat a");
    let mtime_a_before = meta_a_before.modified().expect("mtime a");

    // ------------------------------------------------------------------------
    // SCENARIO B: Target exists with identical size and matching SHA-256 -> skipped (0 disk writes).
    // ------------------------------------------------------------------------
    // Small delay to ensure timestamp resolution difference if any write occurred
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let summary_b = pipeline
        .run_ingest_simple(&to_real_volume(&sd), &profile)
        .await
        .expect("Scenario B failed");

    assert_eq!(summary_b.transferred_files, 0);
    assert_eq!(summary_b.skipped_duplicates, 2);
    assert_eq!(summary_b.total_bytes, 0, "Zero bytes must be transferred for duplicates");

    let manifest_b_str = fs::read_to_string(&summary_b.manifest_path).unwrap();
    let manifest_b: IngestSessionManifest = serde_json::from_str(&manifest_b_str).unwrap();
    assert_eq!(manifest_b.total_transferred, 0);
    assert_eq!(manifest_b.total_skipped, 2);
    assert_eq!(manifest_b.total_bytes, 0);

    for file in &manifest_b.files {
        assert!(file.skipped_duplicate);
        assert_eq!(file.status.as_deref(), Some("duplicate_skipped"));
    }

    // Verify 0 disk writes: mtime and length of dest_photo_a must be identical
    let meta_a_after = fs::metadata(&dest_photo_a).expect("stat a after");
    let mtime_a_after = meta_a_after.modified().expect("mtime a after");
    assert_eq!(
        mtime_a_before, mtime_a_after,
        "Destination file modified time changed! 0 disk writes invariant violated for duplicate!"
    );

    // Verify no stray .tmp files left in destination folders
    let dest_parent = dest_photo_a.parent().unwrap();
    for entry in fs::read_dir(dest_parent).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        assert!(
            !name.contains(".tmp"),
            "Dangling temporary file found in destination folder: {}",
            name
        );
    }

    // ------------------------------------------------------------------------
    // SCENARIO C: Target exists with matching size but DIFFERENT content/hash -> transferred and overwritten.
    // ------------------------------------------------------------------------
    // Corrupt dest_photo_c with different payload of the EXACT same size (3 MB)
    let corrupt_payload = generate_deterministic_payload(file_c_size, 0xEE);
    fs::write(&dest_photo_c, &corrupt_payload).expect("write corrupt dest");
    let corrupt_hash = compute_memory_sha256(&corrupt_payload);
    assert_ne!(corrupt_hash, hash_c, "Corrupt hash must differ from source");

    let summary_c = pipeline
        .run_ingest_simple(&to_real_volume(&sd), &profile)
        .await
        .expect("Scenario C failed");

    // PHOTO_A is still duplicate (skipped), but PHOTO_C must be transferred & overwritten!
    assert_eq!(summary_c.transferred_files, 1, "Corrupted PHOTO_C must be repaired/transferred");
    assert_eq!(summary_c.skipped_duplicates, 1, "PHOTO_A must still be skipped");
    assert_eq!(summary_c.total_bytes, file_c_size as u64);

    let manifest_c_str = fs::read_to_string(&summary_c.manifest_path).unwrap();
    let manifest_c: IngestSessionManifest = serde_json::from_str(&manifest_c_str).unwrap();

    let entry_c = manifest_c
        .files
        .iter()
        .find(|f| f.relative_path.contains("PHOTO_C.JPG"))
        .expect("PHOTO_C entry");
    assert!(!entry_c.skipped_duplicate);
    assert_eq!(entry_c.status.as_deref(), Some("transferred"));
    assert_eq!(entry_c.sha256, hash_c);

    // Verify destination disk file was repaired and matches clean source hash
    let repaired_dest_hash = compute_file_sha256(&dest_photo_c).expect("repaired hash");
    assert_eq!(
        repaired_dest_hash, hash_c,
        "Destination file was not overwritten with clean source content"
    );

    // ------------------------------------------------------------------------
    // SCENARIO D: Target exists with DIFFERENT size -> transferred and overwritten.
    // ------------------------------------------------------------------------
    // Truncate dest_photo_a to 512 bytes
    fs::write(&dest_photo_a, vec![0x11u8; 512]).expect("truncate dest_photo_a");

    let summary_d = pipeline
        .run_ingest_simple(&to_real_volume(&sd), &profile)
        .await
        .expect("Scenario D failed");

    assert_eq!(summary_d.transferred_files, 1, "Size-mismatched file must be re-transferred");
    let meta_a_repaired = fs::metadata(&dest_photo_a).expect("stat repaired a");
    assert_eq!(
        meta_a_repaired.len(),
        file_a_size as u64,
        "Destination file must have full source size after overwrite"
    );
    let repaired_hash_a = compute_file_sha256(&dest_photo_a).expect("repaired hash a");
    assert_eq!(repaired_hash_a, hash_a);
}

// ============================================================================
// 5. Manifest Schema, Typed Fields, and Mathematical Sum Precision
// ============================================================================

#[tokio::test]
async fn test_adv_m4_manifest_schema_and_sum_precision() {
    let sd = SyntheticSdCard::new("MANIFEST_CARD").expect("synthetic SD");
    let folder = sd.add_camera_folder("100CANON").expect("folder");

    let sizes = [500_000, 1_200_000, 3_000_000];
    let mut total_bytes_expected = 0u64;

    for (i, size) in sizes.iter().enumerate() {
        let name = format!("IMG_{:04}.CR3", i + 1);
        let payload = generate_deterministic_payload(*size, (i + 1) as u8);
        fs::write(folder.join(&name), &payload).expect("write");
        total_bytes_expected += *size as u64;
    }

    let target_dir = tempdir().expect("target");
    let profile = CameraProfile {
        camera_id: "SCHEMA_CAM_01".to_string(),
        display_name: "Canon R6 Schema".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let pipeline = IngestPipeline::new(None);
    let summary = pipeline
        .run_ingest_simple(&to_real_volume(&sd), &profile)
        .await
        .expect("ingest failed");

    let manifest_raw = fs::read_to_string(&summary.manifest_path).expect("read manifest");

    // 1. Raw JSON parsing & schema structure validation
    let v: serde_json::Value = serde_json::from_str(&manifest_raw).expect("valid JSON syntax");
    assert!(v.is_object(), "Manifest root must be a JSON object");

    assert!(v["session_id"].is_string());
    assert!(v["session_id"].as_str().unwrap().starts_with("session_"));
    assert!(v["timestamp"].is_string());
    assert!(v["camera_id"].is_string());
    assert_eq!(v["camera_id"].as_str().unwrap(), "SCHEMA_CAM_01");
    assert!(v["target_directory"].is_string());
    assert!(v["volume_label"].is_string());
    assert_eq!(v["volume_label"].as_str().unwrap(), "MANIFEST_CARD");
    assert!(v["total_scanned"].is_number());
    assert_eq!(v["total_scanned"].as_u64().unwrap(), 3);
    assert_eq!(v["total_transferred"].as_u64().unwrap(), 3);
    assert_eq!(v["total_skipped"].as_u64().unwrap(), 0);
    assert_eq!(v["total_bytes"].as_u64().unwrap(), total_bytes_expected);
    assert!(v["duration_ms"].is_number());
    assert!(v["files"].is_array());
    assert_eq!(v["files"].as_array().unwrap().len(), 3);

    // 2. Typed deserialization into IngestSessionManifest
    let manifest: IngestSessionManifest =
        serde_json::from_str(&manifest_raw).expect("typed deserialize");

    // 3. Mathematical precision: byte sum must equal sum of file entries
    let sum_file_bytes: u64 = manifest
        .files
        .iter()
        .filter(|f| !f.skipped_duplicate)
        .map(|f| f.file_size)
        .sum();
    assert_eq!(
        manifest.total_bytes, sum_file_bytes,
        "Manifest total_bytes does not equal sum of non-skipped file entries"
    );
    assert_eq!(manifest.total_bytes, total_bytes_expected);

    // 4. Counts precision
    assert_eq!(
        manifest.total_transferred + manifest.total_skipped,
        manifest.total_scanned.unwrap(),
        "total_transferred + total_skipped != total_scanned"
    );

    // 5. Check individual file entry schema
    for file in &manifest.files {
        assert!(!file.relative_path.is_empty());
        assert!(file.destination_path.is_some());
        assert_eq!(file.sha256.len(), 64, "SHA-256 must be 64 hex characters");
        assert!(
            file.sha256.chars().all(|c| c.is_ascii_hexdigit()),
            "SHA-256 must contain valid hex digits only"
        );
        assert!(!file.skipped_duplicate);
        assert_eq!(file.status.as_deref(), Some("transferred"));
    }
}

// ============================================================================
// 6. Responsive Cancellation Mid-Stream Cleans Up Temporary Files
// ============================================================================

#[tokio::test]
async fn test_adv_m4_cancellation_during_large_stream_cleans_temp_file() {
    let sd = SyntheticSdCard::new("CANCEL_SD").expect("synthetic SD");
    let folder = sd.add_camera_folder("100CANON").expect("folder");

    let file_size = 15 * 1024 * 1024; // 15 MB
    let payload = generate_deterministic_payload(file_size, 0x77);
    let photo_path = folder.join("LARGE_CANCEL.CR3");
    fs::write(&photo_path, &payload).expect("write");

    let target_dir = tempdir().expect("target");
    let profile = CameraProfile {
        camera_id: "CANCEL_CAM".to_string(),
        display_name: "Cancel Cam".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let cancel_token = IngestCancellationToken::new();
    let token_clone = cancel_token.clone();

    let (progress_tx, mut progress_rx) = mpsc::channel::<IngestProgress>(100);

    // Task that monitors progress and cancels after 2 MB is transferred
    tokio::spawn(async move {
        while let Some(evt) = progress_rx.recv().await {
            if evt.current_file_bytes >= 2 * 1024 * 1024 {
                token_clone.cancel();
                break;
            }
        }
    });

    let pipeline = IngestPipeline::new(None).with_cancellation_token(cancel_token);
    let res = pipeline
        .run_ingest(&to_real_volume(&sd), &profile, Some(progress_tx))
        .await;

    // Verify it failed with cancellation error
    assert!(res.is_err(), "Pipeline should return error when cancelled");
    match res.unwrap_err() {
        IngestError::Io(msg) => {
            assert!(
                msg.contains("cancelled") || msg.contains("closed"),
                "Error message should mention cancellation: {}",
                msg
            );
        }
        other => panic!("Unexpected error type on cancel: {:?}", other),
    }

    // Verify NO lingering .tmp files in target directory tree
    let mut tmp_files_found = Vec::new();
    fn search_tmp_files(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    search_tmp_files(&p, out);
                } else if p.is_file() {
                    let name = p.file_name().unwrap().to_string_lossy();
                    if name.contains(".tmp") {
                        out.push(p);
                    }
                }
            }
        }
    }
    search_tmp_files(target_dir.path(), &mut tmp_files_found);
    assert!(
        tmp_files_found.is_empty(),
        "Lingering temporary files found after cancellation: {:?}",
        tmp_files_found
    );

    // Verify source file is untouched
    assert!(photo_path.exists());
    assert_eq!(fs::metadata(&photo_path).unwrap().len(), file_size as u64);
}

// ============================================================================
// 7. Zero-Byte File Transfer & Deduplication Handling
// ============================================================================

#[tokio::test]
async fn test_adv_m4_zero_byte_file_handling() {
    let sd = SyntheticSdCard::new("ZERO_SD").expect("synthetic SD");
    let folder = sd.add_camera_folder("100CANON").expect("folder");

    let zero_file = folder.join("ZERO_BYTE.JPG");
    fs::write(&zero_file, b"").expect("write zero file");

    let target_dir = tempdir().expect("target");
    let profile = CameraProfile {
        camera_id: "ZERO_CAM".to_string(),
        display_name: "Zero Byte Camera".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![64 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let pipeline = IngestPipeline::new(None);

    // Run 1: Transfers 0-byte file cleanly
    let summary1 = pipeline
        .run_ingest_simple(&to_real_volume(&sd), &profile)
        .await
        .expect("run 1 failed");

    assert_eq!(summary1.transferred_files, 1);
    assert_eq!(summary1.skipped_duplicates, 0);
    assert_eq!(summary1.total_bytes, 0);

    let manifest1_str = fs::read_to_string(&summary1.manifest_path).unwrap();
    let manifest1: IngestSessionManifest = serde_json::from_str(&manifest1_str).unwrap();

    // Standard SHA-256 of empty input
    let empty_sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    assert_eq!(manifest1.files[0].sha256, empty_sha256);
    assert_eq!(manifest1.files[0].file_size, 0);

    // Run 2: Detects 0-byte duplicate and skips with 0 writes
    let summary2 = pipeline
        .run_ingest_simple(&to_real_volume(&sd), &profile)
        .await
        .expect("run 2 failed");

    assert_eq!(summary2.transferred_files, 0);
    assert_eq!(summary2.skipped_duplicates, 1);
    assert_eq!(summary2.total_bytes, 0);

    let manifest2_str = fs::read_to_string(&summary2.manifest_path).unwrap();
    let manifest2: IngestSessionManifest = serde_json::from_str(&manifest2_str).unwrap();
    assert!(manifest2.files[0].skipped_duplicate);
    assert_eq!(manifest2.files[0].sha256, empty_sha256);
}

// ============================================================================
// 8. Toast Notification Formatting and Failure Resilience
// ============================================================================

#[tokio::test]
async fn test_adv_m4_toast_notification_payload_and_failure_resilience() {
    let sd = SyntheticSdCard::new("TOAST_SD").expect("synthetic SD");
    let folder = sd.add_camera_folder("100CANON").expect("folder");

    let payload = vec![0x42u8; 1024 * 1024]; // 1 MB
    fs::write(folder.join("TOAST_TEST.JPG"), &payload).expect("write");

    let target_dir = tempdir().expect("target");
    let profile = CameraProfile {
        camera_id: "TOAST_CAM".to_string(),
        display_name: "Nikon Z8 Wildlife".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let mock_toast = Arc::new(MockToastNotifier::new());
    let pipeline = IngestPipeline::new(Some(mock_toast.clone()));

    // Run with successful toast
    let _summary = pipeline
        .run_ingest_simple(&to_real_volume(&sd), &profile)
        .await
        .expect("ingest failed");

    assert_eq!(mock_toast.count(), 1);
    let (title, message) = mock_toast.last_notification().unwrap();
    assert!(
        title.contains("Nikon Z8 Wildlife"),
        "Title should contain camera display name: {}",
        title
    );
    assert!(
        message.contains("Transferred: 1 files"),
        "Message should report transferred files count: {}",
        message
    );
    assert!(
        message.contains(&profile.target_directory.display().to_string()),
        "Message should report destination directory: {}",
        message
    );

    // Test non-fatal error resilience when toast fails
    mock_toast.set_should_fail(true);
    let summary_fail = pipeline
        .run_ingest_simple(&to_real_volume(&sd), &profile)
        .await;

    // Ingest pipeline must SUCCEED even if toast dispatch fails (non-fatal warning)
    assert!(
        summary_fail.is_ok(),
        "Ingest pipeline should not fail when toast notification fails: {:?}",
        summary_fail.err()
    );
}
