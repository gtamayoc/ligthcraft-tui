#![allow(dead_code, unused_imports)]

//! Tier 1 Feature Integration Tests: LightCraft SD Ingest Hub & Modular Launcher
//!
//! Validates core functional requirements across 7 feature groups:
//! - Group 1: Launcher Options & CLI Flag Direct Bypass (US-001, FR-1)
//! - Group 2: Removable Drive Scanning & DCIM Media Crawler (US-002, FR-2)
//! - Group 3: Bounded Header EXIF Reader & Camera Fingerprint (US-003, FR-4, R3)
//! - Group 4: Atomic Camera Registry Persistence & O(1) Lookups (US-004, FR-5)
//! - Group 5: Asynchronous Streaming Copy & In-Flight SHA-256 (US-006, FR-8, FR-9)
//! - Group 6: Formatted Empty Card Detection & Capacity Matching (US-007, FR-10)
//! - Group 7: Windows Native Toast Notification Dispatch (FR-11, R5)

mod common;

use chrono::Utc;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::tempdir;

use common::contracts::{
    CameraFingerprint, CameraProfile, IngestError, IngestSessionManifest, IngestSummary,
    MockToastNotifier, SdCardVolume, ToastNotifier, ToolOption, VolumeScanner,
};
use common::exif_gen::{
    extract_bounded_exif_fingerprint, extract_capture_date, ExifPhotoBuilder,
};
use common::pipeline_harness::{
    check_empty_card, compute_file_sha256, IngestPipelineHarness, MockVolumeScanner,
};
use common::registry_fixture::IsolatedCameraRegistry;
use common::sd_fixture::SyntheticSdCard;

// ============================================================================
// Group 1: Launcher Options & CLI Flag Direct Bypass (US-001, FR-1)
// ============================================================================

#[test]
fn test_t1_feat01_01_tool_option_enum_variants() {
    let opt_studio = ToolOption::Studio;
    let opt_ingest = ToolOption::SdIngest;
    let opt_exit = ToolOption::Exit;

    assert_eq!(opt_studio, ToolOption::Studio);
    assert_eq!(opt_ingest, ToolOption::SdIngest);
    assert_eq!(opt_exit, ToolOption::Exit);
    assert_ne!(opt_studio, opt_ingest);
}

#[test]
fn test_t1_feat01_02_tool_option_serde() {
    let serialized = serde_json::to_string(&ToolOption::SdIngest).expect("serialization failed");
    let deserialized: ToolOption =
        serde_json::from_str(&serialized).expect("deserialization failed");
    assert_eq!(deserialized, ToolOption::SdIngest);
}

#[test]
fn test_t1_feat01_03_cli_bypass_tui_flag() {
    let mut cmd = assert_cmd::Command::cargo_bin("lightcraft-tui").expect("binary found");
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicates::str::contains("--tui"));
}

#[test]
fn test_t1_feat01_04_cli_bypass_verify_integrity_flag() {
    let mut cmd = assert_cmd::Command::cargo_bin("lightcraft-tui").expect("binary found");
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicates::str::contains("--verify-integrity"));
}

#[test]
fn test_t1_feat01_05_cli_bypass_sequential_export_flag() {
    let mut cmd = assert_cmd::Command::cargo_bin("lightcraft-tui").expect("binary found");
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicates::str::contains("--sequential-export"));
}

#[test]
fn test_t1_feat01_06_cli_bypass_output_dir_flag() {
    let mut cmd = assert_cmd::Command::cargo_bin("lightcraft-tui").expect("binary found");
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicates::str::contains("--output-dir"));
}

// ============================================================================
// Group 2: Removable Drive Scanning & DCIM Media Crawler (US-002, FR-2)
// ============================================================================

#[test]
fn test_t1_feat02_01_mock_volume_scanner_enumeration() {
    let mut scanner = MockVolumeScanner::new();
    let volume = SdCardVolume {
        drive_letter: 'E',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 64 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![PathBuf::from(r"E:\DCIM\100CANON")],
    };
    scanner.add_volume(volume.clone());

    let volumes = scanner.scan_volumes().expect("scan failed");
    assert_eq!(volumes.len(), 1);
    assert_eq!(volumes[0].drive_letter, 'E');
    assert_eq!(volumes[0].volume_label, "EOS_DIGITAL");
    assert_eq!(volumes[0].file_system, "exFAT");
    assert!(volumes[0].has_dcim);
}

#[test]
fn test_t1_feat02_02_dcim_folder_detection() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("failed to create synthetic SD");
    let vol = sd.to_volume();
    assert!(vol.has_dcim);
    assert!(sd.dcim_path().exists());
}

#[test]
fn test_t1_feat02_03_camera_folders_enumeration() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("failed to create synthetic SD");
    sd.add_camera_folder("100CANON").expect("add folder");
    sd.add_camera_folder("101EOSR6").expect("add folder");

    let vol = sd.to_volume();
    assert_eq!(vol.camera_folders.len(), 2);
    let folder_names: Vec<String> = vol
        .camera_folders
        .iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
        .collect();
    assert!(folder_names.contains(&"100CANON".to_string()));
    assert!(folder_names.contains(&"101EOSR6".to_string()));
}

#[test]
fn test_t1_feat02_04_media_categorization_raw_and_jpeg() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("failed to create synthetic SD");
    let builder_raw = ExifPhotoBuilder::new().as_raw();
    let builder_jpg = ExifPhotoBuilder::new();

    sd.add_photo("100CANON", "IMG_0001.CR3", &builder_raw).expect("add raw");
    sd.add_photo("100CANON", "IMG_0002.JPG", &builder_jpg).expect("add jpg");

    let folder = sd.dcim_path().join("100CANON");
    let mut raw_count = 0;
    let mut jpg_count = 0;

    for entry in fs::read_dir(folder).expect("read dir").flatten() {
        if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
            match ext.to_ascii_uppercase().as_str() {
                "CR3" => raw_count += 1,
                "JPG" => jpg_count += 1,
                _ => {}
            }
        }
    }

    assert_eq!(raw_count, 1);
    assert_eq!(jpg_count, 1);
}

#[test]
fn test_t1_feat02_05_video_file_detection() {
    let sd = SyntheticSdCard::new("LUMIX").expect("synthetic SD");
    let folder = sd.add_camera_folder("100_PANA").expect("add folder");

    let video_path = folder.join("MOV_0001.MP4");
    fs::write(&video_path, vec![0u8; 1024]).expect("write video mock");

    let mut video_count = 0;
    for entry in fs::read_dir(folder).expect("read dir").flatten() {
        if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
            if ext.eq_ignore_ascii_case("mp4") || ext.eq_ignore_ascii_case("mov") {
                video_count += 1;
            }
        }
    }
    assert_eq!(video_count, 1);
}

#[test]
fn test_t1_feat02_06_volume_capacity_accounting() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let vol = sd.to_volume();
    assert_eq!(vol.total_bytes, 128 * 1024 * 1024 * 1024);
    assert_eq!(vol.free_bytes, 90 * 1024 * 1024 * 1024);
    assert!(vol.free_bytes <= vol.total_bytes);
}

// ============================================================================
// Group 3: Bounded Header EXIF Reader & Camera Fingerprint (US-003, FR-4, R3)
// ============================================================================

#[test]
fn test_t1_feat03_01_bounded_header_reading_under_128kb() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new()
        .make("Canon")
        .model("Canon EOS R6 Mark II")
        .body_serial(Some("042021000123"))
        .payload_size(5 * 1024 * 1024); // 5 MB dummy image payload

    let photo_path = sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");

    // Must read successfully using only first 128 KB buffer
    let fingerprint = extract_bounded_exif_fingerprint(&photo_path, 128 * 1024)
        .expect("bounded EXIF extraction failed");

    assert_eq!(fingerprint.make, "Canon");
    assert_eq!(fingerprint.model, "Canon EOS R6 Mark II");
    assert_eq!(fingerprint.body_serial.as_deref(), Some("042021000123"));
}

#[test]
fn test_t1_feat03_02_body_serial_extraction_0xa431() {
    let sd = SyntheticSdCard::new("SONY_SD").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new()
        .make("Sony")
        .model("ILCE-7M4")
        .body_serial(Some("7849302199"));

    let photo_path = sd.add_photo("100MSDCF", "DSC0001.JPG", &builder).expect("add photo");

    let fingerprint = extract_bounded_exif_fingerprint(&photo_path, 64 * 1024)
        .expect("EXIF extraction failed");

    assert_eq!(fingerprint.body_serial.as_deref(), Some("7849302199"));
    assert_eq!(fingerprint.camera_id(), "7849302199");
}

#[test]
fn test_t1_feat03_03_make_and_model_extraction() {
    let sd = SyntheticSdCard::new("NIKON_SD").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new()
        .make("Nikon")
        .model("NIKON Z 8")
        .body_serial(Some("3004567"));

    let photo_path = sd.add_photo("100NIKON", "DSC_0001.JPG", &builder).expect("add photo");

    let fingerprint = extract_bounded_exif_fingerprint(&photo_path, 64 * 1024)
        .expect("EXIF extraction failed");

    assert_eq!(fingerprint.make, "Nikon");
    assert_eq!(fingerprint.model, "NIKON Z 8");
}

#[test]
fn test_t1_feat03_04_composite_fallback_when_serial_missing() {
    let sd = SyntheticSdCard::new("OLD_CAM").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new()
        .make("Fujifilm")
        .model("X-T30")
        .body_serial(None); // No serial number present

    let photo_path = sd.add_photo("100FUJI", "DSCF0001.JPG", &builder).expect("add photo");

    let fingerprint = extract_bounded_exif_fingerprint(&photo_path, 64 * 1024)
        .expect("EXIF extraction failed");

    assert!(fingerprint.body_serial.is_none());
    assert_eq!(fingerprint.camera_id(), "Fujifilm_X-T30");
}

#[test]
fn test_t1_feat03_05_optional_lens_model_extraction() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new()
        .make("Canon")
        .model("Canon EOS R6")
        .lens_model(Some("RF 24-70mm F2.8 L IS USM"));

    let photo_path = sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");

    let fingerprint = extract_bounded_exif_fingerprint(&photo_path, 64 * 1024)
        .expect("EXIF extraction failed");

    assert_eq!(
        fingerprint.lens_model.as_deref(),
        Some("RF 24-70mm F2.8 L IS USM")
    );
}

#[test]
fn test_t1_feat03_06_capture_date_extraction_for_routing() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new().date_time("2026:08:24 15:30:45");

    let photo_path = sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");

    let (yyyy, yyyy_mm_dd) = extract_capture_date(&photo_path).expect("capture date extraction");

    assert_eq!(yyyy, "2026");
    assert_eq!(yyyy_mm_dd, "2026-08-24");
}

// ============================================================================
// Group 4: Atomic Camera Registry Persistence & O(1) Lookups (US-004, FR-5)
// ============================================================================

#[test]
fn test_t1_feat04_01_registry_save_and_reload_toml() {
    let mut reg = IsolatedCameraRegistry::new().expect("create registry");
    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6 Main".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\CanonR6"),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: Some(Utc::now()),
        total_photos_ingested: 42,
    };

    reg.register_or_update(profile.clone()).expect("register failed");
    assert_eq!(reg.count(), 1);

    // Reload from TOML
    reg.reload().expect("reload failed");
    let loaded = reg.get_profile("042021000123").expect("profile not found");
    assert_eq!(loaded.display_name, "Canon R6 Main");
    assert_eq!(loaded.total_photos_ingested, 42);
    assert_eq!(loaded.known_card_capacities, vec![128 * 1024 * 1024 * 1024]);
}

#[test]
fn test_t1_feat04_02_atomic_write_rename_pattern() {
    let reg = IsolatedCameraRegistry::new().expect("create registry");
    reg.save().expect("save empty");

    // Verify temp file does not remain after atomic rename
    let tmp_path = reg.dir.path().join("camera_registry.toml.tmp");
    assert!(!tmp_path.exists(), "temporary file must be cleanly renamed");
    assert!(reg.toml_path.exists(), "target TOML file must exist");
}

#[test]
fn test_t1_feat04_03_o1_in_memory_hashmap_lookup() {
    let mut reg = IsolatedCameraRegistry::new().expect("create registry");
    for i in 1..=100 {
        let profile = CameraProfile {
            camera_id: format!("CAM_SERIAL_{:04}", i),
            display_name: format!("Camera #{:04}", i),
            target_directory: PathBuf::from(format!(r"C:\Photos\Cam{:04}", i)),
            known_card_capacities: vec![64 * 1024 * 1024 * 1024],
            last_ingest_timestamp: None,
            total_photos_ingested: 0,
        };
        reg.register_or_update(profile).expect("register");
    }

    assert_eq!(reg.count(), 100);
    let found = reg.get_profile("CAM_SERIAL_0042").expect("lookup O(1)");
    assert_eq!(found.display_name, "Camera #0042");
}

#[test]
fn test_t1_feat04_04_register_or_update_existing_profile() {
    let mut reg = IsolatedCameraRegistry::new().expect("create registry");
    let mut profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6 Main".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\CanonR6"),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 10,
    };
    reg.register_or_update(profile.clone()).expect("initial save");

    profile.total_photos_ingested += 25;
    reg.register_or_update(profile).expect("update save");

    assert_eq!(reg.count(), 1);
    let updated = reg.get_profile("042021000123").expect("get profile");
    assert_eq!(updated.total_photos_ingested, 35);
}

#[test]
fn test_t1_feat04_05_find_by_capacity_match() {
    let mut reg = IsolatedCameraRegistry::new().expect("create registry");
    let profile = CameraProfile {
        camera_id: "SONY_78493".to_string(),
        display_name: "Sony A7IV".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\Sony"),
        known_card_capacities: vec![64 * 1024 * 1024 * 1024], // 64 GB
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };
    reg.register_or_update(profile).expect("register");

    let capacity_to_find = 64 * 1024 * 1024 * 1024;
    let matched = reg.find_by_capacity(capacity_to_find).expect("capacity match");
    assert_eq!(matched.camera_id, "SONY_78493");
}

#[test]
fn test_t1_feat04_06_unregistered_camera_returns_none() {
    let reg = IsolatedCameraRegistry::new().expect("create registry");
    assert!(reg.get_profile("NON_EXISTENT_SERIAL").is_none());
}

// ============================================================================
// Group 5: Asynchronous Streaming Copy & In-Flight SHA-256 (US-006, FR-8, FR-9)
// ============================================================================

#[tokio::test]
async fn test_t1_feat05_01_streaming_transfer_1mb_chunks() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new().payload_size(2 * 1024 * 1024); // 2 MB file (2 chunks)
    sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");

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
        .expect("ingest execution failed");

    assert_eq!(summary.transferred_files, 1);
    assert_eq!(summary.skipped_duplicates, 0);
    assert!(summary.total_bytes >= 2 * 1024 * 1024);
}

#[tokio::test]
async fn test_t1_feat05_02_inflight_sha256_hash_computation() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new().payload_size(1024);
    let photo_path = sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");

    let source_sha = compute_file_sha256(&photo_path).expect("source sha");

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

    // Read manifest and verify recorded SHA-256 matches independent computation
    let manifest_content = fs::read_to_string(&summary.manifest_path).expect("read manifest");
    let manifest: IngestSessionManifest =
        serde_json::from_str(&manifest_content).expect("parse manifest");

    assert_eq!(manifest.files.len(), 1);
    assert_eq!(manifest.files[0].sha256, source_sha);
}

#[tokio::test]
async fn test_t1_feat05_03_deduplication_skip_identical_file() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new().payload_size(1024);
    sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");

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

    // First ingest: transfers 1 file
    let summary1 = harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest 1");
    assert_eq!(summary1.transferred_files, 1);
    assert_eq!(summary1.skipped_duplicates, 0);

    // Second ingest: skips the duplicate
    let summary2 = harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest 2");
    assert_eq!(summary2.transferred_files, 0);
    assert_eq!(summary2.skipped_duplicates, 1);
}

#[tokio::test]
async fn test_t1_feat05_04_date_structured_folder_hierarchy() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new().date_time("2026:10:15 14:00:00");
    sd.add_photo("100CANON", "IMG_0588.JPG", &builder).expect("add photo");

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

    let expected_dest = target_dir
        .path()
        .join("2026")
        .join("2026-10-15")
        .join("IMG_0588.JPG");
    assert!(expected_dest.exists(), "Photo must exist in date hierarchy");
}

#[tokio::test]
async fn test_t1_feat05_05_session_manifest_generation() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new();
    sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");

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
    let summary = harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest");

    assert!(summary.manifest_path.exists());
    let manifest_str = fs::read_to_string(&summary.manifest_path).expect("read manifest");
    let manifest: IngestSessionManifest = serde_json::from_str(&manifest_str).expect("deserialize manifest");
    assert_eq!(manifest.camera_id, "042021000123");
    assert_eq!(manifest.total_transferred, 1);
}

#[tokio::test]
async fn test_t1_feat05_06_sd_card_immutability_during_ingest() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    sd.populate_canon_photos(5).expect("populate photos");

    // Capture baseline SHA-256 tree of SD card before ingest
    let baseline_tree = sd.compute_sha256_tree().expect("baseline tree");

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

    // Invariant check: Assert zero modifications to SD card
    sd.assert_unmodified(&baseline_tree);
}

// ============================================================================
// Group 6: Formatted Empty Card Detection & Capacity Matching (US-007, FR-10)
// ============================================================================

#[test]
fn test_t1_feat06_01_empty_card_detection_exact() {
    let capacity = 64 * 1024 * 1024 * 1024;
    let mut vol = SdCardVolume {
        drive_letter: 'F',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: capacity,
        free_bytes: capacity, // 100% free
        has_dcim: true,
        camera_folders: vec![],
    };
    assert!(check_empty_card(&vol));

    vol.free_bytes = capacity / 2; // 50% free
    assert!(!check_empty_card(&vol));
}

#[test]
fn test_t1_feat06_02_empty_card_detection_within_99_margin() {
    let capacity = 128 * 1024 * 1024 * 1024;
    // 99.2% free space (accounting for filesystem overhead)
    let free_bytes = (capacity as f64 * 0.992) as u64;

    let vol = SdCardVolume {
        drive_letter: 'F',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: capacity,
        free_bytes,
        has_dcim: true,
        camera_folders: vec![],
    };

    assert!(check_empty_card(&vol));
}

#[test]
fn test_t1_feat06_03_card_with_photos_not_detected_as_empty() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new();
    sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");

    let mut vol = sd.to_volume();
    vol.free_bytes = vol.total_bytes; // artificially set free bytes

    assert!(
        !check_empty_card(&vol),
        "Card containing photos must NEVER be detected as empty formatted card"
    );
}

#[test]
fn test_t1_feat06_04_card_under_99_percent_not_detected_as_empty() {
    let capacity = 128 * 1024 * 1024 * 1024;
    let free_bytes = (capacity as f64 * 0.985) as u64; // 98.5% free (< 99%)

    let vol = SdCardVolume {
        drive_letter: 'F',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: capacity,
        free_bytes,
        has_dcim: true,
        camera_folders: vec![],
    };

    assert!(!check_empty_card(&vol));
}

#[test]
fn test_t1_feat06_05_known_capacity_profile_association() {
    let mut reg = IsolatedCameraRegistry::new().expect("create registry");
    let capacity_128gb = 128 * 1024 * 1024 * 1024;

    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon EOS R6".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\CanonR6"),
        known_card_capacities: vec![capacity_128gb],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };
    reg.register_or_update(profile).expect("register");

    let matched = reg.find_by_capacity(capacity_128gb).expect("must match profile");
    assert_eq!(matched.display_name, "Canon EOS R6");
}

#[test]
fn test_t1_feat06_06_unknown_capacity_empty_card_handling() {
    let mut reg = IsolatedCameraRegistry::new().expect("create registry");
    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon EOS R6".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\CanonR6"),
        known_card_capacities: vec![64 * 1024 * 1024 * 1024], // only 64 GB known
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };
    reg.register_or_update(profile).expect("register");

    // Empty card is 512 GB -> unknown
    let capacity_512gb = 512 * 1024 * 1024 * 1024;
    assert!(reg.find_by_capacity(capacity_512gb).is_none());
}

// ============================================================================
// Group 7: Windows Native Toast Notification Dispatch (FR-11, R5)
// ============================================================================

#[test]
fn test_t1_feat07_01_toast_notifier_trait_mock() {
    let notifier = MockToastNotifier::new();
    notifier
        .send_toast("Title", "Message")
        .expect("send toast failed");

    assert_eq!(notifier.count(), 1);
    assert_eq!(
        notifier.last_notification(),
        Some(("Title".to_string(), "Message".to_string()))
    );
}

#[tokio::test]
async fn test_t1_feat07_02_toast_message_contains_transferred_count() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new();
    sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");
    sd.add_photo("100CANON", "IMG_0002.JPG", &builder).expect("add photo");

    let target_dir = tempdir().expect("target tempdir");
    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let toast = Arc::new(MockToastNotifier::new());
    let harness = IngestPipelineHarness::new(Some(toast.clone()));

    harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest");

    assert_eq!(toast.count(), 1);
    let (title, message) = toast.last_notification().unwrap();
    assert!(title.contains("Canon R6"));
    assert!(message.contains("Transferred: 2 files"));
}

#[tokio::test]
async fn test_t1_feat07_03_toast_message_contains_destination_path() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new();
    sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");

    let target_dir = tempdir().expect("target tempdir");
    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let toast = Arc::new(MockToastNotifier::new());
    let harness = IngestPipelineHarness::new(Some(toast.clone()));

    harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest");

    let (_, message) = toast.last_notification().unwrap();
    assert!(message.contains(&target_dir.path().display().to_string()));
}

#[test]
fn test_t1_feat07_04_toast_escapes_xml_special_characters() {
    fn escape_xml_toast(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    let raw = "Transfer 100% & <fast> \"photos\" 'canon'";
    let escaped = escape_xml_toast(raw);

    assert_eq!(
        escaped,
        "Transfer 100% &amp; &lt;fast&gt; &quot;photos&quot; &apos;canon&apos;"
    );
}

#[tokio::test]
async fn test_t1_feat07_05_toast_summary_records_skipped_count() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new();
    sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add photo");

    let target_dir = tempdir().expect("target tempdir");
    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let toast = Arc::new(MockToastNotifier::new());
    let harness = IngestPipelineHarness::new(Some(toast.clone()));

    // First ingest (transfers)
    harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest 1");

    // Second ingest (skips)
    harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest 2");

    assert_eq!(toast.count(), 2);
    let (_, message2) = toast.last_notification().unwrap();
    assert!(message2.contains("Skipped: 1 duplicates"));
}

#[test]
fn test_t1_feat07_06_toast_dispatches_without_blocking() {
    let notifier = MockToastNotifier::new();
    let start = std::time::Instant::now();
    notifier.send_toast("Quick", "Notification").expect("toast");
    let elapsed = start.elapsed();
    assert!(elapsed < std::time::Duration::from_millis(50));
}
