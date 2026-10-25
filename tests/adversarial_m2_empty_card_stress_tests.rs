//! Milestone 2 Challenger 1: Adversarial Stress Test Suite
//! Target: `is_empty_formatted_card`, `check_empty_card`, `matches_capacity`,
//! `find_matching_camera_by_capacity`, `find_all_matching_cameras_by_capacity`,
//! and `format_card_capacity`.
//!
//! Authoritative Requirements:
//! - US-007 & FR-10: Formatted empty card detection (`free_bytes >= 99%` and 0 media assets in DCIM).
//! - 1-touch profile re-association based on physical capacity matching within 2% tolerance.

use std::fs::{self, File};
use std::path::PathBuf;
use chrono::{Duration, Utc};
use tempfile::tempdir;

use lightcraft_tui::ingest::empty_handler::{
    check_empty_card, find_all_matching_cameras_by_capacity, find_matching_camera_by_capacity,
    format_card_capacity, is_empty_formatted_card, matches_capacity, CameraProfile,
};
use lightcraft_tui::ingest::scanner::SdCardVolume;

// ============================================================================
// Section 1: Boundary & Threshold Stress Tests on `is_empty_formatted_card`
// ============================================================================

#[test]
fn test_boundary_exact_99_percent_free_space() {
    let capacity: u64 = 100_000_000_000; // 100 GB
    let vol = SdCardVolume {
        drive_letter: 'G',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: capacity,
        free_bytes: 99_000_000_000, // Exactly 99.000%
        has_dcim: true,
        camera_folders: vec![],
    };

    assert!(
        is_empty_formatted_card(&vol),
        "Card with exactly 99.0% free space MUST be detected as empty formatted card"
    );
    assert!(check_empty_card(&vol));
}

#[test]
fn test_boundary_98_99_percent_free_space_rejected() {
    let capacity: u64 = 100_000_000_000; // 100 GB
    let vol = SdCardVolume {
        drive_letter: 'G',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: capacity,
        free_bytes: 98_990_000_000, // 98.99% free space
        has_dcim: true,
        camera_folders: vec![],
    };

    assert!(
        !is_empty_formatted_card(&vol),
        "Card with 98.99% free space (< 99.0%) MUST be rejected"
    );
    assert!(!check_empty_card(&vol));
}

#[test]
fn test_boundary_98_999_percent_free_space_rejected() {
    let capacity: u64 = 100_000_000_000;
    let vol = SdCardVolume {
        drive_letter: 'G',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: capacity,
        free_bytes: 98_999_000_000, // 98.999%
        has_dcim: true,
        camera_folders: vec![],
    };

    assert!(
        !is_empty_formatted_card(&vol),
        "Card with 98.999% free space (< 99.0%) MUST be rejected"
    );
}

#[test]
fn test_boundary_100_percent_free_space_accepted() {
    let capacity: u64 = 128 * 1024 * 1024 * 1024;
    let vol = SdCardVolume {
        drive_letter: 'E',
        volume_label: "SONY_SD".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: capacity,
        free_bytes: capacity, // 100.0% free
        has_dcim: true,
        camera_folders: vec![],
    };

    assert!(
        is_empty_formatted_card(&vol),
        "Card with 100.0% free space MUST be detected as empty formatted card"
    );
}

#[test]
fn test_boundary_99_001_percent_free_space_accepted() {
    let capacity: u64 = 100_000_000_000;
    let vol = SdCardVolume {
        drive_letter: 'G',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: capacity,
        free_bytes: 99_001_000_000, // 99.001%
        has_dcim: true,
        camera_folders: vec![],
    };

    assert!(
        is_empty_formatted_card(&vol),
        "Card with 99.001% free space (> 99.0%) MUST be accepted"
    );
}

#[test]
fn test_boundary_zero_and_inverted_capacity_graceful() {
    // Total bytes = 0 -> must return false without panic
    let zero_vol = SdCardVolume {
        drive_letter: 'H',
        volume_label: "CORRUPT_ZERO".to_string(),
        file_system: "RAW".to_string(),
        total_bytes: 0,
        free_bytes: 0,
        has_dcim: false,
        camera_folders: vec![],
    };
    assert!(!is_empty_formatted_card(&zero_vol));

    // Free bytes > Total bytes -> must return false without panic
    let inverted_vol = SdCardVolume {
        drive_letter: 'H',
        volume_label: "CORRUPT_INVERTED".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 50_000_000_000,
        free_bytes: 75_000_000_000,
        has_dcim: true,
        camera_folders: vec![],
    };
    assert!(!is_empty_formatted_card(&inverted_vol));

    // Free bytes = 0, Total bytes > 0 -> must return false
    let zero_free_vol = SdCardVolume {
        drive_letter: 'H',
        volume_label: "FULL".to_string(),
        file_system: "FAT32".to_string(),
        total_bytes: 64 * 1024 * 1024 * 1024,
        free_bytes: 0,
        has_dcim: true,
        camera_folders: vec![],
    };
    assert!(!is_empty_formatted_card(&zero_free_vol));
}

#[test]
fn test_boundary_extreme_sizes_and_overflow_safety() {
    // 1-byte volume
    let tiny_1 = SdCardVolume {
        drive_letter: 'T',
        volume_label: "TINY1".to_string(),
        file_system: "FAT".to_string(),
        total_bytes: 1,
        free_bytes: 1, // 100%
        has_dcim: false,
        camera_folders: vec![],
    };
    assert!(is_empty_formatted_card(&tiny_1));

    let tiny_0 = SdCardVolume {
        drive_letter: 'T',
        volume_label: "TINY0".to_string(),
        file_system: "FAT".to_string(),
        total_bytes: 1,
        free_bytes: 0, // 0%
        has_dcim: false,
        camera_folders: vec![],
    };
    assert!(!is_empty_formatted_card(&tiny_0));

    // 100-byte volume: 99 bytes free is exact 99%, 98 bytes free is 98%
    let vol_100_99 = SdCardVolume {
        drive_letter: 'T',
        volume_label: "V100".to_string(),
        file_system: "FAT".to_string(),
        total_bytes: 100,
        free_bytes: 99,
        has_dcim: false,
        camera_folders: vec![],
    };
    assert!(is_empty_formatted_card(&vol_100_99));

    let vol_100_98 = SdCardVolume {
        drive_letter: 'T',
        volume_label: "V100".to_string(),
        file_system: "FAT".to_string(),
        total_bytes: 100,
        free_bytes: 98,
        has_dcim: false,
        camera_folders: vec![],
    };
    assert!(!is_empty_formatted_card(&vol_100_98));

    // Massive 2 TB SDXC Card
    let capacity_2tb = 2 * 1024 * 1024 * 1024 * 1024;
    let vol_2tb = SdCardVolume {
        drive_letter: 'Z',
        volume_label: "SDXC_2TB".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: capacity_2tb,
        free_bytes: (capacity_2tb as f64 * 0.995) as u64,
        has_dcim: true,
        camera_folders: vec![],
    };
    assert!(is_empty_formatted_card(&vol_2tb));

    // Maximum u64 capacity
    let vol_max = SdCardVolume {
        drive_letter: 'Z',
        volume_label: "MAX".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: u64::MAX,
        free_bytes: u64::MAX,
        has_dcim: false,
        camera_folders: vec![],
    };
    assert!(is_empty_formatted_card(&vol_max));
}

#[test]
fn test_granular_sweep_around_99_percent_threshold() {
    let total: u64 = 1_000_000_000; // 1 GB in units of 1 byte (1,000,000 steps per 0.1%)

    // Test 100 discrete steps across [98.5%, 99.5%]
    for step in 9850..=9950 {
        let free = (total as f64 * (step as f64 / 10000.0)).round() as u64;
        let vol = SdCardVolume {
            drive_letter: 'E',
            volume_label: "SWEEP".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: total,
            free_bytes: free,
            has_dcim: true,
            camera_folders: vec![],
        };

        let expected_empty = (free as f64 / total as f64) >= 0.99;
        assert_eq!(
            is_empty_formatted_card(&vol),
            expected_empty,
            "Granular step {} (free={}, total={}) mismatch",
            step,
            free,
            total
        );
    }
}

// ============================================================================
// Section 2: Non-Photo Trash Files & Filesystem Tree Stress
// ============================================================================

#[test]
fn test_non_photo_camera_catalog_and_system_trash_files() {
    let dir = tempdir().expect("create tempdir");
    let camera_dir = dir.path().join("100CANON");
    fs::create_dir_all(&camera_dir).expect("create camera folder");

    // Canon catalog files
    File::create(camera_dir.join("CANON.CTG")).expect("create CANON.CTG");
    File::create(camera_dir.join("INDEX.CTG")).expect("create INDEX.CTG");

    // Nikon catalog and parameter files
    File::create(camera_dir.join("NIKON.DAT")).expect("create NIKON.DAT");
    File::create(camera_dir.join("NC_PARAM.DAT")).expect("create NC_PARAM.DAT");

    // Sony index and management files
    File::create(camera_dir.join("SONYCARD.IND")).expect("create SONYCARD.IND");
    File::create(camera_dir.join("AVIN0001.INP")).expect("create AVIN0001.INP");
    File::create(camera_dir.join("AVIN0001.BNP")).expect("create AVIN0001.BNP");
    File::create(camera_dir.join("MEDIANO.BIN")).expect("create MEDIANO.BIN");

    // OS trash & desktop metadata
    File::create(camera_dir.join(".DS_Store")).expect("create .DS_Store");
    File::create(camera_dir.join("Thumbs.db")).expect("create Thumbs.db");
    File::create(camera_dir.join("desktop.ini")).expect("create desktop.ini");
    File::create(camera_dir.join("AUTORUN.INF")).expect("create AUTORUN.INF");
    File::create(camera_dir.join("README.TXT")).expect("create README.TXT");

    let vol = SdCardVolume {
        drive_letter: 'F',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: (128.0 * 1024.0 * 1024.0 * 1024.0 * 0.998) as u64,
        has_dcim: true,
        camera_folders: vec![camera_dir.clone()],
    };

    assert_eq!(
        vol.media_count(),
        0,
        "Catalog, index, and OS files MUST NOT count as photographic media"
    );
    assert!(
        is_empty_formatted_card(&vol),
        "Card with non-photo trash files and 0 media MUST detect as empty formatted card"
    );
}

#[test]
fn test_formatted_card_with_single_photo_fails_empty_check() {
    let photo_extensions = [
        "CR3", "cr3", "CR2", "cr2", "ARW", "arw", "NEF", "nef", "DNG", "dng",
        "JPG", "jpg", "JPEG", "jpeg", "RAF", "raf", "RW2", "rw2", "ORF", "orf",
        "MP4", "mp4", "MOV", "mov",
    ];

    for ext in photo_extensions {
        let dir = tempdir().expect("create tempdir");
        let camera_dir = dir.path().join("100CANON");
        fs::create_dir_all(&camera_dir).expect("create camera folder");

        // Write non-photo catalog
        File::create(camera_dir.join("INDEX.CTG")).expect("create CTG");

        // Write a single photo asset with extension `ext`
        let photo_file = camera_dir.join(format!("SAMPLE.{}", ext));
        File::create(&photo_file).expect("create photo asset");

        let vol = SdCardVolume {
            drive_letter: 'E',
            volume_label: "TEST".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: 128 * 1024 * 1024 * 1024,
            free_bytes: 128 * 1024 * 1024 * 1024, // 100% free by byte calculation
            has_dcim: true,
            camera_folders: vec![camera_dir],
        };

        assert!(
            vol.media_count() >= 1,
            "Extension .{} must count as media",
            ext
        );
        assert!(
            !is_empty_formatted_card(&vol),
            "Card containing photo asset .{} MUST NOT be classified as empty",
            ext
        );
    }
}

#[test]
fn test_zero_byte_photo_asset_prevents_empty_classification() {
    let dir = tempdir().expect("create tempdir");
    let camera_dir = dir.path().join("100CANON");
    fs::create_dir_all(&camera_dir).expect("create camera folder");

    // Zero-byte RAW photo
    let raw_path = camera_dir.join("EMPTY_SHOT.CR3");
    File::create(&raw_path).expect("create empty raw file");
    assert_eq!(fs::metadata(&raw_path).unwrap().len(), 0);

    let vol = SdCardVolume {
        drive_letter: 'F',
        volume_label: "EOS".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 64 * 1024 * 1024 * 1024,
        free_bytes: 64 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![camera_dir],
    };

    assert_eq!(vol.media_count(), 1);
    assert!(
        !is_empty_formatted_card(&vol),
        "A zero-byte photo asset in camera folders must count as a photo and prevent empty card detection"
    );
}

#[test]
fn test_multiple_camera_folders_and_deep_subdirectories() {
    let dir = tempdir().expect("create tempdir");
    let dcim = dir.path().join("DCIM");
    let cam1 = dcim.join("100CANON");
    let cam2 = dcim.join("101CANON");
    let deep_sub = cam1.join("SUBDIR").join("ANOTHER");
    fs::create_dir_all(&deep_sub).expect("create deep dirs");
    fs::create_dir_all(&cam2).expect("create cam2");

    // Only non-photo trash across all levels
    File::create(deep_sub.join("DEBUG.LOG")).expect("create log");
    File::create(cam1.join("CANON.CTG")).expect("create CTG");
    File::create(cam2.join(".DS_Store")).expect("create DS_Store");

    let vol = SdCardVolume {
        drive_letter: 'G',
        volume_label: "EOS".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 64 * 1024 * 1024 * 1024,
        free_bytes: 64 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![cam1, cam2],
    };

    assert_eq!(vol.media_count(), 0);
    assert!(is_empty_formatted_card(&vol));
}

#[test]
fn test_nonexistent_or_unreadable_camera_folders_safe() {
    let vol = SdCardVolume {
        drive_letter: 'X',
        volume_label: "MOCK".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 128 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![
            PathBuf::from(r"X:\DCIM\NON_EXISTENT_FOLDER_1"),
            PathBuf::from(r"X:\DCIM\NON_EXISTENT_FOLDER_2"),
        ],
    };

    assert_eq!(vol.media_count(), 0);
    assert!(is_empty_formatted_card(&vol));
}

// ============================================================================
// Section 3: Capacity Matching Tolerance (`matches_capacity`) Boundary Stress
// ============================================================================

#[test]
fn test_matches_capacity_exact_and_tolerance_boundaries() {
    let capacities = [
        32 * 1024 * 1024 * 1024,  // 32 GB
        64 * 1024 * 1024 * 1024,  // 64 GB
        128 * 1024 * 1024 * 1024, // 128 GB
        256 * 1024 * 1024 * 1024, // 256 GB
        512 * 1024 * 1024 * 1024, // 512 GB
        1024 * 1024 * 1024 * 1024, // 1 TB
    ];

    for &registered in &capacities {
        let tolerance = registered / 50; // Exact 2% (registered / 50)

        // Exact match
        assert!(
            matches_capacity(registered, registered),
            "Exact capacity {} must match",
            registered
        );

        // Exactly +2.0% (upper boundary)
        let upper_bound = registered + tolerance;
        assert!(
            matches_capacity(registered, upper_bound),
            "Capacity at exact +2.0% boundary must match"
        );

        // Exactly -2.0% (lower boundary)
        let lower_bound = registered - tolerance;
        assert!(
            matches_capacity(registered, lower_bound),
            "Capacity at exact -2.0% boundary must match"
        );

        // Just outside upper boundary (+2.0% + 1 byte) -> false
        let outside_upper = registered + tolerance + 1;
        assert!(
            !matches_capacity(registered, outside_upper),
            "Capacity just outside +2.0% boundary (+1 byte) must NOT match"
        );

        // Just outside lower boundary (-2.0% - 1 byte) -> false
        let outside_lower = registered - tolerance - 1;
        assert!(
            !matches_capacity(registered, outside_lower),
            "Capacity just outside -2.0% boundary (-1 byte) must NOT match"
        );

        // Far outside (+5%, -5%, +50%)
        let plus_5_pct = registered + (registered / 20);
        let minus_5_pct = registered - (registered / 20);
        assert!(!matches_capacity(registered, plus_5_pct));
        assert!(!matches_capacity(registered, minus_5_pct));
    }
}

#[test]
fn test_matches_capacity_zeros_and_extremes() {
    assert!(!matches_capacity(0, 1000));
    assert!(!matches_capacity(1000, 0));
    assert!(!matches_capacity(0, 0));

    // Under 50 bytes: registered / 50 == 0, so only exact match works
    assert!(matches_capacity(49, 49));
    assert!(!matches_capacity(49, 48));
    assert!(!matches_capacity(49, 50));

    // Exactly 50 bytes: 50 / 50 == 1
    assert!(matches_capacity(50, 49));
    assert!(matches_capacity(50, 50));
    assert!(matches_capacity(50, 51));
    assert!(!matches_capacity(50, 48));
    assert!(!matches_capacity(50, 52));

    // Extreme u64 values
    assert!(matches_capacity(u64::MAX, u64::MAX));
    assert!(!matches_capacity(u64::MAX, 1));
    assert!(!matches_capacity(1, u64::MAX));
}

// ============================================================================
// Section 4: Multi-Camera Capacity Matching & Priority Ranking Stress
// ============================================================================

#[test]
fn test_find_matching_camera_capacity_diff_priority_over_timestamp() {
    let cap_target = 100_000_000_000; // 100 GB
    let newer_time = Utc::now();
    let older_time = Utc::now() - Duration::days(30);

    // Profile A has a larger difference (1% diff = 1 GB), but newer timestamp
    let profile_a = CameraProfile {
        camera_id: "CAM_A_NEWER_LARGER_DIFF".to_string(),
        display_name: "Camera A (1% diff, Newer)".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\A"),
        known_card_capacities: vec![101_000_000_000],
        last_ingest_timestamp: Some(newer_time),
        total_photos_ingested: 500,
    };

    // Profile B has an exact match (0 diff), but older timestamp
    let profile_b = CameraProfile {
        camera_id: "CAM_B_OLDER_EXACT_DIFF".to_string(),
        display_name: "Camera B (0 diff, Older)".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\B"),
        known_card_capacities: vec![100_000_000_000],
        last_ingest_timestamp: Some(older_time),
        total_photos_ingested: 50,
    };

    // Forward order: [A, B]
    let profiles_fwd = vec![profile_a.clone(), profile_b.clone()];
    let matched_fwd = find_matching_camera_by_capacity(cap_target, &profiles_fwd)
        .expect("must find match");
    assert_eq!(
        matched_fwd.camera_id, "CAM_B_OLDER_EXACT_DIFF",
        "Smallest capacity difference MUST take strict priority over timestamp"
    );

    // Reverse order: [B, A] (Permutation invariance)
    let profiles_rev = vec![profile_b, profile_a];
    let matched_rev = find_matching_camera_by_capacity(cap_target, &profiles_rev)
        .expect("must find match");
    assert_eq!(
        matched_rev.camera_id, "CAM_B_OLDER_EXACT_DIFF",
        "Permutation invariance: order in slice must not affect winner"
    );
}

#[test]
fn test_find_matching_camera_tie_breaking_by_timestamp_descending() {
    let capacity_128gb = 128 * 1024 * 1024 * 1024;
    let t_newest = Utc::now();
    let t_middle = Utc::now() - Duration::hours(5);
    let t_oldest = Utc::now() - Duration::days(10);

    let cam_oldest = CameraProfile {
        camera_id: "CAM_OLDEST".to_string(),
        display_name: "Sony A7".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\Sony"),
        known_card_capacities: vec![capacity_128gb],
        last_ingest_timestamp: Some(t_oldest),
        total_photos_ingested: 10,
    };

    let cam_middle = CameraProfile {
        camera_id: "CAM_MIDDLE".to_string(),
        display_name: "Nikon Z8".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\Nikon"),
        known_card_capacities: vec![capacity_128gb],
        last_ingest_timestamp: Some(t_middle),
        total_photos_ingested: 20,
    };

    let cam_newest = CameraProfile {
        camera_id: "CAM_NEWEST".to_string(),
        display_name: "Canon R6".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\Canon"),
        known_card_capacities: vec![capacity_128gb],
        last_ingest_timestamp: Some(t_newest),
        total_photos_ingested: 30,
    };

    // Test all permutations of order
    let permutations = [
        vec![cam_oldest.clone(), cam_middle.clone(), cam_newest.clone()],
        vec![cam_oldest.clone(), cam_newest.clone(), cam_middle.clone()],
        vec![cam_middle.clone(), cam_oldest.clone(), cam_newest.clone()],
        vec![cam_middle.clone(), cam_newest.clone(), cam_oldest.clone()],
        vec![cam_newest.clone(), cam_oldest.clone(), cam_middle.clone()],
        vec![cam_newest.clone(), cam_middle.clone(), cam_oldest.clone()],
    ];

    for (idx, perm) in permutations.iter().enumerate() {
        let matched = find_matching_camera_by_capacity(capacity_128gb, perm)
            .expect("must match");
        assert_eq!(
            matched.camera_id, "CAM_NEWEST",
            "Permutation {} must break tie in favor of CAM_NEWEST",
            idx
        );
    }
}

#[test]
fn test_find_matching_camera_tie_breaking_some_timestamp_over_none() {
    let capacity_64gb = 64 * 1024 * 1024 * 1024;
    let some_time = Utc::now() - Duration::days(2);

    let cam_none = CameraProfile {
        camera_id: "CAM_NONE".to_string(),
        display_name: "Camera Without Timestamp".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\None"),
        known_card_capacities: vec![capacity_64gb],
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let cam_some = CameraProfile {
        camera_id: "CAM_SOME".to_string(),
        display_name: "Camera With Timestamp".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\Some"),
        known_card_capacities: vec![capacity_64gb],
        last_ingest_timestamp: Some(some_time),
        total_photos_ingested: 10,
    };

    // Order [None, Some]
    let slice1 = [cam_none.clone(), cam_some.clone()];
    let res1 = find_matching_camera_by_capacity(capacity_64gb, &slice1)
        .expect("match");
    assert_eq!(res1.camera_id, "CAM_SOME");

    // Order [Some, None]
    let slice2 = [cam_some, cam_none];
    let res2 = find_matching_camera_by_capacity(capacity_64gb, &slice2)
        .expect("match");
    assert_eq!(res2.camera_id, "CAM_SOME");
}

#[test]
fn test_find_matching_camera_multiple_capacities_per_profile() {
    let cap_32gb = 32 * 1024 * 1024 * 1024;
    let cap_64gb = 64 * 1024 * 1024 * 1024;
    let cap_128gb = 128 * 1024 * 1024 * 1024;
    let cap_256gb = 256 * 1024 * 1024 * 1024;

    let profile_canon = CameraProfile {
        camera_id: "CANON_R5".to_string(),
        display_name: "Canon EOS R5".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\R5"),
        known_card_capacities: vec![cap_128gb, cap_256gb],
        last_ingest_timestamp: Some(Utc::now()),
        total_photos_ingested: 100,
    };

    let profile_sony = CameraProfile {
        camera_id: "SONY_A1".to_string(),
        display_name: "Sony A1".to_string(),
        target_directory: PathBuf::from(r"C:\Photos\A1"),
        known_card_capacities: vec![cap_32gb, cap_64gb],
        last_ingest_timestamp: Some(Utc::now()),
        total_photos_ingested: 100,
    };

    let profiles = vec![profile_canon, profile_sony];

    // Card 32 GB -> SONY_A1
    assert_eq!(
        find_matching_camera_by_capacity(cap_32gb, &profiles).unwrap().camera_id,
        "SONY_A1"
    );

    // Card 64 GB (+1% variance) -> SONY_A1
    let cap_64_variant = (cap_64gb as f64 * 1.01) as u64;
    assert_eq!(
        find_matching_camera_by_capacity(cap_64_variant, &profiles).unwrap().camera_id,
        "SONY_A1"
    );

    // Card 128 GB -> CANON_R5
    assert_eq!(
        find_matching_camera_by_capacity(cap_128gb, &profiles).unwrap().camera_id,
        "CANON_R5"
    );

    // Card 256 GB (-1.5% variance) -> CANON_R5
    let cap_256_variant = (cap_256gb as f64 * 0.985) as u64;
    assert_eq!(
        find_matching_camera_by_capacity(cap_256_variant, &profiles).unwrap().camera_id,
        "CANON_R5"
    );

    // Card 512 GB -> None
    let cap_512gb = 512 * 1024 * 1024 * 1024;
    assert!(find_matching_camera_by_capacity(cap_512gb, &profiles).is_none());
}

#[test]
fn test_find_matching_camera_empty_and_zero_inputs() {
    let capacity_128gb = 128 * 1024 * 1024 * 1024;
    let profiles = vec![CameraProfile::new("C1", "Cam 1", r"C:\Photos")];

    // Zero capacity
    assert!(find_matching_camera_by_capacity(0, &profiles).is_none());

    // Empty profiles slice
    assert!(find_matching_camera_by_capacity(capacity_128gb, &[]).is_none());

    // Profile with empty known_card_capacities
    assert!(find_matching_camera_by_capacity(capacity_128gb, &profiles).is_none());
}

// ============================================================================
// Section 5: `find_all_matching_cameras_by_capacity` Stress Tests
// ============================================================================

#[test]
fn test_find_all_matching_cameras_returns_all_in_tolerance() {
    let cap = 100_000_000_000;

    let p1 = CameraProfile {
        camera_id: "CAM1".to_string(),
        display_name: "Cam 1".to_string(),
        target_directory: PathBuf::from(r"C:\P1"),
        known_card_capacities: vec![100_000_000_000], // exact
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let p2 = CameraProfile {
        camera_id: "CAM2".to_string(),
        display_name: "Cam 2".to_string(),
        target_directory: PathBuf::from(r"C:\P2"),
        known_card_capacities: vec![101_500_000_000], // +1.5% (within 2%)
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let p3 = CameraProfile {
        camera_id: "CAM3".to_string(),
        display_name: "Cam 3".to_string(),
        target_directory: PathBuf::from(r"C:\P3"),
        known_card_capacities: vec![105_000_000_000], // +5% (outside 2%)
        last_ingest_timestamp: None,
        total_photos_ingested: 0,
    };

    let profiles = vec![p1, p2, p3];
    let matched = find_all_matching_cameras_by_capacity(cap, &profiles);

    assert_eq!(matched.len(), 2);
    let ids: Vec<&str> = matched.iter().map(|p| p.camera_id.as_str()).collect();
    assert!(ids.contains(&"CAM1"));
    assert!(ids.contains(&"CAM2"));
    assert!(!ids.contains(&"CAM3"));

    // Zero capacity -> empty
    assert!(find_all_matching_cameras_by_capacity(0, &profiles).is_empty());
    // Empty profiles -> empty
    assert!(find_all_matching_cameras_by_capacity(cap, &[]).is_empty());
}

// ============================================================================
// Section 6: `format_card_capacity` Formatting Stress Tests
// ============================================================================

#[test]
fn test_format_card_capacity_comprehensive() {
    // Zero and tiny
    assert_eq!(format_card_capacity(0), "0 MB");
    assert_eq!(format_card_capacity(100 * 1024 * 1024), "100 MB");
    assert_eq!(format_card_capacity(512 * 1024 * 1024), "512 MB");

    // Exact GiB boundaries
    assert_eq!(format_card_capacity(1024 * 1024 * 1024), "1 GB");
    assert_eq!(format_card_capacity(32 * 1024 * 1024 * 1024), "32 GB");
    assert_eq!(format_card_capacity(64 * 1024 * 1024 * 1024), "64 GB");
    assert_eq!(format_card_capacity(128 * 1024 * 1024 * 1024), "128 GB");
    assert_eq!(format_card_capacity(256 * 1024 * 1024 * 1024), "256 GB");
    assert_eq!(format_card_capacity(512 * 1024 * 1024 * 1024), "512 GB");

    // TiB boundaries
    assert_eq!(format_card_capacity(1024 * 1024 * 1024 * 1024), "1 TB");
    assert_eq!(format_card_capacity(2 * 1024 * 1024 * 1024 * 1024), "2 TB");
    assert_eq!(format_card_capacity(4 * 1024 * 1024 * 1024 * 1024), "4 TB");
}
