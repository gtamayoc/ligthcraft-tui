#![allow(dead_code)]

//! Mock camera registry fixture supporting atomic TOML persistence and O(1) in-memory indexing.
//! Conforms to M3 requirements (US-004, FR-5).

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

use super::contracts::{CameraProfile, IngestError};

/// Isolated Camera Registry fixture for testing.
pub struct IsolatedCameraRegistry {
    pub dir: TempDir,
    pub toml_path: PathBuf,
    profiles: HashMap<String, CameraProfile>,
}

impl IsolatedCameraRegistry {
    /// Create a new empty isolated camera registry in a temporary directory.
    pub fn new() -> Result<Self, IngestError> {
        let dir = tempdir()
            .map_err(|e| IngestError::Io(format!("Failed to create tempdir: {}", e)))?;
        let toml_path = dir.path().join("camera_registry.toml");

        Ok(Self {
            dir,
            toml_path,
            profiles: HashMap::new(),
        })
    }

    /// Retrieve profile by camera_id in O(1).
    pub fn get_profile(&self, camera_id: &str) -> Option<&CameraProfile> {
        self.profiles.get(camera_id)
    }

    /// Register new profile or update existing profile.
    pub fn register_or_update(&mut self, profile: CameraProfile) -> Result<(), IngestError> {
        self.profiles.insert(profile.camera_id.clone(), profile);
        self.save()
    }

    /// Find profile matching a card's physical capacity in bytes.
    pub fn find_by_capacity(&self, capacity_bytes: u64) -> Option<&CameraProfile> {
        self.profiles.values().find(|p| {
            p.known_card_capacities.iter().any(|&c| {
                // Allow exact match or standard within 2% margin due to FAT/exFAT variations
                let diff = if c > capacity_bytes {
                    c - capacity_bytes
                } else {
                    capacity_bytes - c
                };
                diff <= (c / 50) // within 2%
            })
        })
    }

    /// Number of registered camera profiles.
    pub fn count(&self) -> usize {
        self.profiles.len()
    }

    /// Save registry to disk using atomic write + rename pattern.
    pub fn save(&self) -> Result<(), IngestError> {
        let serialized = toml::to_string_pretty(&self.profiles)
            .map_err(|e| IngestError::RegistryError(format!("Serialization failed: {}", e)))?;

        let temp_path = self.dir.path().join("camera_registry.toml.tmp");
        {
            let mut file = File::create(&temp_path)
                .map_err(|e| IngestError::Io(format!("Failed to create temp TOML file: {}", e)))?;
            file.write_all(serialized.as_bytes())
                .map_err(|e| IngestError::Io(format!("Failed to write temp TOML: {}", e)))?;
            file.flush()
                .map_err(|e| IngestError::Io(format!("Failed to flush temp TOML: {}", e)))?;
        }

        // Atomic rename
        fs::rename(&temp_path, &self.toml_path)
            .map_err(|e| IngestError::Io(format!("Atomic rename failed: {}", e)))?;

        Ok(())
    }

    /// Reload registry state from disk.
    pub fn reload(&mut self) -> Result<(), IngestError> {
        if !self.toml_path.exists() {
            self.profiles.clear();
            return Ok(());
        }

        let content = fs::read_to_string(&self.toml_path)
            .map_err(|e| IngestError::Io(format!("Failed to read TOML: {}", e)))?;

        let map: HashMap<String, CameraProfile> = toml::from_str(&content)
            .map_err(|e| IngestError::RegistryError(format!("TOML deserialization failed: {}", e)))?;

        self.profiles = map;
        Ok(())
    }

    /// Seed the registry with a pre-configured profile for test setup.
    pub fn seed_profile(&mut self, profile: CameraProfile) -> Result<(), IngestError> {
        self.profiles.insert(profile.camera_id.clone(), profile);
        self.save()
    }
}
