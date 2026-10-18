//! Batch processing job models, photographic metadata, and dual processing strategies (F-BAT-03).

use std::path::PathBuf;
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

use crate::engine::recipe::Recipe;
use crate::queue::QueueError;

/// Image orientation detected from metadata or dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Orientation {
    Landscape,
    Portrait,
    Square,
}

impl Orientation {
    pub fn from_dimensions(width: u32, height: u32) -> Self {
        if width > height {
            Orientation::Landscape
        } else if height > width {
            Orientation::Portrait
        } else {
            Orientation::Square
        }
    }
}

/// Photographic aspect ratio representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AspectRatio {
    Ratio3x2,   // ~1.50 (e.g. 6000x4000 Canon CR3)
    Ratio16x9,  // ~1.778
    Ratio1x1,   // 1.00
    Ratio4x3,   // ~1.333
    Custom(f32),
}

impl AspectRatio {
    pub fn from_dimensions(width: u32, height: u32) -> Self {
        if height == 0 {
            return AspectRatio::Ratio3x2;
        }
        let w = width.max(height) as f32;
        let h = width.min(height) as f32;
        let ratio = w / h;

        if (ratio - 1.50).abs() < 0.05 {
            AspectRatio::Ratio3x2
        } else if (ratio - (16.0 / 9.0)).abs() < 0.05 {
            AspectRatio::Ratio16x9
        } else if (ratio - 1.00).abs() < 0.05 {
            AspectRatio::Ratio1x1
        } else if (ratio - (4.0 / 3.0)).abs() < 0.05 {
            AspectRatio::Ratio4x3
        } else {
            AspectRatio::Custom(ratio)
        }
    }
}

/// Photographic candidate discovered by the directory scanner.
#[derive(Debug, Clone, PartialEq)]
pub struct ScannedPhoto {
    pub path: PathBuf,
    pub filename: String,
    pub extension: String,
    pub size_bytes: u64,
    pub is_cr3: bool,
    pub is_selected: bool,
    pub dimensions: Option<(u32, u32)>,
    pub orientation: Option<Orientation>,
    pub aspect_ratio: Option<AspectRatio>,
}

/// Dual processing strategy configuration (Requirement F-BAT-03).
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "Preserves existing public BatchStrategy API contract and test fixtures without heap boxing"
)]
pub enum BatchStrategy {
    /// Uniform batch: single master recipe applied across all selected photos (M x 1).
    Uniform {
        recipe: Recipe,
        intensity: f32,
    },
    /// Multi-recipe variations: N recipes applied to each selected photo (M x N).
    MultiRecipeVariations {
        recipes: Vec<(Recipe, f32)>,
    },
}

/// Summary pre-check for a batch processing plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchSummary {
    pub photo_count: usize,
    pub recipe_count: usize,
    pub total_jobs: usize,
    pub estimated_total_output_bytes: u64,
    pub strategy_name: String,
}

/// Lifecycle status of an individual batch processing job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BatchJobStatus {
    Queued,
    Developing { progress_pct: u8 },
    Rendering { progress_pct: u8 },
    Written { output_path: PathBuf, bytes: u64 },
    Done { output_path: PathBuf, bytes_written: u64, elapsed: Duration },
    Failed { error_message: String },
    Cancelled,
    Skipped { reason: String },
}

/// Full job definition representing a single photographic processing task.
#[derive(Debug, Clone)]
pub struct BatchJob {
    pub id: u64,
    pub source_path: PathBuf,
    pub photo_id: Option<i64>,
    pub recipe: Recipe,
    pub recipe_name: String,
    pub intensity: f32,
    pub export_profile_name: String,
    pub profile: crate::export::ExportProfile,
    pub sequence_index: usize,
    pub total_in_batch: usize,
    pub status: BatchJobStatus,
    pub enqueued_at: Instant,
    pub started_at: Option<Instant>,
    pub completed_at: Option<Instant>,
}

/// Engine to validate batch pre-conditions and expand strategies into concrete task vectors.
pub struct StrategyEngine;

impl StrategyEngine {
    /// Validates pre-conditions before launching batch queue.
    pub fn validate(
        selected_photos: &[ScannedPhoto],
        strategy: &BatchStrategy,
    ) -> Result<BatchSummary, QueueError> {
        let m = selected_photos.len();
        if m == 0 {
            return Err(QueueError::NoPhotosSelected);
        }

        match strategy {
            BatchStrategy::Uniform { recipe, .. } => {
                let total_jobs = m;
                let estimated_bytes = total_jobs as u64 * 4_200_000; // ~4.2 MB average JPEG
                Ok(BatchSummary {
                    photo_count: m,
                    recipe_count: 1,
                    total_jobs,
                    estimated_total_output_bytes: estimated_bytes,
                    strategy_name: format!("Uniform ({})", recipe.name),
                })
            }
            BatchStrategy::MultiRecipeVariations { recipes } => {
                let n = recipes.len();
                if n == 0 {
                    return Err(QueueError::NoRecipesConfigured);
                }
                let total_jobs = m * n;
                let estimated_bytes = total_jobs as u64 * 4_200_000;
                Ok(BatchSummary {
                    photo_count: m,
                    recipe_count: n,
                    total_jobs,
                    estimated_total_output_bytes: estimated_bytes,
                    strategy_name: format!("Multi-Recipe Variations ({} variants/photo)", n),
                })
            }
        }
    }

    /// Expands selected photos and strategy into a pre-allocated vector of BatchJob tasks.
    pub fn expand_jobs(
        selected_photos: &[ScannedPhoto],
        strategy: &BatchStrategy,
        export_profile_name: &str,
    ) -> Result<Vec<BatchJob>, QueueError> {
        let summary = Self::validate(selected_photos, strategy)?;
        let mut jobs = Vec::with_capacity(summary.total_jobs);
        let mut job_id = 1u64;

        let profile = if export_profile_name.eq_ignore_ascii_case("web_1080p")
            || export_profile_name.eq_ignore_ascii_case("1080p")
        {
            crate::export::ExportProfile::web_1080p()
        } else if export_profile_name.eq_ignore_ascii_case("archival") {
            crate::export::ExportProfile::archival_tiff_16bit()
        } else {
            crate::export::ExportProfile::default()
        };

        for (seq_idx, photo) in selected_photos.iter().enumerate() {
            match strategy {
                BatchStrategy::Uniform { recipe, intensity } => {
                    jobs.push(BatchJob {
                        id: job_id,
                        source_path: photo.path.clone(),
                        photo_id: None,
                        recipe: recipe.clone(),
                        recipe_name: recipe.name.clone(),
                        intensity: *intensity,
                        export_profile_name: export_profile_name.to_string(),
                        profile: profile.clone(),
                        sequence_index: seq_idx + 1,
                        total_in_batch: summary.total_jobs,
                        status: BatchJobStatus::Queued,
                        enqueued_at: Instant::now(),
                        started_at: None,
                        completed_at: None,
                    });
                    job_id += 1;
                }
                BatchStrategy::MultiRecipeVariations { recipes } => {
                    for (recipe, intensity) in recipes {
                        jobs.push(BatchJob {
                            id: job_id,
                            source_path: photo.path.clone(),
                            photo_id: None,
                            recipe: recipe.clone(),
                            recipe_name: recipe.name.clone(),
                            intensity: *intensity,
                            export_profile_name: export_profile_name.to_string(),
                            profile: profile.clone(),
                            sequence_index: seq_idx + 1,
                            total_in_batch: summary.total_jobs,
                            status: BatchJobStatus::Queued,
                            enqueued_at: Instant::now(),
                            started_at: None,
                            completed_at: None,
                        });
                        job_id += 1;
                    }
                }
            }
        }

        Ok(jobs)
    }
}
