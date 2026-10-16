//! Batch processing queue facade, job models, and concurrency supervisor (F-BAT-01..05).

pub mod job;
pub mod worker;

pub use job::*;
pub use worker::*;

use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

use crate::mcp::client::McpClient;

/// Domain error definitions for batch queue scheduling and worker execution.
#[derive(Debug, Error)]
pub enum QueueError {
    #[error("Directory not found: {0}")]
    DirectoryNotFound(PathBuf),

    #[error("No photos selected for batch processing")]
    NoPhotosSelected,

    #[error("No recipes configured for variation batch")]
    NoRecipesConfigured,

    #[error("Queue is already running")]
    AlreadyRunning,

    #[error("Queue is not currently active")]
    NotRunning,

    #[error("Queue task execution failed for job {job_id}: {reason}")]
    JobFailed { job_id: u64, reason: String },

    #[error("Worker pool panic caught: {0}")]
    WorkerPanicked(String),

    #[error("MCP transport disconnected during batch execution")]
    McpDisconnected,

    #[error("Batch queue cancelled by user")]
    QueueCancelled,

    #[error("Worker pool execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Queue error: {0}")]
    Other(String),
}

/// High-level JobQueue coordinator conforming to PROJECT.md interface contracts:
/// `JobQueue::enqueue(&mut self, job: BatchJob)`
/// `JobQueue::pause(&mut self)`, `JobQueue::resume(&mut self)`, `JobQueue::cancel(&mut self)`
pub struct JobQueue {
    pub jobs: Vec<BatchJob>,
    pub pool: BatchWorkerPool,
}

impl Default for JobQueue {
    fn default() -> Self {
        Self::new(WorkerPoolConfig::default(), None)
    }
}

impl JobQueue {
    pub fn new(config: WorkerPoolConfig, mcp_client: Option<Arc<McpClient>>) -> Self {
        Self {
            jobs: Vec::new(),
            pool: BatchWorkerPool::new(config, mcp_client),
        }
    }

    pub fn enqueue(&mut self, job: BatchJob) {
        self.jobs.push(job);
    }

    pub fn pause(&mut self) {
        self.pool.pause();
    }

    pub fn resume(&mut self) {
        self.pool.resume();
    }

    pub fn cancel(&mut self) {
        self.pool.cancel();
    }
}
