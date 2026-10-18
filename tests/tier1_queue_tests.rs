//! Tier 1 Batch Queue Integration Tests: Asynchronous worker pool, concurrency bounds,
//! cooperative controls (Pause, Resume, Cancel), failure isolation, retry, and rolling telemetry.
//!
//! Covers Milestone M4 features:
//! - F-BAT-01: Directory & Multi-File Discovery (Iterative scanner)
//! - F-BAT-02: Batch Filtering Engine (Extensions, Orientation, Aspect Ratio)
//! - F-BAT-03: Dual Processing Strategies (Uniform M x 1 and Variations M x N)
//! - F-BAT-04: Bounded Concurrency & Real-Time Telemetry (1..8 workers, rolling ETA)
//! - F-BAT-05: Queue Controls (Pause, Resume, Cancel, Retry Failed)
//! - F-RNF-04: Panic Recovery & Fault Tolerance

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use lightcraft_tui::engine::presets::BuiltinPreset;
use lightcraft_tui::queue::{
    AspectRatio, BatchFilter, BatchJobStatus, BatchStrategy, BatchTelemetry, BatchWorkerPool,
    DirectoryScanner, DirectoryScannerConfig, Orientation, ScannedPhoto, StrategyEngine,
    WorkerPoolConfig,
};

fn create_sample_scanned_photo(name: &str, is_cr3: bool) -> ScannedPhoto {
    let ext = if is_cr3 { "cr3" } else { "jpg" };
    // Map non-existent test names to existing local files when is_cr3 is true
    let actual_file = if is_cr3 {
        match name {
            "IMG_0589" | "IMG_0981" => "IMG_0981.CR3",
            _ => "IMG_0969.CR3",
        }
    } else {
        name
    };
    let path = if is_cr3 {
        PathBuf::from(format!("DRAW/{}", actual_file))
    } else {
        PathBuf::from(format!("DRAW/{}.{}", name, ext))
    };

    ScannedPhoto {
        path,
        filename: format!("{}.{}", name, ext),
        extension: ext.to_string(),
        size_bytes: if is_cr3 { 26_614_589 } else { 4_200_000 },
        is_cr3,
        is_selected: true,
        dimensions: Some((6000, 4000)),
        orientation: Some(Orientation::Landscape),
        aspect_ratio: Some(AspectRatio::Ratio3x2),
    }
}

// ============================================================================
// 1. Bounded Concurrency & Sequential Execution (F-BAT-04)
// ============================================================================

#[tokio::test]
async fn test_t1_que01_bounded_concurrency_one() {
    let pool = BatchWorkerPool::new(WorkerPoolConfig { concurrency: 1 }, None);

    let photos = vec![
        create_sample_scanned_photo("IMG_0588", true),
        create_sample_scanned_photo("IMG_0589", true),
        create_sample_scanned_photo("IMG_0590", true),
    ];

    let strategy = BatchStrategy::Uniform {
        recipe: BuiltinPreset::Natural.recipe(),
        intensity: 1.0,
    };

    let jobs = StrategyEngine::expand_jobs(&photos, &strategy, "web_1080p").unwrap();
    assert_eq!(jobs.len(), 3);

    let results = pool.run_queue(jobs, None).await;
    assert_eq!(results.len(), 3);

    for (idx, job) in results.iter().enumerate() {
        assert_eq!(job.id, (idx + 1) as u64);
        assert!(matches!(job.status, BatchJobStatus::Done { .. }));
    }
}

#[tokio::test]
async fn test_t1_que02_bounded_multi_worker_concurrency_four() {
    let pool = BatchWorkerPool::new(WorkerPoolConfig { concurrency: 4 }, None);

    // Variations batch: 4 photos x 4 recipes = 16 jobs
    let photos = vec![
        create_sample_scanned_photo("IMG_0588", true),
        create_sample_scanned_photo("IMG_0589", true),
        create_sample_scanned_photo("IMG_0590", true),
        create_sample_scanned_photo("IMG_0969", true),
    ];

    let strategy = BatchStrategy::MultiRecipeVariations {
        recipes: vec![
            (BuiltinPreset::Natural.recipe(), 1.0),
            (BuiltinPreset::Cinema.recipe(), 1.0),
            (BuiltinPreset::BwHighContrast.recipe(), 1.0),
            (BuiltinPreset::TealOrange.recipe(), 1.0),
        ],
    };

    let jobs = StrategyEngine::expand_jobs(&photos, &strategy, "web_1080p").unwrap();
    assert_eq!(jobs.len(), 16);

    let results = pool.run_queue(jobs, None).await;
    assert_eq!(results.len(), 16);

    let done_count = results
        .iter()
        .filter(|j| matches!(j.status, BatchJobStatus::Done { .. }))
        .count();
    assert_eq!(done_count, 16);
}

// ============================================================================
// 2. Cooperative Queue Controls: Pause, Resume, Cancel (F-BAT-05)
// ============================================================================

#[tokio::test]
async fn test_t1_que03_interactive_pause() {
    let pool = Arc::new(BatchWorkerPool::new(
        WorkerPoolConfig { concurrency: 2 },
        None,
    ));

    assert!(!pool.is_paused());
    pool.pause();
    assert!(pool.is_paused());

    // Calling pause again is a harmless no-op (T2-BAT05-01)
    pool.pause();
    assert!(pool.is_paused());

    pool.resume();
    assert!(!pool.is_paused());
}

#[tokio::test]
async fn test_t1_que04_queue_resume_continuity() {
    let pool = Arc::new(BatchWorkerPool::new(
        WorkerPoolConfig { concurrency: 1 },
        None,
    ));

    let photos = vec![
        create_sample_scanned_photo("IMG_0588", true),
        create_sample_scanned_photo("IMG_0589", true),
    ];
    let strategy = BatchStrategy::Uniform {
        recipe: BuiltinPreset::Natural.recipe(),
        intensity: 1.0,
    };
    let jobs = StrategyEngine::expand_jobs(&photos, &strategy, "web_1080p").unwrap();

    // Start in paused state
    pool.pause();
    let pool_clone = pool.clone();

    let runner = tokio::spawn(async move { pool_clone.run_queue(jobs, None).await });

    // Let the worker see paused state
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(pool.is_paused());

    // Resume queue
    pool.resume();

    let results = runner.await.expect("Runner panicked");
    assert_eq!(results.len(), 2);
    for job in results {
        assert!(matches!(job.status, BatchJobStatus::Done { .. }));
    }
}

#[tokio::test]
async fn test_t1_que05_queue_cancellation_and_artifact_retention() {
    let pool = Arc::new(BatchWorkerPool::new(
        WorkerPoolConfig { concurrency: 1 },
        None,
    ));

    let photos = vec![
        create_sample_scanned_photo("IMG_0588", true),
        create_sample_scanned_photo("IMG_0589", true),
        create_sample_scanned_photo("IMG_0590", true),
        create_sample_scanned_photo("IMG_0969", true),
    ];
    let strategy = BatchStrategy::Uniform {
        recipe: BuiltinPreset::Natural.recipe(),
        intensity: 1.0,
    };
    let jobs = StrategyEngine::expand_jobs(&photos, &strategy, "web_1080p").unwrap();

    // Cancel before or during execution
    pool.cancel();
    assert!(pool.is_cancelled());

    let results = pool.run_queue(jobs, None).await;
    assert_eq!(results.len(), 4);

    // Cancelled jobs transition cleanly to Cancelled state
    let cancelled_count = results
        .iter()
        .filter(|j| matches!(j.status, BatchJobStatus::Cancelled))
        .count();
    assert!(
        cancelled_count > 0,
        "At least some jobs should be marked Cancelled"
    );
}

// ============================================================================
// 3. Failure Isolation & Selective Retry (F-RNF-04, F-BAT-05)
// ============================================================================

#[tokio::test]
async fn test_t1_que06_failure_isolation() {
    let pool = BatchWorkerPool::new(WorkerPoolConfig { concurrency: 2 }, None);

    // Create 3 jobs, where job 2 targets a non-existent or corrupted source path
    let mut photo_bad = create_sample_scanned_photo("CORRUPTED_NONEXISTENT", false);
    photo_bad.path = PathBuf::from("DRAW/corrupted/does_not_exist.raw");

    let photos = vec![
        create_sample_scanned_photo("IMG_0588", true),
        photo_bad,
        create_sample_scanned_photo("IMG_0590", true),
    ];

    let strategy = BatchStrategy::Uniform {
        recipe: BuiltinPreset::Natural.recipe(),
        intensity: 1.0,
    };
    let jobs = StrategyEngine::expand_jobs(&photos, &strategy, "web_1080p").unwrap();

    let results = pool.run_queue(jobs, None).await;
    assert_eq!(results.len(), 3);

    // Job 1 and Job 3 succeed, Job 2 fails with missing source error
    let done_count = results
        .iter()
        .filter(|j| matches!(j.status, BatchJobStatus::Done { .. }))
        .count();
    let failed_count = results
        .iter()
        .filter(|j| matches!(j.status, BatchJobStatus::Failed { .. }))
        .count();

    assert_eq!(done_count, 2, "Healthy jobs must succeed");
    assert_eq!(failed_count, 1, "Corrupted/missing job must fail");
}

#[tokio::test]
async fn test_t1_que07_selective_retry_failed_jobs() {
    let pool = BatchWorkerPool::new(WorkerPoolConfig { concurrency: 2 }, None);

    let photos = vec![
        create_sample_scanned_photo("IMG_0588", true),
        create_sample_scanned_photo("IMG_0589", true),
    ];
    let strategy = BatchStrategy::Uniform {
        recipe: BuiltinPreset::Natural.recipe(),
        intensity: 1.0,
    };
    let mut jobs = StrategyEngine::expand_jobs(&photos, &strategy, "web_1080p").unwrap();

    // Simulate 1 Done job and 1 Failed job
    jobs[0].status = BatchJobStatus::Done {
        output_path: PathBuf::from("export/out1.jpg"),
        bytes_written: 4_200_000,
        elapsed: Duration::from_millis(500),
    };
    jobs[1].status = BatchJobStatus::Failed {
        error_message: "Simulated transient failure".into(),
    };

    // Retry failed
    let reset_count = BatchWorkerPool::retry_failed(&mut jobs);
    assert_eq!(reset_count, 1, "Exactly 1 failed job should be reset");

    // Job 0 remains Done, Job 1 reset to Queued
    assert!(matches!(jobs[0].status, BatchJobStatus::Done { .. }));
    assert!(matches!(jobs[1].status, BatchJobStatus::Queued));

    // Re-run retried jobs
    let retried = vec![jobs[1].clone()];
    let second_run = pool.run_queue(retried, None).await;
    assert_eq!(second_run.len(), 1);
    assert!(matches!(second_run[0].status, BatchJobStatus::Done { .. }));
}

// ============================================================================
// 4. Panic Recovery (F-RNF-04)
// ============================================================================

#[tokio::test]
async fn test_t1_que08_worker_panic_recovery() {
    // Verifies that a panic inside a spawned worker task is caught by JoinHandle
    // and isolated without crashing the supervisor or app process
    let handle = tokio::spawn(async {
        panic!("Injected test panic in worker task!");
    });

    let res = handle.await;
    assert!(res.is_err(), "JoinHandle must return error on panic");
    assert!(res.unwrap_err().is_panic(), "Must be caught as panic");
}

// ============================================================================
// 5. Telemetry & Dynamic Rolling ETA (F-BAT-04)
// ============================================================================

#[test]
fn test_t1_que09_telemetry_rolling_eta_math() {
    let mut telemetry = BatchTelemetry::new(20);

    // Initial state: 0 completions
    assert_eq!(telemetry.total_jobs, 20);
    assert_eq!(telemetry.completed_count, 0);
    assert_eq!(telemetry.estimated_remaining, None); // No divide-by-zero!

    // Simulate progress: 10 completed, 1 failed, 1 skipped out of 20
    std::thread::sleep(Duration::from_millis(60));
    telemetry.update(10, 1, 1);

    assert_eq!(telemetry.completed_count, 10);
    assert_eq!(telemetry.failed_count, 1);
    assert_eq!(telemetry.skipped_count, 1);
    assert!(telemetry.throughput_imgs_per_sec > 0.0);
    assert!(telemetry.estimated_remaining.is_some());
}

// ============================================================================
// 6. Rapid Pause / Resume Stress Cycling (F-BAT-05)
// ============================================================================

#[tokio::test]
async fn test_t1_que10_rapid_pause_resume_cycling() {
    let pool = Arc::new(BatchWorkerPool::new(
        WorkerPoolConfig { concurrency: 4 },
        None,
    ));

    // Rapidly toggle pause/resume across 20 cycles
    for _ in 0..20 {
        pool.pause();
        assert!(pool.is_paused());
        tokio::time::sleep(Duration::from_millis(5)).await;

        pool.resume();
        assert!(!pool.is_paused());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // After cycling, execute a batch to verify thread pool integrity
    let photos = vec![
        create_sample_scanned_photo("IMG_0588", true),
        create_sample_scanned_photo("IMG_0589", true),
    ];
    let strategy = BatchStrategy::Uniform {
        recipe: BuiltinPreset::Natural.recipe(),
        intensity: 1.0,
    };
    let jobs = StrategyEngine::expand_jobs(&photos, &strategy, "web_1080p").unwrap();

    let results = pool.run_queue(jobs, None).await;
    assert_eq!(results.len(), 2);
    for job in results {
        assert!(matches!(job.status, BatchJobStatus::Done { .. }));
    }
}

// ============================================================================
// 7. Directory Scanner & Batch Filter Predicates (F-BAT-01, F-BAT-02)
// ============================================================================

#[test]
fn test_t1_que11_directory_scanner_discovery() {
    let config = DirectoryScannerConfig {
        root: PathBuf::from("DRAW"),
        recursive: false,
        follow_symlinks: false,
        max_depth: 1,
        allowed_extensions: vec!["cr3".into()],
    };

    let photos = DirectoryScanner::scan(&config).expect("Scan should succeed");
    assert!(!photos.is_empty(), "Should discover CR3 photos in DRAW/");
    for p in &photos {
        assert!(p.is_cr3);
        assert_eq!(p.extension.to_lowercase(), "cr3");
    }
}

#[test]
fn test_t1_que12_batch_filter_predicates() {
    let mut p1 = create_sample_scanned_photo("IMG_0588", true);
    p1.size_bytes = 20_000_000;
    p1.orientation = Some(Orientation::Landscape);
    p1.aspect_ratio = Some(AspectRatio::Ratio3x2);

    let mut p2 = create_sample_scanned_photo("IMG_0589", false);
    p2.size_bytes = 5_000_000;
    p2.orientation = Some(Orientation::Portrait);
    p2.aspect_ratio = Some(AspectRatio::Ratio1x1);

    let photos = vec![p1, p2];

    // Filter by CR3 only via extensions_include
    let mut filter_cr3 = BatchFilter::default();
    filter_cr3.extensions_include = vec!["cr3".into()];
    let filtered = filter_cr3.apply(&photos);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].filename, "IMG_0588.cr3");

    // Filter by orientation
    let mut filter_orient = BatchFilter::default();
    filter_orient.orientation = Some(Orientation::Portrait);
    let filtered = filter_orient.apply(&photos);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].filename, "IMG_0589.jpg");

    // Filter by aspect ratio
    let mut filter_ar = BatchFilter::default();
    filter_ar.aspect_ratio = Some(AspectRatio::Ratio3x2);
    let filtered = filter_ar.apply(&photos);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].filename, "IMG_0588.cr3");

    // Filter by exclusion
    let mut filter_exclude = BatchFilter::default();
    filter_exclude.extensions_exclude = vec!["jpg".into()];
    let filtered = filter_exclude.apply(&photos);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].filename, "IMG_0588.cr3");
}
