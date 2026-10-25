#![allow(dead_code, unused_imports)]

//! Milestone 3 Challenger 1: Deep Adversarial Empirical Stress Suite
//!
//! Objectives:
//! 1. Exhaustive test of control characters (\x00..\x1F, \x7F, \t, \n, \r, \0) in BodySerialNumber.
//!    Verify body_serial == None and camera_id() falls back to {clean_make}_{clean_model}.
//! 2. Strict latency benchmark on 10MB RAW photo (< 50ms requirement, sub-millisecond expected).
//! 3. Latency scaling verification across 10MB, 25MB, and 50MB payload RAW files.
//! 4. Bounded buffer verification (Attempt 1 cursor vs Attempt 2 seek fallback beyond 128KB).
//! 5. Truncated container and corrupt TIFF offsets resilience.

mod common;

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use chrono::Utc;
use tempfile::tempdir;

use lightcraft_tui::ingest::exif_fingerprint::{
    clean_field_string, extract_bounded_exif_fingerprint, extract_camera_fingerprint,
    extract_capture_date, CameraFingerprint, DEFAULT_MAX_EXIF_BYTES,
};
use lightcraft_tui::ingest::IngestError;
use common::exif_gen::ExifPhotoBuilder;

// ============================================================================
// 1. Exhaustive Control Character Stress Tests (\x00 .. \x1F, \x7F)
// ============================================================================

#[test]
fn test_adv_all_ascii_control_characters_rejected_to_fallback() {
    let dir = tempdir().expect("tempdir");

    // All standard ASCII control characters from 0x00 to 0x1F, plus 0x7F (DEL)
    let control_chars: Vec<(u8, &'static str)> = vec![
        (0x00, "NUL"),
        (0x01, "SOH"),
        (0x02, "STX"),
        (0x03, "ETX"),
        (0x04, "EOT"),
        (0x05, "ENQ"),
        (0x06, "ACK"),
        (0x07, "BEL"),
        (0x08, "BS"),
        (0x09, "HT_TAB"),
        (0x0A, "LF_NEWLINE"),
        (0x0B, "VT"),
        (0x0C, "FF"),
        (0x0D, "CR"),
        (0x0E, "SO"),
        (0x0F, "SI"),
        (0x10, "DLE"),
        (0x11, "DC1"),
        (0x12, "DC2"),
        (0x13, "DC3"),
        (0x14, "DC4"),
        (0x15, "NAK"),
        (0x16, "SYN"),
        (0x17, "ETB"),
        (0x18, "CAN"),
        (0x19, "EM"),
        (0x1A, "SUB"),
        (0x1B, "ESC"),
        (0x1C, "FS"),
        (0x1D, "GS"),
        (0x1E, "RS"),
        (0x1F, "US"),
        (0x7F, "DEL"),
    ];

    for (byte, name) in control_chars {
        let photo_path = dir.path().join(format!("ctrl_{:02X}_{}.CR3", byte, name));

        let byte_buf = [b' ', byte, b' ', b' '];
        let serial_str = String::from_utf8_lossy(&byte_buf).to_string();


        ExifPhotoBuilder::new()
            .make("Canon")
            .model("Canon EOS R5")
            .body_serial(Some(&serial_str))
            .as_raw()
            .write_to_file(&photo_path)
            .unwrap_or_else(|e| panic!("Failed to write photo for {}: {:?}", name, e));

        let fp = extract_camera_fingerprint(&photo_path)
            .unwrap_or_else(|e| panic!("Extraction failed for {}: {:?}", name, e));

        assert_eq!(
            fp.body_serial, None,
            "Control character 0x{:02X} ({}) must be cleaned to None, but got {:?}",
            byte, name, fp.body_serial
        );

        assert_eq!(
            fp.camera_id(),
            "Canon_Canon_EOS_R5",
            "Control character 0x{:02X} ({}) must cause camera_id to fall back to make_model",
            byte, name
        );
    }
}

#[test]
fn test_adv_combined_control_whitespace_permutations() {
    let dir = tempdir().expect("tempdir");

    let cases: Vec<(&'static str, Vec<u8>)> = vec![
        ("crlf", b"\r\n".to_vec()),
        ("tabs_and_spaces", b"  \t  \t  ".to_vec()),
        ("null_and_controls", b"\x00\x01\x02\x03".to_vec()),
        ("escapes_and_dels", b"\x1B\x7F\x1B\x7F".to_vec()),
        ("mixed_whitespace_controls", b" \t\r\n\x0B\x0C ".to_vec()),
        ("null_prefixed_spaces", b"\x00     ".to_vec()),
        ("spaces_null_spaces", b"   \x00   ".to_vec()),
    ];

    for (name, raw_bytes) in cases {
        let photo_path = dir.path().join(format!("{}.CR3", name));
        let serial_str = String::from_utf8_lossy(&raw_bytes).to_string();

        ExifPhotoBuilder::new()
            .make("Sony")
            .model("ILCE-7RM5")
            .body_serial(Some(&serial_str))
            .as_raw()
            .write_to_file(&photo_path)
            .unwrap_or_else(|e| panic!("Failed to write {}: {:?}", name, e));

        let fp = extract_camera_fingerprint(&photo_path)
            .unwrap_or_else(|e| panic!("Failed to extract {}: {:?}", name, e));

        assert_eq!(
            fp.body_serial, None,
            "Combined control sequence '{}' must be sanitized to None",
            name
        );
        assert_eq!(
            fp.camera_id(),
            "Sony_ILCE-7RM5",
            "Combined control sequence '{}' must fall back to Sony_ILCE-7RM5",
            name
        );
    }
}

#[test]
fn test_adv_valid_serial_surrounded_by_controls_preserved() {
    let dir = tempdir().expect("tempdir");
    let photo_path = dir.path().join("surrounded_serial.CR3");

    // Serial with leading/trailing tabs, spaces, and newlines around a valid alphanumeric serial
    let dirty_serial = " \t\r\n SN-88330011 \r\n\t ";

    ExifPhotoBuilder::new()
        .make("Nikon")
        .model("Z 8")
        .body_serial(Some(dirty_serial))
        .as_raw()
        .write_to_file(&photo_path)
        .expect("write photo");

    let fp = extract_camera_fingerprint(&photo_path).expect("extract photo");

    assert_eq!(
        fp.body_serial,
        Some("SN-88330011".to_string()),
        "Valid serial wrapped in controls/whitespace must be trimmed cleanly"
    );
    assert_eq!(fp.camera_id(), "SN-88330011");
}

#[test]
fn test_adv_serial_with_embedded_control_character() {
    let dir = tempdir().expect("tempdir");
    let photo_path = dir.path().join("embedded_tab.CR3");

    // What happens if BodySerialNumber is "SN\t1234"?
    ExifPhotoBuilder::new()
        .make("Canon")
        .model("Canon EOS R5")
        .body_serial(Some("SN\t1234"))
        .as_raw()
        .write_to_file(&photo_path)
        .expect("write photo");

    let fp = extract_camera_fingerprint(&photo_path).expect("extract photo");
    println!("Embedded tab serial result: {:?}", fp.body_serial);
}

#[test]
fn test_adv_null_terminated_c_string_truncation() {
    let dir = tempdir().expect("tempdir");
    let photo_path = dir.path().join("null_truncated.CR3");

    // Standard C-string convention in EXIF: string is truncated at the first null character
    let c_string = "VALID_PART\0GARBAGE_TRAILING_DATA";

    ExifPhotoBuilder::new()
        .make("Fujifilm")
        .model("X-T5")
        .body_serial(Some(c_string))
        .as_raw()
        .write_to_file(&photo_path)
        .expect("write photo");

    let fp = extract_camera_fingerprint(&photo_path).expect("extract photo");

    assert_eq!(
        fp.body_serial,
        Some("VALID_PART".to_string()),
        "C-string with internal null must be truncated at the first null byte"
    );
    assert_eq!(fp.camera_id(), "VALID_PART");
}

// ============================================================================
// 2. Latency Benchmarks on 10MB, 25MB, and 50MB RAW Files
// ============================================================================

#[test]
fn test_adv_latency_strictly_bounded_10mb_raw_photo() {
    let dir = tempdir().expect("tempdir");
    let photo_path = dir.path().join("bench_10mb.CR3");

    // Create 10MB RAW photo (header + payload)
    ExifPhotoBuilder::new()
        .make("Canon")
        .model("EOS R5")
        .body_serial(Some("BENCH-SERIAL-10MB"))
        .payload_size(10 * 1024 * 1024)
        .as_raw()
        .write_to_file(&photo_path)
        .expect("write 10mb raw");

    let file_len = fs::metadata(&photo_path).expect("metadata").len();
    assert!(file_len >= 10 * 1024 * 1024);

    let iterations = 100;
    let mut durations = Vec::with_capacity(iterations);
    let mut max_duration = Duration::ZERO;
    let mut total_duration = Duration::ZERO;

    for i in 0..iterations {
        let start = Instant::now();
        let fp = extract_camera_fingerprint(&photo_path)
            .unwrap_or_else(|e| panic!("Extraction failed at iteration {}: {:?}", i, e));
        let elapsed = start.elapsed();

        assert_eq!(fp.camera_id(), "BENCH-SERIAL-10MB");
        durations.push(elapsed);
        total_duration += elapsed;
        if elapsed > max_duration {
            max_duration = elapsed;
        }

        // Hard requirement US-003 / R3: strictly < 50ms per extraction
        assert!(
            elapsed < Duration::from_millis(50),
            "Iteration {} exceeded 50ms limit: {:?}",
            i,
            elapsed
        );
    }

    durations.sort();
    let p50 = durations[iterations / 2];
    let p95 = durations[iterations * 95 / 100];
    let p99 = durations[iterations * 99 / 100];
    let avg = total_duration / (iterations as u32);

    println!(
        "[CHALLENGER 10MB BENCHMARK]\n\
         - Iterations: {}\n\
         - Total Time: {:?}\n\
         - Avg Latency: {:?}\n\
         - p50 Latency: {:?}\n\
         - p95 Latency: {:?}\n\
         - p99 Latency: {:?}\n\
         - Max Latency: {:?}",
        iterations, total_duration, avg, p50, p95, p99, max_duration
    );

    // Assert average is well under 5ms (typically < 1ms on modern systems)
    assert!(
        avg < Duration::from_millis(5),
        "Average latency {:?} exceeded 5ms budget",
        avg
    );
}

#[test]
fn test_adv_latency_invariance_on_25mb_and_50mb_raw_files() {
    let dir = tempdir().expect("tempdir");

    for size_mb in [25, 50] {
        let photo_path = dir.path().join(format!("bench_{}mb.CR3", size_mb));

        ExifPhotoBuilder::new()
            .make("Sony")
            .model("ILCE-1")
            .body_serial(Some(&format!("SERIAL-{}MB", size_mb)))
            .payload_size(size_mb * 1024 * 1024)
            .as_raw()
            .write_to_file(&photo_path)
            .unwrap_or_else(|e| panic!("Failed to write {}MB file: {:?}", size_mb, e));

        let file_len = fs::metadata(&photo_path).expect("metadata").len();
        assert!(file_len >= (size_mb as u64) * 1024 * 1024);

        // Perform 20 extractions on large files
        let mut max_dur = Duration::ZERO;
        let mut total_dur = Duration::ZERO;

        for _ in 0..20 {
            let start = Instant::now();
            let fp = extract_camera_fingerprint(&photo_path).expect("extract large");
            let elapsed = start.elapsed();

            assert_eq!(fp.camera_id(), format!("SERIAL-{}MB", size_mb));
            total_dur += elapsed;
            if elapsed > max_dur {
                max_dur = elapsed;
            }

            assert!(
                elapsed < Duration::from_millis(50),
                "{}MB file exceeded 50ms latency: {:?}",
                size_mb,
                elapsed
            );
        }

        let avg = total_dur / 20;
        println!(
            "[CHALLENGER {}MB BENCHMARK] Avg: {:?}, Max: {:?}",
            size_mb, avg, max_dur
        );
        assert!(avg < Duration::from_millis(10));
    }
}

// ============================================================================
// 3. Container Bounds and Fallback Readers
// ============================================================================

#[test]
fn test_adv_file_boundary_sizes() {
    let dir = tempdir().expect("tempdir");

    // Sizes below 16 bytes must return IngestError::ExifError with "too small"
    for size in 0..16 {
        let p = dir.path().join(format!("bound_{}_bytes.CR3", size));
        let mut f = File::create(&p).expect("create file");
        f.write_all(&vec![0xAA; size]).expect("write payload");
        drop(f);

        let res = extract_camera_fingerprint(&p);
        assert!(res.is_err(), "Size {} must fail", size);
        match res.unwrap_err() {
            IngestError::ExifError(msg) => {
                assert!(msg.contains("too small") || msg.contains("EXIF"));
            }
            other => panic!("Expected ExifError for size {}, got {:?}", size, other),
        }
    }
}

#[test]
fn test_adv_capture_date_all_delimiter_variants_and_clock_edge_cases() {
    let dir = tempdir().expect("tempdir");

    let test_dates = vec![
        ("2026:10:24 15:30:00", "2026", "2026-10-24"),
        ("2025-01-05 08:00:00", "2025", "2025-01-05"),
        ("2024/12/31 23:59:59", "2024", "2024-12-31"),
        ("1970:01:01 00:00:00", "1970", "1970-01-01"),
        ("2099:12:31 23:59:59", "2099", "2099-12-31"),
    ];

    for (input, exp_year, exp_ymd) in test_dates {
        let p = dir.path().join(format!("date_{}.CR3", exp_ymd));
        ExifPhotoBuilder::new()
            .date_time(input)
            .as_raw()
            .write_to_file(&p)
            .expect("write photo");

        let (year, ymd) = extract_capture_date(&p).expect("extract date");
        assert_eq!(year, exp_year);
        assert_eq!(ymd, exp_ymd);
    }

    // Invalid or uninitialized clocks: 0000:00:00, 9999:99:99, whitespace
    let invalid_dates = vec![
        "0000:00:00 00:00:00",
        "9999:99:99 99:99:99",
        "    :  :            ",
        "NOT_A_DATE",
        "",
    ];

    for (i, bad_date) in invalid_dates.into_iter().enumerate() {
        let p = dir.path().join(format!("bad_date_{}.CR3", i));
        ExifPhotoBuilder::new()
            .date_time(bad_date)
            .as_raw()
            .write_to_file(&p)
            .expect("write photo");

        let (year, ymd) = extract_capture_date(&p).expect("extract fallback date");
        let now = Utc::now();
        assert_eq!(
            year,
            now.format("%Y").to_string(),
            "Invalid date '{}' must fall back to current UTC year",
            bad_date
        );
        assert_eq!(
            ymd,
            now.format("%Y-%m-%d").to_string(),
            "Invalid date '{}' must fall back to current UTC date",
            bad_date
        );
    }
}
