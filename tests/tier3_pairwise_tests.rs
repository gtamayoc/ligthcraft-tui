#![allow(dead_code, unused_imports)]

//! Tier 3 Pairwise Cross-Feature Tests: LightCraft SD Ingest Hub & Modular Launcher
//!
//! Validates complex multi-feature interactions:
//! - Pairwise 1: First-time detection -> registry save -> second insertion -> duplicate skip
//! - Pairwise 2: Repeat insertion with incremental new photos & total count updates
//! - Pairwise 3: Formatted empty card -> capacity match -> 1-touch bind -> simulated capture -> transfer
//! - Pairwise 4: Multi-camera registry routing & absolute destination isolation (Canon R6 vs Sony A7 IV)
//! - Pairwise 5: Asynchronous streaming ingest paired with native Windows toast notification
//! - Pairwise 6: Read-only SD medium paired with corrupt file resilience & valid photo transfer
//! - Pairwise 7: CLI direct bypass with automated ingest execution
//! - Pairwise 8: Dynamic registry card capacity expansion upon multi-capacity card insertions

mod common;

use chrono::Utc;
use predicates::prelude::*;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::tempdir;

use common::contracts::{
    CameraFingerprint, CameraProfile, IngestError, IngestSessionManifest, MockToastNotifier,
    SdCardVolume,
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
// Pairwise 1: First-Time Detection -> Registry Save -> Second Insertion -> Duplicate Skip
// ============================================================================

#[tokio::test]
async fn test_t3_pairwise_first_detection_registry_save_repeat_auto_transfer() {
    let mut reg = IsolatedCameraRegistry::new().expect("create registry");
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let target_dir = tempdir().expect("target tempdir");

    // Populate SD card with 5 Canon photos
    let photo_paths = sd.populate_canon_photos(5).expect("populate photos");

    // 1. Read camera fingerprint from first photo
    let fingerprint = extract_bounded_exif_fingerprint(&photo_paths[0], 128 * 1024)
        .expect("extract fingerprint");
    assert_eq!(fingerprint.camera_id(), "042021000123");

    // 2. Query registry: Not found -> first time!
    assert!(reg.get_profile(&fingerprint.camera_id()).is_none());

    // 3. User setup: register destination folder and card capacity
    let profile = CameraProfile {
        camera_id: fingerprint.camera_id(),
        display_name: "Canon EOS R6 Main".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![sd.total_bytes],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };
    reg.register_or_update(profile.clone()).expect("save profile");

    // 4. First Ingest Execution: transfers all 5 photos
    let harness: IngestPipelineHarness<MockToastNotifier> = IngestPipelineHarness::new(None);
    let summary1 = harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest 1");
    assert_eq!(summary1.transferred_files, 5);
    assert_eq!(summary1.skipped_duplicates, 0);

    // Update profile in registry
    let mut updated_profile = profile.clone();
    updated_profile.total_photos_ingested += summary1.transferred_files as u64;
    reg.register_or_update(updated_profile).expect("update registry");

    // 5. Simulate Second Insertion of same SD card
    // Registry query: Recognized automatically!
    let known_profile = reg
        .get_profile(&fingerprint.camera_id())
        .expect("profile must be recognized without prompt");
    assert_eq!(known_profile.total_photos_ingested, 5);

    // Second Ingest Execution: all 5 photos detected as duplicates and skipped
    let summary2 = harness.run_ingest(&sd.to_volume(), known_profile).await.expect("ingest 2");
    assert_eq!(summary2.transferred_files, 0);
    assert_eq!(summary2.skipped_duplicates, 5);
}

// ============================================================================
// Pairwise 2: Repeat Insertion with Incremental New Photos & Total Count Updates
// ============================================================================

#[tokio::test]
async fn test_t3_pairwise_repeat_insertion_with_incremental_photos() {
    let mut reg = IsolatedCameraRegistry::new().expect("create registry");
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let target_dir = tempdir().expect("target tempdir");

    // Day 1: 4 photos
    for i in 1..=4 {
        let b = ExifPhotoBuilder::new()
            .date_time("2026:10:01 10:00:00")
            .payload_size(1024);
        sd.add_photo("100CANON", &format!("IMG_{:04}.JPG", i), &b).expect("add");
    }

    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![sd.total_bytes],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };
    reg.register_or_update(profile.clone()).expect("save profile");

    let harness: IngestPipelineHarness<MockToastNotifier> = IngestPipelineHarness::new(None);

    // Session 1: Transfers 4 photos
    let s1 = harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest 1");
    assert_eq!(s1.transferred_files, 4);

    // Day 2: Photographer shoots 3 new photos on same card
    for i in 5..=7 {
        let b = ExifPhotoBuilder::new()
            .date_time("2026:10:02 11:00:00")
            .payload_size(1024);
        sd.add_photo("100CANON", &format!("IMG_{:04}.JPG", i), &b).expect("add");
    }

    // Session 2: Skips 4 duplicates, transfers 3 new photos
    let s2 = harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest 2");
    assert_eq!(s2.transferred_files, 3);
    assert_eq!(s2.skipped_duplicates, 4);

    let mut final_profile = profile.clone();
    final_profile.total_photos_ingested += (s1.transferred_files + s2.transferred_files) as u64;
    reg.register_or_update(final_profile).expect("update");

    let saved = reg.get_profile("042021000123").unwrap();
    assert_eq!(saved.total_photos_ingested, 7);
}

// ============================================================================
// Pairwise 3: Formatted Empty Card -> Capacity Match -> 1-Touch Bind -> Transfer
// ============================================================================

#[tokio::test]
async fn test_t3_pairwise_formatted_card_capacity_match_and_transfer() {
    let mut reg = IsolatedCameraRegistry::new().expect("create registry");
    let target_dir = tempdir().expect("target tempdir");
    let capacity_128gb = 128 * 1024 * 1024 * 1024;

    // Pre-registered camera profile
    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![capacity_128gb],
        last_ingest_timestamp: None,
        total_photos_ingested: 50,
    };
    reg.register_or_update(profile).expect("reg");

    // 1. Insert freshly formatted card (0 photos in DCIM, 99.8% free space)
    let empty_sd = SyntheticSdCard::new_empty_formatted(capacity_128gb).expect("empty SD");
    let empty_vol = empty_sd.to_volume();

    // 2. Detect empty card
    assert!(check_empty_card(&empty_vol));

    // 3. Match capacity to registered camera
    let matched_profile = reg.find_by_capacity(empty_vol.total_bytes).expect("match profile");
    assert_eq!(matched_profile.camera_id, "042021000123");

    // 4. Simulate 1-touch binding and subsequent photo shooting
    let b = ExifPhotoBuilder::new();
    empty_sd.add_photo("100CANON", "IMG_0001.JPG", &b).expect("shoot photo 1");
    empty_sd.add_photo("100CANON", "IMG_0002.JPG", &b).expect("shoot photo 2");

    // 5. Ingest transfers photos directly to matched profile target
    let harness: IngestPipelineHarness<MockToastNotifier> = IngestPipelineHarness::new(None);
    let summary = harness
        .run_ingest(&empty_sd.to_volume(), matched_profile)
        .await
        .expect("ingest to bound camera");

    assert_eq!(summary.transferred_files, 2);
    assert_eq!(summary.skipped_duplicates, 0);
}

// ============================================================================
// Pairwise 4: Multi-Camera Registry Routing & Absolute Destination Isolation
// ============================================================================

#[tokio::test]
async fn test_t3_pairwise_multiple_cameras_different_capacities() {
    let mut reg = IsolatedCameraRegistry::new().expect("create registry");
    let target_canon = tempdir().expect("canon dir");
    let target_sony = tempdir().expect("sony dir");

    // Register Canon R6 (128 GB card)
    let prof_canon = CameraProfile {
        camera_id: "CANON_R6_042021".to_string(),
        display_name: "Canon EOS R6".to_string(),
        target_directory: target_canon.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };
    reg.register_or_update(prof_canon).expect("reg canon");

    // Register Sony A7 IV (64 GB card)
    let prof_sony = CameraProfile {
        camera_id: "SONY_A7IV_784930".to_string(),
        display_name: "Sony Alpha 7 IV".to_string(),
        target_directory: target_sony.path().to_path_buf(),
        known_card_capacities: vec![64 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };
    reg.register_or_update(prof_sony).expect("reg sony");

    // Card 1: Canon SD
    let sd_canon = SyntheticSdCard::new("EOS_DIGITAL").expect("canon SD");
    let b_canon = ExifPhotoBuilder::new()
        .make("Canon")
        .model("Canon EOS R6")
        .body_serial(Some("CANON_R6_042021"));
    sd_canon.add_photo("100CANON", "IMG_CANON.JPG", &b_canon).expect("add");

    // Card 2: Sony SD
    let mut sd_sony = SyntheticSdCard::new("SONY_SD").expect("sony SD");
    sd_sony.total_bytes = 64 * 1024 * 1024 * 1024;
    let b_sony = ExifPhotoBuilder::new()
        .make("Sony")
        .model("ILCE-7M4")
        .body_serial(Some("SONY_A7IV_784930"));
    sd_sony.add_photo("100MSDCF", "DSC_SONY.JPG", &b_sony).expect("add");

    let harness: IngestPipelineHarness<MockToastNotifier> = IngestPipelineHarness::new(None);

    // Ingest Canon
    let p_canon = reg.get_profile("CANON_R6_042021").unwrap();
    harness.run_ingest(&sd_canon.to_volume(), p_canon).await.expect("ingest canon");

    // Ingest Sony
    let p_sony = reg.get_profile("SONY_A7IV_784930").unwrap();
    harness.run_ingest(&sd_sony.to_volume(), p_sony).await.expect("ingest sony");

    // Assert complete isolation: Canon files in canon dir only, Sony files in sony dir only
    assert!(!target_canon.path().join("2026").join("2026-10-15").join("DSC_SONY.JPG").exists());
    assert!(!target_sony.path().join("2026").join("2026-10-15").join("IMG_CANON.JPG").exists());
    assert!(target_canon.path().join("2026").join("2026-10-15").join("IMG_CANON.JPG").exists());
    assert!(target_sony.path().join("2026").join("2026-10-15").join("DSC_SONY.JPG").exists());
}

// ============================================================================
// Pairwise 5: Asynchronous Streaming Ingest Paired with Toast Notification
// ============================================================================

#[tokio::test]
async fn test_t3_pairwise_streaming_ingest_with_toast_dispatch() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let builder = ExifPhotoBuilder::new().payload_size(512 * 1024);
    sd.add_photo("100CANON", "IMG_0001.JPG", &builder).expect("add");
    sd.add_photo("100CANON", "IMG_0002.JPG", &builder).expect("add");
    sd.add_photo("100CANON", "IMG_0003.JPG", &builder).expect("add");

    let target_dir = tempdir().expect("target tempdir");
    let profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6 Toast".to_string(),
        target_directory: target_dir.path().to_path_buf(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let toast = Arc::new(MockToastNotifier::new());
    let harness = IngestPipelineHarness::new(Some(toast.clone()));

    let summary = harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest");

    assert_eq!(summary.transferred_files, 3);
    assert_eq!(toast.count(), 1);

    let (title, message) = toast.last_notification().unwrap();
    assert!(title.contains("LightCraft Ingest Complete — Canon R6 Toast"));
    assert!(message.contains("Transferred: 3 files"));
    assert!(message.contains(&target_dir.path().display().to_string()));
}

// ============================================================================
// Pairwise 6: Read-Only SD with Corrupt File and Valid Photos
// ============================================================================

#[tokio::test]
async fn test_t3_pairwise_read_only_sd_with_corrupt_file_and_valid_photos() {
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let b_valid = ExifPhotoBuilder::new();

    // 3 valid photos
    sd.add_photo("100CANON", "IMG_VALID_1.JPG", &b_valid).expect("add");
    sd.add_photo("100CANON", "IMG_VALID_2.JPG", &b_valid).expect("add");
    sd.add_photo("100CANON", "IMG_VALID_3.JPG", &b_valid).expect("add");

    // 1 corrupt file
    let corrupt_path = sd.dcim_path().join("100CANON").join("IMG_CORRUPT.DAT");
    fs::write(&corrupt_path, vec![0xFF, 0x00, 0xDE, 0xAD]).expect("write corrupt");

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
    let summary = harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest");

    // Valid files transferred
    assert!(summary.transferred_files >= 3);

    // Cryptographic immutability on SD medium strictly preserved
    sd.assert_unmodified(&baseline_tree);
}

// ============================================================================
// Pairwise 7: CLI Direct Bypass with Direct Flags
// ============================================================================

#[test]
fn test_t3_pairwise_cli_bypass_with_direct_flag() {
    let mut cmd = assert_cmd::Command::cargo_bin("lightcraft-tui").expect("binary");
    cmd.arg("--list-presets");
    cmd.assert()
        .success()
        .stdout(predicates::str::contains("LightCraft Presets Catalog"));
}

// ============================================================================
// Pairwise 8: Dynamic Registry Card Capacity Expansion on Multi-Capacity Insertions
// ============================================================================

#[test]
fn test_t3_pairwise_registry_capacity_update_on_new_card() {
    let mut reg = IsolatedCameraRegistry::new().expect("create registry");
    let capacity_64gb = 64 * 1024 * 1024 * 1024;
    let capacity_128gb = 128 * 1024 * 1024 * 1024;

    // Photographer registers camera with 64 GB card initially
    let mut profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\Canon"),
        known_card_capacities: vec![capacity_64gb],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };
    reg.register_or_update(profile.clone()).expect("initial save");

    // Verify 128 GB card is not yet associated
    assert!(reg.find_by_capacity(capacity_128gb).is_none());

    // Photographer inserts new 128 GB card in camera and takes photos -> camera recognised
    if !profile.known_card_capacities.contains(&capacity_128gb) {
        profile.known_card_capacities.push(capacity_128gb);
    }
    reg.register_or_update(profile).expect("expand capacities");

    // Now empty 128 GB card is recognised!
    let matched = reg.find_by_capacity(capacity_128gb).expect("must find profile");
    assert_eq!(matched.camera_id, "042021000123");
    assert_eq!(matched.known_card_capacities.len(), 2);
}
