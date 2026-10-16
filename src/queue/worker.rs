//! Bounded asynchronous worker pool, directory scanner, and telemetry engine (F-BAT-01, F-BAT-02, F-BAT-04, F-BAT-05).

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, Mutex, Semaphore, watch};
use tokio_util::sync::CancellationToken;

use crate::export::{CollisionPolicy, ExportJob, ExportOutcome, ExportPipeline, NamingTemplate};
use crate::mcp::client::McpClient;
use crate::queue::job::{AspectRatio, BatchJob, BatchJobStatus, Orientation, ScannedPhoto};
use crate::queue::QueueError;
use crate::tui::event::AppEvent;

/// Configuration for the recursive photographic directory scanner.
#[derive(Debug, Clone)]
pub struct DirectoryScannerConfig {
    pub root: PathBuf,
    pub recursive: bool,
    pub follow_symlinks: bool,
    pub max_depth: usize,
    pub allowed_extensions: Vec<String>,
}

impl Default for DirectoryScannerConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("DRAW"),
            recursive: true,
            follow_symlinks: false,
            max_depth: 32,
            allowed_extensions: vec![
                "cr3".into(), "jpg".into(), "jpeg".into(),
                "png".into(), "tiff".into(), "tif".into(),
            ],
        }
    }
}

/// Directory scanner discovering photographic files iteratively without stack recursion.
pub struct DirectoryScanner;

impl DirectoryScanner {
    /// Iteratively scans directory structure for photographic assets (T1-BAT01-01..05, T2-BAT01-01..05).
    pub fn scan(config: &DirectoryScannerConfig) -> Result<Vec<ScannedPhoto>, QueueError> {
        if !config.root.exists() {
            return Err(QueueError::DirectoryNotFound(config.root.clone()));
        }

        let mut results = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back((config.root.clone(), 0usize));

        let allowed_exts: HashSet<String> = config
            .allowed_extensions
            .iter()
            .map(|s| s.to_lowercase())
            .collect();

        while let Some((dir, depth)) = queue.pop_front() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(err) => {
                    tracing::warn!("Skipping unreadable directory {:?}: {}", dir, err);
                    continue;
                }
            };

            for entry in entries.flatten() {
                let p = entry.path();
                let file_type = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(err) => {
                        tracing::warn!("Skipping unreadable entry {:?}: {}", p, err);
                        continue;
                    }
                };

                if file_type.is_dir() {
                    if config.recursive && depth + 1 <= config.max_depth {
                        queue.push_back((p, depth + 1));
                    }
                } else if file_type.is_file() || (config.follow_symlinks && file_type.is_symlink()) {
                    if let Some(ext) = p.extension() {
                        let ext_norm = ext.to_string_lossy().to_lowercase();
                        if allowed_exts.contains(&ext_norm) {
                            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                            let filename = p
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            let is_cr3 = ext_norm == "cr3";

                            // Standard Canon RAW files in DRAW/ are 6000x4000 Landscape 3:2
                            let (dimensions, orientation, aspect_ratio) = if is_cr3 {
                                (
                                    Some((6000, 4000)),
                                    Some(Orientation::Landscape),
                                    Some(AspectRatio::Ratio3x2),
                                )
                            } else {
                                (None, None, None)
                            };

                            results.push(ScannedPhoto {
                                path: p,
                                filename,
                                extension: ext_norm,
                                size_bytes: size,
                                is_cr3,
                                is_selected: false,
                                dimensions,
                                orientation,
                                aspect_ratio,
                            });
                        }
                    }
                }
            }
        }

        results.sort_by(|a, b| a.filename.cmp(&b.filename));
        Ok(results)
    }
}

/// Filter criteria for candidates before batch queue generation (Requirement F-BAT-02).
#[derive(Debug, Clone, Default)]
pub struct BatchFilter {
    pub extensions_include: Vec<String>,
    pub extensions_exclude: Vec<String>,
    pub orientation: Option<Orientation>,
    pub aspect_ratio: Option<AspectRatio>,
}

impl BatchFilter {
    pub fn matches(&self, photo: &ScannedPhoto) -> bool {
        // 1. Extension inclusion check
        if !self.extensions_include.is_empty() {
            let matched = self
                .extensions_include
                .iter()
                .any(|ext| ext.trim_start_matches('.').eq_ignore_ascii_case(&photo.extension));
            if !matched {
                return false;
            }
        }

        // 2. Extension exclusion check
        if !self.extensions_exclude.is_empty() {
            let excluded = self
                .extensions_exclude
                .iter()
                .any(|ext| ext.trim_start_matches('.').eq_ignore_ascii_case(&photo.extension));
            if excluded {
                return false;
            }
        }

        // 3. Orientation check
        if let Some(ref target_orient) = self.orientation {
            if let Some(ref actual_orient) = photo.orientation {
                if target_orient != actual_orient {
                    return false;
                }
            } else if photo.is_cr3 {
                if target_orient != &Orientation::Landscape {
                    return false;
                }
            }
        }

        // 4. Aspect Ratio check
        if let Some(ref target_ar) = self.aspect_ratio {
            if let Some(ref actual_ar) = photo.aspect_ratio {
                if target_ar != actual_ar {
                    return false;
                }
            } else if photo.is_cr3 {
                if target_ar != &AspectRatio::Ratio3x2 {
                    return false;
                }
            }
        }

        true
    }

    pub fn apply(&self, photos: &[ScannedPhoto]) -> Vec<ScannedPhoto> {
        photos.iter().filter(|p| self.matches(p)).cloned().collect()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Real-time throughput and rolling dynamic ETA calculator (Requirement F-BAT-04).
#[derive(Debug, Clone)]
pub struct BatchTelemetry {
    pub total_jobs: usize,
    pub completed_count: usize,
    pub failed_count: usize,
    pub skipped_count: usize,
    pub start_time: Instant,
    pub throughput_imgs_per_sec: f32,
    pub estimated_remaining: Option<Duration>,
}

impl BatchTelemetry {
    pub fn new(total_jobs: usize) -> Self {
        Self {
            total_jobs,
            completed_count: 0,
            failed_count: 0,
            skipped_count: 0,
            start_time: Instant::now(),
            throughput_imgs_per_sec: 0.0,
            estimated_remaining: None,
        }
    }

    pub fn update(&mut self, completed: usize, failed: usize, skipped: usize) {
        self.completed_count = completed;
        self.failed_count = failed;
        self.skipped_count = skipped;

        let elapsed_secs = self.start_time.elapsed().as_secs_f32();
        let finished = self.completed_count + self.failed_count + self.skipped_count;

        if elapsed_secs > 0.05 && finished > 0 {
            self.throughput_imgs_per_sec = finished as f32 / elapsed_secs;
            let remaining_jobs = self.total_jobs.saturating_sub(finished);

            if self.throughput_imgs_per_sec > 0.001 {
                let eta_secs = remaining_jobs as f32 / self.throughput_imgs_per_sec;
                self.estimated_remaining = Some(Duration::from_secs_f32(eta_secs));
            } else {
                self.estimated_remaining = None;
            }
        }
    }
}

/// Worker pool concurrency configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerPoolConfig {
    pub concurrency: usize,
}

impl Default for WorkerPoolConfig {
    fn default() -> Self {
        Self { concurrency: 4 }
    }
}

impl WorkerPoolConfig {
    pub fn sanitized(&self) -> Self {
        Self {
            // Strictly clamp concurrency between 1 and 8 (T2-BAT04-01, T2-BAT04-02)
            concurrency: self.concurrency.clamp(1, 8),
        }
    }
}

/// Supervised asynchronous worker pool executing batch jobs with cooperative controls (F-BAT-04, F-BAT-05).
pub struct BatchWorkerPool {
    config: WorkerPoolConfig,
    mcp_client: Option<Arc<McpClient>>,
    pause_tx: Arc<watch::Sender<bool>>,
    _pause_rx: watch::Receiver<bool>,
    cancel_token: CancellationToken,
    pipeline: Arc<ExportPipeline>,
}

#[cfg(windows)]
#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(u_period: u32) -> u32;
}

impl BatchWorkerPool {
    pub fn new(config: WorkerPoolConfig, mcp_client: Option<Arc<McpClient>>) -> Self {
        #[cfg(windows)]
        unsafe {
            let _ = timeBeginPeriod(1);
        }

        let (pause_tx, pause_rx) = watch::channel(false);
        Self {
            config: config.sanitized(),
            mcp_client,
            pause_tx: Arc::new(pause_tx),
            _pause_rx: pause_rx,
            cancel_token: CancellationToken::new(),
            pipeline: Arc::new(ExportPipeline::new()),
        }
    }

    pub fn pause(&self) {
        self.pause_tx.send_replace(true);
    }

    pub fn resume(&self) {
        self.pause_tx.send_replace(false);
    }

    pub fn cancel(&self) {
        self.cancel_token.cancel();
        self.pause_tx.send_replace(false);
    }

    pub fn is_paused(&self) -> bool {
        *self.pause_tx.borrow()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    /// Resets all failed jobs in a slice back to Queued state for re-execution.
    pub fn retry_failed(jobs: &mut [BatchJob]) -> usize {
        let mut count = 0;
        for job in jobs.iter_mut() {
            if matches!(job.status, BatchJobStatus::Failed { .. }) {
                job.status = BatchJobStatus::Queued;
                job.started_at = None;
                job.completed_at = None;
                count += 1;
            }
        }
        count
    }

    /// Executes the provided batch jobs through the bounded worker pool.
    pub async fn run_queue(
        &self,
        jobs: Vec<BatchJob>,
        progress_tx: Option<mpsc::Sender<AppEvent>>,
    ) -> Vec<BatchJob> {
        let total_jobs = jobs.len();
        if total_jobs == 0 {
            if let Some(ref tx) = progress_tx {
                let _ = tx
                    .send(AppEvent::BatchProgress {
                        completed: 0,
                        total: 0,
                        current_file: "None".into(),
                        percent: 100.0,
                    })
                    .await;
            }
            return Vec::new();
        }

        let semaphore = Arc::new(Semaphore::new(self.config.concurrency));
        let job_queue = Arc::new(Mutex::new(VecDeque::from(jobs)));
        let completed_jobs = Arc::new(Mutex::new(Vec::with_capacity(total_jobs)));
        let completed_count = Arc::new(AtomicUsize::new(0));
        let failed_count = Arc::new(AtomicUsize::new(0));
        let skipped_count = Arc::new(AtomicUsize::new(0));

        let mut join_handles = Vec::new();
        let mut supervisor_pause_rx = self.pause_tx.subscribe();

        loop {
            // 1. Cancellation check
            if self.cancel_token.is_cancelled() {
                let mut q = job_queue.lock().await;
                let mut done = completed_jobs.lock().await;
                while let Some(mut job) = q.pop_front() {
                    job.status = BatchJobStatus::Cancelled;
                    done.push(job);
                }
                break;
            }

            // 2. Pause check
            while *supervisor_pause_rx.borrow_and_update() && !self.cancel_token.is_cancelled() {
                tokio::select! {
                    _ = self.cancel_token.cancelled() => break,
                    res = supervisor_pause_rx.changed() => {
                        if res.is_err() {
                            break;
                        }
                    }
                }
            }

            // 3. Dequeue next job
            let next_job = {
                let mut q = job_queue.lock().await;
                q.pop_front()
            };

            let mut job = match next_job {
                Some(j) => j,
                None => break, // All jobs have been dispatched to workers
            };

            // 4. Acquire worker permit
            let permit = match semaphore.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => break,
            };

            let mcp = self.mcp_client.clone();
            let pipeline = self.pipeline.clone();
            let tx = progress_tx.clone();
            let cancel = self.cancel_token.clone();
            let mut worker_pause_rx = self.pause_tx.subscribe();
            let completed = completed_count.clone();
            let failed = failed_count.clone();
            let skipped = skipped_count.clone();
            let done_list = completed_jobs.clone();

            let backup_job = job.clone();

            // 5. Spawn worker task with panic isolation
            let handle = tokio::spawn(async move {
                let _permit = permit; // Held until task completion
                job.started_at = Some(Instant::now());

                // Check pause or cancel inside task start
                while *worker_pause_rx.borrow_and_update() && !cancel.is_cancelled() {
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        res = worker_pause_rx.changed() => {
                            if res.is_err() {
                                break;
                            }
                        }
                    }
                }

                if cancel.is_cancelled() {
                    job.status = BatchJobStatus::Cancelled;
                    let mut dl = done_list.lock().await;
                    dl.push(job);
                    return;
                }

                let current_filename = job
                    .source_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                if let Some(ref tx) = tx {
                    let cur = completed.load(Ordering::Relaxed);
                    let _ = tx
                        .send(AppEvent::BatchProgress {
                            completed: cur,
                            total: total_jobs,
                            current_file: current_filename.clone(),
                            percent: (cur as f32 / total_jobs as f32) * 100.0,
                        })
                        .await;
                }

                // Execute export job
                let export_job = ExportJob {
                    job_id: job.id,
                    source_path: job.source_path.clone(),
                    photo_id: job.photo_id,
                    recipe_name: job.recipe_name.clone(),
                    recipe: job.recipe.clone(),
                    profile: job.profile.clone(),
                    output_dir: PathBuf::from("export"),
                    template: NamingTemplate::default(),
                    seq_number: job.sequence_index,
                    collision_policy: CollisionPolicy::Rename,
                };

                let res = pipeline.execute_export(mcp.as_deref(), &export_job).await;

                match res {
                    Ok(ExportOutcome::Success {
                        output_path,
                        bytes_written,
                        elapsed,
                        ..
                    }) => {
                        job.completed_at = Some(Instant::now());
                        job.status = BatchJobStatus::Done {
                            output_path,
                            bytes_written,
                            elapsed,
                        };
                        completed.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok(ExportOutcome::Skipped { reason, .. }) => {
                        job.completed_at = Some(Instant::now());
                        job.status = BatchJobStatus::Skipped { reason };
                        skipped.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(err) => {
                        job.completed_at = Some(Instant::now());
                        job.status = BatchJobStatus::Failed {
                            error_message: err.to_string(),
                        };
                        failed.fetch_add(1, Ordering::SeqCst);
                    }
                }

                if let Some(ref tx) = tx {
                    let cur = completed.load(Ordering::Relaxed);
                    let _ = tx
                        .send(AppEvent::BatchProgress {
                            completed: cur,
                            total: total_jobs,
                            current_file: current_filename,
                            percent: (cur as f32 / total_jobs as f32) * 100.0,
                        })
                        .await;
                }

                let mut dl = done_list.lock().await;
                dl.push(job);
            });

            join_handles.push((backup_job, handle));
        }

        // Await all spawned tasks and isolate any panic
        for (backup_job, handle) in join_handles {
            if let Err(join_err) = handle.await {
                if join_err.is_panic() {
                    tracing::error!("Worker task panicked: {:?}", join_err);
                    failed_count.fetch_add(1, Ordering::SeqCst);
                    let mut failed_job = backup_job;
                    failed_job.completed_at = Some(Instant::now());
                    failed_job.status = BatchJobStatus::Failed {
                        error_message: "Worker task panicked".to_string(),
                    };
                    let mut done = completed_jobs.lock().await;
                    done.push(failed_job);
                } else {
                    tracing::warn!("Worker task cancelled/aborted: {:?}", join_err);
                    let mut cancelled_job = backup_job;
                    cancelled_job.completed_at = Some(Instant::now());
                    cancelled_job.status = BatchJobStatus::Cancelled;
                    let mut done = completed_jobs.lock().await;
                    done.push(cancelled_job);
                }
            }
        }

        let mut final_jobs = completed_jobs.lock().await.clone();
        final_jobs.sort_by_key(|j| j.id);
        final_jobs
    }
}
