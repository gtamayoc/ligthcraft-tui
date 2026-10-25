//! Formatted empty card detection and camera profile capacity matching.
//!
//! Authoritative requirements:
//! - US-007 & FR-10: Formatted empty card detection (`free_bytes >= 99%` and 0 media assets in DCIM)
//!   and 1-touch profile re-association based on registered card capacities.

pub use super::registry::CameraProfile;
use super::scanner::SdCardVolume;

/// Determine whether a removable volume is an empty formatted SD card (US-007, FR-10).
///
/// Returns `true` if and only if:
/// 1. `total_bytes > 0` and `free_bytes <= total_bytes` (valid volume metrics).
/// 2. Free space is >= 99% of total capacity: `free_bytes as f64 / total_bytes as f64 >= 0.99`.
/// 3. Zero photographic or video assets exist in any camera folders (`volume.media_count() == 0`).
pub fn is_empty_formatted_card(volume: &SdCardVolume) -> bool {
    if volume.total_bytes == 0 || volume.free_bytes > volume.total_bytes {
        return false;
    }

    let free_ratio = volume.free_bytes as f64 / volume.total_bytes as f64;
    if free_ratio < 0.99 {
        return false;
    }

    volume.media_count() == 0
}

/// Alias for `is_empty_formatted_card` conforming to `PROJECT.md` line 90 specification.
pub fn check_empty_card(volume: &SdCardVolume) -> bool {
    is_empty_formatted_card(volume)
}

/// Check if two card capacity values match within a 2% tolerance margin (diff <= registered / 50).
/// Accommodates FAT/exFAT formatting metadata overhead and cluster alignment differences.
pub fn matches_capacity(registered_capacity: u64, actual_capacity: u64) -> bool {
    if registered_capacity == 0 || actual_capacity == 0 {
        return false;
    }
    let diff = registered_capacity.abs_diff(actual_capacity);
    diff <= (registered_capacity / 50)
}

/// Find a matching camera profile for an empty card based on physical capacity.
///
/// Priority ranking:
/// 1. Smallest absolute capacity difference (`diff`).
/// 2. Most recent ingest timestamp (`last_ingest_timestamp` descending).
pub fn find_matching_camera_by_capacity<'a>(
    total_bytes: u64,
    profiles: &'a [CameraProfile],
) -> Option<&'a CameraProfile> {
    if total_bytes == 0 || profiles.is_empty() {
        return None;
    }

    let mut matches: Vec<(&'a CameraProfile, u64)> = profiles
        .iter()
        .filter_map(|p| {
            let min_diff = p
                .known_card_capacities
                .iter()
                .filter(|&&c| matches_capacity(c, total_bytes))
                .map(|&c| c.abs_diff(total_bytes))
                .min();
            min_diff.map(|diff| (p, diff))
        })
        .collect();

    if matches.is_empty() {
        return None;
    }

    matches.sort_by(|(p1, diff1), (p2, diff2)| {
        diff1
            .cmp(diff2)
            .then_with(|| p2.last_ingest_timestamp.cmp(&p1.last_ingest_timestamp))
            .then_with(|| p1.camera_id.cmp(&p2.camera_id))
    });

    Some(matches[0].0)
}

/// Return all camera profiles whose registered capacities match the given card capacity.
pub fn find_all_matching_cameras_by_capacity<'a>(
    total_bytes: u64,
    profiles: &'a [CameraProfile],
) -> Vec<&'a CameraProfile> {
    if total_bytes == 0 || profiles.is_empty() {
        return Vec::new();
    }

    profiles
        .iter()
        .filter(|p| {
            p.known_card_capacities
                .iter()
                .any(|&c| matches_capacity(c, total_bytes))
        })
        .collect()
}

/// Format raw byte capacities into human-readable standard SD card labels (e.g. "128 GB").
pub fn format_card_capacity(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const TIB: u64 = 1024 * 1024 * 1024 * 1024;

    if bytes >= TIB {
        let tib = (bytes as f64 / TIB as f64).round() as u64;
        format!("{} TB", tib)
    } else if bytes >= GIB {
        let gib = (bytes as f64 / GIB as f64).round() as u64;
        format!("{} GB", gib)
    } else {
        let mib = (bytes as f64 / (1024 * 1024) as f64).round() as u64;
        format!("{} MB", mib)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::fs::{self, File};
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn test_is_empty_formatted_card_exact_full_capacity() {
        let capacity = 64 * 1024 * 1024 * 1024;
        let vol = SdCardVolume {
            drive_letter: 'F',
            volume_label: "EOS_DIGITAL".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: capacity,
            free_bytes: capacity, // 100% free
            has_dcim: true,
            camera_folders: vec![],
        };
        assert!(is_empty_formatted_card(&vol));
        assert!(check_empty_card(&vol));
    }

    #[test]
    fn test_is_empty_formatted_card_99_percent_boundaries() {
        let capacity: u64 = 100_000_000_000;

        // 99.001% free -> Empty
        let vol_above = SdCardVolume {
            drive_letter: 'F',
            volume_label: "EOS_DIGITAL".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: capacity,
            free_bytes: 99_001_000_000,
            has_dcim: true,
            camera_folders: vec![],
        };
        assert!(is_empty_formatted_card(&vol_above));

        // Exactly 99.000% free -> Empty
        let vol_exact = SdCardVolume {
            drive_letter: 'F',
            volume_label: "EOS_DIGITAL".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: capacity,
            free_bytes: 99_000_000_000,
            has_dcim: true,
            camera_folders: vec![],
        };
        assert!(is_empty_formatted_card(&vol_exact));

        // 98.999% free -> NOT empty
        let vol_below = SdCardVolume {
            drive_letter: 'F',
            volume_label: "EOS_DIGITAL".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: capacity,
            free_bytes: 98_999_000_000,
            has_dcim: true,
            camera_folders: vec![],
        };
        assert!(!is_empty_formatted_card(&vol_below));
    }

    #[test]
    fn test_is_empty_formatted_card_zero_and_invalid_bytes() {
        // Zero capacity -> false without division by zero panic
        let zero_vol = SdCardVolume {
            drive_letter: 'F',
            volume_label: "EMPTY".to_string(),
            file_system: "FAT32".to_string(),
            total_bytes: 0,
            free_bytes: 0,
            has_dcim: false,
            camera_folders: vec![],
        };
        assert!(!is_empty_formatted_card(&zero_vol));

        // Corrupted metrics: free_bytes > total_bytes -> false
        let corrupt_vol = SdCardVolume {
            drive_letter: 'F',
            volume_label: "CORRUPT".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: 1000,
            free_bytes: 1500,
            has_dcim: true,
            camera_folders: vec![],
        };
        assert!(!is_empty_formatted_card(&corrupt_vol));
    }

    #[test]
    fn test_is_empty_formatted_card_with_photos_fails() {
        let dir = tempdir().expect("tempdir");
        let camera_dir = dir.path().join("100CANON");
        fs::create_dir_all(&camera_dir).expect("create camera dir");

        // Add a mock RAW photo (even 0 bytes counts as photo)
        let photo_path = camera_dir.join("IMG_0001.CR3");
        File::create(&photo_path).expect("create photo");

        let vol = SdCardVolume {
            drive_letter: 'F',
            volume_label: "EOS_DIGITAL".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: 128 * 1024 * 1024 * 1024,
            free_bytes: 128 * 1024 * 1024 * 1024,
            has_dcim: true,
            camera_folders: vec![camera_dir],
        };

        assert!(!is_empty_formatted_card(&vol));
    }

    #[test]
    fn test_is_empty_formatted_card_with_non_photo_files_passes() {
        let dir = tempdir().expect("tempdir");
        let camera_dir = dir.path().join("100CANON");
        fs::create_dir_all(&camera_dir).expect("create camera dir");

        // Add Canon index catalog file (.CTG) and Mac metadata (.DS_Store)
        let ctg_path = camera_dir.join("INDEX.CTG");
        File::create(&ctg_path).expect("create CTG");
        let ds_store = camera_dir.join(".DS_Store");
        File::create(&ds_store).expect("create DS_Store");

        let vol = SdCardVolume {
            drive_letter: 'F',
            volume_label: "EOS_DIGITAL".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: 128 * 1024 * 1024 * 1024,
            free_bytes: (128.0 * 1024.0 * 1024.0 * 1024.0 * 0.995) as u64,
            has_dcim: true,
            camera_folders: vec![camera_dir],
        };

        // CTG and OS metadata do not count as photos; card is empty
        assert!(is_empty_formatted_card(&vol));
    }

    #[test]
    fn test_find_matching_camera_by_capacity_single_profile() {
        let capacity_128gb = 128 * 1024 * 1024 * 1024;
        let profiles = vec![CameraProfile {
            camera_id: "042021000123".to_string(),
            display_name: "Canon EOS R6".to_string(),
            target_directory: PathBuf::from(r"C:\Photos\CanonR6"),
            known_card_capacities: vec![capacity_128gb],
            last_ingest_timestamp: None,
            total_photos_ingested: 0,
        }];

        // Exact match
        let matched = find_matching_camera_by_capacity(capacity_128gb, &profiles);
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().camera_id, "042021000123");

        // 1.5% variance (within 2% margin)
        let formatted_capacity = (capacity_128gb as f64 * 0.985) as u64;
        let matched_variant = find_matching_camera_by_capacity(formatted_capacity, &profiles);
        assert!(matched_variant.is_some());

        // 5% variance (outside 2% margin) -> None
        let mismatch_capacity = (capacity_128gb as f64 * 0.95) as u64;
        assert!(find_matching_camera_by_capacity(mismatch_capacity, &profiles).is_none());

        // Unknown capacity (64 GB card on 128 GB profile) -> None
        let capacity_64gb = 64 * 1024 * 1024 * 1024;
        assert!(find_matching_camera_by_capacity(capacity_64gb, &profiles).is_none());
    }

    #[test]
    fn test_find_matching_camera_by_capacity_multiple_profiles_priority() {
        let capacity_128gb = 128 * 1024 * 1024 * 1024;
        let older_time = Utc::now() - chrono::Duration::days(5);
        let newer_time = Utc::now() - chrono::Duration::hours(2);

        let cam_older = CameraProfile {
            camera_id: "CAM_OLD".to_string(),
            display_name: "Sony A7III".to_string(),
            target_directory: PathBuf::from(r"C:\Photos\Sony"),
            known_card_capacities: vec![capacity_128gb],
            last_ingest_timestamp: Some(older_time),
            total_photos_ingested: 100,
        };

        let cam_newer = CameraProfile {
            camera_id: "CAM_NEW".to_string(),
            display_name: "Canon R6".to_string(),
            target_directory: PathBuf::from(r"C:\Photos\Canon"),
            known_card_capacities: vec![capacity_128gb],
            last_ingest_timestamp: Some(newer_time),
            total_photos_ingested: 50,
        };

        let profiles = vec![cam_older, cam_newer];

        // Should select the most recently used camera profile
        let matched =
            find_matching_camera_by_capacity(capacity_128gb, &profiles).expect("match");
        assert_eq!(matched.camera_id, "CAM_NEW");

        // find_all returns both
        let all = find_all_matching_cameras_by_capacity(capacity_128gb, &profiles);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_find_matching_camera_by_capacity_tertiary_tie_breaker() {
        let capacity_128gb = 128 * 1024 * 1024 * 1024;
        let cam_b = CameraProfile {
            camera_id: "CAM_B".to_string(),
            display_name: "Camera B".to_string(),
            target_directory: PathBuf::from(r"C:\Photos\B"),
            known_card_capacities: vec![capacity_128gb],
            last_ingest_timestamp: None,
            total_photos_ingested: 0,
        };
        let cam_a = CameraProfile {
            camera_id: "CAM_A".to_string(),
            display_name: "Camera A".to_string(),
            target_directory: PathBuf::from(r"C:\Photos\A"),
            known_card_capacities: vec![capacity_128gb],
            last_ingest_timestamp: None,
            total_photos_ingested: 0,
        };
        let profiles = vec![cam_b, cam_a];
        let matched = find_matching_camera_by_capacity(capacity_128gb, &profiles).expect("match");
        assert_eq!(matched.camera_id, "CAM_A");
    }

    #[test]
    fn test_format_card_capacity_labels() {
        assert_eq!(format_card_capacity(128 * 1024 * 1024 * 1024), "128 GB");
        assert_eq!(format_card_capacity(64 * 1024 * 1024 * 1024), "64 GB");
        assert_eq!(format_card_capacity(1024 * 1024 * 1024 * 1024), "1 TB");
        assert_eq!(format_card_capacity(500 * 1024 * 1024), "500 MB");
    }
}
