//! Native EXIF and Image Metadata extraction engine.
//!
//! Provides pure-Rust, autonomous extraction of photographic shooting parameters,
//! dimensions, camera model, lens, and exposure without depending on external daemons or MCP servers.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use exif::{Field, Reader, Tag, Value};
use crate::mcp::tools::PhotoInfo;

/// Detailed shooting and photographic EXIF metadata.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhotoMetadata {
    pub file_name: String,
    pub file_size_bytes: u64,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub orientation: String,
    pub color_space: String,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_model: Option<String>,
    pub focal_length_mm: Option<f32>,
    pub aperture_f_number: Option<f32>,
    pub shutter_speed: Option<String>,
    pub iso: Option<u32>,
    pub exposure_bias_ev: Option<f32>,
    pub metering_mode: Option<String>,
    pub white_balance: Option<String>,
    pub capture_time: Option<String>,
}

impl PhotoMetadata {
    pub fn from_mcp_metadata(info: &PhotoInfo, raw_exif: &[serde_json::Value]) -> Self {
        let mut meta = Self {
            file_name: info.file_name.clone(),
            format: info.format.clone(),
            width: info.width,
            height: info.height,
            camera_model: info.camera.clone(),
            capture_time: info.captured.clone(),
            ..Default::default()
        };

        for entry in raw_exif {
            let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let val_entry = entry.get("value");

            let val_str = val_entry.and_then(|v| v.as_str()).map(|s| s.to_string());
            let val_num = val_entry.and_then(|v| v.as_f64());
            let val_u64 = val_entry.and_then(|v| v.as_u64());

            match name {
                "FNumber" | "ApertureValue" => {
                    meta.aperture_f_number = val_num
                        .map(|n| n as f32)
                        .or_else(|| val_str.as_deref().and_then(|s| s.parse::<f32>().ok()));
                }
                "ExposureTime" | "ShutterSpeedValue" => {
                    meta.shutter_speed = val_str.or_else(|| val_num.map(|n| {
                        if n < 1.0 && n > 0.0 {
                            format!("1/{}s", (1.0 / n).round() as u64)
                        } else {
                            format!("{:.1}s", n)
                        }
                    }));
                }
                "ISOSpeedRatings" | "PhotographicSensitivity" => {
                    meta.iso = val_u64
                        .map(|u| u as u32)
                        .or_else(|| val_str.as_deref().and_then(|s| s.parse::<u32>().ok()))
                        .or_else(|| {
                            val_entry.and_then(|v| v.as_array()).and_then(|arr| {
                                arr.first().and_then(|item| item.as_u64().map(|u| u as u32))
                            })
                        });
                }
                "FocalLength" => {
                    meta.focal_length_mm = val_num
                        .map(|n| n as f32)
                        .or_else(|| val_str.as_deref().and_then(|s| {
                            s.split(' ').next().and_then(|part| part.parse::<f32>().ok())
                        }));
                }
                "Model" if meta.camera_model.is_none() => {
                    meta.camera_model = val_str;
                }
                "Make" => {
                    meta.camera_make = val_str;
                }
                "LensModel" | "LensType" => {
                    meta.lens_model = val_str;
                }
                "ExposureBiasValue" => {
                    meta.exposure_bias_ev = val_num
                        .map(|n| n as f32)
                        .or_else(|| val_str.as_deref().and_then(|s| s.parse::<f32>().ok()));
                }
                "MeteringMode" => {
                    meta.metering_mode = val_str;
                }
                "WhiteBalance" => {
                    meta.white_balance = val_str;
                }
                _ => {}
            }
        }
        meta
    }

    /// Formats aspect ratio as a simplified ratio string (e.g. "3:2", "16:9").
    pub fn aspect_ratio_str(&self) -> String {
        if self.height == 0 {
            return "Unknown".into();
        }
        let gcd_val = gcd(self.width, self.height);
        let rw = self.width / gcd_val;
        let rh = self.height / gcd_val;
        if (rw, rh) == (3, 2)
            || (rw, rh) == (2, 3)
            || (rw, rh) == (16, 9)
            || (rw, rh) == (4, 3)
            || (rw, rh) == (1, 1)
        {
            format!("{}:{}", rw, rh)
        } else {
            let ratio = self.width as f32 / self.height as f32;
            format!("{:.2}:1", ratio)
        }
    }

    /// Megapixel calculation formatted to 1 decimal place.
    pub fn megapixels_str(&self) -> String {
        let mp = (self.width as f64 * self.height as f64) / 1_000_000.0;
        format!("{:.1} MP", mp)
    }

    /// Formats shooting exposure summary: "1/500s  f/2.8  ISO 400  50mm".
    pub fn exposure_summary(&self) -> String {
        format!(
            "{}  {}  {}  {}",
            self.shutter_speed.as_deref().unwrap_or("1/---s"),
            self.aperture_f_number
                .map(|f| format!("f/{:.1}", f))
                .unwrap_or_else(|| "f/--".into()),
            self.iso
                .map(|i| format!("ISO {}", i))
                .unwrap_or_else(|| "ISO ---".into()),
            self.focal_length_mm
                .map(|f| format!("{:.0}mm", f))
                .unwrap_or_else(|| "--mm".into()),
        )
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

/// Extracts photographic metadata directly from a file on disk.
///
/// Attempts native EXIF parsing first (via `kamadak-exif`), and falls back
/// to reading image headers with `image` reader to guarantee accurate dimensions.
pub fn read_native_metadata(path: &Path) -> PhotoMetadata {
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let file_size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let ext_str = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let format = match ext_str.as_str() {
        "cr3" => "Canon RAW CR3".to_string(),
        "jpg" | "jpeg" => "JPEG".to_string(),
        "png" => "PNG".to_string(),
        "tif" | "tiff" => "TIFF".to_string(),
        "webp" => "WebP".to_string(),
        other => other.to_uppercase(),
    };

    let mut meta = PhotoMetadata {
        file_name,
        file_size_bytes,
        format,
        width: 0,
        height: 0,
        orientation: "Normal".to_string(),
        color_space: "sRGB".to_string(),
        camera_make: None,
        camera_model: None,
        lens_model: None,
        focal_length_mm: None,
        aperture_f_number: None,
        shutter_speed: None,
        iso: None,
        exposure_bias_ev: None,
        metering_mode: None,
        white_balance: None,
        capture_time: None,
    };

    // 1. Try reading EXIF tags directly
    if let Ok(file) = File::open(path) {
        let mut bufreader = BufReader::new(file);
        if let Ok(exif) = Reader::new().read_from_container(&mut bufreader) {
            for f in exif.fields() {
                match f.tag {
                    Tag::Make => {
                        meta.camera_make = Some(f.display_value().to_string().trim_matches('"').trim().to_string());
                    }
                    Tag::Model => {
                        meta.camera_model = Some(f.display_value().to_string().trim_matches('"').trim().to_string());
                    }
                    Tag::LensModel => {
                        meta.lens_model = Some(f.display_value().to_string().trim_matches('"').trim().to_string());
                    }
                    Tag::FocalLength => {
                        if let Some(fl) = parse_rational_f32(&f.value) {
                            meta.focal_length_mm = Some(fl);
                        }
                    }
                    Tag::FNumber => {
                        if let Some(fnum) = parse_rational_f32(&f.value) {
                            meta.aperture_f_number = Some(fnum);
                        }
                    }
                    Tag::ExposureTime => {
                        meta.shutter_speed = Some(format_exposure_time(f));
                    }
                    Tag::PhotographicSensitivity => {
                        if let Some(iso) = parse_u32_val(&f.value) {
                            meta.iso = Some(iso);
                        }
                    }
                    Tag::ExposureBiasValue => {
                        if let Some(bias) = parse_srational_f32(&f.value) {
                            meta.exposure_bias_ev = Some(bias);
                        }
                    }
                    Tag::MeteringMode => {
                        meta.metering_mode = Some(f.display_value().to_string().trim_matches('"').trim().to_string());
                    }
                    Tag::WhiteBalance => {
                        meta.white_balance = Some(f.display_value().to_string().trim_matches('"').trim().to_string());
                    }
                    Tag::DateTimeOriginal | Tag::DateTime => {
                        if meta.capture_time.is_none() {
                            meta.capture_time = Some(f.display_value().to_string().trim_matches('"').trim().to_string());
                        }
                    }
                    Tag::PixelXDimension => {
                        if let Some(w) = parse_u32_val(&f.value) {
                            meta.width = w;
                        }
                    }
                    Tag::PixelYDimension => {
                        if let Some(h) = parse_u32_val(&f.value) {
                            meta.height = h;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // 2. If dimensions were not found in EXIF, inspect image headers directly
    if meta.width == 0 || meta.height == 0 {
        if let Ok(reader) = image::ImageReader::open(path) {
            if let Ok(guessed) = reader.with_guessed_format() {
                if let Ok(dimensions) = guessed.into_dimensions() {
                    meta.width = dimensions.0;
                    meta.height = dimensions.1;
                }
            }
        }
    }

    meta
}

fn parse_rational_f32(val: &Value) -> Option<f32> {
    match val {
        Value::Rational(vec) if !vec.is_empty() => {
            let r = vec[0];
            if r.denom != 0 {
                Some(r.num as f32 / r.denom as f32)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn parse_srational_f32(val: &Value) -> Option<f32> {
    match val {
        Value::SRational(vec) if !vec.is_empty() => {
            let r = vec[0];
            if r.denom != 0 {
                Some(r.num as f32 / r.denom as f32)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn parse_u32_val(val: &Value) -> Option<u32> {
    match val {
        Value::Short(vec) if !vec.is_empty() => Some(vec[0] as u32),
        Value::Long(vec) if !vec.is_empty() => Some(vec[0]),
        _ => None,
    }
}

fn format_exposure_time(field: &Field) -> String {
    match &field.value {
        Value::Rational(vec) if !vec.is_empty() => {
            let r = vec[0];
            if r.num == 1 && r.denom > 1 {
                format!("1/{}s", r.denom)
            } else if r.denom != 0 {
                let sec = r.num as f64 / r.denom as f64;
                if sec >= 1.0 {
                    format!("{:.1}s", sec)
                } else {
                    let inv = (1.0 / sec).round() as u64;
                    format!("1/{}s", inv)
                }
            } else {
                "1/---s".to_string()
            }
        }
        _ => field.display_value().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_native_metadata_nonexistent_file() {
        let meta = read_native_metadata(Path::new("nonexistent_test_photo.jpg"));
        assert_eq!(meta.file_name, "nonexistent_test_photo.jpg");
        assert_eq!(meta.format, "JPEG");
        assert_eq!(meta.width, 0);
        assert_eq!(meta.height, 0);
        assert!(meta.camera_model.is_none());
        assert_eq!(meta.exposure_summary(), "1/---s  f/--  ISO ---  --mm");
    }

    #[test]
    fn test_read_native_metadata_fallback_to_image_header() {
        // Create an uncompressed 100x50 PNG file
        let mut tmp = NamedTempFile::new().unwrap();
        let img = image::RgbaImage::new(100, 50);
        img.write_to(&mut tmp, image::ImageFormat::Png).unwrap();

        let meta = read_native_metadata(tmp.path());
        assert_eq!(meta.width, 100);
        assert_eq!(meta.height, 50);
        assert_eq!(meta.aspect_ratio_str(), "2.00:1");
    }
}
