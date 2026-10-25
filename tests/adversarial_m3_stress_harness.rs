//! Milestone 3 Challenger 2 (Iteration 2): Ingest Registry & Empty Handler Stress Harness
//!
//! Authoritative verification for:
//! 1. Corrupt TOML handling & .corrupt.bak backup generation across diverse corrupt patterns
//! 2. Heavy multi-threaded concurrent persistence (32 threads hammering save_to_path)
//! 3. Empty card re-export, tie-breaking determinism, and registry parity

use std::fs;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use tempfile::tempdir;

use lightcraft_tui::ingest::empty_handler::{
    find_all_matching_cameras_by_capacity, find_matching_camera_by_capacity,
    format_card_capacity, is_empty_formatted_card, CameraProfile,
};
use lightcraft_tui::ingest::registry::CameraRegistry;
use lightcraft_tui::ingest::scanner::SdCardVolume;
use lightcraft_tui::ingest::IngestError;

// ============================================================================
// 1. Corrupt TOML Stress Tests
// ============================================================================

#[test]
fn test_stress_fuzzed_corrupt_toml_patterns_all_backup_and_fail() {
    let dir = tempdir().expect("tempdir");

    let corrupt_samples = vec![
        // 1. Incomplete table header
        "[cameras",
        // 2. Unclosed string
        "[cameras.c1]\ncamera_id = \"unclosed",
        // 3. Double equal signs
        "[cameras.c1]\ncamera_id == \"c1\"",
        // 4. Missing value
        "[cameras.c1]\ncamera_id =",
        // 5. Wrong type for target_directory (integer instead of string/path)
        "[cameras.c1]\ncamera_id = \"c1\"\ndisplay_name = \"C1\"\ntarget_directory = 12345\n",
        // 6. Wrong type for known_card_capacities (string instead of array of ints)
        "[cameras.c1]\ncamera_id = \"c1\"\ndisplay_name = \"C1\"\ntarget_directory = \"C:\\\\Photos\"\nknown_card_capacities = \"invalid\"\n",
        // 7. Wrong type for last_ingest_timestamp
        "[cameras.c1]\ncamera_id = \"c1\"\ndisplay_name = \"C1\"\ntarget_directory = \"C:\\\\Photos\"\nlast_ingest_timestamp = true\n",
        // 8. Corrupt array
        "[cameras.c1]\nknown_card_capacities = [123, 456, ]]\n",
        // 9. Raw binary/control chars
        "[cameras]\nkey = \x00\x01\x02\x03\n",
        // 10. Repeated table header
        "[cameras]\n[cameras]\n",
        // 11. Leading comment followed by syntax error
        "# Good comment\n# Another comment\n[cameras]\nfoo bar baz",
        // 12. Flat format with invalid type
        "[\"CAM01\"]\ncamera_id = 999\n",
        // 13. Deeply nested invalid TOML
        "[[[[[[[[[[broken]]]]]]]]]]",
        // 14. Bare dangling quotation mark
        "\" dangling quote",
        // 15. Garbage JSON instead of TOML
        "{\"cameras\": {\"c1\": null}}",
    ];

    for (i, payload) in corrupt_samples.iter().enumerate() {
        let file_path = dir.path().join(format!("registry_corrupt_{}.toml", i));
        let backup_path = dir.path().join(format!("registry_corrupt_{}.toml.corrupt.bak", i));

        fs::write(&file_path, payload).expect("write corrupt payload");

        let res = CameraRegistry::load_from_path(&file_path);
        assert!(
            res.is_err(),
            "Pattern #{} MUST return error, but got {:?}",
            i,
            res
        );

        match res.unwrap_err() {
            IngestError::RegistryError(_) => {}
            other => panic!("Pattern #{} expected RegistryError, got {:?}", i, other),
        }

        assert!(
            backup_path.exists(),
            "Pattern #{} MUST create backup file at {:?}",
            i,
            backup_path
        );

        let backed_up_content = fs::read_to_string(&backup_path).expect("read backup");
        assert_eq!(
            backed_up_content, *payload,
            "Pattern #{} backup content must match original byte-for-byte",
            i
        );
    }
}

#[test]
fn test_stress_leading_comments_and_blank_lines_not_flagged_as_corrupt() {
    let dir = tempdir().expect("tempdir");

    let valid_variations = vec![
        // Empty string
        "",
        // Blank lines
        "\n\n\r\n\t   \n",
        // Comments only
        "# This is a comment\n# Another comment\n",
        // Comments mentioning [cameras] in text
        "# Note: [cameras] section can be added later\n# [profiles] is also supported\n",
        // Empty [cameras] table
        "[cameras]\n",
        // Empty [profiles] table
        "[profiles]\n",
        // Comments before [cameras] table
        "# Leading comment\n\n[cameras]\n# Inner comment\n",
    ];

    for (i, payload) in valid_variations.iter().enumerate() {
        let file_path = dir.path().join(format!("valid_empty_{}.toml", i));
        let backup_path = dir.path().join(format!("valid_empty_{}.toml.corrupt.bak", i));

        fs::write(&file_path, payload).expect("write valid empty variant");

        let res = CameraRegistry::load_from_path(&file_path);
        assert!(
            res.is_ok(),
            "Valid empty variant #{} must load Ok, but got {:?}",
            i,
            res
        );
        let reg = res.unwrap();
        assert_eq!(reg.count(), 0);
        assert!(
            !backup_path.exists(),
            "Valid empty variant #{} must NOT create .corrupt.bak",
            i
        );
    }
}

#[test]
fn test_stress_large_corrupted_file_backup() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("large_corrupt.toml");
    let backup_path = dir.path().join("large_corrupt.toml.corrupt.bak");

    // 1 MB of invalid data
    let large_garbage = "broken_key = [[[\n".repeat(50_000);
    fs::write(&file_path, &large_garbage).expect("write large garbage");

    let res = CameraRegistry::load_from_path(&file_path);
    assert!(res.is_err(), "Large corrupt file must fail");
    assert!(backup_path.exists(), "Large corrupt backup must exist");
    let bak_len = fs::metadata(&backup_path).expect("metadata").len();
    assert_eq!(bak_len, large_garbage.len() as u64);
}

// ============================================================================
// 2. High Concurrency Stress Tests
// ============================================================================

#[test]
fn test_stress_32_threads_concurrent_saves_no_collision() {
    let dir = tempdir().expect("tempdir");
    let toml_path = dir.path().join("camera_registry.toml");

    // Initialize with 1 camera
    let mut initial_reg = CameraRegistry::with_path(&toml_path);
    initial_reg
        .register_or_update(CameraProfile::new("INIT_CAM", "Init Camera", r"C:\Photos\Init"))
        .expect("init save");

    let thread_count = 32;
    let iterations_per_thread = 20;
    let barrier = Arc::new(Barrier::new(thread_count));
    let errors = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut handles = Vec::new();

    for t in 0..thread_count {
        let b = Arc::clone(&barrier);
        let path_clone = toml_path.clone();
        let errs_clone = Arc::clone(&errors);

        handles.push(thread::spawn(move || {
            b.wait(); // Synchronize all 32 threads to hit save at the exact same instant
            for i in 0..iterations_per_thread {
                let mut reg = CameraRegistry::with_path(&path_clone);
                let p = CameraProfile::new(
                    format!("TH_{:02}_CAM_{:03}", t, i),
                    format!("Camera Thread {} Iter {}", t, i),
                    format!(r"C:\Photos\T{:02}_{:03}", t, i),
                );
                if let Err(e) = reg.register_or_update(p) {
                    errs_clone
                        .lock()
                        .unwrap()
                        .push(format!("T{} iter {} error: {}", t, i, e));
                }
            }
        }));
    }

    for h in handles {
        h.join().expect("thread join");
    }

    let recorded_errors = errors.lock().unwrap();
    println!(
        "[32 Threads Stress] Total save attempts: {}, Errors encountered: {}",
        thread_count * iterations_per_thread,
        recorded_errors.len()
    );

    // On Windows with atomic rename retry backoff, errors must be 0
    assert_eq!(
        recorded_errors.len(),
        0,
        "Concurrent saves had collisions: {:?}",
        *recorded_errors
    );

    // Verify the resulting file on disk is valid TOML and readable
    let loaded = CameraRegistry::load_from_path(&toml_path).expect("final load must succeed");
    assert!(
        loaded.count() >= 1,
        "Registry must contain at least 1 profile, found {}",
        loaded.count()
    );

    // Verify no temporary files remain in directory
    for entry in fs::read_dir(dir.path()).expect("read dir") {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().to_string();
        assert!(
            !name.contains(".tmp"),
            "Orphaned temp file found after concurrent saves: {}",
            name
        );
    }
}

#[test]
fn test_stress_concurrent_mixed_readers_and_writers() {
    let dir = tempdir().expect("tempdir");
    let toml_path = dir.path().join("camera_registry.toml");

    let mut reg = CameraRegistry::with_path(&toml_path);
    reg.register_or_update(CameraProfile::new("BASE", "Base Camera", r"C:\Photos\Base"))
        .expect("base save");

    let stop = Arc::new(Mutex::new(false));
    let reader_errs = Arc::new(Mutex::new(Vec::<String>::new()));
    let writer_errs = Arc::new(Mutex::new(Vec::<String>::new()));

    // 8 reader threads
    let mut reader_handles = Vec::new();
    for r in 0..8 {
        let p = toml_path.clone();
        let s = Arc::clone(&stop);
        let errs = Arc::clone(&reader_errs);
        reader_handles.push(thread::spawn(move || {
            let mut reads = 0;
            while !*s.lock().unwrap() {
                match CameraRegistry::load_from_path(&p) {
                    Ok(loaded) => {
                        assert!(loaded.count() >= 1);
                        reads += 1;
                    }
                    Err(e) => {
                        errs.lock().unwrap().push(format!("Reader {}: {}", r, e));
                    }
                }
                thread::yield_now();
            }
            reads
        }));
    }

    // 4 writer threads
    let mut writer_handles = Vec::new();
    for w in 0..4 {
        let p = toml_path.clone();
        let errs = Arc::clone(&writer_errs);
        writer_handles.push(thread::spawn(move || {
            for i in 0..25 {
                let mut reg = CameraRegistry::with_path(&p);
                let profile = CameraProfile::new(
                    format!("W_{}_CAM_{}", w, i),
                    format!("Camera W{} I{}", w, i),
                    format!(r"C:\Photos\W{}_{}", w, i),
                );
                if let Err(e) = reg.register_or_update(profile) {
                    errs.lock().unwrap().push(format!("Writer {}: {}", w, e));
                }
                thread::sleep(Duration::from_millis(1));
            }
        }));
    }

    for wh in writer_handles {
        wh.join().expect("writer join");
    }

    *stop.lock().unwrap() = true;

    let mut total_reads = 0;
    for rh in reader_handles {
        total_reads += rh.join().expect("reader join");
    }

    let w_errs = writer_errs.lock().unwrap();
    let r_errs = reader_errs.lock().unwrap();

    println!(
        "[Mixed Read/Write Stress] Total reads: {}, Reader errors: {}, Writer errors: {}",
        total_reads,
        r_errs.len(),
        w_errs.len()
    );

    assert_eq!(w_errs.len(), 0, "Writer errors occurred: {:?}", *w_errs);
    assert_eq!(r_errs.len(), 0, "Reader errors occurred: {:?}", *r_errs);

    // Final integrity check
    let final_reg = CameraRegistry::load_from_path(&toml_path).expect("final load");
    assert!(final_reg.count() >= 1);
}

// ============================================================================
// 3. Empty Card Re-export & Parity Verification
// ============================================================================

#[test]
fn test_stress_empty_handler_camera_profile_reexport_type_identity() {
    // Verify that CameraProfile imported from empty_handler is identical to registry::CameraProfile
    let p_empty = CameraProfile::new("EMPTY_CAM", "Empty Label", r"C:\Storage\Empty");
    let p_reg: lightcraft_tui::ingest::registry::CameraProfile = p_empty.clone();

    assert_eq!(p_empty.camera_id, p_reg.camera_id);
    assert_eq!(p_empty.display_name, p_reg.display_name);
    assert_eq!(p_empty.target_directory, p_reg.target_directory);
}

#[test]
fn test_stress_find_by_capacity_parity_between_registry_and_empty_handler() {
    // Generate 50 camera profiles with random-like configurations
    let base_capacities = vec![
        16 * 1024 * 1024 * 1024,
        32 * 1024 * 1024 * 1024,
        64 * 1024 * 1024 * 1024,
        128 * 1024 * 1024 * 1024,
        256 * 1024 * 1024 * 1024,
        512 * 1024 * 1024 * 1024,
    ];

    let mut reg = CameraRegistry::new();
    let mut profiles_vec = Vec::new();

    for i in 0..50 {
        let cap = base_capacities[i % base_capacities.len()];
        let mut p = CameraProfile::new(
            format!("CAM_{:03}", i),
            format!("Model {:03}", i),
            format!(r"C:\Photos\{:03}", i),
        )
        .with_capacity(cap);

        if i % 3 == 0 {
            let ts = Utc::now() - chrono::Duration::hours(i as i64);
            p.last_ingest_timestamp = Some(ts);
        }

        reg.profiles.insert(p.camera_id.clone(), p.clone());
        profiles_vec.push(p);
    }

    // Test 100 queries around base capacities with +/- variations
    for cap in &base_capacities {
        for offset_permille in -25..=25 {
            let test_capacity = (*cap as f64 * (1.0 + offset_permille as f64 / 1000.0)) as u64;

            let reg_match = reg.find_by_capacity(test_capacity);
            let handler_match = find_matching_camera_by_capacity(test_capacity, &profiles_vec);

            match (reg_match, handler_match) {
                (Some(r), Some(h)) => {
                    assert_eq!(
                        r.camera_id, h.camera_id,
                        "Parity mismatch at capacity {}: reg found {}, handler found {}",
                        test_capacity, r.camera_id, h.camera_id
                    );
                }
                (None, None) => {}
                (r, h) => {
                    panic!(
                        "Parity mismatch at capacity {}: reg {:?}, handler {:?}",
                        test_capacity,
                        r.map(|p| &p.camera_id),
                        h.map(|p| &p.camera_id)
                    );
                }
            }

            // Also test find_all parity
            let reg_all = reg.find_all_by_capacity(test_capacity);
            let handler_all = find_all_matching_cameras_by_capacity(test_capacity, &profiles_vec);
            assert_eq!(
                reg_all.len(),
                handler_all.len(),
                "find_all parity count mismatch at {}",
                test_capacity
            );
        }
    }
}

#[test]
fn test_stress_empty_card_exact_vs_corrupt_volume_metrics() {
    // 1. Extreme capacity (16 TB)
    let huge_capacity: u64 = 16 * 1024 * 1024 * 1024 * 1024;
    let huge_vol = SdCardVolume {
        drive_letter: 'X',
        volume_label: "HUGE_SD".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: huge_capacity,
        free_bytes: huge_capacity,
        has_dcim: true,
        camera_folders: vec![],
    };
    assert!(is_empty_formatted_card(&huge_vol));
    assert_eq!(format_card_capacity(huge_capacity), "16 TB");

    // 2. free_bytes overflow (free > total)
    let overflow_vol = SdCardVolume {
        drive_letter: 'Y',
        volume_label: "OVERFLOW".to_string(),
        file_system: "exFAT".to_string(),
        total_bytes: 100_000,
        free_bytes: 100_001,
        has_dcim: true,
        camera_folders: vec![],
    };
    assert!(!is_empty_formatted_card(&overflow_vol));

    // 3. Exactly 0 bytes
    let zero_vol = SdCardVolume {
        drive_letter: 'Z',
        volume_label: "ZERO".to_string(),
        file_system: "RAW".to_string(),
        total_bytes: 0,
        free_bytes: 0,
        has_dcim: false,
        camera_folders: vec![],
    };
    assert!(!is_empty_formatted_card(&zero_vol));
}
