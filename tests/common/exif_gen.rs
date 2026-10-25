#![allow(dead_code)]

//! Synthetic EXIF metadata generator and bounded reader for tests.
//! Supports valid TIFF and JPEG containers containing Make, Model, BodySerialNumber (0xa431),
//! DateTimeOriginal (0x9003), and LensModel (0xa434).

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use exif::{Context, In, Reader, Tag, Value};

use super::contracts::{CameraFingerprint, IngestError};

/// Builder for synthetic EXIF photos (JPEG or TIFF/RAW mock).
#[derive(Debug, Clone)]
pub struct ExifPhotoBuilder {
    pub make: String,
    pub model: String,
    pub body_serial: Option<String>,
    pub lens_model: Option<String>,
    pub date_time_original: Option<String>,
    pub dummy_payload_size: usize,
    pub is_jpeg: bool,
}

impl Default for ExifPhotoBuilder {
    fn default() -> Self {
        Self {
            make: "Canon".to_string(),
            model: "Canon EOS R6".to_string(),
            body_serial: Some("042021000123".to_string()),
            lens_model: Some("RF 24-70mm F2.8 L IS USM".to_string()),
            date_time_original: Some("2026:10:15 10:30:00".to_string()),
            dummy_payload_size: 1024,
            is_jpeg: true,
        }
    }
}

impl ExifPhotoBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn make(mut self, make: &str) -> Self {
        self.make = make.to_string();
        self
    }

    pub fn model(mut self, model: &str) -> Self {
        self.model = model.to_string();
        self
    }

    pub fn body_serial(mut self, serial: Option<&str>) -> Self {
        self.body_serial = serial.map(|s| s.to_string());
        self
    }

    pub fn lens_model(mut self, lens: Option<&str>) -> Self {
        self.lens_model = lens.map(|s| s.to_string());
        self
    }

    pub fn date_time(mut self, dt: &str) -> Self {
        self.date_time_original = Some(dt.to_string());
        self
    }

    pub fn payload_size(mut self, size: usize) -> Self {
        self.dummy_payload_size = size;
        self
    }

    pub fn as_raw(mut self) -> Self {
        self.is_jpeg = false;
        self
    }

    /// Build raw binary bytes representing this image file with valid EXIF.
    pub fn build_bytes(&self) -> Vec<u8> {
        let tiff_bytes = self.build_tiff_structure();

        if self.is_jpeg {
            let mut jpeg = Vec::new();
            // SOI marker
            jpeg.extend_from_slice(&[0xFF, 0xD8]);

            // APP1 marker
            jpeg.extend_from_slice(&[0xFF, 0xE1]);
            let app1_len = (2 + 6 + tiff_bytes.len()) as u16;
            jpeg.extend_from_slice(&app1_len.to_be_bytes());
            jpeg.extend_from_slice(b"Exif\0\0");
            jpeg.extend_from_slice(&tiff_bytes);

            // Mock image payload
            if self.dummy_payload_size > 0 {
                jpeg.resize(jpeg.len() + self.dummy_payload_size, 0xAA);
            }

            // EOI marker
            jpeg.extend_from_slice(&[0xFF, 0xD9]);
            jpeg
        } else {
            // TIFF-based RAW mock (starts directly with TIFF header)
            let mut raw = tiff_bytes;
            if self.dummy_payload_size > 0 {
                raw.resize(raw.len() + self.dummy_payload_size, 0xBB);
            }
            raw
        }
    }

    /// Write generated image to disk at the specified path.
    pub fn write_to_file(&self, path: &Path) -> std::io::Result<()> {
        let bytes = self.build_bytes();
        let mut file = File::create(path)?;
        file.write_all(&bytes)?;
        file.flush()?;
        Ok(())
    }

    /// Construct binary TIFF Little-Endian (II) containing IFD0 and Exif SubIFD.
    fn build_tiff_structure(&self) -> Vec<u8> {
        // Collect string values with null terminators
        let make_bytes = null_terminated(&self.make);
        let model_bytes = null_terminated(&self.model);

        let dt_bytes = self.date_time_original.as_ref().map(|s| null_terminated(s));
        let serial_bytes = self.body_serial.as_ref().map(|s| null_terminated(s));
        let lens_bytes = self.lens_model.as_ref().map(|s| null_terminated(s));

        // Layout offsets:
        // 0..8: TIFF Header
        // 8..: IFD0
        // IFD0 entries:
        // - 0x010f: Make (ASCII)
        // - 0x0110: Model (ASCII)
        // - 0x8769: ExifOffset (LONG)
        let ifd0_count: u16 = 3;
        let ifd0_size = 2 + (ifd0_count as usize * 12) + 4; // 2 + 36 + 4 = 42 bytes
        let ifd0_offset = 8usize;
        let exif_subifd_offset = ifd0_offset + ifd0_size; // 50

        // Exif SubIFD entries:
        // - 0x9003: DateTimeOriginal (ASCII) if present
        // - 0xa431: BodySerialNumber (ASCII) if present
        // - 0xa434: LensModel (ASCII) if present
        let mut subifd_entries_count = 0u16;
        if dt_bytes.is_some() {
            subifd_entries_count += 1;
        }
        if serial_bytes.is_some() {
            subifd_entries_count += 1;
        }
        if lens_bytes.is_some() {
            subifd_entries_count += 1;
        }

        let subifd_size = 2 + (subifd_entries_count as usize * 12) + 4;
        let data_offset_start = exif_subifd_offset + subifd_size;

        // Allocate buffer and calculate string offsets
        let mut data_pool = Vec::new();

        let make_offset = (data_offset_start + data_pool.len()) as u32;
        data_pool.extend_from_slice(&make_bytes);

        let model_offset = (data_offset_start + data_pool.len()) as u32;
        data_pool.extend_from_slice(&model_bytes);

        let dt_offset = dt_bytes.as_ref().map(|db| {
            let off = (data_offset_start + data_pool.len()) as u32;
            data_pool.extend_from_slice(db);
            off
        });

        let serial_offset = serial_bytes.as_ref().map(|sb| {
            let off = (data_offset_start + data_pool.len()) as u32;
            data_pool.extend_from_slice(sb);
            off
        });

        let lens_offset = lens_bytes.as_ref().map(|lb| {
            let off = (data_offset_start + data_pool.len()) as u32;
            data_pool.extend_from_slice(lb);
            off
        });

        // Assemble binary TIFF
        let mut buf = Vec::with_capacity(data_offset_start + data_pool.len());

        // TIFF Header: "II" (little-endian), 0x002A (42), offset of IFD0 (8)
        buf.extend_from_slice(b"II");
        buf.extend_from_slice(&42u16.to_le_bytes());
        buf.extend_from_slice(&(ifd0_offset as u32).to_le_bytes());

        // IFD0
        buf.extend_from_slice(&ifd0_count.to_le_bytes());

        // 1. Tag 0x010f (Make, type 2 = ASCII)
        write_ifd_entry(
            &mut buf,
            0x010f,
            2,
            make_bytes.len() as u32,
            make_offset,
        );

        // 2. Tag 0x0110 (Model, type 2 = ASCII)
        write_ifd_entry(
            &mut buf,
            0x0110,
            2,
            model_bytes.len() as u32,
            model_offset,
        );

        // 3. Tag 0x8769 (ExifOffset, type 4 = LONG)
        write_ifd_entry(
            &mut buf,
            0x8769,
            4,
            1,
            exif_subifd_offset as u32,
        );

        // Next IFD offset: 0
        buf.extend_from_slice(&0u32.to_le_bytes());

        // Exif SubIFD
        buf.extend_from_slice(&subifd_entries_count.to_le_bytes());

        // SubIFD tags in ascending numerical order:
        // 0x9003 (DateTimeOriginal)
        if let (Some(db), Some(off)) = (&dt_bytes, dt_offset) {
            write_ifd_entry(&mut buf, 0x9003, 2, db.len() as u32, off);
        }

        // 0xa431 (BodySerialNumber)
        if let (Some(sb), Some(off)) = (&serial_bytes, serial_offset) {
            write_ifd_entry(&mut buf, 0xa431, 2, sb.len() as u32, off);
        }

        // 0xa434 (LensModel)
        if let (Some(lb), Some(off)) = (&lens_bytes, lens_offset) {
            write_ifd_entry(&mut buf, 0xa434, 2, lb.len() as u32, off);
        }

        // Next SubIFD offset: 0
        buf.extend_from_slice(&0u32.to_le_bytes());

        // Data pool with actual string bytes
        buf.extend_from_slice(&data_pool);

        buf
    }
}

fn null_terminated(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

fn write_ifd_entry(buf: &mut Vec<u8>, tag: u16, field_type: u16, count: u32, value_or_offset: u32) {
    buf.extend_from_slice(&tag.to_le_bytes());
    buf.extend_from_slice(&field_type.to_le_bytes());
    buf.extend_from_slice(&count.to_le_bytes());
    buf.extend_from_slice(&value_or_offset.to_le_bytes());
}

/// Bounded EXIF metadata extractor: reads only up to `max_bytes` (default 128 KB)
/// from the beginning of the file to extract CameraFingerprint in < 50ms.
pub fn extract_bounded_exif_fingerprint(
    path: &Path,
    max_bytes: usize,
) -> Result<CameraFingerprint, IngestError> {
    let mut file = File::open(path)
        .map_err(|e| IngestError::Io(format!("Failed to open file for EXIF: {}", e)))?;

    // Read at most max_bytes (e.g. 128 KB) into memory
    let mut header_buf = vec![0u8; max_bytes];
    let bytes_read = file
        .read(&mut header_buf)
        .map_err(|e| IngestError::Io(format!("Failed to read EXIF header: {}", e)))?;
    header_buf.truncate(bytes_read);

    if header_buf.len() < 16 {
        return Err(IngestError::ExifError(
            "File header too small to contain valid EXIF container".to_string(),
        ));
    }

    // Parse EXIF from bounded buffer
    let mut cursor = std::io::Cursor::new(&header_buf);
    let exif = Reader::new()
        .read_from_container(&mut cursor)
        .map_err(|e| IngestError::ExifError(format!("EXIF parsing error: {}", e)))?;

    let make = exif
        .get_field(Tag::Make, In::PRIMARY)
        .and_then(|f| match &f.value {
            Value::Ascii(ref v) => v.first().map(|s| String::from_utf8_lossy(s).trim().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "Unknown".to_string());

    let model = exif
        .get_field(Tag::Model, In::PRIMARY)
        .and_then(|f| match &f.value {
            Value::Ascii(ref v) => v.first().map(|s| String::from_utf8_lossy(s).trim().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "Unknown Model".to_string());

    // Tag::BodySerialNumber = Tag(Context::Exif, 0xa431)
    let body_serial = exif
        .get_field(Tag(Context::Exif, 0xa431), In::PRIMARY)
        .and_then(|f| match &f.value {
            Value::Ascii(ref v) => v.first().map(|s| String::from_utf8_lossy(s).trim().to_string()),
            _ => None,
        });

    // Tag::LensModel = Tag(Context::Exif, 0xa434)
    let lens_model = exif
        .get_field(Tag(Context::Exif, 0xa434), In::PRIMARY)
        .and_then(|f| match &f.value {
            Value::Ascii(ref v) => v.first().map(|s| String::from_utf8_lossy(s).trim().to_string()),
            _ => None,
        });

    Ok(CameraFingerprint {
        make,
        model,
        body_serial,
        lens_model,
    })
}

/// Extract capture date from EXIF for date-based folder routing {YYYY}/{YYYY-MM-DD}.
pub fn extract_capture_date(path: &Path) -> Result<(String, String), IngestError> {
    let mut file = File::open(path)
        .map_err(|e| IngestError::Io(format!("Failed to open file: {}", e)))?;
    let mut buf = vec![0u8; 128 * 1024];
    let n = file.read(&mut buf).unwrap_or(0);
    buf.truncate(n);

    let mut cursor = std::io::Cursor::new(&buf);
    if let Ok(exif) = Reader::new().read_from_container(&mut cursor) {
        if let Some(f) = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY) {
            if let Value::Ascii(ref v) = f.value {
                if let Some(s) = v.first() {
                    let date_str = String::from_utf8_lossy(s);
                    // Standard format: "YYYY:MM:DD HH:MM:SS"
                    let parts: Vec<&str> = date_str.split_whitespace().collect();
                    if let Some(date_part) = parts.first() {
                        let ymd: Vec<&str> = date_part.split(':').collect();
                        if ymd.len() >= 3 {
                            let yyyy = ymd[0].to_string();
                            let yyyy_mm_dd = format!("{}-{}-{}", ymd[0], ymd[1], ymd[2]);
                            return Ok((yyyy, yyyy_mm_dd));
                        }
                    }
                }
            }
        }
    }

    // Fallback: default to current date
    let now = chrono::Utc::now();
    let yyyy = now.format("%Y").to_string();
    let yyyy_mm_dd = now.format("%Y-%m-%d").to_string();
    Ok((yyyy, yyyy_mm_dd))
}
