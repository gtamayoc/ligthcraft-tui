//! LightCraft Automated SD Ingest Subsystem.
//!
//! Provides Windows removable volume detection, DCIM directory scanning,
//! camera EXIF fingerprinting, atomic camera registry, streaming file transfer,
//! and Windows native Toast notification integration.

pub mod empty_handler;
pub mod exif_fingerprint;
pub mod pipeline;
pub mod registry;
pub mod scanner;
pub mod toast;

use thiserror::Error;

/// Domain error type for SD Ingest Hub operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IngestError {
    #[error("I/O error: {0}")]
    Io(String),

    #[error("EXIF extraction failed: {0}")]
    ExifError(String),

    #[error("Registry error: {0}")]
    RegistryError(String),

    #[error("Volume scan error: {0}")]
    ScanError(String),

    #[error("Write integrity violation: attempt to modify read-only SD medium at {0}")]
    ReadOnlyViolation(String),

    #[error("Destination error: {0}")]
    DestinationError(String),

    #[error("Toast notification error: {0}")]
    ToastError(String),

    #[error("Storage full: {0}")]
    StorageFull(String),

    #[error("Card removed: {0}")]
    CardRemoved(String),

    #[error("Volume error: {0}")]
    Volume(String),
}

impl From<std::io::Error> for IngestError {
    fn from(err: std::io::Error) -> Self {
        IngestError::Io(err.to_string())
    }
}

impl From<IngestError> for crate::error::AppError {
    fn from(err: IngestError) -> Self {
        match err {
            IngestError::Io(msg) => {
                crate::error::AppError::Io(std::io::Error::other(msg))
            }
            other => crate::error::AppError::Config(other.to_string()),
        }
    }
}

pub type Result<T, E = IngestError> = std::result::Result<T, E>;

pub use empty_handler::{
    check_empty_card, find_all_matching_cameras_by_capacity, find_matching_camera_by_capacity,
    format_card_capacity, is_empty_formatted_card, matches_capacity,
};
pub use exif_fingerprint::{
    clean_field_string, extract_bounded_exif_fingerprint, extract_camera_fingerprint,
    extract_capture_date, extract_fingerprint_from_volume, CameraFingerprint,
    DEFAULT_EXIF_BOUND_BYTES, DEFAULT_MAX_EXIF_BYTES, TAG_BODY_SERIAL_NUMBER,
    TAG_BODY_SERIAL_NUMBER_TIFF, TAG_LENS_MODEL_EXIF,
};
pub use registry::{CameraProfile, CameraRegistry};
pub use scanner::{
    crawl_camera_folders, crawl_dcim, crawl_media_summary, find_all_media_files,
    find_dcim_dir, find_first_media_file, is_dotfile_or_system_file, is_jpeg_extension,
    is_media_extension, is_raw_extension, is_video_extension, MediaCategory,
    MediaFileInfo, MediaSummary, MockVolumeScanner, SdCardVolume, VolumeScanner,
    WindowsVolumeScanner,
};
pub use pipeline::{
    compute_file_sha256, compute_file_sha256_async, IngestCancellationToken, IngestManifest,
    IngestPipeline, IngestProgress, IngestSessionManifest, IngestStatus, IngestSummary,
    ManifestEntry, TempFileGuard, DEFAULT_CHUNK_SIZE,
};
pub use toast::{
    encode_powershell_script, escape_xml, MockToastNotifier, ToastNotifier, WindowsToastNotifier,
};
