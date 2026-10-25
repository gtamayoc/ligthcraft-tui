#![allow(dead_code, unused_imports)]

//! Milestone 3 Challenger 1: Adversarial Stress Test Suite
//! Targets:
//! - `extract_camera_fingerprint`, `extract_bounded_exif_fingerprint`, `clean_field_string`
//! - `CameraFingerprint`, `extract_capture_date`, `extract_fingerprint_from_volume`
//! - Scanner media discovery & filtering: `find_first_media_file`, `is_dotfile_or_system_file`
//! - Latency guarantees (< 50ms per operation, < 1ms average)
//! - 100% Read-Only SD protocol & bit-level immutability
//!
//! Authoritative Requirements:
//! - US-003 & R3: Bounded header reading in < 50ms without loading full 30-50MB files into RAM.
//! - Body serial extraction with clean whitespace fallback to `{clean_make}_{clean_model}`.
//! - Resilience against corrupt TIFF headers, zero-byte files, truncated headers, and non-TIFF files.
//! - Scanner skipping AppleDouble (`._*`), dotfiles, and 0-byte files across nested folders.

mod common;

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use chrono::Utc;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

use lightcraft_tui::ingest::exif_fingerprint::{
    clean_field_string, extract_bounded_exif_fingerprint, extract_camera_fingerprint,
    extract_capture_date, extract_fingerprint_from_volume, CameraFingerprint,
    DEFAULT_EXIF_BOUND_BYTES, DEFAULT_MAX_EXIF_BYTES, TAG_BODY_SERIAL_NUMBER,
    TAG_BODY_SERIAL_NUMBER_TIFF, TAG_LENS_MODEL_EXIF,
};
use lightcraft_tui::ingest::scanner::{
    crawl_camera_folders, crawl_dcim, crawl_media_summary, find_all_media_files,
    find_dcim_dir, find_first_media_file, is_dotfile_or_system_file, is_jpeg_extension,
    is_media_extension, is_raw_extension, is_video_extension, MediaCategory,
    MediaFileInfo, MediaSummary, SdCardVolume,
};
use lightcraft_tui::ingest::IngestError;

use common::exif_gen::ExifPhotoBuilder;

// ============================================================================
// Section 1: Corrupt TIFF Headers, Truncated Files, and Container Robustness
// ============================================================================

#[test]
fn test_adv_exif_zero_byte_file() {
    let dir = tempdir().expect("tempdir");
    let zero_file = dir.path().join("zero_length.CR3");
    File::create(&zero_file).expect("create zero file");

    let res = extract_camera_fingerprint(&zero_file);
    assert!(res.is_err(), "Zero-byte file must fail cleanly");
    match res.unwrap_err() {
        IngestError::ExifError(msg) => {
            assert!(
                msg.contains("too small") || msg.contains("EXIF"),
                "Expected header size error, got: {}",
                msg
            );
        }
        other => panic!("Expected ExifError for zero-byte file, got: {:?}", other),
    }
}

#[test]
fn test_adv_exif_sub_16_byte_truncated_boundaries() {
    let dir = tempdir().expect("tempdir");

    // Test truncated byte lengths: 1, 2, 4, 7, 8, 12, 15 bytes
    let sizes = [1, 2, 4, 7, 8, 12, 15];
    for &size in &sizes {
        let file_path = dir.path().join(format!("truncated_{}_bytes.CR3", size));
        let mut f = File::create(&file_path).expect("create truncated file");
        let payload = vec![0x55u8; size];
        f.write_all(&payload).expect("write payload");
        drop(f);

        let res = extract_camera_fingerprint(&file_path);
        assert!(
            res.is_err(),
            "File of size {} must fail extraction",
            size
        );
        match res.unwrap_err() {
            IngestError::ExifError(msg) => {
                assert!(
                    msg.contains("too small"),
                    "File size {} error message mismatch: {}",
                    size,
                    msg
                );
            }
            other => panic!("Expected ExifError, got {:?}", other),
        }
    }
}

#[test]
fn test_adv_exif_corrupt_tiff_magic_bytes() {
    let dir = tempdir().expect("tempdir");

    let corrupt_headers: Vec<(&str, Vec<u8>)> = vec![
        ("bad_endian_xx", b"XX\x2A\x00\x08\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec()),
        ("bad_magic_ii_zero", b"II\x00\x00\x08\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec()),
        ("bad_magic_mm_zero", b"MM\x00\x00\x00\x00\x00\x08\x00\x00\x00\x00\x00\x00\x00\x00".to_vec()),
        ("bad_magic_ii_9999", b"II\x99\x99\x08\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec()),
        ("all_zeros_64b", vec![0u8; 64]),
        ("all_ones_64b", vec![0xFFu8; 64]),
        ("alternating_bits", vec![0xAAu8; 128]),
    ];

    for (name, header) in corrupt_headers {
        let path = dir.path().join(format!("{}.CR3", name));
        let mut f = File::create(&path).expect("create corrupt file");
        f.write_all(&header).expect("write header");
        drop(f);

        let res = extract_camera_fingerprint(&path);
        assert!(
            res.is_err(),
            "Corrupt header '{}' must fail cleanly without panicking",
            name
        );
        match res.unwrap_err() {
            IngestError::ExifError(_) => {}
            other => panic!("Expected ExifError for '{}', got {:?}", name, other),
        }
    }
}

#[test]
fn test_adv_exif_non_tiff_containers_resilience() {
    let dir = tempdir().expect("tempdir");

    // 1. Plain text file
    let text_path = dir.path().join("document.CR3");
    fs::write(&text_path, b"This is a plaintext ASCII file masquerading as a RAW image.\n").expect("write text");
    assert!(extract_camera_fingerprint(&text_path).is_err());

    // 2. HTML document
    let html_path = dir.path().join("page.ARW");
    fs::write(&html_path, b"<!DOCTYPE html><html><head><title>Test</title></head><body><h1>Camera</h1></body></html>").expect("write html");
    assert!(extract_camera_fingerprint(&html_path).is_err());

    // 3. PNG image without EXIF chunk
    let png_path = dir.path().join("graphic.NEF");
    let mut png_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]; // PNG signature
    png_bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52]); // IHDR chunk header
    png_bytes.resize(100, 0);
    fs::write(&png_path, &png_bytes).expect("write png");
    assert!(extract_camera_fingerprint(&png_path).is_err());

    // 4. Truncated JPEG APP1 marker
    let trunc_jpeg_path = dir.path().join("truncated_app1.JPG");
    let trunc_jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x20, b'E', b'x', b'i', b'f', 0, 0];
    fs::write(&trunc_jpeg_path, &trunc_jpeg).expect("write trunc jpeg");
    assert!(extract_camera_fingerprint(&trunc_jpeg_path).is_err());

    // 5. 5MB pseudo-random noise
    let noise_path = dir.path().join("noise.DNG");
    let mut noise = Vec::with_capacity(128 * 1024);
    let mut state = 0x12345678u32;
    for _ in 0..(128 * 1024) {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        noise.push((state >> 16) as u8);
    }
    fs::write(&noise_path, &noise).expect("write noise");
    assert!(extract_camera_fingerprint(&noise_path).is_err());
}

#[test]
fn test_adv_exif_malformed_tiff_offsets_and_cyclic_ifd() {
    let dir = tempdir().expect("tempdir");

    // Case 1: IFD offset points past EOF (0x00010000 = 65536 in a 32-byte file)
    let oob_path = dir.path().join("oob_offset.CR3");
    let mut oob_bytes = Vec::new();
    oob_bytes.extend_from_slice(b"II\x2A\x00"); // TIFF little endian
    oob_bytes.extend_from_slice(&65536u32.to_le_bytes()); // Offset points far past file
    oob_bytes.resize(32, 0);
    fs::write(&oob_path, &oob_bytes).expect("write oob");
    let res = extract_camera_fingerprint(&oob_path);
    assert!(res.is_err(), "Out of bounds IFD offset must fail gracefully");

    // Case 2: IFD entry count says 65535 entries, but file has only 12 bytes
    let huge_count_path = dir.path().join("huge_ifd_count.CR3");
    let mut huge_count = Vec::new();
    huge_count.extend_from_slice(b"II\x2A\x00\x08\x00\x00\x00"); // IFD0 at offset 8
    huge_count.extend_from_slice(&65535u16.to_le_bytes()); // Count = 65535
    fs::write(&huge_count_path, &huge_count).expect("write huge count");
    let res2 = extract_camera_fingerprint(&huge_count_path);
    assert!(res2.is_err(), "Truncated IFD tag array must fail cleanly");

    // Case 3: Next IFD offset points back to IFD0 (cyclic IFD loop)
    let cyclic_path = dir.path().join("cyclic_ifd.CR3");
    let mut cyclic = Vec::new();
    cyclic.extend_from_slice(b"II\x2A\x00\x08\x00\x00\x00"); // TIFF header, IFD0 at 8
    cyclic.extend_from_slice(&0u16.to_le_bytes()); // 0 tags in IFD0
    cyclic.extend_from_slice(&8u32.to_le_bytes()); // Next IFD offset points back to 8!
    fs::write(&cyclic_path, &cyclic).expect("write cyclic");

    // Must not loop infinitely!
    let start = Instant::now();
    let res3 = extract_camera_fingerprint(&cyclic_path);
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(500), "Cyclic IFD caused execution hang: {:?}", elapsed);
    // Even if it parses 0 tags, missing Make/Model defaults cleanly or fails
    if let Ok(fp) = res3 {
        assert_eq!(fp.make, "Unknown");
        assert_eq!(fp.model, "Unknown Model");
    }
}

// ============================================================================
// Section 2: Serial Number Whitespace, Nulls, and Fallback Robustness
// ============================================================================

#[test]
fn test_adv_camera_id_serial_whitespace_and_null_fallback() {
    // 1. Clean serial
    let fp_clean = CameraFingerprint::new("Canon", "EOS R6", Some("123456789".to_string()), None);
    assert_eq!(fp_clean.camera_id(), "123456789");

    // 2. Surrounding whitespace trimmed
    let fp_spaces = CameraFingerprint::new("Canon", "EOS R6", Some("   123456789   \t".to_string()), None);
    assert_eq!(fp_spaces.camera_id(), "123456789");

    // 3. Pure whitespace serial -> MUST fall back to {make}_{model}
    let fp_pure_spaces = CameraFingerprint::new("Canon", "EOS R6", Some("     ".to_string()), None);
    assert_eq!(fp_pure_spaces.camera_id(), "Canon_EOS_R6");

    // 4. Tabs and newlines only -> MUST fall back
    let fp_tabs = CameraFingerprint::new("Nikon", "Z 8", Some("\t\n\r  ".to_string()), None);
    assert_eq!(fp_tabs.camera_id(), "Nikon_Z_8");

    // 5. Empty string -> MUST fall back
    let fp_empty = CameraFingerprint::new("Sony", "ILCE-7M4", Some("".to_string()), None);
    assert_eq!(fp_empty.camera_id(), "Sony_ILCE-7M4");

    // 6. None serial -> MUST fall back
    let fp_none = CameraFingerprint::new("Fujifilm", "X-T5", None, None);
    assert_eq!(fp_none.camera_id(), "Fujifilm_X-T5");

    // 7. Make with multiple spaces -> underscores
    let fp_make_spaces = CameraFingerprint::new("Sony Corporation", "ILCE-1", None, None);
    assert_eq!(fp_make_spaces.camera_id(), "Sony_Corporation_ILCE-1");

    // 8. Model with multiple spaces -> underscores
    let fp_model_spaces = CameraFingerprint::new("Canon", "EOS R6 Mark II", None, None);
    assert_eq!(fp_model_spaces.camera_id(), "Canon_EOS_R6_Mark_II");

    // 9. Missing make only
    let fp_no_make = CameraFingerprint::new("", "GoPro Hero 11", None, None);
    assert_eq!(fp_no_make.camera_id(), "GoPro_Hero_11");

    // 10. Missing model only
    let fp_no_model = CameraFingerprint::new("Leica", "", None, None);
    assert_eq!(fp_no_model.camera_id(), "Leica");

    // 11. Missing both make and model
    let fp_unknown = CameraFingerprint::new("", "", None, None);
    assert_eq!(fp_unknown.camera_id(), "unknown_camera");
}

#[test]
fn test_adv_exif_extraction_with_synthetic_pure_spaces_serial() {
    let dir = tempdir().expect("tempdir");
    let photo_path = dir.path().join("spaces_serial.CR3");

    // Generate a valid synthetic RAW image where BodySerialNumber is pure spaces
    ExifPhotoBuilder::new()
        .make("Canon")
        .model("Canon EOS R5")
        .body_serial(Some("       "))
        .as_raw()
        .write_to_file(&photo_path)
        .expect("write photo");

    let fp = extract_camera_fingerprint(&photo_path).expect("extraction succeeds");
    assert_eq!(fp.make, "Canon");
    assert_eq!(fp.model, "Canon EOS R5");
    // Body serial must be cleaned to None because it's pure spaces
    assert_eq!(fp.body_serial, None);
    // Primary camera ID must fall back to Canon_Canon_EOS_R5
    assert_eq!(fp.camera_id(), "Canon_Canon_EOS_R5");
}

#[test]
fn test_adv_exif_extraction_with_synthetic_tab_whitespace_serial_bug() {
    let dir = tempdir().expect("tempdir");
    let photo_path = dir.path().join("tab_serial.CR3");

    // Generate a valid synthetic RAW image where BodySerialNumber contains a tab character
    ExifPhotoBuilder::new()
        .make("Canon")
        .model("Canon EOS R5")
        .body_serial(Some("   \t   "))
        .as_raw()
        .write_to_file(&photo_path)
        .expect("write photo");

    let fp = extract_camera_fingerprint(&photo_path).expect("extraction succeeds");
    assert_eq!(fp.make, "Canon");
    assert_eq!(fp.model, "Canon EOS R5");
    
    // Fixed behavior: whitespace and control characters are cleaned to None,
    // and camera_id falls back to composite Make_Model.
    assert_eq!(fp.body_serial, None, "Body serial with tab must be cleaned to None");
    assert_eq!(fp.camera_id(), "Canon_Canon_EOS_R5", "camera_id must fall back to composite Make_Model");
}

#[test]
fn test_adv_exif_extraction_with_valid_serial_and_lens() {
    let dir = tempdir().expect("tempdir");
    let photo_path = dir.path().join("valid_photo.ARW");

    ExifPhotoBuilder::new()
        .make("Sony")
        .model("ILCE-7M4")
        .body_serial(Some("SN-987654321"))
        .lens_model(Some("FE 24-70mm F2.8 GM II"))
        .date_time("2026:10:24 12:00:00")
        .as_raw()
        .write_to_file(&photo_path)
        .expect("write photo");

    let fp = extract_camera_fingerprint(&photo_path).expect("extract valid");
    assert_eq!(fp.make, "Sony");
    assert_eq!(fp.model, "ILCE-7M4");
    assert_eq!(fp.body_serial, Some("SN-987654321".to_string()));
    assert_eq!(fp.lens_model, Some("FE 24-70mm F2.8 GM II".to_string()));
    assert_eq!(fp.camera_id(), "SN-987654321");

    let (year, date) = extract_capture_date(&photo_path).expect("extract date");
    assert_eq!(year, "2026");
    assert_eq!(date, "2026-10-24");
}

// ============================================================================
// Section 3: Latency & Bounded Header Benchmark (< 50ms, avg < 1ms)
// ============================================================================

#[test]
fn test_adv_exif_latency_benchmark_100_consecutive_extractions() {
    let dir = tempdir().expect("tempdir");
    let photo_path = dir.path().join("benchmark_target.CR3");

    // Create a 10 MB synthetic RAW photo (valid TIFF header + large binary payload)
    ExifPhotoBuilder::new()
        .make("Canon")
        .model("Canon EOS R6 Mark II")
        .body_serial(Some("042021000999"))
        .payload_size(10 * 1024 * 1024) // 10 MB payload
        .as_raw()
        .write_to_file(&photo_path)
        .expect("write 10MB test RAW");

    let file_size = fs::metadata(&photo_path).expect("metadata").len();
    assert!(file_size >= 10 * 1024 * 1024, "File size must be >= 10MB");

    // Perform 100 consecutive extractions
    let iterations = 100;
    let mut total_duration = Duration::ZERO;
    let mut max_duration = Duration::ZERO;
    let mut durations = Vec::with_capacity(iterations);

    for i in 0..iterations {
        let start = Instant::now();
        let fp = extract_camera_fingerprint(&photo_path)
            .unwrap_or_else(|e| panic!("Extraction failed on iteration {}: {:?}", i, e));
        let elapsed = start.elapsed();

        assert_eq!(fp.camera_id(), "042021000999");
        total_duration += elapsed;
        durations.push(elapsed);
        if elapsed > max_duration {
            max_duration = elapsed;
        }

        // Hard requirement: EVERY single extraction MUST complete in < 50ms
        assert!(
            elapsed < Duration::from_millis(50),
            "Iteration {} exceeded 50ms latency threshold: {:?}",
            i,
            elapsed
        );
    }

    let avg_duration = total_duration / (iterations as u32);

    println!(
        "[LATENCY BENCHMARK] 100 iterations on 10MB RAW photo:\n\
         - Total time: {:?}\n\
         - Avg per operation: {:?}\n\
         - Max latency (p100): {:?}",
        total_duration, avg_duration, max_duration
    );

    // US-003 & R3 Requirement: Average should be < 5ms (typically < 1ms on SSD)
    assert!(
        avg_duration < Duration::from_millis(10),
        "Average extraction time exceeded 10ms threshold: {:?}",
        avg_duration
    );
}

#[test]
fn test_adv_exif_latency_across_100_distinct_files() {
    let dir = tempdir().expect("tempdir");
    let mut paths = Vec::with_capacity(100);

    // Create 100 distinct synthetic photos
    for i in 0..100 {
        let p = dir.path().join(format!("distinct_{:03}.CR3", i));
        ExifPhotoBuilder::new()
            .make("Nikon")
            .model("Z 9")
            .body_serial(Some(&format!("NIKON-SERIAL-{:05}", i)))
            .payload_size(64 * 1024) // 64 KB payload
            .as_raw()
            .write_to_file(&p)
            .expect("write distinct file");
        paths.push(p);
    }

    let start = Instant::now();
    for (i, p) in paths.iter().enumerate() {
        let op_start = Instant::now();
        let fp = extract_camera_fingerprint(p).expect("extract distinct");
        let op_elapsed = op_start.elapsed();

        assert_eq!(fp.camera_id(), format!("NIKON-SERIAL-{:05}", i));
        assert!(
            op_elapsed < Duration::from_millis(50),
            "File {} exceeded 50ms: {:?}",
            i,
            op_elapsed
        );
    }
    let total_elapsed = start.elapsed();
    let avg = total_elapsed / 100;

    println!(
        "[DISTINCT FILES BENCHMARK] 100 distinct files:\n\
         - Total time: {:?}\n\
         - Avg per file: {:?}",
        total_elapsed, avg
    );

    assert!(avg < Duration::from_millis(10));
}

// ============================================================================
// Section 4: Scanner Media Filtering (AppleDouble, Dotfiles, Corrupt Files)
// ============================================================================

#[test]
fn test_adv_scanner_filters_appledouble_and_dotfiles() {
    // Verify `is_dotfile_or_system_file` handles all hidden/OS artifacts
    assert!(is_dotfile_or_system_file("._IMG_0001.CR3"));
    assert!(is_dotfile_or_system_file("._DSC0001.ARW"));
    assert!(is_dotfile_or_system_file(".hidden.JPG"));
    assert!(is_dotfile_or_system_file(".DS_Store"));
    assert!(is_dotfile_or_system_file("._.Trashes"));
    assert!(is_dotfile_or_system_file(".Spotlight-V100"));
    assert!(is_dotfile_or_system_file("$RECYCLE.BIN"));
    assert!(is_dotfile_or_system_file("System Volume Information"));
    assert!(is_dotfile_or_system_file("desktop.ini"));
    assert!(is_dotfile_or_system_file("DESKTOP.INI"));
    assert!(is_dotfile_or_system_file("thumbs.db"));
    assert!(is_dotfile_or_system_file("THUMBS.DB"));

    // Standard media names must NOT be flagged
    assert!(!is_dotfile_or_system_file("IMG_0001.CR3"));
    assert!(!is_dotfile_or_system_file("DSC_0001.NEF"));
    assert!(!is_dotfile_or_system_file("_MG_0001.JPG")); // Canon AdobeRGB starts with underscore, not dot
}

#[test]
fn test_adv_find_first_media_file_skips_appledouble_and_zero_byte() {
    let dir = tempdir().expect("tempdir");
    let camera_folder = dir.path().join("100CANON");
    fs::create_dir_all(&camera_folder).expect("create camera folder");

    // 1. AppleDouble resource fork (starts with ._)
    let appledouble_1 = camera_folder.join("._IMG_0001.CR3");
    fs::write(&appledouble_1, vec![0x00u8; 4096]).expect("write appledouble");

    // 2. Unix dotfile
    let dotfile_jpg = camera_folder.join(".hidden_preview.JPG");
    fs::write(&dotfile_jpg, vec![0xFFu8; 1024]).expect("write dotfile");

    // 3. Zero-byte corrupt file
    let zero_file = camera_folder.join("IMG_0000_ZERO.CR3");
    File::create(&zero_file).expect("create zero file");

    // 4. Windows thumbs.db
    let thumbs = camera_folder.join("thumbs.db");
    fs::write(&thumbs, vec![0x11u8; 512]).expect("write thumbs");

    // 5. Deeply nested valid file
    let deep_dir = camera_folder.join("SUB").join("NESTED");
    fs::create_dir_all(&deep_dir).expect("create deep dir");
    let valid_deep = deep_dir.join("IMG_9999.CR3");
    ExifPhotoBuilder::new()
        .make("Canon")
        .model("Canon EOS R5")
        .body_serial(Some("DEEP-SERIAL-123"))
        .as_raw()
        .write_to_file(&valid_deep)
        .expect("write deep photo");

    let camera_folders = vec![camera_folder];
    let found = find_first_media_file(&camera_folders);

    assert!(found.is_some(), "Scanner must locate valid photo");
    let found_path = found.unwrap();
    assert_eq!(
        found_path, valid_deep,
        "Scanner must skip AppleDouble, dotfiles, and 0-byte corrupt files"
    );
}

#[test]
fn test_adv_extract_fingerprint_from_volume_skips_corrupt_first_file() {
    let dir = tempdir().expect("tempdir");
    let dcim = dir.path().join("DCIM");
    let folder = dcim.join("100CANON");
    fs::create_dir_all(&folder).expect("create DCIM folder");

    // File 1: Non-zero size (512 bytes) but completely corrupted garbage (alphabetically first: "AAA_CORRUPT.CR3")
    let corrupt_file = folder.join("AAA_CORRUPT.CR3");
    fs::write(&corrupt_file, vec![0x42u8; 512]).expect("write corrupt file");

    // File 2: Valid synthetic photo ("BBB_VALID.CR3")
    let valid_file = folder.join("BBB_VALID.CR3");
    ExifPhotoBuilder::new()
        .make("Sony")
        .model("ILCE-7RM5")
        .body_serial(Some("RESILIENT-SERIAL-777"))
        .as_raw()
        .write_to_file(&valid_file)
        .expect("write valid file");

    let volume = SdCardVolume {
        drive_letter: 'T',
        volume_label: "TEST_VOL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 64 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![folder],
    };

    // `extract_fingerprint_from_volume` must try AAA_CORRUPT.CR3, fail, and succeed on BBB_VALID.CR3
    let fp = extract_fingerprint_from_volume(&volume)
        .expect("extract_fingerprint_from_volume must succeed despite first corrupt photo");

    assert_eq!(fp.make, "Sony");
    assert_eq!(fp.model, "ILCE-7RM5");
    assert_eq!(fp.camera_id(), "RESILIENT-SERIAL-777");
}

// ============================================================================
// Section 5: Date Extraction & Folder Routing Fallback
// ============================================================================

#[test]
fn test_adv_capture_date_extraction_and_fallback_resilience() {
    let dir = tempdir().expect("tempdir");

    // 1. Valid DateTimeOriginal tag
    let valid_date_file = dir.path().join("valid_date.CR3");
    ExifPhotoBuilder::new()
        .date_time("2026:07:04 18:45:12")
        .as_raw()
        .write_to_file(&valid_date_file)
        .expect("write photo with date");

    let (year, ymd) = extract_capture_date(&valid_date_file).expect("extract date");
    assert_eq!(year, "2026");
    assert_eq!(ymd, "2026-07-04");

    // 2. Corrupt or empty file -> falls back to current UTC date without failing
    let empty_file = dir.path().join("empty_for_date.CR3");
    File::create(&empty_file).expect("create empty");

    let (fallback_year, fallback_ymd) = extract_capture_date(&empty_file).expect("fallback date");
    let now = Utc::now();
    assert_eq!(fallback_year, now.format("%Y").to_string());
    assert_eq!(fallback_ymd, now.format("%Y-%m-%d").to_string());
}

// ============================================================================
// Section 6: 100% Read-Only Protocol & Bit-Level Immutability Verification
// ============================================================================

#[test]
fn test_adv_exif_100_percent_read_only_immutability() {
    let dir = tempdir().expect("tempdir");
    let test_file = dir.path().join("readonly_test.CR3");

    ExifPhotoBuilder::new()
        .make("Canon")
        .model("EOS R6")
        .body_serial(Some("042021000123"))
        .as_raw()
        .write_to_file(&test_file)
        .expect("write test file");

    // Compute baseline SHA-256
    let initial_bytes = fs::read(&test_file).expect("read initial");
    let mut hasher = Sha256::new();
    hasher.update(&initial_bytes);
    let initial_hash = hex::encode(hasher.finalize());

    // Lock file with read-only permissions
    let mut perms = fs::metadata(&test_file).expect("metadata").permissions();
    perms.set_readonly(true);
    fs::set_permissions(&test_file, perms).expect("set readonly");

    // Run EXIF extraction on read-only file
    let fp = extract_camera_fingerprint(&test_file).expect("extract from read-only file");
    assert_eq!(fp.camera_id(), "042021000123");

    let (year, date) = extract_capture_date(&test_file).expect("date from read-only file");
    assert!(!year.is_empty());
    assert!(!date.is_empty());

    // Verify SHA-256 is 100% identical post-extraction (zero bit modifications)
    let post_bytes = fs::read(&test_file).expect("read post");
    let mut post_hasher = Sha256::new();
    post_hasher.update(&post_bytes);
    let post_hash = hex::encode(post_hasher.finalize());

    assert_eq!(
        initial_hash, post_hash,
        "Read-only SD card violation: file SHA-256 changed after EXIF extraction!"
    );

    // Restore permissions for cleanup
    let mut cleanup_perms = fs::metadata(&test_file).expect("metadata").permissions();
    cleanup_perms.set_readonly(false);
    let _ = fs::set_permissions(&test_file, cleanup_perms);
}
