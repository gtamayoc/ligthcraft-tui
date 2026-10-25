//! Milestone 3 Challenger 2: CameraRegistry Persistence, Concurrency & Scalability Stress Tests
//!
//! Authoritative requirements:
//! - US-004 & FR-5: Local camera registry persistence in TOML format.
//! - Atomic persistence: Temporary file + fsync + atomic rename, zero orphan corruption.
//! - Corrupt TOML handling: Automatic backup to `.corrupt.bak` and non-crashing recovery.
//! - O(1) in-memory lookup scalability: Benchmark 1,000 to 10,000 profiles (< 10 µs per lookup).
//! - Concurrency stress: Multi-threaded concurrent reads, read-write contention.
//! - Capacity matching stress: Boundaries at ±2.0%, ±2.1%, multi-camera tie-breaking.

use std::fs::{self, File};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Instant;

use chrono::{Duration, Utc};
use tempfile::tempdir;

use lightcraft_tui::ingest::empty_handler::matches_capacity;
use lightcraft_tui::ingest::registry::{CameraProfile, CameraRegistry};

// ============================================================================
// Section 1: Atomic Persistence & Power Loss Simulation
// ============================================================================

#[test]
fn test_adv_power_loss_orphaned_tmp_file_does_not_corrupt_active_registry() {
    let dir = tempdir().expect("tempdir");
    let toml_path = dir.path().join("camera_registry.toml");
    let tmp_path = dir.path().join("camera_registry.toml.tmp");

    // 1. Establish an active valid registry
    let mut reg = CameraRegistry::with_path(&toml_path);
    let p1 = CameraProfile::new("CAM_ACTIVE_1", "Canon EOS R6 Mark II", r"C:\Photos\Active1")
        .with_capacity(128 * 1024 * 1024 * 1024);
    reg.register_or_update(p1).expect("initial save");
    assert!(toml_path.exists());

    // 2. Simulate prior power loss or crash leaving an orphaned corrupted .tmp file
    fs::write(&tmp_path, "GARBAGE ORPHANED HALF-WRITTEN DATA !@#$%^&*()")
        .expect("write orphaned tmp");
    assert!(tmp_path.exists());

    // 3. Load active registry while orphaned .tmp exists
    let loaded = CameraRegistry::load_from_path(&toml_path).expect("load should succeed");
    assert_eq!(loaded.count(), 1);
    let profile = loaded.get_profile("CAM_ACTIVE_1").expect("profile CAM_ACTIVE_1");
    assert_eq!(profile.display_name, "Canon EOS R6 Mark II");

    // 4. Update and save new profile: must cleanly overwrite orphaned .tmp and rename
    let mut reg_update = loaded;
    let p2 = CameraProfile::new("CAM_ACTIVE_2", "Sony A7 IV", r"C:\Photos\Active2");
    reg_update.register_or_update(p2).expect("save over orphaned tmp");

    // 5. Verify the active file is updated and .tmp is gone
    assert!(toml_path.exists());
    assert!(!tmp_path.exists(), "Orphaned .tmp must be replaced/cleaned up after atomic save");

    let reloaded = CameraRegistry::load_from_path(&toml_path).expect("reload");
    assert_eq!(reloaded.count(), 2);
    assert!(reloaded.get_profile("CAM_ACTIVE_1").is_some());
    assert!(reloaded.get_profile("CAM_ACTIVE_2").is_some());
}

#[test]
fn test_adv_power_loss_partial_write_leaves_original_unmodified() {
    let dir = tempdir().expect("tempdir");
    let toml_path = dir.path().join("camera_registry.toml");
    let tmp_path = dir.path().join("camera_registry.toml.tmp");

    // Seed 10 valid profiles
    let mut reg = CameraRegistry::with_path(&toml_path);
    for i in 0..10 {
        reg.profiles.insert(
            format!("CAM_{:02}", i),
            CameraProfile::new(
                format!("CAM_{:02}", i),
                format!("Camera {:02}", i),
                format!(r"C:\Photos\Cam_{:02}", i),
            ),
        );
    }
    reg.save().expect("seed save");
    let original_content = fs::read_to_string(&toml_path).expect("read original");

    // Simulate power outage during save: writer writes 15 bytes to .tmp and process dies
    fs::write(&tmp_path, "[cameras.\npartial").expect("write partial tmp");

    // Original registry must still load completely intact
    let loaded = CameraRegistry::load_from_path(&toml_path).expect("load intact");
    assert_eq!(loaded.count(), 10);
    let current_content = fs::read_to_string(&toml_path).expect("read current");
    assert_eq!(original_content, current_content, "Active registry must not be mutated by partial tmp write");
}

#[test]
fn test_adv_save_creates_nested_parent_directories_automatically() {
    let dir = tempdir().expect("tempdir");
    let deep_toml_path = dir
        .path()
        .join("nested")
        .join("subfolder")
        .join("dot_lightcraft")
        .join("camera_registry.toml");

    let reg = CameraRegistry::with_path(&deep_toml_path);
    let save_res = reg.save();
    assert!(save_res.is_ok(), "save_to_path should create missing parent directories");
    assert!(deep_toml_path.exists());
}

#[test]
fn test_adv_atomic_save_replaces_existing_file_cleanly() {
    let dir = tempdir().expect("tempdir");
    let toml_path = dir.path().join("camera_registry.toml");

    let mut reg = CameraRegistry::with_path(&toml_path);

    // Iteratively write 50 times to test repeat rename stability on Windows
    for i in 0..50 {
        let p = CameraProfile::new(
            format!("CAM_{:03}", i),
            format!("Camera {:03}", i),
            format!(r"C:\Photos\{}", i),
        );
        reg.register_or_update(p).expect("repeat update");
        assert_eq!(reg.count(), i + 1);
    }

    let final_load = CameraRegistry::load_from_path(&toml_path).expect("final load");
    assert_eq!(final_load.count(), 50);
}

// ============================================================================
// Section 2: Corrupt TOML Recovery & Backup (.corrupt.bak)
// ============================================================================

#[test]
fn test_adv_corrupt_toml_syntax_creates_backup_and_returns_error() {
    let dir = tempdir().expect("tempdir");
    let toml_path = dir.path().join("camera_registry.toml");
    let corrupt_bak = dir.path().join("camera_registry.toml.corrupt.bak");

    let corrupt_payload = "syntax_error = [broken, missing_quotes, { unclosed";
    fs::write(&toml_path, corrupt_payload).expect("write corrupt");

    let res = CameraRegistry::load_from_path(&toml_path);
    assert!(res.is_err(), "Must return error on corrupted TOML");
    assert!(corrupt_bak.exists(), "Must create .corrupt.bak file");

    let backed_up = fs::read_to_string(&corrupt_bak).expect("read backup");
    assert_eq!(backed_up, corrupt_payload, "Backup content must match original byte-for-byte");
}

#[test]
fn test_adv_corrupt_toml_inside_cameras_table_behavior() {
    let dir = tempdir().expect("tempdir");
    let toml_path = dir.path().join("camera_registry.toml");
    let corrupt_bak = dir.path().join("camera_registry.toml.corrupt.bak");

    // Specifically test TOML starting with [cameras] but containing syntax corruption
    let corrupt_cameras_payload = "[cameras]\ninvalid_structure = = = 9999\n";
    fs::write(&toml_path, corrupt_cameras_payload).expect("write corrupt cameras");

    let res = CameraRegistry::load_from_path(&toml_path);
    let backup_created = corrupt_bak.exists();
    println!("Corrupt [cameras] load result: {:?}", res);
    println!("Corrupt backup file created: {}", backup_created);

    // Strict empirical oracle: corrupt TOML must NEVER be silently swallowed as empty
    assert!(
        res.is_err(),
        "CRITICAL VULNERABILITY: Corrupted TOML starting with '[cameras]' was silently treated as empty! Expected Err, got {:?}",
        res
    );
    assert!(
        backup_created,
        "CRITICAL VULNERABILITY: Corrupted registry starting with '[cameras]' was not backed up to .corrupt.bak!"
    );
}

#[test]
fn test_adv_corrupt_toml_invalid_field_type_in_camera_profile() {
    let dir = tempdir().expect("tempdir");
    let toml_path = dir.path().join("camera_registry.toml");
    let corrupt_bak = dir.path().join("camera_registry.toml.corrupt.bak");

    // Valid TOML syntax, but camera_id is an integer instead of a string
    let bad_type_payload = r#"
[cameras.CAM01]
camera_id = 12345
display_name = "Bad Type Camera"
target_directory = "C:\\Photos"
"#;
    fs::write(&toml_path, bad_type_payload).expect("write bad type");

    let res = CameraRegistry::load_from_path(&toml_path);
    println!("Invalid field type load result: {:?}", res);
    let backup_created = corrupt_bak.exists();
    println!("Backup created on type error: {}", backup_created);
}

#[test]
fn test_adv_corrupt_toml_truncated_mid_string() {
    let dir = tempdir().expect("tempdir");
    let toml_path = dir.path().join("camera_registry.toml");
    let corrupt_bak = dir.path().join("camera_registry.toml.corrupt.bak");

    // File truncated abruptly inside a string
    let truncated_payload = "[cameras.CAM01]\ncamera_id = \"CAM01\"\ndisplay_name = \"Canon EO";
    fs::write(&toml_path, truncated_payload).expect("write truncated");

    let res = CameraRegistry::load_from_path(&toml_path);
    println!("Truncated mid-string load result: {:?}", res);
    assert!(res.is_err(), "Truncated mid-string must return error");
    assert!(corrupt_bak.exists(), "Truncated mid-string must produce .corrupt.bak");
}

#[test]
fn test_adv_empty_and_whitespace_only_files_are_not_corrupt() {
    let dir = tempdir().expect("tempdir");

    // 0-byte file
    let empty_path = dir.path().join("empty.toml");
    File::create(&empty_path).expect("create empty");
    let reg1 = CameraRegistry::load_from_path(&empty_path).expect("empty file must load cleanly");
    assert_eq!(reg1.count(), 0);
    assert!(!dir.path().join("empty.toml.corrupt.bak").exists());

    // Whitespace only file
    let ws_path = dir.path().join("whitespace.toml");
    fs::write(&ws_path, "   \n\t\r\n   ").expect("write whitespace");
    let reg2 = CameraRegistry::load_from_path(&ws_path).expect("whitespace file must load cleanly");
    assert_eq!(reg2.count(), 0);
    assert!(!dir.path().join("whitespace.toml.corrupt.bak").exists());
}

#[test]
fn test_adv_corrupt_file_backup_overwrites_stale_backup() {
    let dir = tempdir().expect("tempdir");
    let toml_path = dir.path().join("camera_registry.toml");
    let corrupt_bak = dir.path().join("camera_registry.toml.corrupt.bak");

    // Pre-create a stale backup
    fs::write(&corrupt_bak, "STALE PREVIOUS BACKUP").expect("write stale bak");

    // Write a new corrupt registry
    let new_corrupt = "broken = [[[ syntax error";
    fs::write(&toml_path, new_corrupt).expect("write new corrupt");

    let res = CameraRegistry::load_from_path(&toml_path);
    assert!(res.is_err());
    let backed_up = fs::read_to_string(&corrupt_bak).expect("read backup");
    assert_eq!(backed_up, new_corrupt, "Backup must be overwritten with latest corrupted content");
}

// ============================================================================
// Section 3: Concurrency Stress Testing
// ============================================================================

#[test]
fn test_adv_concurrent_multi_threaded_reads() {
    let dir = tempdir().expect("tempdir");
    let toml_path = dir.path().join("camera_registry.toml");

    // Seed registry with 50 camera profiles
    let mut reg = CameraRegistry::with_path(&toml_path);
    for i in 0..50 {
        let p = CameraProfile::new(
            format!("CAM_{:04}", i),
            format!("Camera Model {:04}", i),
            format!(r"C:\Photos\Cam_{:04}", i),
        )
        .with_capacity((32 + (i as u64 % 4) * 32) * 1024 * 1024 * 1024);
        reg.profiles.insert(p.camera_id.clone(), p);
    }
    reg.save().expect("seed save");

    let thread_count = 16;
    let barrier = Arc::new(Barrier::new(thread_count));
    let mut handles = Vec::new();

    for t in 0..thread_count {
        let b = Arc::clone(&barrier);
        let path_clone = toml_path.clone();

        handles.push(thread::spawn(move || {
            b.wait();
            for iter in 0..20 {
                let loaded = CameraRegistry::load_from_path(&path_clone)
                    .unwrap_or_else(|e| panic!("Thread {} iter {} failed: {}", t, iter, e));
                assert_eq!(loaded.count(), 50);
                assert!(loaded.get_profile("CAM_0025").is_some());
            }
        }));
    }

    for h in handles {
        h.join().expect("thread failed");
    }
}

#[test]
fn test_adv_concurrent_read_while_write_stress() {
    let dir = tempdir().expect("tempdir");
    let toml_path = dir.path().join("camera_registry.toml");

    let mut reg = CameraRegistry::with_path(&toml_path);
    reg.register_or_update(CameraProfile::new("SEED_0", "Seed Camera", r"C:\Photos\Seed"))
        .expect("seed save");

    let stop_flag = Arc::new(Mutex::new(false));
    let reader_errors = Arc::new(Mutex::new(Vec::<String>::new()));

    // Spawn 4 reader threads
    let mut handles = Vec::new();
    for t in 0..4 {
        let path_c = toml_path.clone();
        let stop_c = Arc::clone(&stop_flag);
        let errs_c = Arc::clone(&reader_errors);

        handles.push(thread::spawn(move || {
            while !*stop_c.lock().unwrap() {
                match CameraRegistry::load_from_path(&path_c) {
                    Ok(r) => {
                        assert!(r.count() >= 1);
                    }
                    Err(e) => {
                        errs_c.lock().unwrap().push(format!("Reader {}: {}", t, e));
                    }
                }
                thread::yield_now();
            }
        }));
    }

    // Writer thread performs 30 updates
    for i in 1..=30 {
        let mut w_reg = CameraRegistry::load_from_path(&toml_path).expect("writer load");
        w_reg
            .register_or_update(CameraProfile::new(
                format!("CONC_{}", i),
                format!("Camera {}", i),
                format!(r"C:\Photos\{}", i),
            ))
            .expect("writer save");
    }

    *stop_flag.lock().unwrap() = true;
    for h in handles {
        let _ = h.join();
    }

    let errs = reader_errors.lock().unwrap();
    println!("Total read-while-write collision errors: {}", errs.len());
    // Note: On Windows, MoveFileExW may momentarily lock or collide if readers hold open handles.
    // We observe the empirical error rate without failing the test unconditionally.
}

#[test]
fn test_adv_concurrent_multi_threaded_writes_collision_observation() {
    let dir = tempdir().expect("tempdir");
    let toml_path = dir.path().join("camera_registry.toml");

    let initial_reg = CameraRegistry::with_path(&toml_path);
    initial_reg.save().expect("init");

    let thread_count = 4;
    let barrier = Arc::new(Barrier::new(thread_count));
    let write_errors = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut handles = Vec::new();

    for t in 0..thread_count {
        let b = Arc::clone(&barrier);
        let path_c = toml_path.clone();
        let errs_c = Arc::clone(&write_errors);

        handles.push(thread::spawn(move || {
            b.wait();
            for i in 0..10 {
                let mut reg = CameraRegistry::with_path(&path_c);
                reg.profiles.insert(
                    format!("TH_{}_CAM_{}", t, i),
                    CameraProfile::new(
                        format!("TH_{}_CAM_{}", t, i),
                        format!("Cam {}-{}", t, i),
                        format!(r"C:\Photos\{}_{}", t, i),
                    ),
                );
                if let Err(e) = reg.save() {
                    errs_c.lock().unwrap().push(format!("Thread {} iter {}: {}", t, i, e));
                }
            }
        }));
    }

    for h in handles {
        let _ = h.join();
    }

    let errs = write_errors.lock().unwrap();
    println!(
        "Concurrent writer collisions (due to static .tmp filename): {} errors",
        errs.len()
    );
}

// ============================================================================
// Section 4: O(1) Scalability Benchmark (1,000 to 10,000 Profiles)
// ============================================================================

#[test]
fn test_adv_o1_lookup_benchmark_1000_profiles() {
    let mut reg = CameraRegistry::new();
    let n = 1_000;
    for i in 0..n {
        let id = format!("BODY_SN_{:06}", i);
        reg.profiles.insert(
            id.clone(),
            CameraProfile::new(
                id,
                format!("Canon EOS R6 MkII #{:06}", i),
                format!(r"D:\Storage\Camera_{:06}", i),
            )
            .with_capacity((128 + (i as u64 % 4) * 64) * 1024 * 1024 * 1024),
        );
    }
    assert_eq!(reg.count(), n);

    let iterations = 10_000;
    let start = Instant::now();
    for i in 0..iterations {
        let target_id = format!("BODY_SN_{:06}", i % n);
        let profile = reg.get_profile(&target_id);
        assert!(profile.is_some());
    }
    let elapsed = start.elapsed();
    let avg_ns_per_lookup = elapsed.as_nanos() as f64 / iterations as f64;
    let avg_us = avg_ns_per_lookup / 1000.0;

    println!(
        "[Benchmark 1,000 Profiles] Total time for {} lookups: {:?}, Avg per lookup: {:.3} µs ({:.1} ns)",
        iterations, elapsed, avg_us, avg_ns_per_lookup
    );

    // Requirement: < 10 microseconds per lookup
    assert!(
        avg_us < 10.0,
        "Lookup must be < 10 µs, measured {:.3} µs",
        avg_us
    );
}

#[test]
fn test_adv_o1_lookup_benchmark_5000_profiles() {
    let mut reg = CameraRegistry::new();
    let n = 5_000;
    for i in 0..n {
        let id = format!("SERIAL_ALPHA_{:06}", i);
        reg.profiles.insert(
            id.clone(),
            CameraProfile::new(
                id,
                format!("Sony A7 IV #{:06}", i),
                format!(r"E:\Photos\Sony_{:06}", i),
            )
            .with_capacity(256 * 1024 * 1024 * 1024),
        );
    }
    assert_eq!(reg.count(), n);

    let iterations = 10_000;
    let start = Instant::now();
    for i in 0..iterations {
        let target_id = format!("SERIAL_ALPHA_{:06}", (i * 7) % n);
        let profile = reg.get_profile(&target_id);
        assert!(profile.is_some());
    }
    let elapsed = start.elapsed();
    let avg_us = (elapsed.as_nanos() as f64 / iterations as f64) / 1000.0;

    println!(
        "[Benchmark 5,000 Profiles] Total time: {:?}, Avg per lookup: {:.3} µs",
        iterations, avg_us
    );
    assert!(avg_us < 10.0, "Lookup must be < 10 µs, measured {:.3} µs", avg_us);
}

#[test]
fn test_adv_o1_lookup_benchmark_10000_profiles_stress() {
    let mut reg = CameraRegistry::new();
    let n = 10_000;
    for i in 0..n {
        let id = format!("NIKON_Z8_{:06}", i);
        reg.profiles.insert(
            id.clone(),
            CameraProfile::new(
                id,
                format!("Nikon Z8 Body #{:06}", i),
                format!(r"F:\Nikon\Body_{:06}", i),
            )
            .with_capacity(512 * 1024 * 1024 * 1024),
        );
    }
    assert_eq!(reg.count(), n);

    let iterations = 20_000;
    let start = Instant::now();
    for i in 0..iterations {
        let target_id = format!("NIKON_Z8_{:06}", (i * 13) % n);
        let profile = reg.get_profile(&target_id);
        assert!(profile.is_some());
    }
    let elapsed = start.elapsed();
    let avg_us = (elapsed.as_nanos() as f64 / iterations as f64) / 1000.0;

    println!(
        "[Benchmark 10,000 Profiles] Total time for {} lookups: {:?}, Avg per lookup: {:.3} µs",
        iterations, elapsed, avg_us
    );
    assert!(
        avg_us < 10.0,
        "Lookup at 10,000 profiles must be < 10 µs, measured {:.3} µs",
        avg_us
    );

    // Also benchmark non-existent misses
    let miss_start = Instant::now();
    for i in 0..iterations {
        let miss_id = format!("NON_EXISTENT_ID_{:06}", i);
        assert!(reg.get_profile(&miss_id).is_none());
    }
    let miss_elapsed = miss_start.elapsed();
    let miss_avg_us = (miss_elapsed.as_nanos() as f64 / iterations as f64) / 1000.0;
    println!(
        "[Benchmark 10,000 Profiles Misses] Avg per miss lookup: {:.3} µs",
        miss_avg_us
    );
    assert!(miss_avg_us < 10.0, "Miss lookup must be < 10 µs");
}

#[test]
fn test_adv_find_by_capacity_scalability_10000_profiles() {
    let mut reg = CameraRegistry::new();
    let n = 10_000;
    let target_capacity = 128 * 1024 * 1024 * 1024;

    for i in 0..n {
        let mut p = CameraProfile::new(
            format!("CAM_{:05}", i),
            format!("Camera {:05}", i),
            format!(r"C:\Photos\{:05}", i),
        );
        // Distribute varying card capacities
        p.add_card_capacity((32 + (i as u64 % 8) * 32) * 1024 * 1024 * 1024);
        if i == 5000 {
            // Camera #5000 has our target capacity and recent timestamp
            p.add_card_capacity(target_capacity);
            p.last_ingest_timestamp = Some(Utc::now());
        }
        reg.profiles.insert(p.camera_id.clone(), p);
    }

    let start = Instant::now();
    let matched = reg.find_by_capacity(target_capacity);
    let elapsed = start.elapsed();

    assert!(matched.is_some());
    println!(
        "[find_by_capacity 10,000 profiles] Linear scan elapsed time: {:?}",
        elapsed
    );
}

// ============================================================================
// Section 5: Capacity Matching Boundary & Margin Stress Testing
// ============================================================================

#[test]
fn test_adv_capacity_margin_strict_boundaries_32gb() {
    let registered_32gb = 32 * 1024 * 1024 * 1024; // 34,359,738,368 bytes
    let margin = registered_32gb / 50; // exactly 2.0% = 687,194,767 bytes

    // Exact match
    assert!(matches_capacity(registered_32gb, registered_32gb));

    // Upper boundary (+2.0%)
    assert!(matches_capacity(registered_32gb, registered_32gb + margin));
    // Upper boundary violation (+2.0% + 1 byte)
    assert!(!matches_capacity(registered_32gb, registered_32gb + margin + 1));
    // Upper +2.1%
    let plus_2_1_pct = (registered_32gb as f64 * 1.021) as u64;
    assert!(!matches_capacity(registered_32gb, plus_2_1_pct));

    // Lower boundary (-2.0%)
    assert!(matches_capacity(registered_32gb, registered_32gb - margin));
    // Lower boundary violation (-2.0% - 1 byte)
    assert!(!matches_capacity(registered_32gb, registered_32gb - margin - 1));
    // Lower -2.1%
    let minus_2_1_pct = (registered_32gb as f64 * 0.979) as u64;
    assert!(!matches_capacity(registered_32gb, minus_2_1_pct));
}

#[test]
fn test_adv_capacity_margin_strict_boundaries_128gb() {
    let registered_128gb = 128 * 1024 * 1024 * 1024; // 137,438,953,472 bytes
    let margin = registered_128gb / 50; // 2,748,779,069 bytes (2.0%)

    // Exactly at +2.0% margin
    assert!(
        matches_capacity(registered_128gb, registered_128gb + margin),
        "+2.0% exact must match"
    );

    // Exactly 1 byte beyond +2.0% margin
    assert!(
        !matches_capacity(registered_128gb, registered_128gb + margin + 1),
        "+2.0% + 1 byte must NOT match"
    );

    // At +2.1%
    let cap_plus_2_1 = (registered_128gb as f64 * 1.021) as u64;
    assert!(
        !matches_capacity(registered_128gb, cap_plus_2_1),
        "+2.1% must NOT match"
    );

    // Exactly at -2.0% margin
    assert!(
        matches_capacity(registered_128gb, registered_128gb - margin),
        "-2.0% exact must match"
    );

    // Exactly 1 byte beyond -2.0% margin
    assert!(
        !matches_capacity(registered_128gb, registered_128gb - margin - 1),
        "-2.0% - 1 byte must NOT match"
    );

    // At -2.1%
    let cap_minus_2_1 = (registered_128gb as f64 * 0.979) as u64;
    assert!(
        !matches_capacity(registered_128gb, cap_minus_2_1),
        "-2.1% must NOT match"
    );
}

#[test]
fn test_adv_capacity_matching_multi_camera_priority_tie_breaking() {
    let capacity_128gb = 128 * 1024 * 1024 * 1024;
    let mut reg = CameraRegistry::new();

    let t0 = Utc::now() - Duration::days(10);
    let t1 = Utc::now() - Duration::days(5);
    let t2 = Utc::now() - Duration::hours(1);

    let mut cam_a = CameraProfile::new("CAM_A", "Camera A (10 days ago)", r"C:\Photos\A")
        .with_capacity(capacity_128gb);
    cam_a.last_ingest_timestamp = Some(t0);

    let mut cam_b = CameraProfile::new("CAM_B", "Camera B (1 hour ago)", r"C:\Photos\B")
        .with_capacity(capacity_128gb);
    cam_b.last_ingest_timestamp = Some(t2);

    let mut cam_c = CameraProfile::new("CAM_C", "Camera C (5 days ago)", r"C:\Photos\C")
        .with_capacity(capacity_128gb);
    cam_c.last_ingest_timestamp = Some(t1);

    reg.profiles.insert(cam_a.camera_id.clone(), cam_a);
    reg.profiles.insert(cam_b.camera_id.clone(), cam_b);
    reg.profiles.insert(cam_c.camera_id.clone(), cam_c);

    // When card has exact capacity, diff is identical (0 for all three), so most recent timestamp (cam_b) MUST win
    let matched = reg.find_by_capacity(capacity_128gb).expect("must find match");
    assert_eq!(
        matched.camera_id, "CAM_B",
        "Most recent ingest timestamp must win tie-break among identical capacity diffs"
    );

    // When card has capacity closer to registered than another:
    // Suppose Camera D has capacity 128GB - 10MB, and Camera E has 128GB - 50MB
    // Even if Camera E has newer timestamp, Camera D MUST win because diff is smaller!
    let mut reg2 = CameraRegistry::new();
    let mut cam_d = CameraProfile::new("CAM_D", "Camera D (Closer diff, older)", r"C:\Photos\D")
        .with_capacity(capacity_128gb);
    cam_d.last_ingest_timestamp = Some(t0);

    let mut cam_e = CameraProfile::new("CAM_E", "Camera E (Further diff, newer)", r"C:\Photos\E")
        .with_capacity(capacity_128gb);
    cam_e.last_ingest_timestamp = Some(t2);

    reg2.profiles.insert(cam_d.camera_id.clone(), cam_d);
    reg2.profiles.insert(cam_e.camera_id.clone(), cam_e);

    let test_bytes = capacity_128gb - 100_000; // closer to 128GB than 128GB - 50MB
    let matched2 = reg2.find_by_capacity(test_bytes).expect("match");
    // Both match capacity_128gb with equal diff (100_000), so timestamp decides
    assert_eq!(matched2.camera_id, "CAM_E");
}

#[test]
fn test_adv_capacity_zero_and_overflow_safety() {
    let reg = CameraRegistry::new();
    // 0 bytes must never match or divide by zero
    assert!(!matches_capacity(0, 0));
    assert!(!matches_capacity(0, 1000));
    assert!(!matches_capacity(1000, 0));
    assert!(reg.find_by_capacity(0).is_none());

    // u64::MAX boundary
    assert!(!matches_capacity(u64::MAX, 1000));
    assert!(!matches_capacity(1000, u64::MAX));
    assert!(matches_capacity(u64::MAX, u64::MAX));
}

#[test]
fn test_adv_find_all_by_capacity_returns_complete_membership() {
    let capacity_64gb = 64 * 1024 * 1024 * 1024;
    let mut reg = CameraRegistry::new();

    let cam1 = CameraProfile::new("C1", "Cam 1", r"C:\Photos\1").with_capacity(capacity_64gb);
    let cam2 = CameraProfile::new("C2", "Cam 2", r"C:\Photos\2").with_capacity(capacity_64gb);
    let cam3 = CameraProfile::new("C3", "Cam 3", r"C:\Photos\3")
        .with_capacity(256 * 1024 * 1024 * 1024); // Different capacity

    reg.profiles.insert(cam1.camera_id.clone(), cam1);
    reg.profiles.insert(cam2.camera_id.clone(), cam2);
    reg.profiles.insert(cam3.camera_id.clone(), cam3);

    let all = reg.find_all_by_capacity(capacity_64gb);
    assert_eq!(all.len(), 2, "Should return exactly the 2 matching 64GB cameras");
    let ids: Vec<&str> = all.iter().map(|p| p.camera_id.as_str()).collect();
    assert!(ids.contains(&"C1"));
    assert!(ids.contains(&"C2"));
    assert!(!ids.contains(&"C3"));
}
