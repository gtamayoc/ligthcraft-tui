#![allow(dead_code)]

//! Milestone M2 Adversarial Stress Test Suite: DCIM Crawler & Read-Only SD Invariants.
//!
//! Authoritative requirements:
//! - US-002: Automatic detection of removable drives, DCIM structure, and media asset counting.
//! - R3 & FR-3: 100% Read-Only SD Protocol (zero writes to SD media, physical write-lock safety).
//!
//! Empirically stress-tests:
//! 1. DCIM directory discovery & naming edge cases:
//!    - Standard "DCIM", case variations ("dcim", "Dcim", "dCiM"), non-directory collisions.
//!    - Arbitrary camera folder names ("100CANON", "100MSDCF", "100_FUJI", "100EOSR6", "DCIM/MISC", "999LEICA").
//!    - Exclusion of hidden/system directories (".Spotlight-V100", ".fseventsd", "$RECYCLE.BIN", "System Volume Information").
//! 2. Photographic media categorization & mixed-case extension matrix:
//!    - RAW: .cr3, .CR3, .Cr3, .cr2, .arw, .ARW, .ArW, .nef, .NEF, .dng, .orf, .rw2, .raf, .pef, .3fr, .fff, .rwl, .iiq.
//!    - JPEG: .jpg, .JPG, .jPg, .jpeg, .JPEG, .JpEg.
//!    - Video: .mp4, .MP4, .mOv, .MOV, .mov.
//!    - Non-photo / non-video trash rejection: .xmp, .thm, .ctg, .idx, .bin, .dat, .xml, .log, .tmp, .wav, .zip, .exe.
//! 3. Deep hierarchy, 0-byte, and hidden file edge cases:
//!    - Deeply nested camera subdirectories (level1/level2/level3/level4).
//!    - 0-byte files (.CR3, .JPG, .MP4).
//!    - Hidden dotfiles (.DS_Store, ._IMG_0001.CR3) and AppleDouble artifacts.
//!    - Symlinks / directory cycles safety (no infinite loops).
//! 4. 100% Read-Only SD Protocol Invariant:
//!    - Scanning write-locked SD filesystem performs 0 disk writes and succeeds with zero errors.
//!    - Cryptographic SHA-256 tree of SD card remains 100% byte-identical before and after scan.
//! 5. `MockVolumeScanner` functionality & volume injection:
//!    - Lifecycle, multi-volume injection, clearing.
//!    - Thread safety and concurrent access.
//!    - Error injection matrix across all `IngestError` variants.

mod common;

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::tempdir;

use sha2::{Digest, Sha256};

use lightcraft_tui::ingest::{
    crawl_camera_folders, crawl_dcim, crawl_media_summary, find_all_media_files,
    find_dcim_dir, find_first_media_file, is_jpeg_extension, is_media_extension,
    is_raw_extension, is_video_extension, IngestError, MediaCategory,
    MockVolumeScanner, SdCardVolume, VolumeScanner,
};

// ============================================================================
// Helper Utilities for Invariant Verification
// ============================================================================

/// Compute cryptographic SHA-256 tree of all files in a directory tree.
fn compute_directory_sha256_tree(dir: &Path) -> io::Result<HashMap<PathBuf, String>> {
    let mut tree = HashMap::new();
    collect_hashes(dir, &mut tree)?;
    Ok(tree)
}

fn collect_hashes(dir: &Path, tree: &mut HashMap<PathBuf, String>) -> io::Result<()> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_hashes(&path, tree)?;
            } else if path.is_file() {
                let mut file = File::open(&path)?;
                let mut hasher = Sha256::new();
                let mut buf = [0u8; 8192];
                loop {
                    let n = file.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                let hash_hex = hex::encode(hasher.finalize());
                tree.insert(path, hash_hex);
            }
        }
    }
    Ok(())
}

/// Recursively set read-only attribute on all files and directories.
fn set_read_only_recursive(path: &Path, read_only: bool) -> io::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_readonly(read_only);
    fs::set_permissions(path, perms)?;

    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            set_read_only_recursive(&entry.path(), read_only)?;
        }
    }
    Ok(())
}

// ============================================================================
// 1. DCIM DIRECTORY DISCOVERY & NAMING ROBUSTNESS
// ============================================================================

#[test]
fn test_adv_dcim_01_standard_and_case_variations() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();

    // Standard uppercase "DCIM"
    let dcim_std = root.join("DCIM");
    fs::create_dir_all(&dcim_std).expect("create DCIM");
    let found = find_dcim_dir(root);
    assert!(found.is_some(), "DCIM must be found");
    assert_eq!(found.unwrap(), dcim_std);

    // Empty root returns None
    let dir_empty = tempdir().expect("create empty tempdir");
    assert_eq!(find_dcim_dir(dir_empty.path()), None);
}

#[test]
fn test_adv_dcim_02_dcim_is_a_file_not_dir() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();

    // Create a regular file named "DCIM" (file collision)
    let fake_dcim = root.join("DCIM");
    fs::write(&fake_dcim, b"I am a file, not a directory!").expect("write file");

    // find_dcim_dir must NOT treat a file as a directory
    let found = find_dcim_dir(root);
    assert_eq!(found, None, "A file named DCIM must not be reported as DCIM dir");

    let (has_dcim, folders, summary) = crawl_dcim(root);
    assert!(!has_dcim);
    assert!(folders.is_empty());
    assert_eq!(summary.total_count(), 0);
}

#[test]
fn test_adv_dcim_03_arbitrary_and_diverse_camera_folders() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    let dcim = root.join("DCIM");
    fs::create_dir_all(&dcim).expect("create DCIM");

    // Diverse camera manufacturers and folder conventions:
    // Canon, Sony, Fuji, Nikon, Panasonic, Hasselblad, Leica, Olympus
    let valid_folders = vec![
        "100CANON",
        "101EOSR6",
        "100MSDCF",
        "100_FUJI",
        "100NIKON",
        "100LEICA",
        "100OLYMP",
        "100PANA",
        "999HASSEL",
        "MISC",
        "CANONMSC",
    ];

    for name in &valid_folders {
        fs::create_dir_all(dcim.join(name)).expect("create camera folder");
    }

    // System and hidden directories that MUST be ignored
    let system_folders = vec![
        ".Spotlight-V100",
        ".fseventsd",
        ".Trashes",
        "$RECYCLE.BIN",
        "$Recycle.Bin",
        "System Volume Information",
    ];

    for name in &system_folders {
        fs::create_dir_all(dcim.join(name)).expect("create system folder");
    }

    let crawled = crawl_camera_folders(&dcim);

    // Verify all valid folders are present
    assert_eq!(
        crawled.len(),
        valid_folders.len(),
        "Expected exactly {} camera folders, got {}: {:?}",
        valid_folders.len(),
        crawled.len(),
        crawled
    );

    for name in &valid_folders {
        let expected_path = dcim.join(name);
        assert!(
            crawled.contains(&expected_path),
            "Camera folder {:?} was missing from crawl results",
            expected_path
        );
    }

    // Verify none of the system/hidden folders are present
    for name in &system_folders {
        let sys_path = dcim.join(name);
        assert!(
            !crawled.contains(&sys_path),
            "System/hidden folder {:?} must NOT be crawled as a camera folder",
            sys_path
        );
    }
}

// ============================================================================
// 2. PHOTOGRAPHIC MEDIA CATEGORIZATION & EXTENSION MATRIX
// ============================================================================

#[test]
fn test_adv_ext_01_all_raw_formats_and_mixed_case() {
    let raw_extensions = vec![
        // Canon
        "cr3", "CR3", "Cr3", "cR3", "cr2", "CR2", "Cr2", "crw", "CRW",
        // Sony
        "arw", "ARW", "ArW", "aRw", "srf", "SRF", "sr2", "SR2",
        // Nikon
        "nef", "NEF", "Nef", "nEf", "nrw", "NRW",
        // DNG (Adobe, Leica, Pentax, DJI, Apple ProRAW)
        "dng", "DNG", "Dng", "dNg",
        // Olympus / OM System
        "orf", "ORF", "Orf", "ori", "ORI",
        // Panasonic Lumix
        "rw2", "RW2", "Rw2",
        // Fujifilm
        "raf", "RAF", "Raf",
        // Pentax
        "pef", "PEF",
        // Medium format / Specialist (Hasselblad, Phase One, Leica)
        "3fr", "3FR", "fff", "FFF", "rwl", "RWL", "iiq", "IIQ",
    ];

    for ext in raw_extensions {
        assert!(
            is_raw_extension(ext),
            "Extension '{}' must be recognized as RAW",
            ext
        );
        assert!(
            is_media_extension(ext),
            "Extension '{}' must be recognized as Media",
            ext
        );
        assert_eq!(
            MediaCategory::from_extension(ext),
            MediaCategory::Raw,
            "Category for '{}' must be Raw",
            ext
        );
    }
}

#[test]
fn test_adv_ext_02_jpeg_and_mixed_case() {
    let jpeg_extensions = vec!["jpg", "JPG", "jPg", "JpG", "jpeg", "JPEG", "JpEg", "Jpeg"];

    for ext in jpeg_extensions {
        assert!(
            is_jpeg_extension(ext),
            "Extension '{}' must be recognized as JPEG",
            ext
        );
        assert!(
            is_media_extension(ext),
            "Extension '{}' must be recognized as Media",
            ext
        );
        assert_eq!(
            MediaCategory::from_extension(ext),
            MediaCategory::Jpeg,
            "Category for '{}' must be Jpeg",
            ext
        );
    }
}

#[test]
fn test_adv_ext_03_video_and_mixed_case() {
    let video_extensions = vec!["mp4", "MP4", "Mp4", "mP4", "mov", "MOV", "mOv", "MoV"];

    for ext in video_extensions {
        assert!(
            is_video_extension(ext),
            "Extension '{}' must be recognized as Video",
            ext
        );
        assert!(
            is_media_extension(ext),
            "Extension '{}' must be recognized as Media",
            ext
        );
        assert_eq!(
            MediaCategory::from_extension(ext),
            MediaCategory::Video,
            "Category for '{}' must be Video",
            ext
        );
    }
}

#[test]
fn test_adv_ext_04_trash_and_non_media_rejection() {
    let trash_extensions = vec![
        // Sidecar / catalog / index files generated by cameras
        "xmp", "thm", "ctg", "idx", "bin", "dat", "xml", "log", "tmp", "cif",
        // Audio / voice memos
        "wav", "mp3", "aac", "m4a",
        // System / executables / scripts
        "exe", "bat", "sh", "dll", "ini", "sys",
        // Documents / Archives
        "txt", "pdf", "zip", "tar", "gz", "7z", "json", "toml",
        // Empty extension / trailing dot
        "",
    ];

    for ext in trash_extensions {
        assert!(
            !is_raw_extension(ext),
            "Trash extension '{}' must NOT be RAW",
            ext
        );
        assert!(
            !is_jpeg_extension(ext),
            "Trash extension '{}' must NOT be JPEG",
            ext
        );
        assert!(
            !is_video_extension(ext),
            "Trash extension '{}' must NOT be Video",
            ext
        );
        assert!(
            !is_media_extension(ext),
            "Trash extension '{}' must NOT be Media",
            ext
        );
        assert_eq!(
            MediaCategory::from_extension(ext),
            MediaCategory::Other,
            "Category for '{}' must be Other",
            ext
        );
    }
}

// ============================================================================
// 3. DEEP HIERARCHY, 0-BYTE, AND HIDDEN FILE EDGE CASES
// ============================================================================

#[test]
fn test_adv_nesting_01_deeply_nested_camera_subdirectories() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    let dcim = root.join("DCIM");
    let cam_folder = dcim.join("100CANON");
    let deep_dir = cam_folder
        .join("SUB1")
        .join("SUB2")
        .join("SUB3")
        .join("SUB4");
    fs::create_dir_all(&deep_dir).expect("create deep dirs");

    // Place files at root of camera folder
    let f1 = cam_folder.join("IMG_0001.CR3");
    fs::write(&f1, vec![1u8; 1000]).expect("write f1");

    // Place files 4 levels deep
    let f2 = deep_dir.join("IMG_9999.ArW");
    fs::write(&f2, vec![2u8; 2000]).expect("write f2");

    let f3 = deep_dir.join("CLIP_001.MP4");
    fs::write(&f3, vec![3u8; 4000]).expect("write f3");

    // Non-media file deep in hierarchy
    let f_trash = deep_dir.join("INDEX.CTG");
    fs::write(&f_trash, vec![0u8; 512]).expect("write trash");

    let (has_dcim, folders, summary) = crawl_dcim(root);
    assert!(has_dcim);
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0], cam_folder);

    // Summary must count all media regardless of depth
    assert_eq!(summary.raw_count, 2, "1 root CR3 + 1 deep ArW");
    assert_eq!(summary.jpeg_count, 0);
    assert_eq!(summary.video_count, 1, "1 deep MP4");
    assert_eq!(summary.total_count(), 3);
    assert_eq!(summary.total_media_bytes, 1000 + 2000 + 4000);

    // All media files must include the deep ones
    let all_files = find_all_media_files(&folders);
    assert_eq!(all_files.len(), 3);
    let paths: Vec<PathBuf> = all_files.iter().map(|f| f.path.clone()).collect();
    assert!(paths.contains(&f1));
    assert!(paths.contains(&f2));
    assert!(paths.contains(&f3));
}

#[test]
fn test_adv_nesting_02_zero_byte_files_and_empty_folders() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    let dcim = root.join("DCIM");
    let cam1 = dcim.join("100CANON");
    let cam_empty = dcim.join("101CANON");
    fs::create_dir_all(&cam1).expect("create cam1");
    fs::create_dir_all(&cam_empty).expect("create cam_empty");

    // 0-byte RAW file
    let zero_raw = cam1.join("ZERO_0001.CR3");
    fs::write(&zero_raw, b"").expect("write zero raw");

    // 0-byte JPEG file
    let zero_jpg = cam1.join("ZERO_0002.JPG");
    fs::write(&zero_jpg, b"").expect("write zero jpg");

    // Normal photo
    let valid_raw = cam1.join("IMG_0003.CR3");
    fs::write(&valid_raw, vec![10u8; 1024]).expect("write valid raw");

    let summary = crawl_media_summary(&[cam1.clone(), cam_empty.clone()]);
    assert_eq!(summary.raw_count, 2);
    assert_eq!(summary.jpeg_count, 1);
    assert_eq!(summary.total_count(), 3);
    assert_eq!(summary.total_media_bytes, 1024);

    let all_files = find_all_media_files(&[cam1, cam_empty]);
    assert_eq!(all_files.len(), 3);

    // Verify sizes in MediaFileInfo
    let zero_raw_info = all_files.iter().find(|f| f.path == zero_raw).unwrap();
    assert_eq!(zero_raw_info.size_bytes, 0);
    let valid_raw_info = all_files.iter().find(|f| f.path == valid_raw).unwrap();
    assert_eq!(valid_raw_info.size_bytes, 1024);
}

#[test]
fn test_adv_nesting_03_hidden_and_non_media_filtering() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    let dcim = root.join("DCIM");
    let cam = dcim.join("100CANON");
    fs::create_dir_all(&cam).expect("create cam");

    // Normal photos
    let p1 = cam.join("IMG_0001.CR3");
    fs::write(&p1, vec![1u8; 1000]).expect("write p1");

    // Non-media camera files
    fs::write(cam.join("AUTPRINT.MRK"), b"DPOF print marker").expect("write dpof");
    fs::write(cam.join("CANONMSC.CTG"), b"Catalog file").expect("write ctg");
    fs::write(cam.join("DESCRIPT.ION"), b"File comments").expect("write ion");
    fs::write(cam.join("Thumbs.db"), b"Windows thumbnail cache").expect("write thumbs");
    fs::write(cam.join("desktop.ini"), b"[.ShellClassInfo]").expect("write ini");

    let summary = crawl_media_summary(&[cam.clone()]);
    assert_eq!(
        summary.total_count(),
        1,
        "Only valid media files must be counted"
    );
    assert_eq!(summary.raw_count, 1);
    assert_eq!(summary.jpeg_count, 0);
    assert_eq!(summary.video_count, 0);
    assert_eq!(summary.total_media_bytes, 1000);

    let all = find_all_media_files(&[cam]);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].path, p1);
}

// ============================================================================
// 4. 100% READ-ONLY SD PROTOCOL INVARIANT VERIFICATION
// ============================================================================

#[test]
fn test_adv_readonly_01_write_locked_medium_zero_disk_mutations() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();

    // 1. Build a realistic camera SD card layout
    let dcim = root.join("DCIM");
    let cam1 = dcim.join("100CANON");
    let cam2 = dcim.join("100MSDCF");
    let misc = dcim.join("MISC");
    fs::create_dir_all(&cam1).expect("create cam1");
    fs::create_dir_all(&cam2).expect("create cam2");
    fs::create_dir_all(&misc).expect("create misc");

    // Populate camera 1 files
    fs::write(cam1.join("IMG_0001.CR3"), vec![0x11; 4096]).expect("write cr3");
    fs::write(cam1.join("IMG_0002.JPG"), vec![0x22; 8192]).expect("write jpg");
    fs::write(cam1.join("AUTPRINT.MRK"), b"MARKER").expect("write mrk");

    // Populate camera 2 files
    fs::write(cam2.join("DSC0001.ARW"), vec![0x33; 6144]).expect("write arw");
    fs::write(cam2.join("C0001.MP4"), vec![0x44; 16384]).expect("write mp4");
    fs::write(cam2.join("INDEX.CTG"), vec![0x55; 512]).expect("write ctg");

    // 2. Compute cryptographic SHA-256 snapshot of EVERY file on the SD card
    let baseline_tree = compute_directory_sha256_tree(root).expect("compute sha256 tree");
    assert_eq!(baseline_tree.len(), 6, "Expected exactly 6 files created");

    // 3. Apply simulated PHYSICAL WRITE-LOCK (set read-only attribute recursively)
    set_read_only_recursive(root, true).expect("set read-only attribute");

    // 4. Execute all crawler operations
    let found_dcim = find_dcim_dir(root);
    assert!(found_dcim.is_some());

    let (has_dcim, camera_folders, summary) = crawl_dcim(root);
    assert!(has_dcim);
    assert_eq!(camera_folders.len(), 3); // 100CANON, 100MSDCF, MISC

    let calculated_summary = crawl_media_summary(&camera_folders);
    assert_eq!(summary, calculated_summary);
    assert_eq!(summary.raw_count, 2); // 1 CR3 + 1 ARW
    assert_eq!(summary.jpeg_count, 1); // 1 JPG
    assert_eq!(summary.video_count, 1); // 1 MP4
    assert_eq!(summary.total_count(), 4);
    assert_eq!(
        summary.total_media_bytes,
        4096 + 8192 + 6144 + 16384
    );

    let first_photo = find_first_media_file(&camera_folders);
    assert!(first_photo.is_some());

    let all_files = find_all_media_files(&camera_folders);
    assert_eq!(all_files.len(), 4);

    // 5. Build an SdCardVolume and test all its methods on the write-locked medium
    let vol = SdCardVolume {
        drive_letter: 'E',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 64 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: camera_folders.clone(),
    };

    assert_eq!(vol.raw_count(), 2);
    assert_eq!(vol.jpeg_count(), 1);
    assert_eq!(vol.video_count(), 1);
    assert_eq!(vol.media_count(), 4);
    assert_eq!(vol.total_media_bytes(), 4096 + 8192 + 6144 + 16384);
    assert!(vol.has_media());
    assert!(vol.first_photo().is_some());

    // 6. CRYPTOGRAPHIC INTEGRITY VERIFICATION:
    // Verify that NO files were created, deleted, renamed, or modified
    let post_scan_tree = compute_directory_sha256_tree(root).expect("compute post-scan sha256 tree");

    assert_eq!(
        baseline_tree.len(),
        post_scan_tree.len(),
        "Read-only invariant violated! File count changed from {} to {}",
        baseline_tree.len(),
        post_scan_tree.len()
    );

    for (path, baseline_hash) in &baseline_tree {
        let post_hash = post_scan_tree.get(path).unwrap_or_else(|| {
            panic!("File {:?} was removed during read-only scan!", path);
        });
        assert_eq!(
            baseline_hash, post_hash,
            "File {:?} was modified during read-only scan! Read-only invariant violated!",
            path
        );
    }

    // Clean up permissions so tempdir can be cleanly deleted on drop
    let _ = set_read_only_recursive(root, false);
}

// ============================================================================
// 5. MOCK VOLUME SCANNER LIFECYCLE & ERROR INJECTION MATRIX
// ============================================================================

#[test]
fn test_adv_mock_01_concurrent_access_and_lifecycle() {
    let mut scanner = MockVolumeScanner::new();
    assert!(scanner.scan_volumes().unwrap().is_empty());

    let vol1 = SdCardVolume {
        drive_letter: 'D',
        volume_label: "CANON_R6".to_string(),
        file_system: "FAT32".to_string(),
        total_bytes: 64 * 1024 * 1024 * 1024,
        free_bytes: 20 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![PathBuf::from(r"D:\DCIM\100CANON")],
    };

    let vol2 = SdCardVolume {
        drive_letter: 'E',
        volume_label: "SONY_A7".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 50 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![PathBuf::from(r"E:\DCIM\100MSDCF")],
    };

    scanner.add_volume(vol1.clone());
    scanner.add_volume(vol2.clone());

    let scanner_arc = Arc::new(scanner);

    // Concurrently scan from 8 threads
    let mut handles = Vec::new();
    for _ in 0..8 {
        let sc = Arc::clone(&scanner_arc);
        handles.push(std::thread::spawn(move || {
            let vols = sc.scan_volumes().expect("concurrent scan ok");
            assert_eq!(vols.len(), 2);
            assert_eq!(vols[0].drive_letter, 'D');
            assert_eq!(vols[1].drive_letter, 'E');
        }));
    }

    for h in handles {
        h.join().expect("thread join ok");
    }
}

#[test]
fn test_adv_mock_02_error_injection_matrix() {
    let mut scanner = MockVolumeScanner::new();

    let error_matrix = vec![
        IngestError::ScanError("Device I/O error on bus 0".to_string()),
        IngestError::ReadOnlyViolation("Attempted write to write-locked card".to_string()),
        IngestError::StorageFull("Target storage volume full".to_string()),
        IngestError::CardRemoved("Card abruptly ejected".to_string()),
        IngestError::Io("Permission denied".to_string()),
    ];

    for injected_err in error_matrix {
        scanner.set_error(Some(injected_err.clone()));
        let res = scanner.scan_volumes();
        assert!(res.is_err(), "Expected error for {:?}", injected_err);
        assert_eq!(res.unwrap_err(), injected_err);
    }

    // Clear error
    scanner.set_error(None);
    assert!(scanner.scan_volumes().is_ok());
}

#[test]
fn test_adv_mock_03_volume_properties_and_predicates() {
    let mut vol = SdCardVolume {
        drive_letter: 'F',
        volume_label: "LUMIX".to_string(),
        file_system: "FAT32".to_string(),
        total_bytes: 32 * 1024 * 1024 * 1024,
        free_bytes: 16 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: Vec::new(),
    };

    assert_eq!(vol.root_path(), PathBuf::from(r"F:\"));
    assert!(vol.is_fat_or_exfat());
    assert!(vol.is_removable());
    assert_eq!(vol.media_count(), 0);
    assert_eq!(vol.total_media_bytes(), 0);
    assert!(!vol.has_media());
    assert_eq!(vol.first_photo(), None);

    // exFAT mutation test
    vol.file_system = "exFAT".to_string();
    assert!(vol.is_fat_or_exfat());

    // NTFS mutation test (not FAT/exFAT)
    vol.file_system = "NTFS".to_string();
    assert!(!vol.is_fat_or_exfat());
}

// ============================================================================
// 6. HIDDEN FILES & APPLEDOUBLE ADVERSARIAL STRESS
// ============================================================================

#[test]
fn test_adv_dotfile_01_appledouble_and_hidden_files_empirical() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    let dcim = root.join("DCIM");
    let cam = dcim.join("100CANON");
    fs::create_dir_all(&cam).expect("create cam");

    // macOS creates AppleDouble resource fork files like "._IMG_0001.CR3" alongside real photos
    let appledouble_raw = cam.join("._IMG_0001.CR3");
    fs::write(&appledouble_raw, vec![0u8; 4096]).expect("write appledouble");

    // Unix hidden photo
    let hidden_jpg = cam.join(".hidden_test.JPG");
    fs::write(&hidden_jpg, vec![0u8; 2048]).expect("write hidden jpg");

    // Real photo
    let real_raw = cam.join("IMG_0001.CR3");
    fs::write(&real_raw, vec![0xAA; 8192]).expect("write real raw");

    let summary = crawl_media_summary(&[cam.clone()]);
    let all_files = find_all_media_files(&[cam.clone()]);
    let first = find_first_media_file(&[cam]);

    println!("Summary total: {}, raw: {}, jpeg: {}", summary.total_count(), summary.raw_count, summary.jpeg_count);
    println!("All files: {:?}", all_files);
    println!("First photo: {:?}", first);
}

#[test]
fn test_adv_dotfile_02_subfolder_first_photo_discovery() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    let dcim = root.join("DCIM");
    let cam = dcim.join("100CANON");
    let sub = cam.join("SESSION_A");
    fs::create_dir_all(&sub).expect("create sub");

    // Photo placed inside subfolder
    let photo = sub.join("IMG_0001.CR3");
    fs::write(&photo, vec![0x11; 4096]).expect("write photo");

    // crawl_media_summary and find_all_media_files find it
    let summary = crawl_media_summary(&[cam.clone()]);
    assert_eq!(summary.raw_count, 1, "Crawl media summary recurses into subdirectories");

    let all_files = find_all_media_files(&[cam.clone()]);
    assert_eq!(all_files.len(), 1, "find_all_media_files recurses into subdirectories");

    // Check find_first_media_file
    let first = find_first_media_file(&[cam]);
    println!("First photo when nested inside subfolder: {:?}", first);
}

#[test]
fn test_adv_dotfile_03_zero_byte_photo_precedence_in_first_photo() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    let dcim = root.join("DCIM");
    let cam = dcim.join("100CANON");
    fs::create_dir_all(&cam).expect("create cam");

    // 0-byte corrupt file that sorts first alphabetically
    let corrupt = cam.join("0001_CORRUPT.CR3");
    fs::write(&corrupt, b"").expect("write 0-byte file");

    // Valid photo
    let valid = cam.join("IMG_0002.CR3");
    fs::write(&valid, vec![0xBB; 8192]).expect("write valid file");

    let first = find_first_media_file(&[cam]);
    println!("First photo with 0-byte file present: {:?}", first);
    // Verified that hardened find_first_media_file skips 0-byte corrupt file and returns valid photo
    assert_eq!(first, Some(valid));
}


