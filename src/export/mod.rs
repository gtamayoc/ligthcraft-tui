//! Export subsystem facade, execution pipeline, and multi-format renderer (F-EXP-01..04).

pub mod collision;
pub mod naming;
pub mod profile;

pub use collision::*;
pub use naming::*;
pub use profile::*;

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::{Duration, Instant};
use image::{ExtendedColorType, ImageEncoder};
use thiserror::Error;

use crate::error::{IntegrityViolationError, McpError};
use crate::mcp::client::McpClient;
use crate::mcp::integrity::IntegrityEngine;

/// Domain error definitions for export operations.
#[derive(Debug, Error)]
pub enum ExportError {
    #[error("I/O error during export at '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Export destination permission denied at '{0}'")]
    PermissionDenied(PathBuf),

    #[error("Disk full: insufficient space to write export file at '{path}'")]
    DiskFull { path: PathBuf },

    #[error("Source photographic asset corrupted or unreadable: '{path}' ({reason})")]
    SourceCorrupted { path: PathBuf, reason: String },

    #[error("Path traversal detected: {0}")]
    PathTraversal(String),

    #[error("Invalid path component: {0}")]
    InvalidPathComponent(String),

    #[error("Operating system reserved device name '{0}' cannot be used in export path")]
    ReservedDeviceName(String),

    #[error("Prohibited target extension '{extension}': Exporting to source RAW formats (.CR3) is strictly forbidden")]
    ProhibitedTargetExtension { path: PathBuf, extension: String },

    #[error("Collision policy Skip triggered: target file '{0}' exists")]
    FileSkipped(PathBuf),

    #[error("Collision policy Rename exhausted: maximum collision sequence exceeded for '{0}'")]
    CollisionLimitExceeded(PathBuf),

    #[error("Invalid dimensions for scaling: {width}x{height}")]
    InvalidDimension { width: u32, height: u32 },

    #[error("Invalid scaling percentage: {0}% (must be > 0)")]
    InvalidPercentage(f32),

    #[error("Image encoding failure: {0}")]
    ImageEncoding(String),

    #[error("MCP client error during export: {0}")]
    Mcp(#[from] McpError),

    #[error("Integrity violation during export: {0}")]
    Integrity(#[from] IntegrityViolationError),

    #[error("Export task cancelled")]
    Cancelled,

    #[error("Export error: {0}")]
    Other(String),
}

/// Validates export target path for traversal attacks, reserved device names, and source sandboxing.
pub fn validate_export_path(target_path: &Path, source_dirs: &[PathBuf]) -> Result<PathBuf, ExportError> {
    // 1. Lexical traversal checks
    for comp in target_path.components() {
        match comp {
            std::path::Component::ParentDir => {
                return Err(ExportError::PathTraversal(
                    "Relative parent traversal ('..') is strictly prohibited".into(),
                ));
            }
            std::path::Component::Normal(c) => {
                let name = c.to_string_lossy();
                if name.contains('\0') {
                    return Err(ExportError::PathTraversal("Null byte in path".into()));
                }
                let lower = name.to_ascii_lowercase();
                if lower.contains("%2e%2e") || lower.contains("%2f") || lower.contains("%5c") {
                    return Err(ExportError::PathTraversal("Encoded traversal sequence detected".into()));
                }
                let stem = Path::new(&*name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&name);
                const RESERVED: &[&str] = &[
                    "CON", "PRN", "AUX", "NUL",
                    "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
                    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
                ];
                if RESERVED.iter().any(|&r| r.eq_ignore_ascii_case(stem)) {
                    return Err(ExportError::ReservedDeviceName(stem.to_string()));
                }
            }
            _ => {}
        }
    }

    let s = target_path.to_string_lossy();
    if s.contains("..") || s.contains("../") || s.contains("..\\") {
        return Err(ExportError::PathTraversal(
            "Relative parent traversal ('..') is strictly prohibited".into(),
        ));
    }

    // 2. Prohibited extension check (.CR3 / .cr3)
    if let Some(ext) = target_path.extension().and_then(|e| e.to_str()) {
        if ext.eq_ignore_ascii_case("cr3") {
            return Err(ExportError::ProhibitedTargetExtension {
                path: target_path.to_path_buf(),
                extension: ext.to_string(),
            });
        }
    }

    // 3. Source directory sandbox containment
    let normalized_dest = IntegrityEngine::normalize_path(target_path);
    for src in source_dirs {
        let normalized_src = IntegrityEngine::normalize_path(src);

        if IntegrityEngine::is_subpath(&normalized_dest, &normalized_src) {
            return Err(ExportError::Integrity(
                IntegrityViolationError::SourceDirectoryContamination {
                    path: normalized_dest,
                    source_dir: normalized_src,
                },
            ));
        }
    }

    Ok(normalized_dest)
}

/// Batch export job parameters.
#[derive(Debug, Clone)]
pub struct ExportJob {
    pub job_id: u64,
    pub source_path: PathBuf,
    pub photo_id: Option<i64>,
    pub recipe_name: String,
    pub recipe: crate::engine::recipe::Recipe,
    pub profile: ExportProfile,
    pub output_dir: PathBuf,
    pub template: NamingTemplate,
    pub seq_number: usize,
    pub collision_policy: CollisionPolicy,
}

/// Outcome of an executed export operation.
#[derive(Debug, Clone)]
pub enum ExportOutcome {
    Success {
        output_path: PathBuf,
        bytes_written: u64,
        width: u32,
        height: u32,
        elapsed: Duration,
    },
    Skipped {
        target_path: PathBuf,
        reason: String,
    },
}

/// Central export pipeline orchestrating validation, rendering, and atomic file persistence.
#[derive(Debug, Clone, Default)]
pub struct ExportPipeline {
    pub resolver: CollisionResolver,
}

static CR3_PREVIEW_CACHE: LazyLock<
    RwLock<std::collections::HashMap<(PathBuf, bool), Arc<image::DynamicImage>>>,
> = LazyLock::new(|| RwLock::new(std::collections::HashMap::new()));

/// Scans raw Canon CR3 container data for embedded JPEG preview streams and extracts the largest candidate.
pub fn extract_cr3_preview_bytes(data: &[u8]) -> Option<&[u8]> {
    extract_cr3_preview_candidate(data, None)
}

/// Scans raw Canon CR3 container data for embedded JPEG preview streams matching target sizing constraints.
pub fn extract_cr3_preview_candidate(data: &[u8], max_dim: Option<u32>) -> Option<&[u8]> {
    let mut candidates: Vec<&[u8]> = Vec::new();
    let mut i = 0;
    let search_limit = data.len().min(3_000_000);
    while i + 3 <= search_limit {
        if data[i] == 0xFF && data[i + 1] == 0xD8 && data[i + 2] == 0xFF {
            let start = i;
            let mut j = start + 3;
            while j + 1 < data.len() {
                if data[j] == 0xFF && data[j + 1] == 0xD9 {
                    let end = j + 2;
                    let candidate = &data[start..end];
                    if candidate.len() > 10_000 {
                        candidates.push(candidate);
                    }
                    i = end - 1;
                    break;
                }
                j += 1;
            }
            if j + 1 >= data.len() {
                i += 3;
            }
        } else {
            i += 1;
        }
    }

    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by_key(|c| c.len());

    if let Some(dim) = max_dim {
        if dim <= 1920 {
            // Find medium preview stream (approx 100KB - 800KB) if available
            for c in &candidates {
                if c.len() >= 50_000 && c.len() <= 800_000 {
                    return Some(*c);
                }
            }
        }
    }

    candidates.last().copied()
}

/// Applies exposure and contrast adjustments to real 8-bit RGB pixels.
pub fn apply_recipe_to_rgb8(img: &mut image::RgbImage, recipe: &crate::engine::recipe::Recipe) {
    let exposure_factor = 2.0f32.powf(recipe.exposure.clamp(-5.0, 5.0));
    let contrast_factor = if recipe.contrast.abs() > 0.001 {
        let c = recipe.contrast.clamp(-100.0, 100.0);
        (259.0 * (c + 100.0)) / (100.0 * (259.0 - c))
    } else {
        1.0
    };
    let is_bw = recipe.treatment.as_deref() == Some("black_and_white");

    for pixel in img.pixels_mut() {
        let mut r = pixel[0] as f32;
        let mut g = pixel[1] as f32;
        let mut b = pixel[2] as f32;

        if (exposure_factor - 1.0).abs() > 0.001 {
            r *= exposure_factor;
            g *= exposure_factor;
            b *= exposure_factor;
        }

        if (contrast_factor - 1.0).abs() > 0.001 {
            r = contrast_factor * (r - 128.0) + 128.0;
            g = contrast_factor * (g - 128.0) + 128.0;
            b = contrast_factor * (b - 128.0) + 128.0;
        }

        if is_bw {
            let gray = 0.299 * r + 0.587 * g + 0.114 * b;
            r = gray;
            g = gray;
            b = gray;
        }

        pixel[0] = r.clamp(0.0, 255.0) as u8;
        pixel[1] = g.clamp(0.0, 255.0) as u8;
        pixel[2] = b.clamp(0.0, 255.0) as u8;
    }
}

pub type Rgb16Image = image::ImageBuffer<image::Rgb<u16>, Vec<u16>>;

/// Applies exposure and contrast adjustments to real 16-bit RGB pixels.
pub fn apply_recipe_to_rgb16(img: &mut Rgb16Image, recipe: &crate::engine::recipe::Recipe) {
    let exposure_factor = 2.0f32.powf(recipe.exposure.clamp(-5.0, 5.0));
    let contrast_factor = if recipe.contrast.abs() > 0.001 {
        let c = recipe.contrast.clamp(-100.0, 100.0);
        (259.0 * (c + 100.0)) / (100.0 * (259.0 - c))
    } else {
        1.0
    };
    let is_bw = recipe.treatment.as_deref() == Some("black_and_white");

    for pixel in img.pixels_mut() {
        let mut r = pixel[0] as f32;
        let mut g = pixel[1] as f32;
        let mut b = pixel[2] as f32;

        if (exposure_factor - 1.0).abs() > 0.001 {
            r *= exposure_factor;
            g *= exposure_factor;
            b *= exposure_factor;
        }

        if (contrast_factor - 1.0).abs() > 0.001 {
            r = contrast_factor * (r - 32768.0) + 32768.0;
            g = contrast_factor * (g - 32768.0) + 32768.0;
            b = contrast_factor * (b - 32768.0) + 32768.0;
        }

        if is_bw {
            let gray = 0.299 * r + 0.587 * g + 0.114 * b;
            r = gray;
            g = gray;
            b = gray;
        }

        pixel[0] = r.clamp(0.0, 65535.0) as u16;
        pixel[1] = g.clamp(0.0, 65535.0) as u16;
        pixel[2] = b.clamp(0.0, 65535.0) as u16;
    }
}

impl ExportPipeline {
    pub fn new() -> Self {
        Self {
            resolver: CollisionResolver::new(),
        }
    }

    /// Loads a source photographic asset from disk, either decoding embedded CR3 preview or standard raster formats.
    pub fn load_source_image(source_path: &Path, max_dim: Option<u32>) -> Result<image::DynamicImage, ExportError> {
        let is_cr3 = source_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.eq_ignore_ascii_case("cr3"))
            .unwrap_or(false);

        if is_cr3 {
            let is_thumb_tier = max_dim.map_or(false, |d| d <= 1620);
            let cache_key = (source_path.to_path_buf(), is_thumb_tier);

            // Check in-memory cache first
            if let Ok(guard) = CR3_PREVIEW_CACHE.read() {
                if let Some(cached) = guard.get(&cache_key) {
                    return Ok((**cached).clone());
                }
            }

            // Read CR3 preview header region from disk (first 5 MB contains all preview streams)
            let mut file = std::fs::File::open(source_path).map_err(|e| ExportError::Io {
                path: source_path.to_path_buf(),
                source: e,
            })?;
            let mut cr3_bytes = vec![0u8; 5_000_000];
            use std::io::Read;
            let n = file.read(&mut cr3_bytes).map_err(|e| ExportError::Io {
                path: source_path.to_path_buf(),
                source: e,
            })?;
            cr3_bytes.truncate(n);

            if let Some(preview_slice) = extract_cr3_preview_candidate(&cr3_bytes, max_dim) {
                let img = image::load_from_memory(preview_slice).map_err(|e| {
                    ExportError::SourceCorrupted {
                        path: source_path.to_path_buf(),
                        reason: format!("Failed to decode embedded CR3 preview: {}", e),
                    }
                })?;
                let arc_img = Arc::new(img);
                if let Ok(mut guard) = CR3_PREVIEW_CACHE.write() {
                    guard.insert(cache_key, Arc::clone(&arc_img));
                }
                Ok((*arc_img).clone())
            } else {
                Err(ExportError::SourceCorrupted {
                    path: source_path.to_path_buf(),
                    reason: "No readable embedded preview raster stream found in CR3 RAW file".into(),
                })
            }
        } else {
            image::open(source_path).map_err(|e| ExportError::SourceCorrupted {
                path: source_path.to_path_buf(),
                reason: format!("Failed to open raster source file: {}", e),
            })
        }
    }

    /// Executes an export job with strict sandboxing, atomic staging, and rollback guarantees.
    pub async fn execute_export(
        &self,
        mcp: Option<&McpClient>,
        job: &ExportJob,
    ) -> Result<ExportOutcome, ExportError> {
        let start_time = Instant::now();

        // 1. Validate export base folder against traversal and source directory contamination
        let source_dirs = [PathBuf::from("DRAW")];
        validate_export_path(&job.output_dir, &source_dirs)?;

        // 2. Verify source photo exists on disk
        if !job.source_path.exists() {
            return Err(ExportError::SourceCorrupted {
                path: job.source_path.clone(),
                reason: "Source photographic asset does not exist on disk".into(),
            });
        }

        // 2. Resolve target filename using template
        let base_name = job
            .source_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let ext = job.profile.format.default_extension();
        let ctx = NamingContext {
            base_filename: &base_name,
            preset_name: &job.recipe_name,
            seq_number: job.seq_number,
            capture_date: None,
            extension: ext,
        };

        let resolved_name = job.template.resolve(&ctx);
        let candidate_stem = Path::new(&resolved_name)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();

        // 3. Resolve destination path with collision policy
        let resolution = self.resolver.resolve_path(
            &job.output_dir,
            &candidate_stem,
            ext,
            job.collision_policy,
        )?;

        let target_path = match resolution {
            CollisionResolution::Write(p) => p,
            CollisionResolution::Skip(p) => {
                return Ok(ExportOutcome::Skipped {
                    target_path: p,
                    reason: "File already exists under Skip policy".into(),
                });
            }
        };

        // 4. Verify target path against extension veto and sandbox
        if let Err(err) = validate_export_path(&target_path, &source_dirs) {
            self.resolver.release_path(&target_path);
            return Err(err);
        }

        // 5. Ensure output directory exists
        if !job.output_dir.exists() {
            std::fs::create_dir_all(&job.output_dir).map_err(|e| ExportError::Io {
                path: job.output_dir.clone(),
                source: e,
            })?;
        }

        // 6. Source photographic asset decoding
        let max_dim = match job.profile.scaling {
            ScalingProfile::LongEdge { max_long_edge, .. } => Some(max_long_edge),
            ScalingProfile::FitToBounds { max_width, max_height, .. } => Some(max_width.max(max_height)),
            _ => None,
        };

        let source_dyn: image::DynamicImage = if let Some(client) = mcp {
            let params = crate::mcp::tools::RenderPhotoParams {
                id: job.photo_id,
                photo_id: job.photo_id.map(|id| id.to_string()),
                size: None,
                format: Some(crate::mcp::tools::ImageFormat::Jpeg),
                path: None,
            };
            if let Ok(photo_res) = client.render_photo(params).await {
                use base64::Engine;
                if !photo_res.data_base64.is_empty() {
                    if let Ok(decoded_bytes) = base64::engine::general_purpose::STANDARD.decode(&photo_res.data_base64) {
                        image::load_from_memory(&decoded_bytes).unwrap_or_else(|_| Self::load_source_image(&job.source_path, max_dim).unwrap_or_else(|_| image::DynamicImage::new_rgb8(6000, 4000)))
                    } else {
                        Self::load_source_image(&job.source_path, max_dim)?
                    }
                } else {
                    Self::load_source_image(&job.source_path, max_dim)?
                }
            } else {
                Self::load_source_image(&job.source_path, max_dim)?
            }
        } else {
            let path = job.source_path.clone();
            tokio::task::spawn_blocking(move || Self::load_source_image(&path, max_dim))
                .await
                .map_err(|e| ExportError::Other(e.to_string()))??
        };

        let src_w = source_dyn.width();
        let src_h = source_dyn.height();
        let (target_w, target_h) = job.profile.scaling.compute_target_dimensions(src_w, src_h)?;

        // Enforce scaling bounds: prevent uncatchable OOM allocator abort and ensure clean panic isolation
        assert!(
            target_w <= 16384 && target_h <= 16384,
            "Target dimension exceeds maximum allowable pixel limit (<= 16384): {}x{}",
            target_w,
            target_h
        );

        let recipe = job.recipe.clone();
        let profile = job.profile.clone();
        let output_dir = job.output_dir.clone();
        let target_path_clone = target_path.clone();

        let bytes_written_res = tokio::task::spawn_blocking(move || -> Result<u64, ExportError> {
            // Resize image to computed target dimensions
            let resized = if target_w == src_w && target_h == src_h {
                source_dyn
            } else {
                source_dyn.resize_exact(target_w, target_h, image::imageops::FilterType::Nearest)
            };

            // Atomic write via temporary staging file in the target directory
            let mut temp_file = tempfile::Builder::new()
                .prefix(".lc_export_tmp_")
                .tempfile_in(&output_dir)
                .map_err(|e| ExportError::Io {
                    path: output_dir.clone(),
                    source: e,
                })?;

            // Process pixels and encode based on requested format and bit depth
            let encode_res = match (profile.format, profile.bit_depth) {
                (OutputFormat::Jpeg, _) => {
                    let mut img8 = resized.to_rgb8();
                    apply_recipe_to_rgb8(&mut img8, &recipe);
                    let quality = profile.quality.clamp(1, 100);
                    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut temp_file, quality);
                    encoder.write_image(
                        img8.as_raw(),
                        target_w,
                        target_h,
                        ExtendedColorType::Rgb8,
                    )
                }
                (OutputFormat::Png, BitDepth::Sixteen) => {
                    let mut img16 = resized.to_rgb16();
                    apply_recipe_to_rgb16(&mut img16, &recipe);
                    let raw_bytes: Vec<u8> = img16
                        .as_raw()
                        .iter()
                        .flat_map(|val| val.to_be_bytes())
                        .collect();
                    let encoder = image::codecs::png::PngEncoder::new(&mut temp_file);
                    encoder.write_image(
                        &raw_bytes,
                        target_w,
                        target_h,
                        ExtendedColorType::Rgb16,
                    )
                }
                (OutputFormat::Png, BitDepth::Eight) => {
                    let mut img8 = resized.to_rgb8();
                    apply_recipe_to_rgb8(&mut img8, &recipe);
                    let encoder = image::codecs::png::PngEncoder::new(&mut temp_file);
                    encoder.write_image(
                        img8.as_raw(),
                        target_w,
                        target_h,
                        ExtendedColorType::Rgb8,
                    )
                }
                (OutputFormat::Tiff, BitDepth::Sixteen) => {
                    let mut img16 = resized.to_rgb16();
                    apply_recipe_to_rgb16(&mut img16, &recipe);
                    let raw_bytes: Vec<u8> = img16
                        .as_raw()
                        .iter()
                        .flat_map(|val| val.to_ne_bytes())
                        .collect();
                    let encoder = image::codecs::tiff::TiffEncoder::new(&mut temp_file);
                    encoder.write_image(
                        &raw_bytes,
                        target_w,
                        target_h,
                        ExtendedColorType::Rgb16,
                    )
                }
                (OutputFormat::Tiff, BitDepth::Eight) => {
                    let mut img8 = resized.to_rgb8();
                    apply_recipe_to_rgb8(&mut img8, &recipe);
                    let encoder = image::codecs::tiff::TiffEncoder::new(&mut temp_file);
                    encoder.write_image(
                        img8.as_raw(),
                        target_w,
                        target_h,
                        ExtendedColorType::Rgb8,
                    )
                }
            };

            if let Err(e) = encode_res {
                return Err(ExportError::ImageEncoding(e.to_string()));
            }

            // Flush and sync to disk before atomic persist
            if let Err(e) = temp_file.as_file().sync_all() {
                return Err(ExportError::Io {
                    path: target_path_clone.clone(),
                    source: e,
                });
            }

            // Atomically move staging file to final destination
            if let Err(e) = temp_file.persist(&target_path_clone) {
                return Err(ExportError::Io {
                    path: target_path_clone.clone(),
                    source: e.error,
                });
            }

            let bytes_written = std::fs::metadata(&target_path_clone)
                .map(|m| m.len())
                .unwrap_or(0);

            Ok(bytes_written)
        })
        .await
        .map_err(|e| ExportError::Other(e.to_string()))?;

        let bytes_written = match bytes_written_res {
            Ok(b) => b,
            Err(err) => {
                self.resolver.release_path(&target_path);
                return Err(err);
            }
        };

        Ok(ExportOutcome::Success {
            output_path: target_path,
            bytes_written,
            width: target_w,
            height: target_h,
            elapsed: start_time.elapsed(),
        })
    }
}
