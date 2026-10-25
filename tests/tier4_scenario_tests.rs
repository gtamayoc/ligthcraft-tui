#![allow(dead_code, unused_imports)]

//! Tier 4 Real-World Scenario Tests: LightCraft SD Ingest Hub & Modular Launcher
//!
//! Realistic end-to-end user workflows:
//! - Scenario 1: Photographer inserts Canon R6 SD card, auto-ingests 50 photos,
//!   verifies date organization, SHA-256 manifest, zero SD modifications, and toast.
//! - Scenario 2: Photographer returns on day 2 with 20 incremental new photos;
//!   system auto-detects profile, skips 50 duplicates, transfers 20, updates registry.
//! - Scenario 3: Photographer formats card in-camera; system identifies empty card,
//!   matches known capacity, 1-touch binds, and completes workflow upon photo capture.
//! - Scenario 4: Multi-camera event workflow (Canon R6 sports + Sony A7 IV portraits)
//!   with dual registry routing and absolute folder isolation.

mod common;

use chrono::Utc;
use std::collections::HashMap;
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
    check_empty_card, compute_file_sha256, IngestPipelineHarness,
};
use common::registry_fixture::IsolatedCameraRegistry;
use common::sd_fixture::SyntheticSdCard;

// ============================================================================
// Scenario 1: Photographer Canon R6 50 Photos End-to-End Ingest
// ============================================================================

#[tokio::test]
async fn test_t4_scenario_photographer_canon_r6_50_photos_ingest() {
    let mut registry = IsolatedCameraRegistry::new().expect("create registry");
    let target_archive = tempdir().expect("target archive tempdir");
    let toast = Arc::new(MockToastNotifier::new());

    // 1. Photographer inserts 128 GB SD card from Canon EOS R6
    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");
    let initial_photo_paths = sd.populate_canon_photos(50).expect("populate 50 photos");
    assert_eq!(initial_photo_paths.len(), 50);

    // Snapshot SD card before ingest for strict read-only immutability verification
    let sd_baseline_tree = sd.compute_sha256_tree().expect("SD baseline tree");
    assert_eq!(sd_baseline_tree.len(), 50);

    // 2. System identifies camera from EXIF metadata in < 100ms
    let start_fingerprint = std::time::Instant::now();
    let fingerprint = extract_bounded_exif_fingerprint(&initial_photo_paths[0], 128 * 1024)
        .expect("extract camera fingerprint");
    let elapsed_fingerprint = start_fingerprint.elapsed();
    assert!(
        elapsed_fingerprint < std::time::Duration::from_millis(100),
        "EXIF extraction must complete in < 100ms"
    );

    assert_eq!(fingerprint.make, "Canon");
    assert_eq!(fingerprint.model, "Canon EOS R6");
    assert_eq!(fingerprint.body_serial.as_deref(), Some("042021000123"));
    let camera_id = fingerprint.camera_id();
    assert_eq!(camera_id, "042021000123");

    // 3. First-time detection: Not in registry -> prompts user for target path
    assert!(registry.get_profile(&camera_id).is_none());

    let canon_target_dir = target_archive.path().join("Photos_Canon_R6");
    let profile = CameraProfile {
        camera_id: camera_id.clone(),
        display_name: "Canon EOS R6 — Juan".to_string(),
        target_directory: canon_target_dir.clone(),
        known_card_capacities: vec![sd.total_bytes],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };
    registry.register_or_update(profile.clone()).expect("save profile");

    // 4. Ingest Pipeline runs: 1 MB streaming copy, SHA-256 in-flight, date organization
    let harness = IngestPipelineHarness::new(Some(toast.clone()));
    let summary = harness
        .run_ingest(&sd.to_volume(), &profile)
        .await
        .expect("ingest execution failed");

    // 5. Verification: Transfer counts
    assert_eq!(summary.transferred_files, 50);
    assert_eq!(summary.skipped_duplicates, 0);
    assert!(summary.total_bytes > 0);

    // 6. Verification: Manifest audit trail
    assert!(summary.manifest_path.exists());
    let manifest_str = fs::read_to_string(&summary.manifest_path).expect("read manifest");
    let manifest: IngestSessionManifest = serde_json::from_str(&manifest_str).expect("parse manifest");
    assert_eq!(manifest.camera_id, "042021000123");
    assert_eq!(manifest.total_transferred, 50);
    assert_eq!(manifest.files.len(), 50);

    // Verify all 50 files exist in target date hierarchy and match SHA-256
    for entry in &manifest.files {
        let dest_file_path = canon_target_dir.join(&entry.relative_path);
        assert!(
            dest_file_path.exists(),
            "Destination file {:?} must exist on disk",
            dest_file_path
        );
        let actual_sha = compute_file_sha256(&dest_file_path).expect("compute sha");
        assert_eq!(
            entry.sha256, actual_sha,
            "Manifest SHA-256 must match file on disk"
        );
    }

    // 7. Verification: STRICT 100% Read-Only SD Protocol Invariant
    sd.assert_unmodified(&sd_baseline_tree);

    // 8. Verification: Native Windows Toast Notification
    assert_eq!(toast.count(), 1);
    let (toast_title, toast_body) = toast.last_notification().unwrap();
    assert!(toast_title.contains("Canon EOS R6 — Juan"));
    assert!(toast_body.contains("Transferred: 50 files"));
    assert!(toast_body.contains(&canon_target_dir.display().to_string()));
}

// ============================================================================
// Scenario 2: Photographer Day 2 Incremental Ingest (Auto-Recognize & Skip Duplicates)
// ============================================================================

#[tokio::test]
async fn test_t4_scenario_photographer_second_day_incremental_ingest() {
    let mut registry = IsolatedCameraRegistry::new().expect("create registry");
    let target_archive = tempdir().expect("target archive tempdir");
    let toast = Arc::new(MockToastNotifier::new());
    let canon_target_dir = target_archive.path().join("Photos_Canon_R6");

    // Pre-seed registry from previous day
    let mut profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon EOS R6".to_string(),
        target_directory: canon_target_dir.clone(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: Some(Utc::now()),
        total_photos_ingested: 50,
    };
    registry.register_or_update(profile.clone()).expect("seed registry");

    let sd = SyntheticSdCard::new("EOS_DIGITAL").expect("synthetic SD");

    // Populate initial 50 photos (simulating previously ingested files)
    sd.populate_canon_photos(50).expect("populate 50");

    let harness = IngestPipelineHarness::new(Some(toast.clone()));

    // Run first ingest to establish destination baseline
    harness.run_ingest(&sd.to_volume(), &profile).await.expect("ingest initial");

    // Photographer shoots 20 new photos on Day 2 in folder 101CANON
    for i in 51..=70 {
        let b = ExifPhotoBuilder::new()
            .make("Canon")
            .model("Canon EOS R6")
            .body_serial(Some("042021000123"))
            .date_time("2026:10:16 11:00:00")
            .payload_size(2048)
            .as_raw();
        sd.add_photo("101CANON", &format!("IMG_{:04}.CR3", i), &b).expect("add day 2 photo");
    }

    let sd_baseline_tree = sd.compute_sha256_tree().expect("baseline tree");
    assert_eq!(sd_baseline_tree.len(), 70);

    // Insert card on Day 2: System queries registry -> automatically recognized!
    let known_profile = registry
        .get_profile("042021000123")
        .expect("must be recognized automatically");
    assert_eq!(known_profile.total_photos_ingested, 50);

    // Run unattended ingest
    let summary2 = harness
        .run_ingest(&sd.to_volume(), known_profile)
        .await
        .expect("day 2 ingest");

    // Verifies: exactly 20 new photos transferred, 50 duplicates skipped
    assert_eq!(summary2.transferred_files, 20);
    assert_eq!(summary2.skipped_duplicates, 50);

    // Update registry total count
    profile.total_photos_ingested += summary2.transferred_files as u64;
    registry.register_or_update(profile).expect("update count");
    assert_eq!(
        registry.get_profile("042021000123").unwrap().total_photos_ingested,
        70
    );

    // Toast notifies 20 transferred, 50 skipped
    let (_, toast_body) = toast.last_notification().unwrap();
    assert!(toast_body.contains("Transferred: 20 files"));
    assert!(toast_body.contains("Skipped: 50 duplicates"));

    // SD medium strictly unmodified
    sd.assert_unmodified(&sd_baseline_tree);
}

// ============================================================================
// Scenario 3: Photographer Formats Card in Field & Re-Associates with 1 Touch
// ============================================================================

#[tokio::test]
async fn test_t4_scenario_photographer_formatted_card_in_field() {
    let mut registry = IsolatedCameraRegistry::new().expect("create registry");
    let target_archive = tempdir().expect("tempdir");
    let capacity_64gb = 64 * 1024 * 1024 * 1024;

    let profile = CameraProfile {
        camera_id: "CANON_R6_FIELD".to_string(),
        display_name: "Canon R6 Field".to_string(),
        target_directory: target_archive.path().join("Field_Shoot"),
        known_card_capacities: vec![capacity_64gb],
        last_ingest_timestamp: None,
        total_photos_ingested: 100,
    };
    registry.register_or_update(profile).expect("reg");

    // 1. Photographer formats 64 GB card in-camera
    let formatted_sd = SyntheticSdCard::new_empty_formatted(capacity_64gb).expect("empty SD");
    let vol = formatted_sd.to_volume();

    // 2. Empty card detector validates capacity within 99% margin and 0 photos
    assert!(check_empty_card(&vol));

    // 3. System matches capacity against registered profile
    let matched = registry.find_by_capacity(vol.total_bytes).expect("match capacity");
    assert_eq!(matched.camera_id, "CANON_R6_FIELD");

    // 4. User presses [Enter] (1-touch binding confirmation)
    // Photographer captures 5 photos on the freshly bound card
    for i in 1..=5 {
        let b = ExifPhotoBuilder::new()
            .make("Canon")
            .model("Canon EOS R6")
            .body_serial(Some("CANON_R6_FIELD"));
        formatted_sd
            .add_photo("100CANON", &format!("IMG_{:04}.JPG", i), &b)
            .expect("capture");
    }

    // 5. Ingest transfers photos directly to the bound profile
    let harness: IngestPipelineHarness<MockToastNotifier> = IngestPipelineHarness::new(None);
    let summary = harness
        .run_ingest(&formatted_sd.to_volume(), matched)
        .await
        .expect("ingest");

    assert_eq!(summary.transferred_files, 5);
    assert_eq!(summary.skipped_duplicates, 0);
}

// ============================================================================
// Scenario 4: Multi-Camera Event Workflow (Canon Sports + Sony Portraits)
// ============================================================================

#[tokio::test]
async fn test_t4_scenario_multi_camera_event_photographer_workflow() {
    let mut registry = IsolatedCameraRegistry::new().expect("create registry");
    let event_root = tempdir().expect("event root");
    let toast = Arc::new(MockToastNotifier::new());

    let sports_dir = event_root.path().join("Sports_Action_Canon");
    let portraits_dir = event_root.path().join("Portraits_Sony");

    // Register Canon Sports
    let canon_profile = CameraProfile {
        camera_id: "042021000123".to_string(),
        display_name: "Canon R6 (Sports)".to_string(),
        target_directory: sports_dir.clone(),
        known_card_capacities: vec![128 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };
    registry.register_or_update(canon_profile).expect("reg canon");

    // Register Sony Portraits
    let sony_profile = CameraProfile {
        camera_id: "7849302199".to_string(),
        display_name: "Sony A7IV (Portraits)".to_string(),
        target_directory: portraits_dir.clone(),
        known_card_capacities: vec![64 * 1024 * 1024 * 1024],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };
    registry.register_or_update(sony_profile).expect("reg sony");

    // Build Canon Sports Card (15 photos)
    let sd_canon = SyntheticSdCard::new("EOS_DIGITAL").expect("canon SD");
    for i in 1..=15 {
        let b = ExifPhotoBuilder::new()
            .make("Canon")
            .model("Canon EOS R6")
            .body_serial(Some("042021000123"))
            .date_time("2026:10:20 09:30:00")
            .as_raw();
        sd_canon.add_photo("100CANON", &format!("SPORT_{:03}.CR3", i), &b).expect("add");
    }

    // Build Sony Portraits Card (12 photos)
    let mut sd_sony = SyntheticSdCard::new("SONY_SD").expect("sony SD");
    sd_sony.total_bytes = 64 * 1024 * 1024 * 1024;
    for i in 1..=12 {
        let b = ExifPhotoBuilder::new()
            .make("Sony")
            .model("ILCE-7M4")
            .body_serial(Some("7849302199"))
            .date_time("2026:10:20 14:15:00");
        sd_sony.add_photo("100MSDCF", &format!("PORTRAIT_{:03}.JPG", i), &b).expect("add");
    }

    let harness = IngestPipelineHarness::new(Some(toast.clone()));

    // Ingest Canon Sports SD
    let p_canon = registry.get_profile("042021000123").unwrap();
    let sum_canon = harness.run_ingest(&sd_canon.to_volume(), p_canon).await.expect("ingest canon");
    assert_eq!(sum_canon.transferred_files, 15);

    // Ingest Sony Portraits SD
    let p_sony = registry.get_profile("7849302199").unwrap();
    let sum_sony = harness.run_ingest(&sd_sony.to_volume(), p_sony).await.expect("ingest sony");
    assert_eq!(sum_sony.transferred_files, 12);

    // Verify folder isolation
    assert!(sports_dir.exists());
    assert!(portraits_dir.exists());

    let canon_sample = sports_dir.join("2026").join("2026-10-20").join("SPORT_001.CR3");
    let sony_sample = portraits_dir.join("2026").join("2026-10-20").join("PORTRAIT_001.JPG");
    assert!(canon_sample.exists());
    assert!(sony_sample.exists());

    // Cross-contamination check: Ensure no Sony photos in Sports and no Canon photos in Portraits
    let wrong_canon_in_portraits = portraits_dir.join("2026").join("2026-10-20").join("SPORT_001.CR3");
    let wrong_sony_in_sports = sports_dir.join("2026").join("2026-10-20").join("PORTRAIT_001.JPG");
    assert!(!wrong_canon_in_portraits.exists());
    assert!(!wrong_sony_in_sports.exists());

    // Both toasts dispatched
    assert_eq!(toast.count(), 2);
}
