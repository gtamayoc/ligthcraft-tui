//! Streaming Ingest Pipeline, In-Flight SHA-256 Checksumming, Deduplication & Atomic Creation.
//!
//! Authoritative source: `PROJECT.md` § Streaming Ingest Pipeline Layer & `tasks/prd-sd-ingest-hub.md` (US-006).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::exif_fingerprint::extract_capture_date;
use super::registry::CameraProfile;
use super::scanner::{find_all_media_files, SdCardVolume};
use super::toast::ToastNotifier;
use super::IngestError;

/// Default streaming buffer size: 1 MB (1,048,576 bytes) for zero-memory impact.
pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;

static TMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Ingest progress transmission payload for TUI status updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestProgress {
    pub current_file: String,
    pub current_file_index: usize,
    pub total_files: usize,
    pub current_file_bytes: u64,
    pub current_file_total_bytes: u64,
    pub total_bytes_transferred: u64,
    pub total_expected_bytes: u64,
    pub skipped_duplicates: usize,
    pub status: IngestStatus,
}

/// Operational state of the ingest pipeline during file streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IngestStatus {
    Starting,
    Transferring,
    SkippedDuplicate,
    FileCompleted,
    OverallCompleted,
}

/// Thread-safe cancellation token for responsive abort of active transfers.
#[derive(Debug, Clone, Default)]
pub struct IngestCancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl IngestCancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// RAII Temporary File Guard ensuring clean deletion of partial files on error, abort, or panic.
pub struct TempFileGuard<'a> {
    path: &'a Path,
    active: bool,
}

impl<'a> TempFileGuard<'a> {
    pub fn new(path: &'a Path) -> Self {
        Self { path, active: true }
    }

    pub fn disarm(&mut self) {
        self.active = false;
    }
}

impl<'a> Drop for TempFileGuard<'a> {
    fn drop(&mut self) {
        if self.active && self.path.exists() {
            let _ = std::fs::remove_file(self.path);
        }
    }
}

/// Summary metrics returned upon completion of an ingest pipeline execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestSummary {
    /// Count of newly copied and integrity-verified files.
    pub transferred_files: usize,
    /// Count of existing files skipped due to identical size and SHA-256.
    pub skipped_duplicates: usize,
    /// Total bytes transferred across all newly copied files.
    pub total_bytes: u64,
    /// Path to the generated session manifest JSON file.
    pub manifest_path: PathBuf,
}

/// Manifest entry audit record for an ingested photographic asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Relative path within target directory (e.g. "2026/2026-10-15/IMG_0001.JPG").
    pub relative_path: String,
    /// Full destination path on target disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_path: Option<String>,
    /// Size of the file in bytes.
    pub file_size: u64,
    /// Hex-encoded SHA-256 cryptographic digest.
    pub sha256: String,
    /// True if skipped due to matching destination duplicate.
    pub skipped_duplicate: bool,
    /// Transfer status string ("transferred", "duplicate_skipped", "failed").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Session manifest serialized to JSON on ingest completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestSessionManifest {
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
    pub camera_id: String,
    pub target_directory: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_scanned: Option<usize>,
    pub total_transferred: usize,
    pub total_skipped: usize,
    pub total_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub files: Vec<ManifestEntry>,
}

/// Type alias for interface contract compatibility.
pub type IngestManifest = IngestSessionManifest;

/// Streaming Ingest Pipeline orchestrator.
pub struct IngestPipeline {
    toast_notifier: Option<Arc<dyn ToastNotifier>>,
    chunk_size: usize,
    cancellation_token: Option<IngestCancellationToken>,
}

impl Default for IngestPipeline {
    fn default() -> Self {
        Self::new(None)
    }
}

impl IngestPipeline {
    /// Create a new IngestPipeline with an optional toast notifier.
    pub fn new(toast_notifier: Option<Arc<dyn ToastNotifier>>) -> Self {
        Self {
            toast_notifier,
            chunk_size: DEFAULT_CHUNK_SIZE,
            cancellation_token: None,
        }
    }

    /// Configure a custom streaming chunk size (default 1 MB).
    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size.max(1024);
        self
    }

    /// Attach a thread-safe cancellation token.
    pub fn with_cancellation_token(mut self, token: IngestCancellationToken) -> Self {
        self.cancellation_token = Some(token);
        self
    }

    /// Simple ingest execution without progress channel.
    pub async fn run_ingest_simple(
        &self,
        volume: &SdCardVolume,
        profile: &CameraProfile,
    ) -> Result<IngestSummary, IngestError> {
        self.run_ingest(volume, profile, None).await
    }

    /// Execute the complete ingest pipeline asynchronously with optional progress channel.
    pub async fn run_ingest(
        &self,
        volume: &SdCardVolume,
        profile: &CameraProfile,
        progress_tx: Option<mpsc::Sender<IngestProgress>>,
    ) -> Result<IngestSummary, IngestError> {
        let start_instant = std::time::Instant::now();

        tokio::fs::create_dir_all(&profile.target_directory)
            .await
            .map_err(|e| {
                IngestError::DestinationError(format!(
                    "Failed to create target directory '{:?}': {}",
                    profile.target_directory, e
                ))
            })?;

        // 1. Gather all media files recursively (100% read-only scan)
        let media_files = find_all_media_files(&volume.camera_folders);
        let total_files = media_files.len();
        let total_expected_bytes: u64 = media_files.iter().map(|f| f.size_bytes).sum();

        if let Some(ref tx) = progress_tx {
            let _ = tx
                .send(IngestProgress {
                    current_file: String::new(),
                    current_file_index: 0,
                    total_files,
                    current_file_bytes: 0,
                    current_file_total_bytes: 0,
                    total_bytes_transferred: 0,
                    total_expected_bytes,
                    skipped_duplicates: 0,
                    status: IngestStatus::Starting,
                })
                .await;
        }

        let mut transferred_files = 0usize;
        let mut skipped_duplicates = 0usize;
        let mut total_bytes_transferred = 0u64;
        let mut manifest_entries = Vec::with_capacity(total_files);

        for (idx, file_info) in media_files.into_iter().enumerate() {
            // Check cancellation
            if let Some(ref token) = self.cancellation_token {
                if token.is_cancelled() {
                    return Err(IngestError::Io("Ingest cancelled by user".to_string()));
                }
            }
            if let Some(ref tx) = progress_tx {
                if tx.is_closed() {
                    return Err(IngestError::Io(
                        "Ingest progress channel closed (cancelled)".to_string(),
                    ));
                }
            }

            let source_path = file_info.path;
            let filename = match source_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            // 2. Date-based routing: extract (YYYY, YYYY-MM-DD)
            let (yyyy, yyyy_mm_dd) = extract_capture_date(&source_path).unwrap_or_else(|_| {
                let now = Utc::now();
                (
                    now.format("%Y").to_string(),
                    now.format("%Y-%m-%d").to_string(),
                )
            });

            let dest_dir = profile.target_directory.join(&yyyy).join(&yyyy_mm_dd);
            tokio::fs::create_dir_all(&dest_dir).await.map_err(|e| {
                IngestError::DestinationError(format!(
                    "Failed to create dest subfolder '{:?}': {}",
                    dest_dir, e
                ))
            })?;

            let dest_path = dest_dir.join(&filename);
            let source_size = file_info.size_bytes;

            let params = TransferParams {
                source_path: &source_path,
                dest_dir: &dest_dir,
                dest_path: &dest_path,
                filename: &filename,
                source_size,
                file_index: idx + 1,
                total_files,
                accumulated_bytes: total_bytes_transferred,
                total_expected_bytes,
                skipped_duplicates,
            };

            // 3. Streaming copy with in-flight SHA-256 and single-pass deduplication
            let (sha256_hex, is_duplicate) = self
                .copy_with_inflight_hash(&params, &progress_tx)
                .await?;

            let rel_path = format!("{}/{}/{}", yyyy, yyyy_mm_dd, filename);
            let dest_path_str = dest_path.to_string_lossy().to_string();

            if is_duplicate {
                skipped_duplicates += 1;
                manifest_entries.push(ManifestEntry {
                    relative_path: rel_path,
                    destination_path: Some(dest_path_str),
                    file_size: source_size,
                    sha256: sha256_hex,
                    skipped_duplicate: true,
                    status: Some("duplicate_skipped".to_string()),
                });

                if let Some(ref tx) = progress_tx {
                    let _ = tx
                        .send(IngestProgress {
                            current_file: filename,
                            current_file_index: idx + 1,
                            total_files,
                            current_file_bytes: source_size,
                            current_file_total_bytes: source_size,
                            total_bytes_transferred,
                            total_expected_bytes,
                            skipped_duplicates,
                            status: IngestStatus::SkippedDuplicate,
                        })
                        .await;
                }
            } else {
                transferred_files += 1;
                total_bytes_transferred += source_size;
                manifest_entries.push(ManifestEntry {
                    relative_path: rel_path,
                    destination_path: Some(dest_path_str),
                    file_size: source_size,
                    sha256: sha256_hex,
                    skipped_duplicate: false,
                    status: Some("transferred".to_string()),
                });

                if let Some(ref tx) = progress_tx {
                    let _ = tx
                        .send(IngestProgress {
                            current_file: filename,
                            current_file_index: idx + 1,
                            total_files,
                            current_file_bytes: source_size,
                            current_file_total_bytes: source_size,
                            total_bytes_transferred,
                            total_expected_bytes,
                            skipped_duplicates,
                            status: IngestStatus::FileCompleted,
                        })
                        .await;
                }
            }
        }

        let duration_ms = start_instant.elapsed().as_millis() as u64;

        // 4. Manifest generation
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
            volume_label: Some(volume.volume_label.clone()),
            total_scanned: Some(total_files),
            total_transferred: transferred_files,
            total_skipped: skipped_duplicates,
            total_bytes: total_bytes_transferred,
            duration_ms: Some(duration_ms),
            files: manifest_entries,
        };

        let manifest_json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| IngestError::Io(format!("Manifest serialization error: {}", e)))?;

        // Atomic write of manifest file
        let manifest_tmp = profile
            .target_directory
            .join(format!(".{}.tmp", manifest_filename));
        tokio::fs::write(&manifest_tmp, &manifest_json)
            .await
            .map_err(|e| IngestError::Io(format!("Manifest temp write error: {}", e)))?;
        tokio::fs::rename(&manifest_tmp, &manifest_path)
            .await
            .map_err(|e| IngestError::Io(format!("Manifest rename error: {}", e)))?;

        // 5. Native Windows Toast notification
        if let Some(ref notifier) = self.toast_notifier {
            let title = format!("LightCraft Ingest Complete — {}", profile.display_name);
            let message = format!(
                "Transferred: {} files ({} MB). Skipped: {} duplicates.\nDestination: {}",
                transferred_files,
                total_bytes_transferred / (1024 * 1024),
                skipped_duplicates,
                profile.target_directory.display()
            );
            if let Err(e) = notifier.send_toast(&title, &message) {
                warn!("Toast notification failed (non-fatal): {}", e);
            }
        }

        if let Some(ref tx) = progress_tx {
            let _ = tx
                .send(IngestProgress {
                    current_file: String::new(),
                    current_file_index: total_files,
                    total_files,
                    current_file_bytes: 0,
                    current_file_total_bytes: 0,
                    total_bytes_transferred,
                    total_expected_bytes,
                    skipped_duplicates,
                    status: IngestStatus::OverallCompleted,
                })
                .await;
        }

        info!(
            "Ingest complete: {} transferred ({} bytes), {} duplicates skipped. Manifest: {:?}",
            transferred_files, total_bytes_transferred, skipped_duplicates, manifest_path
        );

        Ok(IngestSummary {
            transferred_files,
            skipped_duplicates,
            total_bytes: total_bytes_transferred,
            manifest_path,
        })
    }
}

/// Internal parameters for streaming transfer of a single photographic asset.
struct TransferParams<'a> {
    source_path: &'a Path,
    dest_dir: &'a Path,
    dest_path: &'a Path,
    filename: &'a str,
    source_size: u64,
    file_index: usize,
    total_files: usize,
    accumulated_bytes: u64,
    total_expected_bytes: u64,
    skipped_duplicates: usize,
}

impl IngestPipeline {
    /// Streaming transfer with in-flight SHA-256 calculation and single-pass deduplication.
    ///
    /// Preserves the 100% Read-Only SD Protocol Invariant: source files are opened exclusively
    /// with read-only descriptors (`File::open`). Zero write calls target the source medium.
    async fn copy_with_inflight_hash(
        &self,
        params: &TransferParams<'_>,
        progress_tx: &Option<mpsc::Sender<IngestProgress>>,
    ) -> Result<(String, bool), IngestError> {
        let source_path = params.source_path;
        let dest_dir = params.dest_dir;
        let dest_path = params.dest_path;
        let filename = params.filename;
        let source_size = params.source_size;
        let file_index = params.file_index;
        let total_files = params.total_files;
        let accumulated_bytes = params.accumulated_bytes;
        let total_expected_bytes = params.total_expected_bytes;
        let skipped_duplicates = params.skipped_duplicates;

        // Fast deduplication check: if destination file exists and byte size matches exactly
        let mut is_potential_duplicate = false;
        if dest_path.exists() {
            if let Ok(dest_meta) = tokio::fs::metadata(dest_path).await {
                if dest_meta.len() == source_size {
                    is_potential_duplicate = true;
                }
            }
        }

        if is_potential_duplicate {
            // Read existing destination file on fast local disk to compute destination hash
            let dest_hash = compute_file_sha256_async(dest_path).await?;

            // Read source file in read-only mode to compute source hash (0 disk writes)
            let mut source_file = tokio::fs::File::open(source_path).await.map_err(|e| {
                IngestError::Io(format!(
                    "Failed to open source file '{}': {}",
                    source_path.display(),
                    e
                ))
            })?;

            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; self.chunk_size];
            loop {
                if let Some(ref token) = self.cancellation_token {
                    if token.is_cancelled() {
                        return Err(IngestError::Io("Ingest cancelled by user".to_string()));
                    }
                }

                let n = source_file.read(&mut buf).await.map_err(|e| {
                    IngestError::Io(format!(
                        "Source read error during deduplication check on '{}': {}",
                        source_path.display(),
                        e
                    ))
                })?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }

            let source_hash = hex::encode(hasher.finalize());
            if source_hash == dest_hash {
                // Genuine duplicate: matching size and SHA-256 hash. 0 disk writes performed!
                return Ok((source_hash, true));
            }
            // If hashes differ (corrupt destination or collision), fall through to atomic overwrite
        }

        // Open source file in STRICT READ-ONLY mode
        let mut source_file = tokio::fs::File::open(source_path).await.map_err(|e| {
            IngestError::Io(format!(
                "Failed to open source file '{}': {}",
                source_path.display(),
                e
            ))
        })?;

        // Create temporary destination file in target directory for atomic write
        let pid = std::process::id();
        let counter = TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp_filename = format!(".{}.tmp.{}.{}.{}", filename, pid, nanos, counter);
        let temp_path = dest_dir.join(&temp_filename);

        // Guard must be declared before temp_file so temp_file handle drops FIRST, closing Windows handle
        let mut guard = TempFileGuard::new(&temp_path);
        let mut temp_file = tokio::fs::File::create(&temp_path).await.map_err(|e| {
            IngestError::DestinationError(format!(
                "Failed to create temporary destination file '{}': {}",
                temp_path.display(),
                e
            ))
        })?;

        let mut hasher = Sha256::new();
        let mut chunk_buf = vec![0u8; self.chunk_size];
        let mut file_bytes_transferred = 0u64;

        loop {
            // Check cancellation per chunk
            if let Some(ref token) = self.cancellation_token {
                if token.is_cancelled() {
                    drop(temp_file);
                    return Err(IngestError::Io("Ingest cancelled by user".to_string()));
                }
            }

            let n = source_file.read(&mut chunk_buf).await.map_err(|e| {
                IngestError::Io(format!(
                    "Source streaming read error on '{}': {}",
                    source_path.display(),
                    e
                ))
            })?;

            if n == 0 {
                break;
            }

            hasher.update(&chunk_buf[..n]);
            temp_file
                .write_all(&chunk_buf[..n])
                .await
                .map_err(|e| {
                    IngestError::DestinationError(format!(
                        "Destination write error on '{}': {}",
                        temp_path.display(),
                        e
                    ))
                })?;

            file_bytes_transferred += n as u64;

            if let Some(ref tx) = progress_tx {
                let _ = tx
                    .send(IngestProgress {
                        current_file: filename.to_string(),
                        current_file_index: file_index,
                        total_files,
                        current_file_bytes: file_bytes_transferred,
                        current_file_total_bytes: source_size,
                        total_bytes_transferred: accumulated_bytes + file_bytes_transferred,
                        total_expected_bytes,
                        skipped_duplicates,
                        status: IngestStatus::Transferring,
                    })
                    .await;
            }
        }

        // Flush and sync temp file
        temp_file.flush().await.map_err(|e| {
            IngestError::DestinationError(format!(
                "Flush error on '{}': {}",
                temp_path.display(),
                e
            ))
        })?;
        temp_file.sync_all().await.map_err(|e| {
            IngestError::DestinationError(format!(
                "Sync error on '{}': {}",
                temp_path.display(),
                e
            ))
        })?;

        // CRITICAL ON WINDOWS: Drop file handle explicitly before calling rename to prevent ERROR_SHARING_VIOLATION
        drop(temp_file);

        let source_sha256 = hex::encode(hasher.finalize());

        // Atomically rename temporary file over final destination file
        tokio::fs::rename(&temp_path, dest_path).await.map_err(|e| {
            IngestError::DestinationError(format!(
                "Failed to rename temp file '{}' to destination '{}': {}",
                temp_path.display(),
                dest_path.display(),
                e
            ))
        })?;

        guard.disarm();
        Ok((source_sha256, false))
    }
}

/// Compute SHA-256 of an existing local file asynchronously in 1 MB chunks.
pub async fn compute_file_sha256_async(path: &Path) -> Result<String, IngestError> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| {
        IngestError::Io(format!(
            "Failed to open file for SHA-256 hashing at '{}': {}",
            path.display(),
            e
        ))
    })?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; DEFAULT_CHUNK_SIZE];
    loop {
        let n = file.read(&mut buf).await.map_err(|e| {
            IngestError::Io(format!(
                "Read error while hashing at '{}': {}",
                path.display(),
                e
            ))
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Compute SHA-256 of an existing local file synchronously.
pub fn compute_file_sha256(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; DEFAULT_CHUNK_SIZE];
    loop {
        let n = std::io::Read::read(&mut file, &mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::toast::MockToastNotifier;
    use tempfile::tempdir;

    #[test]
    fn test_cancellation_token() {
        let token = IngestCancellationToken::new();
        assert!(!token.is_cancelled());
        let token_clone = token.clone();
        token.cancel();
        assert!(token.is_cancelled());
        assert!(token_clone.is_cancelled());
    }

    #[test]
    fn test_temp_file_guard_drops_unarmed() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.tmp");
        std::fs::write(&path, b"temporary content").unwrap();
        assert!(path.exists());

        {
            let _guard = TempFileGuard::new(&path);
            // goes out of scope without disarm
        }

        assert!(!path.exists(), "TempFileGuard must delete unarmed temp file");
    }

    #[test]
    fn test_temp_file_guard_preserves_disarmed() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.tmp");
        std::fs::write(&path, b"permanent content").unwrap();

        {
            let mut guard = TempFileGuard::new(&path);
            guard.disarm();
        }

        assert!(path.exists(), "Disarmed TempFileGuard must NOT delete file");
    }

    #[tokio::test]
    async fn test_compute_file_sha256_async() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("payload.dat");
        let data = b"Hello, LightCraft streaming ingest sha256 verification!";
        tokio::fs::write(&path, data).await.unwrap();

        let async_hash = compute_file_sha256_async(&path).await.unwrap();
        let sync_hash = compute_file_sha256(&path).unwrap();

        let mut expected_hasher = Sha256::new();
        expected_hasher.update(data);
        let expected_hash = hex::encode(expected_hasher.finalize());

        assert_eq!(async_hash, expected_hash);
        assert_eq!(sync_hash, expected_hash);
    }

    #[tokio::test]
    async fn test_pipeline_basic_streaming_and_manifest() {
        let sd_dir = tempdir().unwrap();
        let camera_folder = sd_dir.path().join("DCIM").join("100CANON");
        std::fs::create_dir_all(&camera_folder).unwrap();

        let photo1 = camera_folder.join("IMG_0001.JPG");
        let payload1 = vec![0xABu8; 500_000]; // 500 KB
        std::fs::write(&photo1, &payload1).unwrap();

        let volume = SdCardVolume {
            drive_letter: 'E',
            volume_label: "CANON_SD".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: 64 * 1024 * 1024 * 1024,
            free_bytes: 60 * 1024 * 1024 * 1024,
            has_dcim: true,
            camera_folders: vec![camera_folder],
        };

        let target_dir = tempdir().unwrap();
        let profile = CameraProfile {
            camera_id: "CANON_R6_001".to_string(),
            display_name: "Canon EOS R6".to_string(),
            target_directory: target_dir.path().to_path_buf(),
            known_card_capacities: vec![64 * 1024 * 1024 * 1024],
            last_ingest_timestamp: None,
            total_photos_ingested: 0,
        };

        let toast = Arc::new(MockToastNotifier::new());
        let pipeline = IngestPipeline::new(Some(toast.clone())).with_chunk_size(64 * 1024);

        let (progress_tx, mut progress_rx) = mpsc::channel(100);
        let progress_task = tokio::spawn(async move {
            let mut events = Vec::new();
            while let Some(evt) = progress_rx.recv().await {
                events.push(evt);
            }
            events
        });

        let summary = pipeline
            .run_ingest(&volume, &profile, Some(progress_tx))
            .await
            .unwrap();

        assert_eq!(summary.transferred_files, 1);
        assert_eq!(summary.skipped_duplicates, 0);
        assert_eq!(summary.total_bytes, 500_000);
        assert!(summary.manifest_path.exists());

        // Check progress events
        let progress_events = progress_task.await.unwrap();
        assert!(!progress_events.is_empty());
        assert_eq!(progress_events.last().unwrap().status, IngestStatus::OverallCompleted);

        // Check toast notification
        assert_eq!(toast.count(), 1);
        let (title, message) = toast.last_notification().unwrap();
        assert!(title.contains("Canon EOS R6"));
        assert!(message.contains("Transferred: 1 files"));

        // Check manifest content
        let manifest_str = std::fs::read_to_string(&summary.manifest_path).unwrap();
        let manifest: IngestSessionManifest = serde_json::from_str(&manifest_str).unwrap();
        assert_eq!(manifest.camera_id, "CANON_R6_001");
        assert_eq!(manifest.total_transferred, 1);
        assert_eq!(manifest.total_skipped, 0);
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.files[0].file_size, 500_000);
        assert!(!manifest.files[0].skipped_duplicate);
    }

    #[tokio::test]
    async fn test_pipeline_deduplication_skips_identical_files() {
        let sd_dir = tempdir().unwrap();
        let camera_folder = sd_dir.path().join("DCIM").join("100CANON");
        std::fs::create_dir_all(&camera_folder).unwrap();

        let photo = camera_folder.join("IMG_0002.JPG");
        let payload = vec![0xCDu8; 200_000];
        std::fs::write(&photo, &payload).unwrap();

        let volume = SdCardVolume {
            drive_letter: 'E',
            volume_label: "CANON_SD".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: 64 * 1024 * 1024 * 1024,
            free_bytes: 60 * 1024 * 1024 * 1024,
            has_dcim: true,
            camera_folders: vec![camera_folder],
        };

        let target_dir = tempdir().unwrap();
        let profile = CameraProfile {
            camera_id: "CANON_R6_001".to_string(),
            display_name: "Canon EOS R6".to_string(),
            target_directory: target_dir.path().to_path_buf(),
            known_card_capacities: vec![64 * 1024 * 1024 * 1024],
            last_ingest_timestamp: None,
            total_photos_ingested: 0,
        };

        let toast = Arc::new(MockToastNotifier::new());
        let pipeline = IngestPipeline::new(Some(toast.clone()));

        // Run 1: transfers file
        let summary1 = pipeline.run_ingest_simple(&volume, &profile).await.unwrap();
        assert_eq!(summary1.transferred_files, 1);
        assert_eq!(summary1.skipped_duplicates, 0);

        // Run 2: skips identical duplicate
        let summary2 = pipeline.run_ingest_simple(&volume, &profile).await.unwrap();
        assert_eq!(summary2.transferred_files, 0);
        assert_eq!(summary2.skipped_duplicates, 1);
        assert_eq!(summary2.total_bytes, 0); // 0 bytes transferred for duplicates

        // Manifest must reflect duplicate
        let manifest_str = std::fs::read_to_string(&summary2.manifest_path).unwrap();
        let manifest: IngestSessionManifest = serde_json::from_str(&manifest_str).unwrap();
        assert_eq!(manifest.total_transferred, 0);
        assert_eq!(manifest.total_skipped, 1);
        assert!(manifest.files[0].skipped_duplicate);
    }

    #[tokio::test]
    async fn test_pipeline_cancellation() {
        let sd_dir = tempdir().unwrap();
        let camera_folder = sd_dir.path().join("DCIM").join("100CANON");
        std::fs::create_dir_all(&camera_folder).unwrap();

        let photo = camera_folder.join("IMG_LARGE.JPG");
        let payload = vec![0xFFu8; 1_000_000];
        std::fs::write(&photo, &payload).unwrap();

        let volume = SdCardVolume {
            drive_letter: 'E',
            volume_label: "CANON_SD".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: 64 * 1024 * 1024 * 1024,
            free_bytes: 60 * 1024 * 1024 * 1024,
            has_dcim: true,
            camera_folders: vec![camera_folder],
        };

        let target_dir = tempdir().unwrap();
        let profile = CameraProfile {
            camera_id: "CANON_R6_001".to_string(),
            display_name: "Canon EOS R6".to_string(),
            target_directory: target_dir.path().to_path_buf(),
            known_card_capacities: vec![64 * 1024 * 1024 * 1024],
            last_ingest_timestamp: None,
            total_photos_ingested: 0,
        };

        let cancel_token = IngestCancellationToken::new();
        cancel_token.cancel(); // Pre-cancel

        let pipeline = IngestPipeline::new(None)
            .with_cancellation_token(cancel_token);

        let res = pipeline.run_ingest_simple(&volume, &profile).await;
        assert!(res.is_err());
        match res.unwrap_err() {
            IngestError::Io(msg) => assert!(msg.contains("cancelled")),
            other => panic!("Unexpected error: {:?}", other),
        }
    }
}
