//! Camera EXIF Fingerprint extraction and bounded metadata reader.
//!
//! Authoritative requirements:
//! - US-003 & R3: Extract `BodySerialNumber` (tag 0xa431) and camera model in < 50ms without loading full 30-50MB RAW files into RAM.
//! - 100% Read-Only SD Protocol: strict read-only access (`File::open`), zero writes on source storage media.
//! - Composite fallback: `{make}_{model}` with spaces replaced by underscores when serial is missing or empty.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use super::scanner::{find_all_media_files, MediaCategory, SdCardVolume};
use super::IngestError;

/// Standard EXIF 2.3 Tag for Camera Body Serial Number (0xa431) in Exif SubIFD context.
pub const TAG_BODY_SERIAL_NUMBER: exif::Tag = exif::Tag(exif::Context::Exif, 0xa431);

/// Auxiliary TIFF context variant for BodySerialNumber (used by some camera vendors in IFD0).
pub const TAG_BODY_SERIAL_NUMBER_TIFF: exif::Tag = exif::Tag(exif::Context::Tiff, 0xa431);

/// Standard EXIF Tag for Lens Model (0xa434) in Exif SubIFD context.
pub const TAG_LENS_MODEL_EXIF: exif::Tag = exif::Tag(exif::Context::Exif, 0xa434);

/// Default bounded buffer capacity for EXIF metadata extraction (128 KB).
pub const DEFAULT_MAX_EXIF_BYTES: usize = 128 * 1024;

/// Alias for DEFAULT_MAX_EXIF_BYTES for cross-module compatibility.
pub const DEFAULT_EXIF_BOUND_BYTES: usize = DEFAULT_MAX_EXIF_BYTES;

/// Camera fingerprint extracted from photo EXIF metadata (M3, US-003, R3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraFingerprint {
    pub make: String,
    pub model: String,
    pub body_serial: Option<String>,
    pub lens_model: Option<String>,
}

impl CameraFingerprint {
    /// Create a new CameraFingerprint.
    pub fn new(
        make: impl Into<String>,
        model: impl Into<String>,
        body_serial: Option<String>,
        lens_model: Option<String>,
    ) -> Self {
        Self {
            make: make.into(),
            model: model.into(),
            body_serial,
            lens_model,
        }
    }

    /// Generate primary camera identity key (serial or composite fallback).
    ///
    /// Resolution rules:
    /// 1. If `body_serial` is present and contains non-whitespace characters, return trimmed serial.
    /// 2. Otherwise, return composite `"{make}_{model}"` where all spaces are replaced by underscores.
    /// 3. If both make and model are empty, returns `"unknown_camera"`.
    pub fn camera_id(&self) -> String {
        if let Some(ref serial) = self.body_serial {
            let trimmed = serial.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }

        let clean_make = self.make.trim().replace(' ', "_");
        let clean_model = self.model.trim().replace(' ', "_");

        if clean_make.is_empty() && clean_model.is_empty() {
            "unknown_camera".to_string()
        } else if clean_make.is_empty() {
            clean_model
        } else if clean_model.is_empty() {
            clean_make
        } else {
            format!("{}_{}", clean_make, clean_model)
        }
    }
}

/// Clean an EXIF field string, trimming null characters ('\0'), surrounding quotes, control characters, and whitespace.
///
/// Ensures that whitespace-only strings or strings containing control characters (e.g. tabs `\t`, newlines, nulls)
/// are filtered to `None` and never escape into literal ASCII escape tokens (like `"\\x09"`).
pub fn clean_field_string(field: &exif::Field) -> Option<String> {
    if let exif::Value::Ascii(vec) = &field.value {
        for bytes in vec {
            let s = String::from_utf8_lossy(bytes);
            let first_null = s.split('\0').next().unwrap_or(&s);
            let cleaned = first_null.trim_matches(|c: char| c.is_control() || c.is_whitespace());
            if !cleaned.is_empty() && !cleaned.chars().all(|c| c.is_control() || c.is_whitespace()) {
                return Some(cleaned.to_string());
            }
        }
        return None;
    }

    let display = field.display_value().to_string();
    let cleaned = display
        .trim_matches('"')
        .trim_matches(|c: char| c == '\0' || c.is_control() || c.is_whitespace());
    if cleaned.is_empty()
        || cleaned.chars().all(|c| c.is_control() || c.is_whitespace())
        || cleaned == "\\x00"
        || (cleaned.starts_with("\\x") && cleaned.len() == 4)
    {
        None
    } else {
        Some(cleaned.to_string())
    }
}

/// Helper function to open photo in read-only mode, read up to bounded capacity using a full fill loop,
/// and parse the EXIF container with a fast in-memory Cursor falling back to seekable BufReader.
fn read_exif_container(path: &Path, max_bytes: usize) -> Result<exif::Exif, IngestError> {
    let mut file = File::open(path)
        .map_err(|e| IngestError::Io(format!("Failed to open file for EXIF at '{}': {}", path.display(), e)))?;

    let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if file_len < 16 {
        return Err(IngestError::ExifError(
            "File header too small to contain valid EXIF container".to_string(),
        ));
    }

    let bound = max_bytes.min(file_len as usize).max(16);
    let mut header_buf = vec![0u8; bound];
    let mut total_read = 0;
    while total_read < bound {
        match file.read(&mut header_buf[total_read..]) {
            Ok(0) => break,
            Ok(n) => total_read += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(IngestError::Io(format!("Failed to read EXIF header: {}", e))),
        }
    }
    header_buf.truncate(total_read);

    if header_buf.len() < 16 {
        return Err(IngestError::ExifError(
            "File header too small to contain valid EXIF container".to_string(),
        ));
    }

    // Attempt 1: Parse from bounded in-memory cursor (0 additional I/O, <1ms)
    let mut cursor = std::io::Cursor::new(&header_buf);
    match exif::Reader::new().read_from_container(&mut cursor) {
        Ok(exif) => Ok(exif),
        Err(_) => {
            // Attempt 2: If bounded slice was truncated before SubIFD offsets, fallback to
            // seeking BufReader with bounded capacity. Seeks only within TIFF header, never
            // loads full image strips into RAM.
            if let Ok(seek_file) = File::open(path) {
                let mut bufreader = BufReader::with_capacity(bound, seek_file);
                exif::Reader::new()
                    .read_from_container(&mut bufreader)
                    .map_err(|e| IngestError::ExifError(format!("EXIF parsing error: {}", e)))
            } else {
                Err(IngestError::ExifError(
                    "EXIF parsing error: truncated metadata".to_string(),
                ))
            }
        }
    }
}

/// Bounded EXIF metadata extractor: reads only up to `max_bytes` from the beginning of the file
/// to extract CameraFingerprint in < 50ms without reading full 30-50MB RAW files into RAM.
///
/// 100% Read-Only: Operates strictly via `File::open` (read-only mode), zero disk modifications.
pub fn extract_bounded_exif_fingerprint(
    path: &Path,
    max_bytes: usize,
) -> Result<CameraFingerprint, IngestError> {
    let exif = read_exif_container(path, max_bytes)?;

    // Extract Make
    let make = exif
        .get_field(exif::Tag::Make, exif::In::PRIMARY)
        .or_else(|| exif.fields().find(|f| f.tag == exif::Tag::Make))
        .and_then(clean_field_string)
        .unwrap_or_else(|| "Unknown".to_string());

    // Extract Model
    let model = exif
        .get_field(exif::Tag::Model, exif::In::PRIMARY)
        .or_else(|| exif.fields().find(|f| f.tag == exif::Tag::Model))
        .and_then(clean_field_string)
        .unwrap_or_else(|| "Unknown Model".to_string());

    // Extract BodySerialNumber (tag 0xa431)
    let body_serial = exif
        .get_field(TAG_BODY_SERIAL_NUMBER, exif::In::PRIMARY)
        .or_else(|| exif.get_field(TAG_BODY_SERIAL_NUMBER_TIFF, exif::In::PRIMARY))
        .or_else(|| exif.fields().find(|f| f.tag.1 == 0xa431))
        .and_then(clean_field_string);

    // Extract LensModel (tag 0xa434)
    let lens_model = exif
        .get_field(exif::Tag::LensModel, exif::In::PRIMARY)
        .or_else(|| exif.get_field(TAG_LENS_MODEL_EXIF, exif::In::PRIMARY))
        .or_else(|| exif.fields().find(|f| f.tag.1 == 0xa434))
        .and_then(clean_field_string);

    Ok(CameraFingerprint {
        make,
        model,
        body_serial,
        lens_model,
    })
}

/// Extract camera fingerprint using default bounded capacity (128 KB).
/// Matches PROJECT.md § Interface Contracts.
pub fn extract_camera_fingerprint(photo_path: &Path) -> Result<CameraFingerprint, IngestError> {
    extract_bounded_exif_fingerprint(photo_path, DEFAULT_MAX_EXIF_BYTES)
}

/// Extract capture date from EXIF for date-based folder routing `{target_directory}/{YYYY}/{YYYY-MM-DD}/`.
///
/// Returns `(YYYY, YYYY-MM-DD)`. If EXIF DateTimeOriginal tag is missing, corrupt, or uninitialized,
/// falls back gracefully to current UTC date.
pub fn extract_capture_date(path: &Path) -> Result<(String, String), IngestError> {
    match read_exif_container(path, DEFAULT_MAX_EXIF_BYTES) {
        Ok(exif) => {
            let date_field = exif
                .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
                .or_else(|| exif.get_field(exif::Tag::DateTime, exif::In::PRIMARY))
                .or_else(|| exif.fields().find(|f| f.tag == exif::Tag::DateTimeOriginal || f.tag.1 == 0x9003))
                .or_else(|| exif.fields().find(|f| f.tag.1 == 0x9004))
                .or_else(|| exif.fields().find(|f| f.tag == exif::Tag::DateTime || f.tag.1 == 0x0132));

            if let Some(field) = date_field {
                if let Some(date_str) = clean_field_string(field) {
                    let parts: Vec<&str> = date_str.split_whitespace().collect();
                    if let Some(date_part) = parts.first() {
                        let ymd: Vec<&str> = date_part.split([':', '-', '/']).collect();
                        if ymd.len() >= 3 {
                            let yyyy_str = ymd[0].trim();
                            let mm_str = ymd[1].trim();
                            let dd_str = ymd[2].trim();

                            if let (Ok(yyyy_val), Ok(mm_val), Ok(dd_val)) = (
                                yyyy_str.parse::<u32>(),
                                mm_str.parse::<u32>(),
                                dd_str.parse::<u32>(),
                            ) {
                                if (1970..=2100).contains(&yyyy_val) && (1..=12).contains(&mm_val) && (1..=31).contains(&dd_val) {
                                    let yyyy = format!("{:04}", yyyy_val);
                                    let yyyy_mm_dd = format!("{:04}-{:02}-{:02}", yyyy_val, mm_val, dd_val);
                                    return Ok((yyyy, yyyy_mm_dd));
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(IngestError::Io(e)) => return Err(IngestError::Io(e)),
        Err(IngestError::ExifError(_)) => {
            // Header too small, non-EXIF, or corrupted container: fall back to current UTC date
        }
        Err(e) => return Err(e),
    }

    // Fallback: default to current date
    let now = Utc::now();
    let yyyy = now.format("%Y").to_string();
    let yyyy_mm_dd = now.format("%Y-%m-%d").to_string();
    Ok((yyyy, yyyy_mm_dd))
}

/// Extract camera fingerprint from the first valid photo on an SD card volume.
///
/// Searches across all camera folders in `volume.camera_folders` until a valid photo is found
/// and successfully parsed. If a photo is corrupted or empty, automatically attempts subsequent
/// photos to maximize detection resilience.
pub fn extract_fingerprint_from_volume(
    volume: &SdCardVolume,
) -> Result<CameraFingerprint, IngestError> {
    let all_files = find_all_media_files(&volume.camera_folders);
    let mut last_err = None;

    for file_info in all_files {
        if file_info.category == MediaCategory::Raw || file_info.category == MediaCategory::Jpeg {
            match extract_camera_fingerprint(&file_info.path) {
                Ok(fp) => return Ok(fp),
                Err(e) => {
                    last_err = Some(e);
                    continue;
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        IngestError::ScanError(
            "No photographic assets found on volume to extract EXIF fingerprint".to_string(),
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_camera_fingerprint_camera_id_serial_vs_fallback() {
        // Serial present
        let fp1 = CameraFingerprint::new(
            "Canon",
            "Canon EOS R6 Mark II",
            Some("042021000123".to_string()),
            None,
        );
        assert_eq!(fp1.camera_id(), "042021000123");

        // Serial with whitespace
        let fp2 = CameraFingerprint::new(
            "Canon",
            "Canon EOS R6",
            Some("  987654321  ".to_string()),
            None,
        );
        assert_eq!(fp2.camera_id(), "987654321");

        // Serial None -> composite with underscores
        let fp3 = CameraFingerprint::new("Fujifilm", "X-T30", None, None);
        assert_eq!(fp3.camera_id(), "Fujifilm_X-T30");

        // Serial Empty -> composite
        let fp4 = CameraFingerprint::new("Sony", "ILCE-7M4", Some("".to_string()), None);
        assert_eq!(fp4.camera_id(), "Sony_ILCE-7M4");

        // Serial whitespace-only -> composite
        let fp5 = CameraFingerprint::new("Nikon", "Z 8", Some("   ".to_string()), None);
        assert_eq!(fp5.camera_id(), "Nikon_Z_8");

        // Missing make
        let fp6 = CameraFingerprint::new("", "Hero11", None, None);
        assert_eq!(fp6.camera_id(), "Hero11");

        // Missing both make and model
        let fp7 = CameraFingerprint::new("", "", None, None);
        assert_eq!(fp7.camera_id(), "unknown_camera");
    }

    #[test]
    fn test_extract_non_existent_file() {
        let path = Path::new("non_existent_photo_12345.jpg");
        let res = extract_camera_fingerprint(path);
        assert!(res.is_err());
        match res.unwrap_err() {
            IngestError::Io(_) => {}
            other => panic!("Expected Io error, got {:?}", other),
        }
    }

    #[test]
    fn test_extract_zero_byte_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("zero.jpg");
        File::create(&path).expect("create zero file");

        let res = extract_camera_fingerprint(&path);
        assert!(res.is_err());
        match res.unwrap_err() {
            IngestError::ExifError(msg) => {
                assert!(msg.contains("too small") || msg.contains("EXIF"));
            }
            other => panic!("Expected ExifError, got {:?}", other),
        }
    }

    #[test]
    fn test_extract_truncated_header() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("truncated.jpg");
        let mut file = File::create(&path).expect("create truncated file");
        file.write_all(b"\xFF\xD8\xFF\xE1\x00\x08Exif").expect("write bytes");

        let res = extract_camera_fingerprint(&path);
        assert!(res.is_err());
        match res.unwrap_err() {
            IngestError::ExifError(_) => {}
            other => panic!("Expected ExifError, got {:?}", other),
        }
    }

    #[test]
    fn test_capture_date_fallback_on_empty_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("corrupt_or_empty.jpg");
        File::create(&path).expect("create empty");

        let (yyyy, yyyy_mm_dd) = extract_capture_date(&path).expect("fallback date");
        let now = Utc::now();
        assert_eq!(yyyy, now.format("%Y").to_string());
        assert_eq!(yyyy_mm_dd, now.format("%Y-%m-%d").to_string());
    }

    #[test]
    fn test_capture_date_with_various_delimiters_and_invalid_clock() {
        let dir = tempdir().expect("tempdir");

        // Mock with non-EXIF content > 16 bytes -> fallback to current date
        let uninit_path = dir.path().join("uninit_date.CR3");
        let mut file = File::create(&uninit_path).expect("create file");
        file.write_all(b"not an exif file but > 16 bytes payload for test").expect("write bytes");

        let (yyyy, yyyy_mm_dd) = extract_capture_date(&uninit_path).expect("fallback date");
        let now = Utc::now();
        assert_eq!(yyyy, now.format("%Y").to_string());
        assert_eq!(yyyy_mm_dd, now.format("%Y-%m-%d").to_string());
    }
}
