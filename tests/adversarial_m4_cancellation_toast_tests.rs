#![allow(dead_code, unused_imports)]

//! Milestone 4 Challenger 2: Toast Non-Blocking & Abort/Cancellation Stress Tests.
//!
//! Authoritative requirements:
//! - US-006 & FR-11, R5: Non-blocking Windows Toast notifications via PowerShell WinRT XML DOM.
//! - Non-blocking latency: Dispatch must complete in < 50ms without stalling caller.
//! - XML escaping & injection resilience: Quotes, angle brackets, ampersands, unicode emojis, single quotes.
//! - Toast failure resilience: Pipeline must succeed cleanly even if ToastNotifier returns Err.
//! - Cancellation & RAII temp file cleanup: Mid-transfer cancellation halts cleanly; TempFileGuard
//!   cleans up all partial `.tmp` files with zero orphaned files remaining on disk.
//! - Windows file locking stress: Atomic rename without `ERROR_SHARING_VIOLATION` (OS error 32) under
//!   consecutive, overwrite, and concurrent transfer workloads.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use tokio::sync::mpsc;

use lightcraft_tui::ingest::pipeline::{
    compute_file_sha256, IngestCancellationToken, IngestPipeline, IngestProgress,
    IngestSessionManifest, IngestStatus, IngestSummary, TempFileGuard, DEFAULT_CHUNK_SIZE,
};
use lightcraft_tui::ingest::registry::CameraProfile;
use lightcraft_tui::ingest::scanner::SdCardVolume;
use lightcraft_tui::ingest::toast::{
    encode_powershell_script, escape_xml, MockToastNotifier, ToastNotifier, WindowsToastNotifier,
};
use lightcraft_tui::ingest::IngestError;

// ============================================================================
// Helper: Helper to compute independent SHA-256
// ============================================================================
fn calculate_sha256(path: &Path) -> String {
    let mut file = File::open(path).expect("open file for sha256");
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).expect("read chunk");
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    hex::encode(hasher.finalize())
}

/// Recursively find all files in `dir` matching `*.tmp*`
fn find_tmp_files(dir: &Path) -> Vec<PathBuf> {
    let mut tmp_files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                tmp_files.extend(find_tmp_files(&path));
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.contains(".tmp") {
                    tmp_files.push(path);
                }
            }
        }
    }
    tmp_files
}

/// Create a synthetic photo file on disk with dummy payload
fn create_test_photo(dir: &Path, filename: &str, size_bytes: usize) -> PathBuf {
    let path = dir.join(filename);
    let mut f = File::create(&path).expect("create test photo");
    let chunk = vec![0xAB; 64 * 1024];
    let mut written = 0;
    while written < size_bytes {
        let to_write = (size_bytes - written).min(chunk.len());
        f.write_all(&chunk[..to_write]).expect("write chunk");
        written += to_write;
    }
    f.flush().expect("flush test photo");
    path
}

// ============================================================================
// Section 1: Toast Non-Blocking & Latency (< 50ms)
// ============================================================================

#[test]
fn test_adv_toast_single_dispatch_latency_under_50ms() {
    let notifier = WindowsToastNotifier::new();
    let start = Instant::now();
    let res = notifier.send_toast("LightCraft Test", "Single dispatch latency check");
    let elapsed = start.elapsed();

    // Verify non-blocking: WindowsToastNotifier spawns background child thread
    assert!(res.is_ok(), "Toast dispatch should succeed: {:?}", res);
    assert!(
        elapsed < Duration::from_millis(50),
        "Toast dispatch took {:?}, exceeding the 50ms non-blocking budget!",
        elapsed
    );
}

#[test]
fn test_adv_toast_burst_dispatch_latency_under_50ms_per_call() {
    let notifier = WindowsToastNotifier::new();
    let mut latencies = Vec::with_capacity(10);

    for i in 0..10 {
        let title = format!("Burst Toast #{}", i);
        let msg = format!("Ingest burst iteration {}", i);
        let start = Instant::now();
        let res = notifier.send_toast(&title, &msg);
        let elapsed = start.elapsed();
        assert!(res.is_ok(), "Toast dispatch failed on iter {}", i);
        latencies.push(elapsed);
    }

    let max_latency = latencies.iter().copied().max().unwrap();
    let avg_latency = latencies.iter().sum::<Duration>() / latencies.len() as u32;

    println!(
        "[STRESS TEST] Burst 10 toasts: avg = {:?}, max = {:?}",
        avg_latency, max_latency
    );

    for (idx, lat) in latencies.iter().enumerate() {
        assert!(
            *lat < Duration::from_millis(50),
            "Burst toast #{} took {:?} (> 50ms)",
            idx,
            lat
        );
    }
}

#[test]
fn test_adv_toast_concurrent_dispatch_stress() {
    let notifier = Arc::new(WindowsToastNotifier::new());
    let mut handles = Vec::new();

    for t in 0..6 {
        let notif_clone = Arc::clone(&notifier);
        let handle = thread::spawn(move || {
            for i in 0..4 {
                let start = Instant::now();
                let res = notif_clone.send_toast(
                    &format!("Thread {} - Ingest {}", t, i),
                    "Concurrent toast dispatch validation",
                );
                let elapsed = start.elapsed();
                assert!(res.is_ok(), "Concurrent toast failed in thread {}", t);
                assert!(
                    elapsed < Duration::from_millis(50),
                    "Thread {} toast {} took {:?} (> 50ms)",
                    t,
                    i,
                    elapsed
                );
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().expect("thread panicked");
    }
}

// ============================================================================
// Section 2: Toast Special Characters, XML Injection & PowerShell Safety
// ============================================================================

#[test]
fn test_adv_toast_xml_escaping_all_delimiters() {
    let raw = r#"Canon & Nikon <raw> "photos" '2026' &amp; <tag>"#;
    let escaped = escape_xml(raw);

    // Verify all 5 special chars are replaced
    assert!(!escaped.contains('<'));
    assert!(!escaped.contains('>'));
    assert!(!escaped.contains('"'));
    assert!(!escaped.contains('\''));

    // Verify correct entity encodings
    assert!(escaped.contains("&amp;"));
    assert!(escaped.contains("&lt;"));
    assert!(escaped.contains("&gt;"));
    assert!(escaped.contains("&quot;"));
    assert!(escaped.contains("&apos;"));

    // Expected exact string
    let expected = "Canon &amp; Nikon &lt;raw&gt; &quot;photos&quot; &apos;2026&apos; &amp;amp; &lt;tag&gt;";
    assert_eq!(escaped, expected);
}

#[test]
fn test_adv_toast_winrt_xml_injection_neutralization() {
    // Malicious XML payloads attempting to inject DOM nodes or close tags
    let payloads = vec![
        r#"</text><text>Injected Second Node</text>"#,
        r#"</binding></visual><visual><binding template="ToastGeneric">"#,
        r#"<toast activationType="protocol" launch="calc.exe"/>"#,
        r#"<image src="file:///C:/malicious.png"/>"#,
        r#"&lt;text&gt;Double Escaped&lt;/text&gt;"#,
    ];

    for payload in payloads {
        let escaped = escape_xml(payload);
        // Neutralized: no raw XML brackets remain
        assert!(
            !escaped.contains('<') && !escaped.contains('>'),
            "Payload was not fully neutralized: {}",
            escaped
        );
    }
}

#[test]
fn test_adv_toast_unicode_emojis_and_multilingual() {
    let title = "LightCraft Ingest 📸 — 写真 100% 完了 🎉";
    let message = "Transferred 42 files (1.2 GB) into D:\\Fotografías (2026) €/¥\nSpecial: <>&\"' 🚀";

    let notifier = WindowsToastNotifier::new();
    let start = Instant::now();
    let res = notifier.send_toast(title, message);
    let elapsed = start.elapsed();

    assert!(res.is_ok(), "Multilingual toast dispatch should succeed");
    assert!(elapsed < Duration::from_millis(50));
}

#[test]
fn test_adv_toast_powershell_script_encoding_roundtrip() {
    let script = r#"
        [void][Windows.UI.Notifications.ToastNotificationManager]
        $title = "Test: 100% <safe> & 'quoted'"
        $emoji = "📸✨🚀"
        Write-Host "$title $emoji"
    "#;

    let encoded = encode_powershell_script(script);
    assert!(!encoded.is_empty());

    // Decode Base64 and UTF-16LE
    use base64::Engine;
    let decoded_bytes = base64::engine::general_purpose::STANDARD
        .decode(&encoded)
        .expect("base64 decode");
    assert_eq!(decoded_bytes.len() % 2, 0);

    let utf16_units: Vec<u16> = decoded_bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let decoded_script = String::from_utf16(&utf16_units).expect("utf16 decode");

    assert_eq!(decoded_script, script);
}

#[test]
fn test_adv_toast_powershell_dollar_sign_dispatch_safety() {
    let notifier = WindowsToastNotifier::new();
    // Test titles/messages containing $, $env:PATH, $(calc), and quotes
    let hostile_title = "Event: $500 Session — $env:COMPUTERNAME";
    let hostile_msg = r#"$(Write-Host 'pwned'); "$pid" and `backtick` and "@ newline here-string"#;

    let res = notifier.send_toast(hostile_title, hostile_msg);
    assert!(res.is_ok(), "Hostile PowerShell characters must not crash or fail dispatch");
}

// ============================================================================
// Section 3: Toast Failure Resilience in Pipeline
// ============================================================================

#[tokio::test]
async fn test_adv_toast_failure_isolation_in_pipeline() {
    let source_dir = tempdir().expect("source tempdir");
    let target_dir = tempdir().expect("target tempdir");

    let cam_folder = source_dir.path().join("DCIM").join("100CANON");
    fs::create_dir_all(&cam_folder).expect("create dcim folder");

    let _p1 = create_test_photo(&cam_folder, "IMG_0001.JPG", 64 * 1024);
    let _p2 = create_test_photo(&cam_folder, "IMG_0002.JPG", 128 * 1024);
    let _p3 = create_test_photo(&cam_folder, "IMG_0003.JPG", 32 * 1024);

    let volume = SdCardVolume {
        drive_letter: 'E',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 90 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![cam_folder],
    };

    let profile = CameraProfile::new("CAM_FAIL_TOAST", "Toast Failure Camera", target_dir.path());

    // Configure MockToastNotifier with simulated failure
    let mock_toast = Arc::new(MockToastNotifier::new());
    mock_toast.set_should_fail(true);

    let pipeline = IngestPipeline::new(Some(mock_toast.clone()));
    let result = pipeline.run_ingest_simple(&volume, &profile).await;

    // Toast error must NOT fail the ingest pipeline
    assert!(
        result.is_ok(),
        "Ingest pipeline must succeed even if ToastNotifier fails: {:?}",
        result
    );

    let summary = result.unwrap();
    assert_eq!(summary.transferred_files, 3);
    assert_eq!(summary.skipped_duplicates, 0);
    assert_eq!(summary.total_bytes, (64 + 128 + 32) * 1024);
    assert!(summary.manifest_path.exists());

    // Manifest must be valid JSON
    let manifest_content = fs::read_to_string(&summary.manifest_path).expect("read manifest");
    let manifest: IngestSessionManifest =
        serde_json::from_str(&manifest_content).expect("parse manifest");
    assert_eq!(manifest.total_transferred, 3);
}

#[tokio::test]
async fn test_adv_toast_none_notifier_in_pipeline() {
    let source_dir = tempdir().expect("source tempdir");
    let target_dir = tempdir().expect("target tempdir");

    let cam_folder = source_dir.path().join("DCIM").join("100CANON");
    fs::create_dir_all(&cam_folder).expect("create dcim folder");

    create_test_photo(&cam_folder, "IMG_0001.JPG", 16 * 1024);

    let volume = SdCardVolume {
        drive_letter: 'E',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 90 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![cam_folder],
    };

    let profile = CameraProfile::new("CAM_NONE_TOAST", "None Toast Camera", target_dir.path());

    // Pipeline with None notifier
    let pipeline = IngestPipeline::new(None);
    let result = pipeline.run_ingest_simple(&volume, &profile).await;

    assert!(result.is_ok());
    let summary = result.unwrap();
    assert_eq!(summary.transferred_files, 1);
}

// ============================================================================
// Section 4: Cancellation & Temp File Cleanup
// ============================================================================

#[tokio::test]
async fn test_adv_cancellation_pre_cancelled_token() {
    let source_dir = tempdir().expect("source tempdir");
    let target_dir = tempdir().expect("target tempdir");

    let cam_folder = source_dir.path().join("DCIM").join("100CANON");
    fs::create_dir_all(&cam_folder).expect("create dcim folder");

    create_test_photo(&cam_folder, "IMG_0001.JPG", 64 * 1024);
    create_test_photo(&cam_folder, "IMG_0002.JPG", 64 * 1024);

    let volume = SdCardVolume {
        drive_letter: 'E',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 90 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![cam_folder],
    };

    let profile = CameraProfile::new("CAM_PRE_CANCEL", "Pre-cancelled Camera", target_dir.path());

    // Create token and cancel it BEFORE running
    let token = IngestCancellationToken::new();
    token.cancel();
    assert!(token.is_cancelled());

    let pipeline = IngestPipeline::new(None).with_cancellation_token(token);
    let result = pipeline.run_ingest_simple(&volume, &profile).await;

    assert!(result.is_err(), "Pre-cancelled ingest must return Err");
    match result.unwrap_err() {
        IngestError::Io(msg) => {
            assert!(msg.contains("cancelled by user"), "Error message: {}", msg);
        }
        other => panic!("Unexpected error type: {:?}", other),
    }

    // Verify ZERO .tmp files and ZERO transferred files in target directory
    let tmp_files = find_tmp_files(target_dir.path());
    assert!(
        tmp_files.is_empty(),
        "No .tmp files should exist after pre-cancellation, found: {:?}",
        tmp_files
    );
}

#[tokio::test]
async fn test_adv_cancellation_mid_transfer_streaming_cleanup() {
    let source_dir = tempdir().expect("source tempdir");
    let target_dir = tempdir().expect("target tempdir");

    let cam_folder = source_dir.path().join("DCIM").join("100CANON");
    fs::create_dir_all(&cam_folder).expect("create dcim folder");

    // 4 MB file with small 64 KB chunk size so it requires 64 chunks
    let file_size = 4 * 1024 * 1024;
    create_test_photo(&cam_folder, "IMG_LARGE.JPG", file_size);

    let volume = SdCardVolume {
        drive_letter: 'E',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 90 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![cam_folder],
    };

    let profile = CameraProfile::new("CAM_MID_CANCEL", "Mid-cancel Camera", target_dir.path());

    let token = IngestCancellationToken::new();
    let token_clone = token.clone();

    let (tx, mut rx) = mpsc::channel::<IngestProgress>(32);

    // Cancel token on the second progress event (mid-transfer)
    let cancel_task = tokio::spawn(async move {
        let mut chunks_seen = 0;
        while let Some(prog) = rx.recv().await {
            if prog.status == IngestStatus::Transferring {
                chunks_seen += 1;
                if chunks_seen >= 2 {
                    token_clone.cancel();
                    break;
                }
            }
        }
    });

    let pipeline = IngestPipeline::new(None)
        .with_chunk_size(64 * 1024)
        .with_cancellation_token(token);

    let result = pipeline.run_ingest(&volume, &profile, Some(tx)).await;
    let _ = cancel_task.await;

    assert!(result.is_err(), "Mid-transfer cancelled ingest must fail");

    // Empirically check that all .tmp files are cleaned up by RAII TempFileGuard
    let tmp_files = find_tmp_files(target_dir.path());
    assert!(
        tmp_files.is_empty(),
        "CRITICAL: Dangling .tmp files found after mid-transfer abort: {:?}",
        tmp_files
    );
}

#[tokio::test]
async fn test_adv_cancellation_mid_batch_partial_state_and_cleanup() {
    let source_dir = tempdir().expect("source tempdir");
    let target_dir = tempdir().expect("target tempdir");

    let cam_folder = source_dir.path().join("DCIM").join("100CANON");
    fs::create_dir_all(&cam_folder).expect("create dcim folder");

    let p1 = create_test_photo(&cam_folder, "IMG_0001.JPG", 128 * 1024);
    let p2 = create_test_photo(&cam_folder, "IMG_0002.JPG", 128 * 1024);
    let _p3 = create_test_photo(&cam_folder, "IMG_0003.JPG", 128 * 1024);
    let _p4 = create_test_photo(&cam_folder, "IMG_0004.JPG", 128 * 1024);

    let hash1 = calculate_sha256(&p1);
    let hash2 = calculate_sha256(&p2);

    let volume = SdCardVolume {
        drive_letter: 'E',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 90 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![cam_folder],
    };

    let profile = CameraProfile::new("CAM_BATCH_CANCEL", "Batch Cancel Camera", target_dir.path());

    let token = IngestCancellationToken::new();
    let token_clone = token.clone();

    let (tx, mut rx) = mpsc::channel::<IngestProgress>(32);

    // Cancel after IMG_0002 completes
    let cancel_task = tokio::spawn(async move {
        while let Some(prog) = rx.recv().await {
            if prog.status == IngestStatus::FileCompleted && prog.current_file_index == 2 {
                token_clone.cancel();
                break;
            }
        }
    });

    let pipeline = IngestPipeline::new(None).with_cancellation_token(token);
    let result = pipeline.run_ingest(&volume, &profile, Some(tx)).await;
    let _ = cancel_task.await;

    assert!(result.is_err(), "Batch cancel must return Err");

    // Files 1 and 2 should exist and have matching SHA-256
    let yyyy_now = Utc::now().format("%Y").to_string();
    let yyyy_mm_dd_now = Utc::now().format("%Y-%m-%d").to_string();
    let dest_subfolder = target_dir.path().join(&yyyy_now).join(&yyyy_mm_dd_now);

    let dest1 = dest_subfolder.join("IMG_0001.JPG");
    let dest2 = dest_subfolder.join("IMG_0002.JPG");
    let dest3 = dest_subfolder.join("IMG_0003.JPG");
    let dest4 = dest_subfolder.join("IMG_0004.JPG");

    assert!(dest1.exists(), "IMG_0001.JPG should exist");
    assert_eq!(calculate_sha256(&dest1), hash1);

    assert!(dest2.exists(), "IMG_0002.JPG should exist");
    assert_eq!(calculate_sha256(&dest2), hash2);

    // Files 3 and 4 must not exist
    assert!(!dest3.exists(), "IMG_0003.JPG should NOT exist");
    assert!(!dest4.exists(), "IMG_0004.JPG should NOT exist");

    // ZERO dangling .tmp files anywhere in target directory
    let tmp_files = find_tmp_files(target_dir.path());
    assert!(
        tmp_files.is_empty(),
        "CRITICAL: Orphaned .tmp files found: {:?}",
        tmp_files
    );
}

#[test]
fn test_adv_temp_file_guard_raii_drop_on_panic() {
    let dir = tempdir().expect("tempdir");
    let tmp_path = dir.path().join("partial_file.tmp");
    fs::write(&tmp_path, "partial uncommitted stream bytes").expect("write tmp");
    assert!(tmp_path.exists());

    // Simulate panic inside a closure
    let result = std::panic::catch_unwind(|| {
        let _guard = TempFileGuard::new(&tmp_path);
        panic!("Simulated worker panic during copy!");
    });

    assert!(result.is_err(), "Panic should have been caught");
    // Verify guard's Drop implementation executed and cleaned up the file
    assert!(
        !tmp_path.exists(),
        "TempFileGuard must delete partial file upon panic unwinding"
    );
}

#[test]
fn test_adv_temp_file_guard_disarm_preserves_file() {
    let dir = tempdir().expect("tempdir");
    let committed_path = dir.path().join("committed_file.jpg");
    fs::write(&committed_path, "committed bytes").expect("write committed");
    assert!(committed_path.exists());

    {
        let mut guard = TempFileGuard::new(&committed_path);
        guard.disarm();
    } // guard drops here

    assert!(
        committed_path.exists(),
        "Disarmed TempFileGuard must NOT delete the committed file"
    );
}

// ============================================================================
// Section 5: Windows File Locking & Atomic Rename Stress
// ============================================================================

#[tokio::test]
async fn test_adv_windows_rapid_sequential_transfers_no_sharing_violation() {
    let source_dir = tempdir().expect("source tempdir");
    let target_dir = tempdir().expect("target tempdir");

    let cam_folder = source_dir.path().join("DCIM").join("100CANON");
    fs::create_dir_all(&cam_folder).expect("create dcim folder");

    let count = 25;
    for i in 1..=count {
        create_test_photo(&cam_folder, &format!("IMG_{:04}.JPG", i), 16 * 1024);
    }

    let volume = SdCardVolume {
        drive_letter: 'E',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 90 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![cam_folder],
    };

    let profile = CameraProfile::new("CAM_RAPID", "Rapid Ingest Camera", target_dir.path());

    let pipeline = IngestPipeline::new(None);
    let start = Instant::now();
    let result = pipeline.run_ingest_simple(&volume, &profile).await;
    let elapsed = start.elapsed();

    assert!(
        result.is_ok(),
        "Rapid sequential transfers failed: {:?}",
        result
    );
    let summary = result.unwrap();
    assert_eq!(summary.transferred_files, count);

    println!(
        "[STRESS TEST] Rapid 25 file transfers completed in {:?}",
        elapsed
    );

    // Verify zero orphaned .tmp files
    let tmp_files = find_tmp_files(target_dir.path());
    assert!(tmp_files.is_empty(), "Found .tmp files: {:?}", tmp_files);
}

#[tokio::test]
async fn test_adv_windows_overwrite_atomic_rename_no_sharing_violation() {
    let source_dir = tempdir().expect("source tempdir");
    let target_dir = tempdir().expect("target tempdir");

    let cam_folder = source_dir.path().join("DCIM").join("100CANON");
    fs::create_dir_all(&cam_folder).expect("create dcim folder");

    let file_size = 128 * 1024;
    let src_photo = create_test_photo(&cam_folder, "IMG_0001.JPG", file_size);
    let src_hash = calculate_sha256(&src_photo);

    let volume = SdCardVolume {
        drive_letter: 'E',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 90 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![cam_folder],
    };

    let profile = CameraProfile::new("CAM_OVERWRITE", "Overwrite Camera", target_dir.path());

    // Pre-populate destination directory with corrupted file of exact matching size
    let yyyy_now = Utc::now().format("%Y").to_string();
    let yyyy_mm_dd_now = Utc::now().format("%Y-%m-%d").to_string();
    let dest_subfolder = target_dir.path().join(&yyyy_now).join(&yyyy_mm_dd_now);
    fs::create_dir_all(&dest_subfolder).expect("create dest subfolder");

    let corrupt_dest = dest_subfolder.join("IMG_0001.JPG");
    let corrupt_data = vec![0xEE; file_size]; // different bytes
    fs::write(&corrupt_dest, &corrupt_data).expect("write corrupt dest");
    assert_eq!(corrupt_dest.metadata().unwrap().len(), file_size as u64);
    assert_ne!(calculate_sha256(&corrupt_dest), src_hash);

    // Ingest: Should detect size match, but hash mismatch -> atomic overwrite
    let pipeline = IngestPipeline::new(None);
    let result = pipeline.run_ingest_simple(&volume, &profile).await;

    assert!(result.is_ok(), "Overwrite ingest should succeed: {:?}", result);
    let summary = result.unwrap();
    assert_eq!(summary.transferred_files, 1);
    assert_eq!(summary.skipped_duplicates, 0);

    // Verify destination was cleanly replaced and matches source hash bit-for-bit
    let new_dest_hash = calculate_sha256(&corrupt_dest);
    assert_eq!(
        new_dest_hash, src_hash,
        "Destination must be cleanly overwritten with authentic data"
    );

    // Verify zero .tmp files
    let tmp_files = find_tmp_files(target_dir.path());
    assert!(tmp_files.is_empty(), "Found .tmp files: {:?}", tmp_files);
}

#[tokio::test]
async fn test_adv_windows_manifest_atomic_persistence() {
    let source_dir = tempdir().expect("source tempdir");
    let target_dir = tempdir().expect("target tempdir");

    let cam_folder = source_dir.path().join("DCIM").join("100CANON");
    fs::create_dir_all(&cam_folder).expect("create dcim folder");

    create_test_photo(&cam_folder, "IMG_0001.JPG", 32 * 1024);

    let volume = SdCardVolume {
        drive_letter: 'E',
        volume_label: "EOS_DIGITAL".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 90 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![cam_folder],
    };

    let profile = CameraProfile::new("CAM_MANIFEST", "Manifest Camera", target_dir.path());

    let pipeline = IngestPipeline::new(None);
    let result = pipeline.run_ingest_simple(&volume, &profile).await;

    assert!(result.is_ok());
    let summary = result.unwrap();

    // Verify manifest exists and is valid JSON
    assert!(summary.manifest_path.exists());
    let manifest_str = fs::read_to_string(&summary.manifest_path).expect("read manifest");
    let manifest: IngestSessionManifest =
        serde_json::from_str(&manifest_str).expect("deserialize manifest");

    assert_eq!(manifest.camera_id, "CAM_MANIFEST");
    assert_eq!(manifest.total_transferred, 1);
    assert_eq!(manifest.total_skipped, 0);
    assert_eq!(manifest.files.len(), 1);

    // Verify zero manifest tmp files
    let manifest_tmp = target_dir
        .path()
        .join(format!(".{}.tmp", summary.manifest_path.file_name().unwrap().to_str().unwrap()));
    assert!(
        !manifest_tmp.exists(),
        "Manifest temp file must not linger: {:?}",
        manifest_tmp
    );
}

#[tokio::test]
async fn test_adv_windows_concurrent_pipelines_isolation() {
    let source_dir1 = tempdir().expect("source tempdir 1");
    let target_dir1 = tempdir().expect("target tempdir 1");
    let cam1 = source_dir1.path().join("DCIM").join("100CANON");
    fs::create_dir_all(&cam1).expect("create dcim 1");
    create_test_photo(&cam1, "IMG_0001.JPG", 64 * 1024);

    let source_dir2 = tempdir().expect("source tempdir 2");
    let target_dir2 = tempdir().expect("target tempdir 2");
    let cam2 = source_dir2.path().join("DCIM").join("100SONY");
    fs::create_dir_all(&cam2).expect("create dcim 2");
    create_test_photo(&cam2, "DSC_0001.JPG", 64 * 1024);

    let vol1 = SdCardVolume {
        drive_letter: 'E',
        volume_label: "CANON_CARD".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 64 * 1024 * 1024 * 1024,
        free_bytes: 40 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![cam1],
    };

    let vol2 = SdCardVolume {
        drive_letter: 'F',
        volume_label: "SONY_CARD".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 80 * 1024 * 1024 * 1024,
        has_dcim: true,
        camera_folders: vec![cam2],
    };

    let prof1 = CameraProfile::new("CAM_1", "Canon Camera", target_dir1.path());
    let prof2 = CameraProfile::new("CAM_2", "Sony Camera", target_dir2.path());

    let p1 = IngestPipeline::new(None);
    let p2 = IngestPipeline::new(None);

    let (res1, res2) = tokio::join!(
        p1.run_ingest_simple(&vol1, &prof1),
        p2.run_ingest_simple(&vol2, &prof2)
    );

    assert!(res1.is_ok(), "Pipeline 1 failed: {:?}", res1);
    assert!(res2.is_ok(), "Pipeline 2 failed: {:?}", res2);

    assert_eq!(res1.unwrap().transferred_files, 1);
    assert_eq!(res2.unwrap().transferred_files, 1);

    assert!(find_tmp_files(target_dir1.path()).is_empty());
    assert!(find_tmp_files(target_dir2.path()).is_empty());
}
