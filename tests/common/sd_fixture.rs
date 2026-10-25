#![allow(dead_code)]

//! Synthetic SD card fixture generator for opaque-box testing.
//! Emulates Win32 removable volumes with exFAT/FAT32 filesystems, DCIM folder trees,
//! authentic EXIF media, and cryptographic immutability snapshots.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

use sha2::{Digest, Sha256};

use super::contracts::SdCardVolume;
use super::exif_gen::ExifPhotoBuilder;

/// Synthetic SD Card fixture backing a temporary directory structure.
pub struct SyntheticSdCard {
    pub dir: TempDir,
    pub drive_letter: char,
    pub volume_label: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

impl SyntheticSdCard {
    /// Create a new synthetic SD card with standard defaults (128 GB exFAT).
    pub fn new(volume_label: &str) -> io::Result<Self> {
        let dir = tempdir()?;
        let total_bytes = 128 * 1024 * 1024 * 1024; // 128 GB
        let free_bytes = 90 * 1024 * 1024 * 1024;  // 90 GB free

        // Standard DCIM folder
        let dcim = dir.path().join("DCIM");
        fs::create_dir_all(&dcim)?;

        Ok(Self {
            dir,
            drive_letter: 'E',
            volume_label: volume_label.to_string(),
            file_system: "exFAT".to_string(),
            total_bytes,
            free_bytes,
        })
    }

    /// Create a formatted empty card (free space within 99% margin, empty DCIM).
    pub fn new_empty_formatted(capacity_bytes: u64) -> io::Result<Self> {
        let dir = tempdir()?;
        let dcim = dir.path().join("DCIM");
        fs::create_dir_all(&dcim)?;

        // Formatted filesystem has slight overhead (e.g. 99.8% free)
        let free_bytes = (capacity_bytes as f64 * 0.998) as u64;

        Ok(Self {
            dir,
            drive_letter: 'F',
            volume_label: "EOS_DIGITAL".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: capacity_bytes,
            free_bytes,
        })
    }

    /// Path to root mount of the synthetic SD card.
    pub fn root_path(&self) -> &Path {
        self.dir.path()
    }

    /// Path to DCIM folder.
    pub fn dcim_path(&self) -> PathBuf {
        self.dir.path().join("DCIM")
    }

    /// Create a camera subfolder in DCIM (e.g. "100CANON").
    pub fn add_camera_folder(&self, folder_name: &str) -> io::Result<PathBuf> {
        let folder = self.dcim_path().join(folder_name);
        fs::create_dir_all(&folder)?;
        Ok(folder)
    }

    /// Add a synthetic photo with EXIF metadata to a camera folder.
    pub fn add_photo(
        &self,
        folder_name: &str,
        file_name: &str,
        builder: &ExifPhotoBuilder,
    ) -> io::Result<PathBuf> {
        let folder = self.add_camera_folder(folder_name)?;
        let photo_path = folder.join(file_name);
        builder.write_to_file(&photo_path)?;
        Ok(photo_path)
    }

    /// Populate with N synthetic Canon photos across multiple days.
    pub fn populate_canon_photos(&self, count: usize) -> io::Result<Vec<PathBuf>> {
        let mut paths = Vec::with_capacity(count);
        for i in 1..=count {
            let day = (i % 28) + 1;
            let date_time = format!("2026:10:{:02} 12:{:02}:00", day, i % 60);
            let filename = format!("IMG_{:04}.CR3", i);
            let builder = ExifPhotoBuilder::new()
                .make("Canon")
                .model("Canon EOS R6")
                .body_serial(Some("042021000123"))
                .date_time(&date_time)
                .payload_size(2048)
                .as_raw();
            let path = self.add_photo("100CANON", &filename, &builder)?;
            paths.push(path);
        }
        Ok(paths)
    }

    /// Return `SdCardVolume` descriptor corresponding to this synthetic card.
    pub fn to_volume(&self) -> SdCardVolume {
        let mut camera_folders = Vec::new();
        let dcim = self.dcim_path();
        if let Ok(entries) = fs::read_dir(&dcim) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    camera_folders.push(entry.path());
                }
            }
        }

        SdCardVolume {
            drive_letter: self.drive_letter,
            volume_label: self.volume_label.clone(),
            file_system: self.file_system.clone(),
            total_bytes: self.total_bytes,
            free_bytes: self.free_bytes,
            has_dcim: dcim.exists(),
            camera_folders,
        }
    }

    /// Compute cryptographic SHA-256 tree of all files on the synthetic SD card.
    pub fn compute_sha256_tree(&self) -> io::Result<HashMap<PathBuf, String>> {
        let mut tree = HashMap::new();
        self.collect_hashes_recursive(self.root_path(), &mut tree)?;
        Ok(tree)
    }

    fn collect_hashes_recursive(
        &self,
        dir: &Path,
        tree: &mut HashMap<PathBuf, String>,
    ) -> io::Result<()> {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    self.collect_hashes_recursive(&path, tree)?;
                } else if path.is_file() {
                    let mut file = File::open(&path)?;
                    let mut hasher = Sha256::new();
                    let mut buf = [0u8; 8192];
                    loop {
                        let n = file.read(&mut buf)?;
                        if n == 0 {
                            break;
                        }
                        hasher.update(&buf[..n]);
                    }
                    let hash_hex = hex::encode(hasher.finalize());
                    tree.insert(path, hash_hex);
                }
            }
        }
        Ok(())
    }

    /// Invariant assertion: Verify that the SD card was 100% UNMODIFIED.
    /// Checks that file list and SHA-256 hashes are identical to the baseline snapshot.
    pub fn assert_unmodified(&self, baseline_tree: &HashMap<PathBuf, String>) {
        let current_tree = self
            .compute_sha256_tree()
            .expect("Failed to compute post-ingest SHA-256 tree");

        assert_eq!(
            baseline_tree.len(),
            current_tree.len(),
            "SD medium file count changed! Baseline had {} files, now has {}",
            baseline_tree.len(),
            current_tree.len()
        );

        for (path, expected_hash) in baseline_tree {
            let current_hash = current_tree.get(path).unwrap_or_else(|| {
                panic!("File {:?} was deleted or renamed from the SD card!", path)
            });
            assert_eq!(
                expected_hash, current_hash,
                "File {:?} was modified on the SD card! Read-only invariant violated!",
                path
            );
        }
    }

    /// Set simulated write-lock on all files and directories (read-only attribute).
    pub fn set_read_only(&self, read_only: bool) -> io::Result<()> {
        self.set_permissions_recursive(self.root_path(), read_only)
    }

    fn set_permissions_recursive(&self, path: &Path, read_only: bool) -> io::Result<()> {
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_readonly(read_only);
        fs::set_permissions(path, perms)?;

        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                self.set_permissions_recursive(&entry.path(), read_only)?;
            }
        }
        Ok(())
    }
}
