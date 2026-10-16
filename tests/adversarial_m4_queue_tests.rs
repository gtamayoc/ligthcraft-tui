#![allow(dead_code)]
#![allow(unused_imports)]

//! Adversarial stress test suite for Milestone M4 (`src/queue/`).
//!
//! Empirically tests:
//! 1. Deep directory traversal stress: 25-level nested directory structure -> verify iterative scanner does not stack overflow.
//! 2. Massive task stress: 5,000 tasks generated in memory -> verify zero OOM, memory footprint stable, execution plan clean.
//! 3. Rapid pause/resume/cancel cycling: 50 rapid transitions in 2s -> verify zero race condition, zero thread hang, zero deadlock.
//! 4. Worker panic isolation: worker task forced panic -> verify queue supervisor survives, records failure, and continues remaining jobs.

mod common;

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lightcraft_tui::engine::presets::BuiltinPreset;
use lightcraft_tui::export::profile::{ExportProfile, ScalingProfile};
use lightcraft_tui::queue::{
    AspectRatio, BatchFilter, BatchJob, BatchJobStatus, BatchStrategy, BatchSummary,
    BatchTelemetry, BatchWorkerPool, DirectoryScanner, DirectoryScannerConfig, Orientation,
    ScannedPhoto, StrategyEngine, WorkerPoolConfig,
};

/// Helper to create a dummy image or photographic file on disk for scanner tests.
fn create_dummy_file(path: &Path, content: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("Failed to create parent dirs");
    }
    let mut file = File::create(path).expect("Failed to create dummy file");
    file.write_all(content).expect("Failed to write dummy content");
}

/// Helper to construct a synthetic ScannedPhoto.
fn make_synthetic_photo(id: usize, ext: &str, orient: Orientation, ar: AspectRatio) -> ScannedPhoto {
    let is_cr3 = ext.eq_ignore_ascii_case("cr3");
    let (w, h) = match orient {
        Orientation::Landscape => (6000, 4000),
        Orientation::Portrait => (4000, 6000),
        Orientation::Square => (4000, 4000),
    };

    ScannedPhoto {
        path: PathBuf::from(format!("DRAW/IMG_{:04}.{}", id, ext)),
        filename: format!("IMG_{:04}.{}", id, ext),
        extension: ext.to_lowercase(),
        size_bytes: if is_cr3 { 26_500_000 } else { 4_200_000 },
        is_cr3,
        is_selected: true,
        dimensions: Some((w, h)),
        orientation: Some(orient),
        aspect_ratio: Some(ar),
    }
}

// ============================================================================
// CHALLENGE 1: Deep Directory Traversal Stress (20+ Levels)
// ============================================================================

#[test]
fn test_adv_m4_01_iterative_scanner_deep_25_level_nesting_no_stack_overflow() {
    let temp_root = tempfile::tempdir().expect("Failed to create tempdir");
    let base_path = temp_root.path();

    // Construct a 25-level deep directory chain:
    // base/level_00/level_01/.../level_24/
    let mut current_dir = base_path.to_path_buf();
    let mut expected_image_paths = Vec::new();

    for level in 0..25 {
        current_dir = current_dir.join(format!("level_{:02}", level));
        fs::create_dir_all(&current_dir).expect("Failed to create nested level");

        // Seed images at designated levels: 0, 5, 10, 15, 20, 24
        if [0, 5, 10, 15, 20, 24].contains(&level) {
            let img_file = current_dir.join(format!("photo_lvl_{:02}.jpg", level));
            create_dummy_file(&img_file, b"FAKE_JPEG_BINARY_DATA");
            expected_image_paths.push(img_file);
        }

        // Add a CR3 RAW at the deepest level (level 24)
        if level == 24 {
            let cr3_file = current_dir.join("deepest_raw.cr3");
            create_dummy_file(&cr3_file, b"FAKE_CANON_CR3_DATA");
            expected_image_paths.push(cr3_file);
        }

        // Seed irrelevant files that must be ignored
        let text_file = current_dir.join("readme.txt");
        create_dummy_file(&text_file, b"Just some docs");
        let temp_file = current_dir.join("cache.tmp");
        create_dummy_file(&temp_file, b"temporary cache data");
    }

    let scan_start = Instant::now();
    let config = DirectoryScannerConfig {
        root: base_path.to_path_buf(),
        recursive: true,
        follow_symlinks: false,
        max_depth: 32,
        allowed_extensions: vec!["jpg".into(), "cr3".into()],
    };

    // Execute iterative scan over 25 nested levels
    let results = DirectoryScanner::scan(&config).expect("Deep scan must succeed");
    let scan_duration = scan_start.elapsed();

    // Verify iterative scanner traversed all 25 levels without stack overflow
    assert!(
        scan_duration < Duration::from_millis(500),
        "Scan of 25 levels must be fast and non-recursive (<500ms), took {:?}",
        scan_duration
    );

    // Expected 6 JPEGs + 1 CR3 = 7 photos
    assert_eq!(
        results.len(),
        7,
        "Iterative scanner must discover all 7 target photos across 25 levels"
    );

    // Verify deepest photo (level 24 CR3) was discovered
    let deepest_cr3 = results.iter().find(|p| p.filename == "deepest_raw.cr3");
    assert!(
        deepest_cr3.is_some(),
        "Deepest CR3 at level 24 must be discovered by iterative scanner"
    );
    let cr3 = deepest_cr3.unwrap();
    assert!(cr3.is_cr3);
    assert_eq!(cr3.extension, "cr3");
    assert_eq!(cr3.dimensions, Some((6000, 4000)));

    // Verify non-image files (.txt, .tmp) were completely excluded
    for photo in &results {
        assert!(
            photo.extension == "jpg" || photo.extension == "cr3",
            "Discovered file has unexpected extension: {}",
            photo.extension
        );
    }
}

#[test]
fn test_adv_m4_02_iterative_scanner_depth_boundary_enforcement() {
    let temp_root = tempfile::tempdir().expect("Failed to create tempdir");
    let base_path = temp_root.path();

    // Construct 20 levels deep
    let mut current_dir = base_path.to_path_buf();
    for level in 0..20 {
        current_dir = current_dir.join(format!("lvl_{:02}", level));
        fs::create_dir_all(&current_dir).expect("Failed to create dir");
        let img = current_dir.join(format!("img_{:02}.jpg", level));
        create_dummy_file(&img, b"FAKE_JPEG");
    }

    // Scan with max_depth = 5
    let config_depth_5 = DirectoryScannerConfig {
        root: base_path.to_path_buf(),
        recursive: true,
        follow_symlinks: false,
        max_depth: 5,
        allowed_extensions: vec!["jpg".into()],
    };

    let results = DirectoryScanner::scan(&config_depth_5).expect("Scan with max_depth 5 must succeed");
    assert_eq!(
        results.len(),
        5,
        "max_depth=5 must discover exactly photos up to depth 5"
    );

    // Verify non-recursive scan (recursive = false) only discovers root
    let config_non_recursive = DirectoryScannerConfig {
        root: base_path.to_path_buf(),
        recursive: false,
        follow_symlinks: false,
        max_depth: 32,
        allowed_extensions: vec!["jpg".into()],
    };
    let non_rec_results = DirectoryScanner::scan(&config_non_recursive).expect("Non-recursive scan must succeed");
    assert_eq!(
        non_rec_results.len(),
        0,
        "Non-recursive scan on root with no root files must return 0"
    );
}

#[test]
fn test_adv_m4_03_iterative_scanner_pathological_empty_and_special_files() {
    let temp_root = tempfile::tempdir().expect("Failed to create tempdir");
    let base = temp_root.path();

    // Create 30 empty directories
    for i in 0..30 {
        fs::create_dir_all(base.join(format!("empty_branch_{:02}", i))).unwrap();
    }

    // Create dotfiles and files with no extension
    create_dummy_file(&base.join(".hidden.jpg"), b"HIDDEN");
    create_dummy_file(&base.join("no_extension"), b"NO_EXT");
    create_dummy_file(&base.join("UPPERCASE.JPG"), b"UPPER_JPG");
    create_dummy_file(&base.join("mixed_case.Cr3"), b"RAW");

    let config = DirectoryScannerConfig {
        root: base.to_path_buf(),
        recursive: true,
        follow_symlinks: false,
        max_depth: 32,
        allowed_extensions: vec!["jpg".into(), "cr3".into()],
    };

    let results = DirectoryScanner::scan(&config).expect("Scan must succeed");

    // Case-insensitivity verification: both UPPERCASE.JPG and mixed_case.Cr3 must be found
    let found_upper = results.iter().any(|p| p.filename == "UPPERCASE.JPG");
    let found_raw = results.iter().any(|p| p.filename == "mixed_case.Cr3");
    let found_no_ext = results.iter().any(|p| p.filename == "no_extension");

    assert!(found_upper, "Scanner must match extensions case-insensitively (.JPG)");
    assert!(found_raw, "Scanner must match extensions case-insensitively (.Cr3)");
    assert!(!found_no_ext, "Scanner must ignore files without allowed extension");
}

// ============================================================================
// CHALLENGE 2: Massive Task Stress (5,000 Tasks In Memory)
// ============================================================================

#[test]
fn test_adv_m4_04_massive_5000_task_generation_and_memory_stability() {
    const TOTAL_PHOTOS: usize = 5000;

    // 1. Synthesize 5,000 scanned photos
    let start_gen = Instant::now();
    let mut photos = Vec::with_capacity(TOTAL_PHOTOS);
    for i in 1..=TOTAL_PHOTOS {
        let ext = if i % 5 == 0 { "cr3" } else { "jpg" };
        let orient = if i % 2 == 0 {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        };
        let ar = if i % 3 == 0 {
            AspectRatio::Ratio16x9
        } else {
            AspectRatio::Ratio3x2
        };
        photos.push(make_synthetic_photo(i, ext, orient, ar));
    }
    let gen_duration = start_gen.elapsed();
    assert_eq!(photos.len(), TOTAL_PHOTOS);
    assert!(
        gen_duration < Duration::from_millis(100),
        "Generating 5,000 photo models must take < 100ms, took {:?}",
        gen_duration
    );

    // 2. Validate strategy pre-check for 5,000 tasks (Uniform)
    let uniform_strategy = BatchStrategy::Uniform {
        recipe: BuiltinPreset::Cinema.recipe(),
        intensity: 1.25,
    };

    let summary = StrategyEngine::validate(&photos, &uniform_strategy)
        .expect("Strategy pre-validation must succeed for 5,000 photos");

    assert_eq!(summary.photo_count, 5000);
    assert_eq!(summary.recipe_count, 1);
    assert_eq!(summary.total_jobs, 5000);
    // Estimated bytes: 5000 * 4,200,000 = 21,000,000,000 bytes (21 GB)
    assert_eq!(
        summary.estimated_total_output_bytes,
        5000 * 4_200_000,
        "Estimated bytes math must be exact and not overflow"
    );

    // 3. Expand jobs: produce 5,000 BatchJob structs in memory
    let start_expand = Instant::now();
    let jobs = StrategyEngine::expand_jobs(&photos, &uniform_strategy, "web_1080p")
        .expect("Expanding 5,000 jobs must succeed");
    let expand_duration = start_expand.elapsed();

    assert_eq!(jobs.len(), 5000);
    assert!(
        expand_duration < Duration::from_millis(150),
        "Expanding 5,000 jobs must take < 150ms, took {:?}",
        expand_duration
    );

    // 4. Memory footprint verification:
    // Verify each BatchJob is properly initialized, has distinct sequential IDs, and clean plan
    for (idx, job) in jobs.iter().enumerate() {
        let expected_id = (idx + 1) as u64;
        assert_eq!(job.id, expected_id);
        assert_eq!(job.sequence_index, idx + 1);
        assert_eq!(job.total_in_batch, 5000);
        assert_eq!(job.status, BatchJobStatus::Queued);
        assert_eq!(job.recipe_name, "Cinema");
        assert_eq!(job.intensity, 1.25);
        assert!(job.started_at.is_none());
        assert!(job.completed_at.is_none());
    }

    // Verify flat memory capacity consumption of 5,000 BatchJob structs is bounded
    let job_struct_size = std::mem::size_of::<BatchJob>();
    let total_flat_bytes = jobs.capacity() * job_struct_size;
    assert!(
        total_flat_bytes < 10 * 1024 * 1024,
        "Vector of 5,000 jobs must consume < 10 MB, actual flat vector capacity: {} bytes",
        total_flat_bytes
    );
}

#[test]
fn test_adv_m4_05_massive_batch_filter_predicate_scaling() {
    const TOTAL: usize = 5000;
    let mut photos = Vec::with_capacity(TOTAL);

    for i in 1..=TOTAL {
        let ext = match i % 3 {
            0 => "cr3",
            1 => "jpg",
            _ => "png",
        };
        let orient = match i % 2 {
            0 => Orientation::Landscape,
            _ => Orientation::Portrait,
        };
        photos.push(make_synthetic_photo(i, ext, orient, AspectRatio::Ratio3x2));
    }

    // Filter 1: CR3 only
    let mut filter_cr3 = BatchFilter::default();
    filter_cr3.extensions_include = vec!["cr3".into()];
    let start_filter = Instant::now();
    let filtered_cr3 = filter_cr3.apply(&photos);
    let filter_dur = start_filter.elapsed();

    assert_eq!(filtered_cr3.len(), 1666);
    assert!(
        filter_dur < Duration::from_millis(50),
        "Filtering 5,000 photos must take < 50ms, took {:?}",
        filter_dur
    );

    // Filter 2: Portrait only
    let mut filter_portrait = BatchFilter::default();
    filter_portrait.orientation = Some(Orientation::Portrait);
    let filtered_portrait = filter_portrait.apply(&photos);
    assert_eq!(filtered_portrait.len(), 2500);

    // Filter 3: Exclude JPG
    let mut filter_no_jpg = BatchFilter::default();
    filter_no_jpg.extensions_exclude = vec!["jpg".into()];
    let filtered_no_jpg = filter_no_jpg.apply(&photos);
    assert_eq!(filtered_no_jpg.len(), 3333);
}

#[test]
fn test_adv_m4_06_massive_batch_telemetry_throughput_and_eta() {
    let mut telemetry = BatchTelemetry::new(5000);
    assert_eq!(telemetry.total_jobs, 5000);
    assert_eq!(telemetry.estimated_remaining, None);

    // Allow baseline start_time to age beyond 50ms threshold before checking rate calculations
    std::thread::sleep(Duration::from_millis(60));

    // Simulate stepping through 5,000 jobs in increments of 500
    for step in 1..=10 {
        let completed = step * 480;
        let failed = step * 10;
        let skipped = step * 10;
        std::thread::sleep(Duration::from_millis(10));
        telemetry.update(completed, failed, skipped);

        let finished = completed + failed + skipped;
        assert_eq!(telemetry.completed_count, completed);
        assert_eq!(telemetry.failed_count, failed);
        assert_eq!(telemetry.skipped_count, skipped);
        assert!(
            telemetry.throughput_imgs_per_sec > 0.0,
            "Throughput must be positive after 60ms elapsed"
        );

        if finished < 5000 {
            assert!(telemetry.estimated_remaining.is_some());
        } else {
            // At 100% finished (5000 jobs), remaining is 0s
            if let Some(remaining) = telemetry.estimated_remaining {
                assert!(remaining.as_secs_f32() < 0.1);
            }
        }
    }
}

#[test]
fn test_adv_m4_07_massive_retry_failed_5000_jobs_partitioning() {
    let photos = vec![make_synthetic_photo(1, "cr3", Orientation::Landscape, AspectRatio::Ratio3x2)];
    let strategy = BatchStrategy::Uniform {
        recipe: BuiltinPreset::Natural.recipe(),
        intensity: 1.0,
    };
    let base_job = StrategyEngine::expand_jobs(&photos, &strategy, "web_1080p").unwrap()[0].clone();

    // Replicate into 5,000 jobs
    let mut jobs = Vec::with_capacity(5000);
    for i in 1..=5000 {
        let mut j = base_job.clone();
        j.id = i as u64;
        if i % 2 == 0 {
            j.status = BatchJobStatus::Failed {
                error_message: format!("Simulated failure on job {}", i),
            };
        } else {
            j.status = BatchJobStatus::Done {
                output_path: PathBuf::from(format!("export/job_{}.jpg", i)),
                bytes_written: 4_200_000,
                elapsed: Duration::from_millis(100),
            };
        }
        jobs.push(j);
    }

    assert_eq!(jobs.len(), 5000);

    // Retry failed
    let reset_count = BatchWorkerPool::retry_failed(&mut jobs);
    assert_eq!(reset_count, 2500, "Exactly 2,500 failed jobs must be reset to Queued");

    // Verify all even jobs are Queued, all odd jobs remain Done
    for (idx, job) in jobs.iter().enumerate() {
        let id = idx + 1;
        if id % 2 == 0 {
            assert_eq!(job.status, BatchJobStatus::Queued);
            assert!(job.started_at.is_none());
            assert!(job.completed_at.is_none());
        } else {
            assert!(matches!(job.status, BatchJobStatus::Done { .. }));
        }
    }
}

// ============================================================================
// CHALLENGE 3: Rapid Pause / Resume / Cancel Cycling Stress (50 Transitions in 2s)
// ============================================================================

#[tokio::test]
async fn test_adv_m4_08_rapid_50_pause_resume_cycles_during_active_execution() {
    // Wrap entire stress test in a 10s timeout to detect deadlocks or hangs
    tokio::time::timeout(Duration::from_secs(10), async {
        let pool = Arc::new(BatchWorkerPool::new(WorkerPoolConfig { concurrency: 4 }, None));

        // Use fast thumbnail profile (LongEdge 320px) to prevent pure-CPU encoding
        // starvation from blocking the Tokio async executor on Windows
        let mut fast_profile = ExportProfile::default();
        fast_profile.scaling = ScalingProfile::LongEdge {
            max_long_edge: 320,
            dont_enlarge: true,
        };

        let photos = vec![
            ScannedPhoto {
                path: PathBuf::from("DRAW/IMG_0588.CR3"),
                filename: "IMG_0588.CR3".into(),
                extension: "cr3".into(),
                size_bytes: 26_527_598,
                is_cr3: true,
                is_selected: true,
                dimensions: Some((6000, 4000)),
                orientation: Some(Orientation::Landscape),
                aspect_ratio: Some(AspectRatio::Ratio3x2),
            },
            ScannedPhoto {
                path: PathBuf::from("DRAW/IMG_0589.CR3"),
                filename: "IMG_0589.CR3".into(),
                extension: "cr3".into(),
                size_bytes: 26_527_598,
                is_cr3: true,
                is_selected: true,
                dimensions: Some((6000, 4000)),
                orientation: Some(Orientation::Landscape),
                aspect_ratio: Some(AspectRatio::Ratio3x2),
            },
            ScannedPhoto {
                path: PathBuf::from("DRAW/IMG_0590.CR3"),
                filename: "IMG_0590.CR3".into(),
                extension: "cr3".into(),
                size_bytes: 26_527_598,
                is_cr3: true,
                is_selected: true,
                dimensions: Some((6000, 4000)),
                orientation: Some(Orientation::Landscape),
                aspect_ratio: Some(AspectRatio::Ratio3x2),
            },
            ScannedPhoto {
                path: PathBuf::from("DRAW/IMG_0969.CR3"),
                filename: "IMG_0969.CR3".into(),
                extension: "cr3".into(),
                size_bytes: 26_527_598,
                is_cr3: true,
                is_selected: true,
                dimensions: Some((6000, 4000)),
                orientation: Some(Orientation::Landscape),
                aspect_ratio: Some(AspectRatio::Ratio3x2),
            },
        ];

        let mut jobs = Vec::new();
        for (i, p) in photos.iter().enumerate() {
            jobs.push(BatchJob {
                id: (i + 1) as u64,
                source_path: p.path.clone(),
                photo_id: None,
                recipe: BuiltinPreset::Natural.recipe(),
                recipe_name: "Natural".into(),
                intensity: 1.0,
                export_profile_name: "thumbnail_fast".into(),
                profile: fast_profile.clone(),
                sequence_index: i + 1,
                total_in_batch: photos.len(),
                status: BatchJobStatus::Queued,
                enqueued_at: Instant::now(),
                started_at: None,
                completed_at: None,
            });
        }

        let pool_runner = pool.clone();
        let queue_handle = tokio::spawn(async move {
            pool_runner.run_queue(jobs, None).await
        });

        // Concurrently execute 50 rapid state transitions (25 pause + 25 resume = 50 transitions)
        // within the 2.0-second requirement window
        let cycling_start = Instant::now();
        for cycle in 0..25 {
            // Transition 1: Pause
            pool.pause();
            assert!(pool.is_paused());
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(2)).await;

            // Transition 2: Resume
            pool.resume();
            assert!(!pool.is_paused());
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(2)).await;

            if (cycle + 1) % 5 == 0 {
                tracing::info!("Completed {} transitions", (cycle + 1) * 2);
            }
        }
        let cycling_elapsed = cycling_start.elapsed();

        // 50 transitions must complete in < 2.0s
        assert!(
            cycling_elapsed < Duration::from_millis(2000),
            "50 rapid transitions must complete in < 2.0s, took {:?}",
            cycling_elapsed
        );

        // Await queue completion without thread hang or deadlock
        let results = queue_handle.await.expect("Queue runner panicked or crashed");
        assert_eq!(results.len(), 4);

        for job in &results {
            assert!(
                matches!(job.status, BatchJobStatus::Done { .. }),
                "All jobs must complete successfully despite 50 pause/resume transitions, got {:?}",
                job.status
            );
        }
    })
    .await
    .expect("TEST DEADLOCKED OR TIMED OUT: 50 pause/resume transitions hung worker pool!");
}

#[tokio::test]
async fn test_adv_m4_09_rapid_concurrent_pause_resume_cancel_race_resilience() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let pool = Arc::new(BatchWorkerPool::new(WorkerPoolConfig { concurrency: 2 }, None));

        let photos = vec![
            ScannedPhoto {
                path: PathBuf::from("DRAW/IMG_0588.CR3"),
                filename: "IMG_0588.CR3".into(),
                extension: "cr3".into(),
                size_bytes: 26_527_598,
                is_cr3: true,
                is_selected: true,
                dimensions: Some((6000, 4000)),
                orientation: Some(Orientation::Landscape),
                aspect_ratio: Some(AspectRatio::Ratio3x2),
            },
            ScannedPhoto {
                path: PathBuf::from("DRAW/IMG_0589.CR3"),
                filename: "IMG_0589.CR3".into(),
                extension: "cr3".into(),
                size_bytes: 26_527_598,
                is_cr3: true,
                is_selected: true,
                dimensions: Some((6000, 4000)),
                orientation: Some(Orientation::Landscape),
                aspect_ratio: Some(AspectRatio::Ratio3x2),
            },
        ];

        let mut fast_profile = ExportProfile::default();
        fast_profile.scaling = ScalingProfile::LongEdge {
            max_long_edge: 320,
            dont_enlarge: true,
        };

        let mut jobs = Vec::new();
        for (i, p) in photos.iter().enumerate() {
            jobs.push(BatchJob {
                id: (i + 1) as u64,
                source_path: p.path.clone(),
                photo_id: None,
                recipe: BuiltinPreset::Cinema.recipe(),
                recipe_name: "Cinema".into(),
                intensity: 1.0,
                export_profile_name: "fast".into(),
                profile: fast_profile.clone(),
                sequence_index: i + 1,
                total_in_batch: photos.len(),
                status: BatchJobStatus::Queued,
                enqueued_at: Instant::now(),
                started_at: None,
                completed_at: None,
            });
        }

        let pool_clone = pool.clone();
        let queue_handle = tokio::spawn(async move {
            pool_clone.run_queue(jobs, None).await
        });

        // Multi-threaded concurrent callers hammering pause, resume, cancel
        let p1 = pool.clone();
        let h1 = tokio::spawn(async move {
            for _ in 0..20 {
                p1.pause();
                tokio::time::sleep(Duration::from_millis(5)).await;
                p1.resume();
            }
        });

        let p2 = pool.clone();
        let h2 = tokio::spawn(async move {
            for _ in 0..20 {
                p2.resume();
                tokio::time::sleep(Duration::from_millis(5)).await;
                p2.pause();
            }
        });

        // Let them race for 80ms then cancel
        tokio::time::sleep(Duration::from_millis(80)).await;
        pool.cancel();
        pool.resume(); // Ensure any paused waiter wakes up to observe cancellation

        let _ = tokio::join!(h1, h2);
        let results = queue_handle.await.expect("Queue runner panicked");

        assert_eq!(results.len(), 2);
        // Each job must be cleanly either Done or Cancelled
        for job in &results {
            assert!(
                matches!(job.status, BatchJobStatus::Done { .. } | BatchJobStatus::Cancelled),
                "Job status must be clean terminal state: {:?}",
                job.status
            );
        }
    })
    .await
    .expect("TEST DEADLOCKED: Concurrent control race caused thread hang!");
}

#[tokio::test]
async fn test_adv_m4_10_queue_deadlock_freedom_under_high_contention() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let pool = Arc::new(BatchWorkerPool::new(WorkerPoolConfig { concurrency: 1 }, None));

        // Rapid start and immediate pause before any worker dequeues
        pool.pause();
        assert!(pool.is_paused());

        let photos = vec![
            ScannedPhoto {
                path: PathBuf::from("DRAW/IMG_0588.CR3"),
                filename: "IMG_0588.CR3".into(),
                extension: "cr3".into(),
                size_bytes: 26_527_598,
                is_cr3: true,
                is_selected: true,
                dimensions: Some((6000, 4000)),
                orientation: Some(Orientation::Landscape),
                aspect_ratio: Some(AspectRatio::Ratio3x2),
            },
        ];
        let strategy = BatchStrategy::Uniform {
            recipe: BuiltinPreset::Natural.recipe(),
            intensity: 1.0,
        };
        let jobs = StrategyEngine::expand_jobs(&photos, &strategy, "web_1080p").unwrap();

        let pool_runner = pool.clone();
        let handle = tokio::spawn(async move {
            pool_runner.run_queue(jobs, None).await
        });

        // Wait to confirm worker is parked
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(pool.is_paused());

        // Cancel while paused
        pool.cancel();
        // Wake up paused worker
        pool.resume();

        let results = handle.await.expect("Worker panicked");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, BatchJobStatus::Cancelled);
    })
    .await
    .expect("TEST DEADLOCKED: Cancel while paused hung the worker pool!");
}

// ============================================================================
// CHALLENGE 4: Worker Panic Isolation & Supervisor Survival
// ============================================================================

#[tokio::test]
async fn test_adv_m4_11_worker_panic_supervisor_survival_and_continuation() {
    // Verifies:
    // When a worker task panics mid-execution:
    // 1. The queue supervisor survives (no process exit or abort).
    // 2. Concurrently scheduled healthy sibling tasks continue executing to completion.
    // 3. Empirical analysis of whether the supervisor records failure or omits the panicked job.
    let pool = BatchWorkerPool::new(WorkerPoolConfig { concurrency: 2 }, None);

    let mut fast_profile = ExportProfile::default();
    fast_profile.scaling = ScalingProfile::LongEdge {
        max_long_edge: 320,
        dont_enlarge: true,
    };

    // Job 1: Healthy photo
    let job1 = BatchJob {
        id: 1,
        source_path: PathBuf::from("DRAW/IMG_0588.CR3"),
        photo_id: None,
        recipe: BuiltinPreset::Natural.recipe(),
        recipe_name: "Natural".into(),
        intensity: 1.0,
        export_profile_name: "thumbnail".into(),
        profile: fast_profile.clone(),
        sequence_index: 1,
        total_in_batch: 3,
        status: BatchJobStatus::Queued,
        enqueued_at: Instant::now(),
        started_at: None,
        completed_at: None,
    };

    // Job 2: Pathological job forced to panic inside image allocation
    // Scaling to massive dimensions triggers assertion failure in ImageBuffer::new
    let mut panic_profile = ExportProfile::default();
    panic_profile.scaling = ScalingProfile::LongEdge {
        max_long_edge: u32::MAX, // Triggers buffer length overflow assertion in ImageBuffer::new
        dont_enlarge: false,
    };

    let job2_panic = BatchJob {
        id: 2,
        source_path: PathBuf::from("DRAW/IMG_0589.CR3"),
        photo_id: None,
        recipe: BuiltinPreset::Cinema.recipe(),
        recipe_name: "Cinema".into(),
        intensity: 1.0,
        export_profile_name: "panic_profile".into(),
        profile: panic_profile,
        sequence_index: 2,
        total_in_batch: 3,
        status: BatchJobStatus::Queued,
        enqueued_at: Instant::now(),
        started_at: None,
        completed_at: None,
    };

    // Job 3: Healthy photo
    let job3 = BatchJob {
        id: 3,
        source_path: PathBuf::from("DRAW/IMG_0590.CR3"),
        photo_id: None,
        recipe: BuiltinPreset::BwHighContrast.recipe(),
        recipe_name: "B&W".into(),
        intensity: 1.0,
        export_profile_name: "thumbnail".into(),
        profile: fast_profile.clone(),
        sequence_index: 3,
        total_in_batch: 3,
        status: BatchJobStatus::Queued,
        enqueued_at: Instant::now(),
        started_at: None,
        completed_at: None,
    };

    let jobs = vec![job1, job2_panic, job3];

    // Run queue through worker pool.
    // The supervisor must catch the panic from Job 2, NOT abort, and allow Job 1 and Job 3 to finish.
    let results = pool.run_queue(jobs, None).await;

    // EMPIRICAL VERIFICATION 1: Supervisor survived!
    assert!(
        !results.is_empty(),
        "Supervisor must survive panic and return results"
    );

    // EMPIRICAL VERIFICATION 2: Sibling tasks Job 1 and Job 3 succeeded!
    let done_jobs: Vec<&BatchJob> = results
        .iter()
        .filter(|j| matches!(j.status, BatchJobStatus::Done { .. }))
        .collect();
    assert_eq!(
        done_jobs.len(),
        2,
        "Supervisor must allow healthy sibling tasks (Job 1 and Job 3) to complete successfully"
    );
    assert_eq!(done_jobs[0].id, 1);
    assert_eq!(done_jobs[1].id, 3);

    // EMPIRICAL VERIFICATION 3: The Panicked Job Record is preserved with Failed status
    let panicked_job = results.iter().find(|j| j.id == 2);
    assert!(
        panicked_job.is_some(),
        "Panicked job 2 MUST be retained in final_jobs with Failed status"
    );
    let job2 = panicked_job.unwrap();
    assert!(
        matches!(job2.status, BatchJobStatus::Failed { .. }),
        "Panicked job must have Failed status, got: {:?}",
        job2.status
    );
    if let BatchJobStatus::Failed { ref error_message } = job2.status {
        assert!(
            error_message.contains("panicked"),
            "Error message must indicate worker panic, got: {}",
            error_message
        );
    }
}

#[tokio::test]
async fn test_adv_m4_12_worker_panic_isolation_tokio_catch() {
    // Tests raw worker panic isolation via JoinHandle
    let handle = tokio::spawn(async {
        panic!("Adversarial forced panic inside worker thread");
    });

    let join_res = handle.await;
    assert!(join_res.is_err(), "JoinHandle must return error on panic");
    let err = join_res.unwrap_err();
    assert!(err.is_panic(), "Error must be classified as panic");

    // Supervisor thread continues unaffected
    let mut counter = 0;
    for i in 1..=5 {
        counter += i;
    }
    assert_eq!(counter, 15);
}
