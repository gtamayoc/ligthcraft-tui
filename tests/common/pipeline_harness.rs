#![allow(dead_code)]

//! Ingest pipeline test harness for streaming copy, in-flight SHA-256, deduplication,
//! date organization, manifest generation, and toast dispatch.

use chrono::Utc;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::contracts::{
    CameraProfile, IngestError, IngestSessionManifest, IngestSummary, ManifestEntry,
    SdCardVolume, ToastNotifier, VolumeScanner,
};
use super::exif_gen::extract_capture_date;

/// Mock volume scanner for testing volume discovery.
#[derive(Debug, Clone, Default)]
pub struct MockVolumeScanner {
    volumes: Vec<SdCardVolume>,
}

impl MockVolumeScanner {
    pub fn new() -> Self {
        Self {
            volumes: Vec::new(),
        }
    }

    pub fn with_volumes(volumes: Vec<SdCardVolume>) -> Self {
        Self { volumes }
    }

    pub fn add_volume(&mut self, volume: SdCardVolume) {
        self.volumes.push(volume);
    }
}

impl VolumeScanner for MockVolumeScanner {
    fn scan_volumes(&self) -> Result<Vec<SdCardVolume>, IngestError> {
        Ok(self.volumes.clone())
    }
}

/// Check if a volume is an empty formatted SD card (US-007, FR-10).
/// Verifies that free space is >= 99% of total capacity and DCIM contains 0 photos.
pub fn check_empty_card(volume: &SdCardVolume) -> bool {
    if volume.total_bytes == 0 {
        return false;
    }

    let free_ratio = volume.free_bytes as f64 / volume.total_bytes as f64;
    let capacity_empty = free_ratio >= 0.99;

    let zero_photos = volume.camera_folders.iter().all(|folder| {
        if let Ok(entries) = fs::read_dir(folder) {
            let photo_count = entries
                .flatten()
                .filter(|e| {
                    if let Some(ext) = e.path().extension().and_then(|s| s.to_str()) {
                        matches!(
                            ext.to_ascii_lowercase().as_str(),
                            "cr3" | "cr2" | "arw" | "nef" | "jpg" | "jpeg" | "mp4" | "mov"
                        )
                    } else {
                        false
                    }
                })
                .count();
            photo_count == 0
        } else {
            true
        }
    });

    capacity_empty && zero_photos
}

/// Ingest Pipeline execution engine for E2E tests.
pub struct IngestPipelineHarness<T: ToastNotifier> {
    pub toast_notifier: Option<Arc<T>>,
    pub chunk_size: usize,
}

impl<T: ToastNotifier> IngestPipelineHarness<T> {
    pub fn new(toast_notifier: Option<Arc<T>>) -> Self {
        Self {
            toast_notifier,
            chunk_size: 1024 * 1024, // 1 MB streaming chunks
        }
    }

    /// Execute the full ingest pipeline asynchronously.
    pub async fn run_ingest(
        &self,
        volume: &SdCardVolume,
        profile: &CameraProfile,
    ) -> Result<IngestSummary, IngestError> {
        let mut transferred_files = 0usize;
        let mut skipped_duplicates = 0usize;
        let mut total_bytes_transferred = 0u64;
        let mut manifest_entries = Vec::new();

        fs::create_dir_all(&profile.target_directory).map_err(|e| {
            IngestError::DestinationError(format!(
                "Failed to create target dir {:?}: {}",
                profile.target_directory, e
            ))
        })?;

        // Collect all media files from camera folders
        for folder in &volume.camera_folders {
            if let Ok(entries) = fs::read_dir(folder) {
                for entry in entries.flatten() {
                    let source_path = entry.path();
                    if source_path.is_file() {
                        let filename = match source_path.file_name().and_then(|n| n.to_str()) {
                            Some(n) => n.to_string(),
                            None => continue,
                        };

                        // Extract date for directory hierarchy {YYYY}/{YYYY-MM-DD}
                        let (yyyy, yyyy_mm_dd) = extract_capture_date(&source_path)
                            .unwrap_or_else(|_| ("2026".to_string(), "2026-10-15".to_string()));

                        let dest_dir = profile.target_directory.join(&yyyy).join(&yyyy_mm_dd);
                        fs::create_dir_all(&dest_dir).map_err(|e| {
                            IngestError::DestinationError(format!(
                                "Failed to create dest subfolder {:?}: {}",
                                dest_dir, e
                            ))
                        })?;

                        let dest_path = dest_dir.join(&filename);
                        let source_size = fs::metadata(&source_path)
                            .map_err(|e| IngestError::Io(format!("Source stat error: {}", e)))?
                            .len();

                        // Streaming copy with in-flight SHA-256 calculation
                        let (sha256_hex, is_duplicate) = self
                            .copy_with_inflight_hash(&source_path, &dest_path, source_size)
                            .await?;

                        if is_duplicate {
                            skipped_duplicates += 1;
                            manifest_entries.push(ManifestEntry {
                                relative_path: format!("{}/{}/{}", yyyy, yyyy_mm_dd, filename),
                                file_size: source_size,
                                sha256: sha256_hex,
                                skipped_duplicate: true,
                            });
                        } else {
                            transferred_files += 1;
                            total_bytes_transferred += source_size;
                            manifest_entries.push(ManifestEntry {
                                relative_path: format!("{}/{}/{}", yyyy, yyyy_mm_dd, filename),
                                file_size: source_size,
                                sha256: sha256_hex,
                                skipped_duplicate: false,
                            });
                        }
                    }
                }
            }
        }

        // Generate session manifest in target directory
        let timestamp_now = Utc::now();
        let manifest_filename = format!(
            "ingest_manifest_{}.json",
            timestamp_now.format("%Y%m%d_%H%M%S")
        );
        let manifest_path = profile.target_directory.join(&manifest_filename);

        let manifest = IngestSessionManifest {
            session_id: format!("session_{}", timestamp_now.timestamp_millis()),
            timestamp: timestamp_now,
            camera_id: profile.camera_id.clone(),
            target_directory: profile.target_directory.to_string_lossy().to_string(),
            total_transferred: transferred_files,
            total_skipped: skipped_duplicates,
            total_bytes: total_bytes_transferred,
            files: manifest_entries,
        };

        let manifest_json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| IngestError::Io(format!("Manifest serialization error: {}", e)))?;
        fs::write(&manifest_path, manifest_json)
            .map_err(|e| IngestError::Io(format!("Manifest write error: {}", e)))?;

        // Dispatch Windows native Toast notification
        if let Some(ref notifier) = self.toast_notifier {
            let title = format!("LightCraft Ingest Complete — {}", profile.display_name);
            let message = format!(
                "Transferred: {} files ({} MB). Skipped: {} duplicates.\nDestination: {}",
                transferred_files,
                total_bytes_transferred / (1024 * 1024),
                skipped_duplicates,
                profile.target_directory.display()
            );
            let _ = notifier.send_toast(&title, &message);
        }

        Ok(IngestSummary {
            transferred_files,
            skipped_duplicates,
            total_bytes: total_bytes_transferred,
            manifest_path,
        })
    }

    /// Asynchronous streaming copy in 1 MB chunks with in-flight SHA-256 calculation.
    /// Checks for deduplication if destination already exists.
    async fn copy_with_inflight_hash(
        &self,
        source_path: &Path,
        dest_path: &Path,
        source_size: u64,
    ) -> Result<(String, bool), IngestError> {
        // Compute source hash while reading
        let mut source_file = tokio::fs::File::open(source_path)
            .await
            .map_err(|e| IngestError::Io(format!("Failed to open source file {:?}: {}", source_path, e)))?;

        // Deduplication check: if destination exists with same size
        if dest_path.exists() {
            if let Ok(dest_meta) = fs::metadata(dest_path) {
                if dest_meta.len() == source_size {
                    // Check dest hash
                    let dest_hash = compute_file_sha256(dest_path).map_err(|e| {
                        IngestError::Io(format!("Failed to hash existing dest file: {}", e))
                    })?;

                    // Compute source hash
                    let mut hasher = Sha256::new();
                    let mut buf = vec![0u8; self.chunk_size];
                    loop {
                        let n = source_file
                            .read(&mut buf)
                            .await
                            .map_err(|e| IngestError::Io(format!("Source read error: {}", e)))?;
                        if n == 0 {
                            break;
                        }
                        hasher.update(&buf[..n]);
                    }
                    let source_hash = hex::encode(hasher.finalize());

                    if dest_hash == source_hash {
                        // Matching duplicate! Skip transfer.
                        return Ok((source_hash, true));
                    }
                }
            }
        }

        // Rewind source file if it was read
        drop(source_file);
        let mut source_file = tokio::fs::File::open(source_path)
            .await
            .map_err(|e| IngestError::Io(format!("Re-opening source failed: {}", e)))?;

        // Stream to destination using 1 MB chunks while computing SHA-256 in-flight
        let mut dest_file = tokio::fs::File::create(dest_path)
            .await
            .map_err(|e| IngestError::DestinationError(format!("Dest create error {:?}: {}", dest_path, e)))?;

        let mut hasher = Sha256::new();
        let mut chunk_buf = vec![0u8; self.chunk_size];

        loop {
            let n = source_file
                .read(&mut chunk_buf)
                .await
                .map_err(|e| IngestError::Io(format!("Source streaming read error: {}", e)))?;
            if n == 0 {
                break;
            }
            hasher.update(&chunk_buf[..n]);
            dest_file
                .write_all(&chunk_buf[..n])
                .await
                .map_err(|e| IngestError::DestinationError(format!("Dest write error: {}", e)))?;
        }

        dest_file
            .flush()
            .await
            .map_err(|e| IngestError::DestinationError(format!("Dest flush error: {}", e)))?;

        let sha256_hex = hex::encode(hasher.finalize());
        Ok((sha256_hex, false))
    }
}

/// Compute SHA-256 of an existing file synchronously.
pub fn compute_file_sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 16384];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}
