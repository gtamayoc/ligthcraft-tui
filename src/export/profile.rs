//! Export profiles, format definitions, and dimension scaling engine (F-EXP-01, F-EXP-02).

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

use crate::export::collision::{CollisionPolicy, CollisionResolver};
use crate::export::naming::{NamingContext, NamingTemplate};
use crate::export::ExportError;

/// Output image container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Jpeg,
    Png,
    Tiff,
}

impl OutputFormat {
    pub fn default_extension(&self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Tiff => "tif",
        }
    }

    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Tiff => "image/tiff",
        }
    }
}

/// Bit depth for encoded image containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BitDepth {
    Eight = 8,
    Sixteen = 16,
}

/// Color space for ICC profile embedding and pixel encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorSpace {
    Srgb,
    AdobeRgb,
    DisplayP3,
    ProPhoto,
}

/// Compression algorithm for TIFF files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TiffCompression {
    None,
    Lzw,
    Zip,
}

/// Dimension and resolution scaling profile (Requirement F-EXP-02).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "params")]
pub enum ScalingProfile {
    /// Native full resolution: preserves original pixel dimensions.
    Native,

    /// Fit within max_width x max_height bounding box preserving exact aspect ratio.
    FitToBounds {
        max_width: u32,
        max_height: u32,
        dont_enlarge: bool,
    },

    /// Constrain longest edge to max_long_edge pixels preserving aspect ratio.
    LongEdge {
        max_long_edge: u32,
        dont_enlarge: bool,
    },

    /// Percentage scaling (e.g. 50.0 for 50%, 100.0 for 100%).
    Percentage {
        percent: f32,
    },
}

impl ScalingProfile {
    /// Computes the destination (width, height) given source dimensions.
    /// Preserves aspect ratio and prevents zero-dimension results.
    pub fn compute_target_dimensions(&self, src_w: u32, src_h: u32) -> Result<(u32, u32), ExportError> {
        if src_w == 0 || src_h == 0 {
            return Err(ExportError::InvalidDimension {
                width: src_w,
                height: src_h,
            });
        }

        match *self {
            ScalingProfile::Native => Ok((src_w, src_h)),
            ScalingProfile::FitToBounds {
                max_width,
                max_height,
                dont_enlarge,
            } => {
                if max_width == 0 || max_height == 0 {
                    return Err(ExportError::InvalidDimension {
                        width: max_width,
                        height: max_height,
                    });
                }
                let scale_w = max_width as f64 / src_w as f64;
                let scale_h = max_height as f64 / src_h as f64;
                let mut scale = scale_w.min(scale_h);
                if dont_enlarge && scale > 1.0 {
                    scale = 1.0;
                }
                let target_w = (src_w as f64 * scale).round().max(1.0) as u32;
                let target_h = (src_h as f64 * scale).round().max(1.0) as u32;
                Ok((target_w.min(16384), target_h.min(16384)))
            }
            ScalingProfile::LongEdge {
                max_long_edge,
                dont_enlarge,
            } => {
                if max_long_edge == 0 {
                    return Err(ExportError::InvalidDimension {
                        width: 0,
                        height: 0,
                    });
                }
                let src_long = src_w.max(src_h);
                if dont_enlarge && src_long <= max_long_edge {
                    return Ok((src_w, src_h));
                }
                let mut scale = max_long_edge as f64 / src_long as f64;
                if dont_enlarge && scale > 1.0 {
                    scale = 1.0;
                }
                let target_w = (src_w as f64 * scale).round().max(1.0) as u32;
                let target_h = (src_h as f64 * scale).round().max(1.0) as u32;
                Ok((target_w, target_h))
            }
            ScalingProfile::Percentage { percent } => {
                if !percent.is_finite() || percent <= 0.0 || percent > 500.0 {
                    return Err(ExportError::InvalidPercentage(percent));
                }
                let scale = (percent as f64) / 100.0;
                let target_w = (src_w as f64 * scale).round().max(1.0) as u32;
                let target_h = (src_h as f64 * scale).round().max(1.0) as u32;
                Ok((target_w.min(16384), target_h.min(16384)))
            }
        }
    }
}

/// Comprehensive photographic export configuration profile (Requirement F-EXP-01).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportProfile {
    pub name: String,
    pub format: OutputFormat,
    pub quality: u8,               // 1..=100 (for JPEG)
    pub bit_depth: BitDepth,       // 8 or 16 (for PNG / TIFF)
    pub color_space: ColorSpace,
    pub scaling: ScalingProfile,
    pub tiff_compression: TiffCompression,
    pub ppi: u32,
    pub embed_metadata: bool,
}

impl Default for ExportProfile {
    fn default() -> Self {
        Self {
            name: "High Quality JPEG (Full Size)".to_string(),
            format: OutputFormat::Jpeg,
            quality: 90,
            bit_depth: BitDepth::Eight,
            color_space: ColorSpace::Srgb,
            scaling: ScalingProfile::Native,
            tiff_compression: TiffCompression::Lzw,
            ppi: 240,
            embed_metadata: true,
        }
    }
}

impl ExportProfile {
    pub fn web_1080p() -> Self {
        Self {
            name: "Web 1080p JPEG".to_string(),
            format: OutputFormat::Jpeg,
            quality: 85,
            bit_depth: BitDepth::Eight,
            color_space: ColorSpace::Srgb,
            scaling: ScalingProfile::FitToBounds {
                max_width: 1920,
                max_height: 1080,
                dont_enlarge: true,
            },
            tiff_compression: TiffCompression::None,
            ppi: 72,
            embed_metadata: true,
        }
    }

    pub fn archival_tiff_16bit() -> Self {
        Self {
            name: "Archival Master 16-bit TIFF".to_string(),
            format: OutputFormat::Tiff,
            quality: 100,
            bit_depth: BitDepth::Sixteen,
            color_space: ColorSpace::AdobeRgb,
            scaling: ScalingProfile::Native,
            tiff_compression: TiffCompression::Lzw,
            ppi: 300,
            embed_metadata: true,
        }
    }

    /// Interface contract defined in PROJECT.md:
    /// `ExportProfile::resolve_output_path(&self, source_path: &Path, preset_name: &str, seq: usize, collision: CollisionPolicy) -> Result<PathBuf, ExportError>`
    pub fn resolve_output_path(
        &self,
        source_path: &Path,
        preset_name: &str,
        seq: usize,
        collision: CollisionPolicy,
    ) -> Result<PathBuf, ExportError> {
        let base_name = source_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let ext = self.format.default_extension();
        let template = NamingTemplate::default();
        let ctx = NamingContext {
            base_filename: &base_name,
            preset_name,
            seq_number: seq,
            capture_date: None,
            extension: ext,
        };

        let file_name = template.resolve(&ctx);
        let output_dir = PathBuf::from("export");
        let resolver = CollisionResolver::default();
        let file_stem = Path::new(&file_name)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();

        match resolver.resolve_path(&output_dir, &file_stem, ext, collision)? {
            crate::export::collision::CollisionResolution::Write(p) => Ok(p),
            crate::export::collision::CollisionResolution::Skip(p) => Err(ExportError::FileSkipped(p)),
        }
    }
}
