#![allow(dead_code)]

//! Interface contracts for LightCraft SD Ingest Hub & Modular Launcher.
//! Authoritative source: PROJECT.md § Interface Contracts & tasks/prd-sd-ingest-hub.md

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Errors that can occur during SD Ingest Hub operations.
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
}

/// Tool options available in the Modular Launcher Hub (M1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolOption {
    /// LightCraft Studio (RAW developer)
    Studio,
    /// SD Ingest & Storage Analyzer
    SdIngest,
    /// Exit application
    Exit,
}

/// Representation of a removable storage volume (M2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdCardVolume {
    pub drive_letter: char,
    pub volume_label: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub has_dcim: bool,
    pub camera_folders: Vec<PathBuf>,
}

/// Trait abstracting volume scanning for removable drives (M2).
pub trait VolumeScanner: Send + Sync {
    fn scan_volumes(&self) -> Result<Vec<SdCardVolume>, IngestError>;
}

/// Camera fingerprint extracted from photo EXIF metadata (M3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraFingerprint {
    pub make: String,
    pub model: String,
    pub body_serial: Option<String>,
    pub lens_model: Option<String>,
}

impl CameraFingerprint {
    /// Generate primary camera identity key (serial or composite fallback).
    pub fn camera_id(&self) -> String {
        if let Some(ref serial) = self.body_serial {
            let trimmed = serial.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        format!("{}_{}", self.make.trim(), self.model.trim())
    }
}

/// Persistent camera profile stored in camera_registry.toml (M3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraProfile {
    pub camera_id: String,
    pub display_name: String,
    pub target_directory: PathBuf,
    pub known_card_capacities: Vec<u64>,
    pub last_ingest_timestamp: Option<DateTime<Utc>>,
    pub total_photos_ingested: u64,
}

/// Summary of an ingest execution session (M4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestSummary {
    pub transferred_files: usize,
    pub skipped_duplicates: usize,
    pub total_bytes: u64,
    pub manifest_path: PathBuf,
}

/// Manifest recording audit trail of transferred files (M4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub relative_path: String,
    pub file_size: u64,
    pub sha256: String,
    pub skipped_duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestSessionManifest {
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
    pub camera_id: String,
    pub target_directory: String,
    pub total_transferred: usize,
    pub total_skipped: usize,
    pub total_bytes: u64,
    pub files: Vec<ManifestEntry>,
}

/// Trait for dispatching native OS toast notifications (M4).
pub trait ToastNotifier: Send + Sync {
    fn send_toast(&self, title: &str, message: &str) -> Result<(), IngestError>;
}

/// Mock Toast Notifier for recording toast dispatches during tests.
#[derive(Debug, Default, Clone)]
pub struct MockToastNotifier {
    pub records: Arc<Mutex<Vec<(String, String)>>>,
}

impl MockToastNotifier {
    pub fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn last_notification(&self) -> Option<(String, String)> {
        self.records.lock().unwrap().last().cloned()
    }

    pub fn count(&self) -> usize {
        self.records.lock().unwrap().len()
    }
}

impl ToastNotifier for MockToastNotifier {
    fn send_toast(&self, title: &str, message: &str) -> Result<(), IngestError> {
        self.records
            .lock()
            .unwrap()
            .push((title.to_string(), message.to_string()));
        Ok(())
    }
}
