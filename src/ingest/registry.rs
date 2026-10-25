//! Local camera registry and atomic TOML persistence.
//!
//! Authoritative requirements:
//! - US-004 & FR-5: Local camera registry stored at `~/.lightcraft/camera_registry.toml`.
//! - Atomic persistence: Temporary file in target directory + fsync + atomic rename.
//! - In-memory `HashMap<String, CameraProfile>` for O(1) lookups.
//! - Resilient corrupt-file handling: auto-backup to `.corrupt.bak` if invalid TOML.
//! - Automatic parent directory creation (`fs::create_dir_all`).
//! - 100% Read-Only SD Protocol: strictly zero writes to SD media.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use super::empty_handler::matches_capacity;
use super::IngestError;

static TMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Persistent camera profile representing a known camera body and its target folder binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraProfile {
    /// Primary key: BodySerialNumber from EXIF (tag 0xa431) or composite Make_Model.
    pub camera_id: String,

    /// Human-readable camera label (e.g. "Canon EOS R6 Mark II").
    pub display_name: String,

    /// Target photo destination directory (e.g. "D:\Photos\Canon_R6").
    pub target_directory: PathBuf,

    /// Known SD card capacities in bytes associated with this camera.
    #[serde(default)]
    pub known_card_capacities: Vec<u64>,

    /// Timestamp of the last successful ingest session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ingest_timestamp: Option<DateTime<Utc>>,

    /// Total number of photos successfully ingested for this camera profile.
    #[serde(default)]
    pub total_photos_ingested: u64,
}

impl CameraProfile {
    /// Create a new CameraProfile with empty metrics.
    pub fn new(
        camera_id: impl Into<String>,
        display_name: impl Into<String>,
        target_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            camera_id: camera_id.into(),
            display_name: display_name.into(),
            target_directory: target_directory.into(),
            known_card_capacities: Vec::new(),
            last_ingest_timestamp: None,
            total_photos_ingested: 0,
        }
    }

    /// Builder method to attach an initial card capacity.
    pub fn with_capacity(mut self, capacity: u64) -> Self {
        self.add_card_capacity(capacity);
        self
    }

    /// Add a known card capacity if not already recorded.
    pub fn add_card_capacity(&mut self, capacity: u64) {
        if capacity > 0 && !self.known_card_capacities.contains(&capacity) {
            self.known_card_capacities.push(capacity);
        }
    }

    /// Record completed ingest metrics in-memory.
    pub fn record_ingest(&mut self, photos_count: u64) {
        self.total_photos_ingested = self.total_photos_ingested.saturating_add(photos_count);
        self.last_ingest_timestamp = Some(Utc::now());
    }
}

/// Camera registry holding in-memory profiles for O(1) lookups and atomic disk persistence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraRegistry {
    /// In-memory table indexed by camera_id primary key for O(1) lookups.
    #[serde(rename = "cameras", alias = "profiles", default)]
    pub profiles: HashMap<String, CameraProfile>,

    /// Path to the registry file on disk (skipped during serialization).
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

impl Default for CameraRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraRegistry {
    /// Create a new empty in-memory camera registry without associated path.
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
            path: None,
        }
    }

    /// Create an empty camera registry with a specified persistent file path.
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self {
            profiles: HashMap::new(),
            path: Some(path.into()),
        }
    }

    /// Resolve standard persistent registry path: `~/.lightcraft/camera_registry.toml`.
    pub fn default_path() -> Result<PathBuf, IngestError> {
        let home = dirs::home_dir().ok_or_else(|| {
            IngestError::RegistryError("Could not resolve user home directory".to_string())
        })?;
        Ok(home.join(".lightcraft").join("camera_registry.toml"))
    }

    /// Load the camera registry from standard `~/.lightcraft/camera_registry.toml`.
    pub fn load() -> Result<Self, IngestError> {
        let path = Self::default_path()?;
        Self::load_from_path(&path)
    }

    /// Load camera registry from a specific path. If file does not exist, returns clean empty registry.
    /// If file contains corrupt TOML, backs up to `.corrupt.bak` and returns `IngestError::RegistryError`.
    pub fn load_from_path(path: &Path) -> Result<Self, IngestError> {
        if !path.exists() {
            debug!("Registry file not found at '{}'; initializing empty", path.display());
            return Ok(Self {
                profiles: HashMap::new(),
                path: Some(path.to_path_buf()),
            });
        }

        let content = fs::read_to_string(path).map_err(|e| {
            IngestError::Io(format!("Failed to read registry at '{}': {}", path.display(), e))
        })?;

        match parse_registry_content(&content) {
            Ok(profiles) => {
                debug!("Loaded {} camera profiles from '{}'", profiles.len(), path.display());
                Ok(Self {
                    profiles,
                    path: Some(path.to_path_buf()),
                })
            }
            Err(err) => {
                let backup_path = backup_corrupt_file(path);
                warn!(
                    "Corrupted camera registry at '{}' backed up to '{}': {}",
                    path.display(),
                    backup_path.display(),
                    err
                );
                Err(IngestError::RegistryError(format!(
                    "Corrupt TOML at '{}' backed up to '{}': {}",
                    path.display(),
                    backup_path.display(),
                    err
                )))
            }
        }
    }

    /// Reload the registry in-place from its configured path (or standard path).
    pub fn reload(&mut self) -> Result<(), IngestError> {
        let path = match self.path {
            Some(ref p) => p.clone(),
            None => Self::default_path()?,
        };

        let loaded = Self::load_from_path(&path)?;
        self.profiles = loaded.profiles;
        self.path = Some(path);
        Ok(())
    }

    /// Save the camera registry to its configured path (or standard path) using atomic write + rename.
    pub fn save(&self) -> Result<(), IngestError> {
        let path = match self.path {
            Some(ref p) => p.clone(),
            None => Self::default_path()?,
        };
        self.save_to_path(&path)
    }

    /// Atomically persist registry to target path: write to `.tmp` file, fsync, and atomic rename.
    pub fn save_to_path(&self, path: &Path) -> Result<(), IngestError> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|e| {
            IngestError::Io(format!(
                "Failed to create registry directory '{}': {}",
                parent.display(),
                e
            ))
        })?;

        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("camera_registry.toml");

        // Clean up legacy .tmp file if present to guarantee clean state
        let legacy_tmp = parent.join(format!("{}.tmp", file_name));
        if legacy_tmp.exists() {
            let _ = fs::remove_file(&legacy_tmp);
        }

        // Generate unique temporary path per save operation
        let pid = std::process::id();
        let counter = TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp_path = parent.join(format!("{}.tmp.{}.{}.{}", file_name, pid, nanos, counter));

        let serialized = toml::to_string_pretty(self).map_err(|e| {
            IngestError::RegistryError(format!("Failed to serialize camera registry: {}", e))
        })?;

        let write_op = (|| -> Result<(), IngestError> {
            let mut file = File::create(&tmp_path).map_err(|e| {
                IngestError::Io(format!(
                    "Failed to create temporary registry file '{}': {}",
                    tmp_path.display(),
                    e
                ))
            })?;

            file.write_all(serialized.as_bytes()).map_err(|e| {
                IngestError::Io(format!(
                    "Failed to write temporary registry file '{}': {}",
                    tmp_path.display(),
                    e
                ))
            })?;

            file.sync_all().map_err(|e| {
                IngestError::Io(format!(
                    "Failed to flush and sync temporary registry file '{}': {}",
                    tmp_path.display(),
                    e
                ))
            })?;

            // Drop file to release lock on Windows prior to rename
            drop(file);

            // Robust atomic replace on Windows with brief backoff retry
            let mut rename_res = fs::rename(&tmp_path, path);
            for attempt in 0..5 {
                if rename_res.is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2 * (attempt + 1)));
                rename_res = fs::rename(&tmp_path, path);
            }

            rename_res.map_err(|e| {
                IngestError::Io(format!(
                    "Failed to atomically rename registry file from '{}' to '{}': {}",
                    tmp_path.display(),
                    path.display(),
                    e
                ))
            })?;

            Ok(())
        })();

        if write_op.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }

        write_op
    }

    /// Retrieve profile by `camera_id` in O(1) time complexity.
    pub fn get_profile(&self, camera_id: &str) -> Option<&CameraProfile> {
        self.profiles.get(camera_id)
    }

    /// Retrieve mutable profile by `camera_id`.
    pub fn get_profile_mut(&mut self, camera_id: &str) -> Option<&mut CameraProfile> {
        self.profiles.get_mut(camera_id)
    }

    /// Check whether a camera ID is registered.
    pub fn contains(&self, camera_id: &str) -> bool {
        self.profiles.contains_key(camera_id)
    }

    /// Register a new camera profile or update an existing one, persisting immediately.
    pub fn register_or_update(&mut self, profile: CameraProfile) -> Result<(), IngestError> {
        self.profiles.insert(profile.camera_id.clone(), profile);
        self.save()
    }

    /// Record an ingest session for a camera and persist immediately to disk.
    pub fn record_ingest(&mut self, camera_id: &str, photos_count: u64) -> Result<(), IngestError> {
        let profile = self.profiles.get_mut(camera_id).ok_or_else(|| {
            IngestError::RegistryError(format!("Camera ID '{}' not found in registry", camera_id))
        })?;

        profile.record_ingest(photos_count);
        self.save()
    }

    /// Find a matching camera profile for an empty card by physical capacity within 2% tolerance.
    /// Prioritizes smallest absolute difference, then most recent ingest timestamp.
    pub fn find_by_capacity(&self, total_bytes: u64) -> Option<&CameraProfile> {
        if total_bytes == 0 || self.profiles.is_empty() {
            return None;
        }

        let mut matches: Vec<(&CameraProfile, u64)> = self
            .profiles
            .values()
            .filter_map(|p| {
                let min_diff = p
                    .known_card_capacities
                    .iter()
                    .filter(|&&c| matches_capacity(c, total_bytes))
                    .map(|&c| c.abs_diff(total_bytes))
                    .min();
                min_diff.map(|diff| (p, diff))
            })
            .collect();

        if matches.is_empty() {
            return None;
        }

        matches.sort_by(|(p1, diff1), (p2, diff2)| {
            diff1
                .cmp(diff2)
                .then_with(|| p2.last_ingest_timestamp.cmp(&p1.last_ingest_timestamp))
                .then_with(|| p1.camera_id.cmp(&p2.camera_id))
        });

        Some(matches[0].0)
    }

    /// Find all camera profiles whose registered capacities match the given card capacity.
    pub fn find_all_by_capacity(&self, total_bytes: u64) -> Vec<&CameraProfile> {
        if total_bytes == 0 || self.profiles.is_empty() {
            return Vec::new();
        }

        self.profiles
            .values()
            .filter(|p| {
                p.known_card_capacities
                    .iter()
                    .any(|&c| matches_capacity(c, total_bytes))
            })
            .collect()
    }

    /// Total count of registered camera profiles.
    pub fn count(&self) -> usize {
        self.profiles.len()
    }

    /// Returns true if the registry contains no registered profiles.
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// Remove a camera profile by ID and persist changes.
    pub fn remove_profile(&mut self, camera_id: &str) -> Result<Option<CameraProfile>, IngestError> {
        let removed = self.profiles.remove(camera_id);
        if removed.is_some() {
            self.save()?;
        }
        Ok(removed)
    }

    /// List all profiles ordered by most recent ingest timestamp descending.
    pub fn list_profiles(&self) -> Vec<&CameraProfile> {
        let mut list: Vec<&CameraProfile> = self.profiles.values().collect();
        list.sort_by(|a, b| {
            b.last_ingest_timestamp
                .cmp(&a.last_ingest_timestamp)
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        list
    }
}

/// Back up a corrupted TOML file to `<filename>.corrupt.bak`.
fn backup_corrupt_file(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("camera_registry.toml");
    let backup_path = parent.join(format!("{}.corrupt.bak", file_name));

    if let Err(e) = fs::copy(path, &backup_path) {
        warn!(
            "Failed to create backup of corrupt registry file from '{}' to '{}': {}",
            path.display(),
            backup_path.display(),
            e
        );
    } else {
        info!(
            "Backed up corrupt registry file from '{}' to '{}'",
            path.display(),
            backup_path.display()
        );
    }

    backup_path
}

/// Helper struct for deserializing top-level wrapped registry formats.
#[derive(Deserialize)]
struct WrappedRegistry {
    #[serde(alias = "profiles")]
    cameras: Option<HashMap<String, CameraProfile>>,
}

/// Parse TOML registry string supporting both `[cameras."..."]` / `[profiles."..."]` and flat root maps.
fn parse_registry_content(content: &str) -> Result<HashMap<String, CameraProfile>, toml::de::Error> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(HashMap::new());
    }

    // 1. Validate TOML syntax first. Any malformed syntax must return Err immediately
    // so that callers trigger corrupt-file recovery (.corrupt.bak).
    let toml_val = toml::from_str::<toml::Value>(trimmed)?;

    // 2. If the document has no keys (e.g. comment-only files), return clean empty map.
    if let Some(table) = toml_val.as_table() {
        if table.is_empty() {
            return Ok(HashMap::new());
        }
    }

    // 3. If explicit wrapped table `[cameras]` or `[profiles]` exists, deserialize as WrappedRegistry.
    let has_wrapped = toml_val.get("cameras").is_some() || toml_val.get("profiles").is_some();
    if has_wrapped {
        let wrapped = toml::from_str::<WrappedRegistry>(trimmed)?;
        return Ok(wrapped.cameras.unwrap_or_default());
    }

    // 4. Otherwise deserialize as flat root map: `["camera_id"] = { ... }`
    toml::from_str::<HashMap<String, CameraProfile>>(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_profile_new_and_methods() {
        let mut profile = CameraProfile::new(
            "212033001854",
            "Canon EOS R6 Mark II",
            r"C:\Photos\CanonR6",
        );
        assert_eq!(profile.camera_id, "212033001854");
        assert_eq!(profile.display_name, "Canon EOS R6 Mark II");
        assert_eq!(profile.target_directory, PathBuf::from(r"C:\Photos\CanonR6"));
        assert_eq!(profile.total_photos_ingested, 0);
        assert!(profile.last_ingest_timestamp.is_none());

        profile.add_card_capacity(128 * 1024 * 1024 * 1024);
        profile.add_card_capacity(128 * 1024 * 1024 * 1024); // Deduplication
        assert_eq!(profile.known_card_capacities.len(), 1);

        profile.record_ingest(25);
        assert_eq!(profile.total_photos_ingested, 25);
        assert!(profile.last_ingest_timestamp.is_some());
    }

    #[test]
    fn test_registry_save_and_reload_roundtrip() {
        let dir = tempdir().expect("tempdir");
        let toml_path = dir.path().join("camera_registry.toml");

        let mut reg = CameraRegistry::with_path(&toml_path);
        let profile = CameraProfile::new("CAM01", "Canon R6", r"C:\Photos\Canon")
            .with_capacity(128 * 1024 * 1024 * 1024);

        reg.register_or_update(profile).expect("register");
        assert_eq!(reg.count(), 1);

        let reloaded = CameraRegistry::load_from_path(&toml_path).expect("load");
        assert_eq!(reloaded.count(), 1);
        let p = reloaded.get_profile("CAM01").expect("profile CAM01");
        assert_eq!(p.display_name, "Canon R6");
        assert_eq!(p.known_card_capacities, vec![128 * 1024 * 1024 * 1024]);
    }

    #[test]
    fn test_atomic_persistence_leaves_no_tmp_file() {
        let dir = tempdir().expect("tempdir");
        let toml_path = dir.path().join("camera_registry.toml");
        let tmp_path = dir.path().join("camera_registry.toml.tmp");

        let reg = CameraRegistry::with_path(&toml_path);
        reg.save().expect("save");

        assert!(toml_path.exists(), "Target TOML must exist");
        assert!(!tmp_path.exists(), "Temp file must be cleanly renamed without leaving residue");
    }

    #[test]
    fn test_o1_lookup_performance_and_accuracy() {
        let dir = tempdir().expect("tempdir");
        let toml_path = dir.path().join("camera_registry.toml");

        let mut reg = CameraRegistry::with_path(&toml_path);
        for i in 1..=100 {
            let p = CameraProfile::new(
                format!("SERIAL_{:04}", i),
                format!("Camera {:04}", i),
                format!(r"C:\Photos\Cam{:04}", i),
            );
            reg.profiles.insert(p.camera_id.clone(), p);
        }
        reg.save().expect("save batch");

        assert_eq!(reg.count(), 100);
        let item = reg.get_profile("SERIAL_0042").expect("lookup O(1)");
        assert_eq!(item.display_name, "Camera 0042");
        assert!(reg.get_profile("NON_EXISTENT").is_none());
    }

    #[test]
    fn test_record_ingest_updates_metrics_and_saves() {
        let dir = tempdir().expect("tempdir");
        let toml_path = dir.path().join("camera_registry.toml");

        let mut reg = CameraRegistry::with_path(&toml_path);
        let p = CameraProfile::new("CAM_REC", "Sony A7IV", r"C:\Photos\Sony");
        reg.register_or_update(p).expect("register");

        reg.record_ingest("CAM_REC", 50).expect("record ingest");
        let updated = reg.get_profile("CAM_REC").expect("get CAM_REC");
        assert_eq!(updated.total_photos_ingested, 50);
        assert!(updated.last_ingest_timestamp.is_some());

        // Verify persisted to disk
        let disk_reg = CameraRegistry::load_from_path(&toml_path).expect("reload");
        let disk_p = disk_reg.get_profile("CAM_REC").expect("get disk profile");
        assert_eq!(disk_p.total_photos_ingested, 50);
    }

    #[test]
    fn test_find_by_capacity_matching_and_priority() {
        let capacity_128gb = 128 * 1024 * 1024 * 1024;
        let mut reg = CameraRegistry::new();

        let p1 = CameraProfile::new("CAM_OLD", "Older Camera", r"C:\Photos\Old")
            .with_capacity(capacity_128gb);
        let mut p2 = CameraProfile::new("CAM_NEW", "Newer Camera", r"C:\Photos\New")
            .with_capacity(capacity_128gb);
        p2.last_ingest_timestamp = Some(Utc::now());

        reg.profiles.insert(p1.camera_id.clone(), p1);
        reg.profiles.insert(p2.camera_id.clone(), p2);

        // Newer timestamp wins tie-break
        let matched = reg.find_by_capacity(capacity_128gb).expect("match");
        assert_eq!(matched.camera_id, "CAM_NEW");

        // 1.5% margin tolerance matches
        let formatted_bytes = (capacity_128gb as f64 * 0.985) as u64;
        assert!(reg.find_by_capacity(formatted_bytes).is_some());

        // 5% margin fails
        let out_of_bounds = (capacity_128gb as f64 * 0.95) as u64;
        assert!(reg.find_by_capacity(out_of_bounds).is_none());
    }

    #[test]
    fn test_corrupt_toml_creates_backup_and_returns_error() {
        let dir = tempdir().expect("tempdir");
        let toml_path = dir.path().join("camera_registry.toml");
        let corrupt_bak = dir.path().join("camera_registry.toml.corrupt.bak");

        fs::write(&toml_path, "invalid_toml = [[ broken syntax {{{{").expect("write corrupt");

        let res = CameraRegistry::load_from_path(&toml_path);
        assert!(res.is_err(), "Corrupted TOML must return error");
        match res.unwrap_err() {
            IngestError::RegistryError(_) => {}
            other => panic!("Expected RegistryError, got {:?}", other),
        }

        assert!(corrupt_bak.exists(), "Corrupt backup file must be created");
        let backed_up = fs::read_to_string(&corrupt_bak).expect("read backup");
        assert_eq!(backed_up, "invalid_toml = [[ broken syntax {{{{");
    }

    #[test]
    fn test_missing_or_empty_file_loads_clean_empty_registry() {
        let dir = tempdir().expect("tempdir");
        let missing_path = dir.path().join("non_existent.toml");

        // Missing file -> Ok(empty)
        let reg = CameraRegistry::load_from_path(&missing_path).expect("missing file is clean");
        assert_eq!(reg.count(), 0);

        // Empty file (0 bytes) -> Ok(empty)
        let empty_path = dir.path().join("empty.toml");
        File::create(&empty_path).expect("create empty");
        let empty_reg = CameraRegistry::load_from_path(&empty_path).expect("empty file is clean");
        assert_eq!(empty_reg.count(), 0);
    }

    #[test]
    fn test_parse_registry_corrupt_cameras_table_returns_error() {
        let dir = tempdir().expect("tempdir");
        let toml_path = dir.path().join("camera_registry.toml");
        let corrupt_bak = dir.path().join("camera_registry.toml.corrupt.bak");

        fs::write(&toml_path, "[cameras]\ninvalid = = = 999\n").expect("write corrupt");
        let res = CameraRegistry::load_from_path(&toml_path);
        assert!(res.is_err(), "Corrupt [cameras] table must return Err");
        assert!(corrupt_bak.exists(), "Corrupt file must be backed up to .corrupt.bak");
    }

    #[test]
    fn test_parse_registry_empty_cameras_with_leading_comments() {
        let dir = tempdir().expect("tempdir");
        let toml_path = dir.path().join("camera_registry.toml");

        let content = "# Authoritative Camera Registry\n# Maintained by LightCraft\n\n[cameras]\n";
        fs::write(&toml_path, content).expect("write commented");
        let res = CameraRegistry::load_from_path(&toml_path).expect("load commented");
        assert_eq!(res.count(), 0);
    }

    #[test]
    fn test_find_by_capacity_tertiary_tie_breaker_by_camera_id() {
        let capacity_128gb = 128 * 1024 * 1024 * 1024;
        let mut reg = CameraRegistry::new();

        let p_b = CameraProfile::new("CAM_B", "Camera B", r"C:\Photos\B")
            .with_capacity(capacity_128gb);
        let p_a = CameraProfile::new("CAM_A", "Camera A", r"C:\Photos\A")
            .with_capacity(capacity_128gb);

        reg.profiles.insert(p_b.camera_id.clone(), p_b);
        reg.profiles.insert(p_a.camera_id.clone(), p_a);

        // Both have diff == 0 and last_ingest_timestamp == None
        // Tertiary comparator must deterministically select "CAM_A" (alphabetical)
        let matched = reg.find_by_capacity(capacity_128gb).expect("match");
        assert_eq!(matched.camera_id, "CAM_A");
    }
}
