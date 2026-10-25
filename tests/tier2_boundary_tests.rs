#![allow(dead_code, unused_imports)]

//! Tier 2 Boundary & Corner Case Tests: LightCraft SD Ingest Hub & Modular Launcher
//!
//! Validates edge cases, boundary conditions, fault injection, and security invariants:
//! - Exact 99% formatted capacity threshold boundary
//! - 0-byte corrupt photo files handling
//! - Truncated and corrupted EXIF headers (< 64 bytes, invalid TIFF magic)
//! - Strict 100% read-only SD medium permissions (physical write-lock simulation)
//! - Cryptographic zero-modification invariant verification
//! - Duplicate file name collisions across folders with differing hashes
//! - Multi-chunk streaming memory boundary
//! - Unwritable/unreachable destination paths
//! - Deeply nested, unicode, and space-containing destination paths
//! - Malformed registry TOML recovery
//! - Zero-byte volume capacity edge case

mod common;

use chrono::Utc;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::tempdir;

use common::contracts::{
    CameraProfile, IngestError, IngestSessionManifest, MockToastNotifier, SdCardVolume,
};
use common::exif_gen::{
    extract_bounded_exif_fingerprint, extract_capture_date, ExifPhotoBuilder,
};
use common::pipeline_harness::{
    check_empty_card, compute_file_sha256, IngestPipelineHarness,
};
use common::registry_fixture::IsolatedCameraRegistry;
use common::sd_fixture::SyntheticSdCard;

// ============================================================================
// Boundary 1: Formatted Empty Card 99% Margin Exact Threshold
// ============================================================================

#[test]
fn test_t2_boundary_exact_99_percent_margin_threshold() {
    let capacity: u64 = 100_000_000_000; // 100 GB

    // Exactly 99.001% free (99,001,000,000 bytes) -> Above threshold, considered empty
    let vol_above = SdCardVolume {
        drive_letter: 'F',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: capacity,
        free_bytes: 99_001_000_000,
        has_dcim: true,
        camera_folders: vec![],
    };
    assert!(
        check_empty_card(&vol_above),
        "99.001% free space must qualify as formatted empty card"
    );

    // Exactly 98.999% free (98,999,000,000 bytes) -> Below threshold, NOT considered empty
    let vol_below = SdCardVolume {
        drive_letter: 'F',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: capacity,
        free_bytes: 98_999_000_000,
        has_dcim: true,
        camera_folders: vec![],
    };
    assert!(
        !check_empty_card(&vol_below),
        "98.999% free space must NOT qualify as formatted empty card"
    );
}

// ============================================================================
// Boundary 2: 0-Byte Photo Handling
// ============================================================================

#[test]
fn test_t2_boundary_zero_byte_photo_handling() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let folder = sd.add_camera_folder("100CANON").expect("add folder");
    let zero_byte_photo = folder.join("IMG_EMPTY.JPG");

    // Create 0-byte file
    File::create(&zero_byte_photo).expect("create empty file");

    let result = extract_bounded_exif_fingerprint(&zero_byte_photo, 128 * 1024);
    assert!(
        result.is_err(),
        "0-byte photo must return an error without panicking"
    );
    match result.unwrap_err() {
        IngestError::ExifError(msg) => {
            assert!(msg.contains("too small") || msg.contains("EXIF"));
        }
        other => panic!("Unexpected error variant: {:?}", other),
    }
}

// ============================================================================
// Boundary 3: Truncated EXIF Header (< 64 Bytes)
// ============================================================================

#[test]
fn test_t2_boundary_truncated_exif_header_under_64_bytes() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let folder = sd.add_camera_folder("100CANON").expect("add folder");
    let truncated_path = folder.join("IMG_TRUNC.JPG");

    // Only 12 bytes of data (valid JPEG SOI + APP1 marker but truncated before TIFF header)
    let truncated_data = vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xE1, // APP1
        0x00, 0x08, // length 8
        b'E', b'x', b'i', b'f', 0x00, 0x00, // Exif marker only, no TIFF
    ];
    fs::write(&truncated_path, &truncated_data).expect("write truncated file");

    let result = extract_bounded_exif_fingerprint(&truncated_path, 128 * 1024);
    assert!(result.is_err(), "Truncated header must not panic");
}

// ============================================================================
// Boundary 4: Corrupted TIFF Magic Bytes
// ============================================================================

#[test]
fn test_t2_boundary_corrupted_tiff_magic_bytes() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let folder = sd.add_camera_folder("100CANON").expect("add folder");
    let corrupt_path = folder.join("IMG_CORRUPT.JPG");

    // Valid JPEG SOI + APP1, but invalid TIFF header magic b"ZZ" instead of b"II"
    let mut corrupt_data = vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xE1, // APP1
        0x00, 0x20, // length 32
        b'E', b'x', b'i', b'f', 0x00, 0x00, // Exif\0\0
        b'Z', b'Z', // Corrupt byte order
        0x99, 0x99, // Corrupt magic
    ];
    corrupt_data.resize(64, 0xEE);

    fs::write(&corrupt_path, &corrupt_data).expect("write corrupt file");

    let result = extract_bounded_exif_fingerprint(&corrupt_path, 128 * 1024);
    assert!(result.is_err(), "Corrupt TIFF magic must return Err");
}

// ============================================================================
// Boundary 5: Simulated Physical Write-Lock (Read-Only Medium Permissions)
// ============================================================================

#[tokio::test]
async fn test_t2_boundary_source_read_only_permissions_lock() {
    let sd = SyntheticSdCard::new("LOCKED_SD").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new();
    sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");

    // Snapshot SD card before locking
    let baseline_tree = sd.compute_sha256_tree().expect("baseline tree");

    // Simulate physical SD card lock switch by setting read-only filesystem permissions
    sd.set_read_only(true).expect("set read only permissions");

    let target_dir = tempdir().expect("target tempdir");
    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6 Locked Card".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let harness: IngestPipelineHarness<MockToastNotifier> = IngestPipelineHarness::new(None);
    let summary = harness
        .run_ingest(&sd.to_volume(), &profile)
        .await
        .expect("ingest on read-only SD must succeed cleanly");

    assert_eq!(summary.transferred_files, 1);

    // Invariant: SD card must be 100% untouched
    sd.assert_unmodified(&baseline_tree);

    // Reset permissions for cleanup
    let _ = sd.set_read_only(false);
}

// ============================================================================
// Boundary 6: Strict Cryptographic Zero-Modification Invariant
// ============================================================================

#[tokio::test]
async fn test_t2_boundary_zero_writes_strict_cryptographic_verification() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new().payload_size(4096);
    sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");
    sd.add_photo("100CANON", "IMG_0002.CR3", &builder.clone().as_raw()).expect("add photo");

    let baseline_tree = sd.compute_sha256_tree().expect("baseline");

    let target_dir = tempdir().expect("target tempdir");
    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let harness: IngestPipelineHarness<MockToastNotifier> = IngestPipelineHarness::new(None);
    harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest");

    // Invariant check: zero byte alterations, zero extra files created
    sd.assert_unmodified(&baseline_tree);
}

// ============================================================================
// Boundary 7: Duplicate Filename Across Subfolders with Differing Hashes
// ============================================================================

#[tokio::test]
async fn test_t2_boundary_duplicate_filename_collision_different_hash() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");

    // Folder 1: Day 1 IMG_0001.JPG
    let b1 = ExifPhotoBuilder::new()
        .date_time("2026:10:01 10:00:00")
        .payload_size(1024);
    sd.add_photo("100CANON", "IMG_0001.JPG", &b1).expect("add photo 1");

    // Folder 2: Day 2 IMG_0001.JPG (different day and different payload size)
    let b2 = ExifPhotoBuilder::new()
        .date_time("2026:10:02 10:00:00")
        .payload_size(2048);
    sd.add_photo("101CANON", "IMG_0001.JPG", &b2).expect("add photo 2");

    let target_dir = tempdir().expect("target tempdir");
    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let harness: IngestPipelineHarness<MockToastNotifier> = IngestPipelineHarness::new(None);
    let summary = harness
        .run_ingest(&sd.to_volume(), &profile)
        .await
        .expect("ingest");

    // Because they were taken on different dates, they route to different subdirectories:
    // 2026/2026-10-01/IMG_0001.JPG and 2026/2026-10-02/IMG_0001.JPG
    assert_eq!(summary.transferred_files, 2);
    assert_eq!(summary.skipped_duplicates, 0);

    let dest1 = target_dir.path().join("2026").join("2026-10-01").join("IMG_0001.JPG");
    let dest2 = target_dir.path().join("2026").join("2026-10-02").join("IMG_0001.JPG");
    assert!(dest1.exists());
    assert!(dest2.exists());
}

// ============================================================================
// Boundary 8: Multi-Chunk 1 MB Streaming Verification
// ============================================================================

#[tokio::test]
async fn test_t2_boundary_multi_chunk_streaming_without_memory_blowup() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    // 4.5 MB photo payload (spans 5 1-MB chunks)
    let builder = ExifPhotoBuilder::new().payload_size(4_500_000);
    let photo_path = sd.add_photo("100CANON", "IMG_LARGE.JPG", &builder).expect("add large photo");

    let expected_sha256 = compute_file_sha256(&photo_path).expect("source sha256");

    let target_dir = tempdir().expect("target tempdir");
    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let harness: IngestPipelineHarness<MockToastNotifier> = IngestPipelineHarness::new(None);
    let summary = harness
        .run_ingest(&sd.to_volume(), &profile)
        .await
        .expect("multi-chunk ingest");

    assert_eq!(summary.transferred_files, 1);
    assert!(summary.total_bytes >= 4_500_000);

    let manifest_str = fs::read_to_string(&summary.manifest_path).expect("read manifest");
    let manifest: IngestSessionManifest = serde_json::from_str(&manifest_str).expect("manifest");
    assert_eq!(manifest.files[0].sha256, expected_sha256);
}

// ============================================================================
// Boundary 9: Destination Unwritable Path Error Handling
// ============================================================================

#[tokio::test]
async fn test_t2_boundary_destination_unwritable_directory_error() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new();
    sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");

    // Invalid non-existent drive letter path on Windows
    let invalid_target = PathBuf::from(r"Z:\NonExistentDrive\Photos\Subfolder");

    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6".to_string(),
        target_directory: invalid_target,
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let harness: IngestPipelineHarness<MockToastNotifier> = IngestPipelineHarness::new(None);
    let result = harness.run_ingest(&sd.to_volume(), &profile).await;

    assert!(result.is_err(), "Unwritable destination must return Err");
    match result.unwrap_err() {
        IngestError::DestinationError(_) => {}
        other => panic!("Expected DestinationError, got {:?}", other),
    }
}

// ============================================================================
// Boundary 10: Special Characters, Spaces & Unicode in Destination Path
// ============================================================================

#[tokio::test]
async fn test_t2_boundary_special_characters_and_spaces_in_destination_path() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new();
    sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");

    let root_temp = tempdir().expect("tempdir");
    // Path with spaces, dashes, symbols and accents
    let complex_dest = root_temp
        .path()
        .join("Sesión Fotográfica — Canon R6 (2026)")
        .join("Exportación & Archivo Final");

    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6 Unicode".to_string(),
        target_directory: complex_dest.clone(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let harness: IngestPipelineHarness<MockToastNotifier> = IngestPipelineHarness::new(None);
    let summary = harness
        .run_ingest(&sd.to_volume(), &profile)
        .await
        .expect("complex unicode path must be handled cleanly");

    assert_eq!(summary.transferred_files, 1);
    assert!(complex_dest.exists());
}

// ============================================================================
// Boundary 11: Missing Date Fallback to Current Date
// ============================================================================

#[test]
fn test_t2_boundary_missing_date_fallback_to_current_date() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    // Photo builder without DateTimeOriginal tag
    let mut builder = ExifPhotoBuilder::new();
    builder.date_time_original = None;

    let photo_path = sd.add_photo("100CANON", "IMG_NODATE.JPG", &builder).expect("add photo");

    let (yyyy, yyyy_mm_dd) = extract_capture_date(&photo_path).expect("extract capture date");

    let now = Utc::now();
    assert_eq!(yyyy, now.format("%Y").to_string());
    assert_eq!(yyyy_mm_dd, now.format("%Y-%m-%d").to_string());
}

// ============================================================================
// Boundary 12: Corrupted Registry TOML Recovery
// ============================================================================

#[test]
fn test_t2_boundary_registry_corrupted_toml_recovery() {
    let mut reg = IsolatedCameraRegistry::new().expect("create registry");

    // Overwrite TOML with syntax error
    fs::write(&reg.toml_path, "invalid_toml = [[ broken syntax {{{{").expect("write corrupt TOML");

    let result = reg.reload();
    assert!(result.is_err(), "Corrupted TOML must return RegistryError");
    match result.unwrap_err() {
        IngestError::RegistryError(_) => {}
        other => panic!("Expected RegistryError, got {:?}", other),
    }
}

// ============================================================================
// Boundary 13: Volume with Zero Capacity Edge Case
// ============================================================================

#[test]
fn test_t2_boundary_volume_with_zero_bytes_capacity() {
    let zero_vol = SdCardVolume {
        drive_letter: 'G',
        volume_label: "EMPTY_DRIVE".to_string(),
        file_system: "FAT32".to_string(),
        total_bytes: 0,
        free_bytes: 0,
        has_dcim: false,
        camera_folders: vec![],
    };

    // Must return false without zero-division panic
    assert!(!check_empty_card(&zero_vol));
}

// ============================================================================
// Boundary 14: Multiple Cameras Matching Same Card Capacity
// ============================================================================

#[test]
fn test_t2_boundary_multiple_cameras_matching_same_capacity() {
    let mut reg = IsolatedCameraRegistry::new().expect("create registry");
    let capacity_128gb = 128 * 1024 * 1024 * 1024;

    let cam1 = CameraProfile {
        camera_id: "CAM_CANON_01".to_string(),
        display_name: "Canon R6".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\Canon"),
        known_card_capacities: vec![capacity_128gb],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };
    let cam2 = CameraProfile {
        camera_id: "CAM_SONY_02".to_string(),
        display_name: "Sony A7IV".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\Sony"),
        known_card_capacities: vec![capacity_128gb],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    reg.register_or_update(cam1).expect("reg 1");
    reg.register_or_update(cam2).expect("reg 2");

    // When querying by capacity, registry returns a valid matching candidate
    let matched = reg.find_by_capacity(capacity_128gb);
    assert!(matched.is_some());
}
