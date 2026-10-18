//! Implementation of disk storage, TOML/JSON serialization, and preset management.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

use crate::engine::presets::BuiltinPreset;
use crate::engine::recipe::Recipe;
use crate::error::{SerializationError, StorageError};

/// Metadata attributes associated with a photographic preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetMetadata {
    /// Unique machine identifier (slug format: `^[a-zA-Z0-9_-]{1,64}$`).
    pub id: String,

    /// Human-readable display name shown in the TUI (e.g., "B&W High Contrast").
    pub name: String,

    /// Optional descriptive summary of the aesthetic look and recommended usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Schema specification version (default: 1).
    #[serde(default = "default_version")]
    pub version: u32,

    /// Creator or organization attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,

    /// Categorization tags for filtering in the TUI library.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Timestamp when the preset was originally created (UTC ISO-8601).
    pub created_at: DateTime<Utc>,

    /// Timestamp when the preset was last modified (UTC ISO-8601).
    pub modified_at: DateTime<Utc>,

    /// Whether this preset is a factory/system built-in (read-only, cannot be deleted).
    #[serde(default)]
    pub is_builtin: bool,

    /// Broad aesthetic category (e.g., "Cinematic", "Monochrome", "Portrait").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,

    /// User favorite flag for rapid access.
    #[serde(default)]
    pub favorite: bool,

    /// Recommended default intensity scaling factor (0.0 to 150.0, default: 100.0).
    #[serde(default = "default_intensity")]
    pub recommended_intensity: f32,
}

fn default_version() -> u32 {
    1
}

fn default_intensity() -> f32 {
    100.0
}

/// Top-level persistent photographic preset container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetFile {
    /// Associated metadata describing the preset and its provenance.
    pub metadata: PresetMetadata,
    /// Photographic recipe adjustments.
    pub recipe: Recipe,
}

impl PresetFile {
    pub fn new(metadata: PresetMetadata, recipe: Recipe) -> Self {
        Self { metadata, recipe }
    }
}

impl From<BuiltinPreset> for PresetFile {
    fn from(p: BuiltinPreset) -> Self {
        let r = p.recipe();
        let meta = PresetMetadata {
            id: p.id().to_string(),
            name: p.display_name().to_string(),
            description: Some(p.description().to_string()),
            version: 1,
            author: Some("LightCraft Team".to_string()),
            tags: vec![p.id().to_string(), p.category_name().to_string()],
            created_at: Utc::now(),
            modified_at: Utc::now(),
            is_builtin: true,
            category: Some(p.category_name().to_string()),
            favorite: false,
            recommended_intensity: 100.0,
        };
        PresetFile::new(meta, r)
    }
}

/// Supported preset disk serialization formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresetFormat {
    /// TOML format (.toml) — Default for user-editable configuration files.
    Toml,
    /// JSON format (.json) — Standard interchange format.
    Json,
}

impl PresetFormat {
    /// File extension without leading dot.
    pub fn extension(&self) -> &'static str {
        match self {
            PresetFormat::Toml => "toml",
            PresetFormat::Json => "json",
        }
    }

    /// Detect format from file path extension.
    pub fn from_path(path: &Path) -> Result<Self, StorageError> {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .ok_or_else(|| StorageError::InvalidFormat {
                path: path.to_path_buf(),
                reason: "File has no extension; expected .toml or .json".to_string(),
            })?;

        match ext.as_str() {
            "toml" => Ok(PresetFormat::Toml),
            "json" => Ok(PresetFormat::Json),
            other => Err(StorageError::InvalidFormat {
                path: path.to_path_buf(),
                reason: format!(
                    "Unsupported preset extension '.{}'; expected .toml or .json",
                    other
                ),
            }),
        }
    }

    /// Serialize a preset file to a formatted string.
    pub fn serialize(&self, preset: &PresetFile) -> Result<String, StorageError> {
        match self {
            PresetFormat::Toml => toml::to_string_pretty(preset)
                .map_err(|e| StorageError::Serialization(SerializationError::TomlSer(e))),
            PresetFormat::Json => serde_json::to_string_pretty(preset)
                .map_err(|e| StorageError::Serialization(SerializationError::Json(e))),
        }
    }

    /// Deserialize a preset file from a formatted string.
    pub fn deserialize(
        &self,
        content: &str,
        source_path: &Path,
    ) -> Result<PresetFile, StorageError> {
        match self {
            PresetFormat::Toml => {
                toml::from_str(content).map_err(|e| StorageError::InvalidFormat {
                    path: source_path.to_path_buf(),
                    reason: format!("TOML deserialization error: {}", e),
                })
            }
            PresetFormat::Json => {
                serde_json::from_str(content).map_err(|e| StorageError::InvalidFormat {
                    path: source_path.to_path_buf(),
                    reason: format!("JSON deserialization error: {}", e),
                })
            }
        }
    }
}

/// Resolves the default persistent preset storage directory with graceful fallbacks.
pub fn resolve_default_preset_dir() -> PathBuf {
    // 1. Try %APPDATA%/lightcraft-tui/presets/ or ~/.config/lightcraft-tui/presets/
    if let Some(config_dir) = dirs::config_dir() {
        let dir = config_dir.join("lightcraft-tui").join("presets");
        if fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }

    // 2. Try %APPDATA%/lightcraft/presets/ or ~/.local/share/lightcraft/presets/
    if let Some(data_dir) = dirs::data_dir() {
        let dir = data_dir.join("lightcraft").join("presets");
        if fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }

    // 3. Fallback to local ./presets in the current working directory
    let local_dir = PathBuf::from("./presets");
    let _ = fs::create_dir_all(&local_dir);
    local_dir
}

/// Validates that an identifier contains only safe characters and cannot escape the storage root.
pub fn validate_identifier(id: &str) -> Result<(), StorageError> {
    if id.is_empty() || id.len() > 64 {
        return Err(StorageError::InvalidIdentifier(id.to_string()));
    }

    // Reject path separators or null bytes immediately
    if id.contains('/') || id.contains('\\') || id.contains('\0') || id.contains("..") {
        return Err(StorageError::PathTraversal(id.to_string()));
    }

    // Check whitelist: [a-zA-Z0-9_-]
    let is_valid = id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !is_valid {
        return Err(StorageError::InvalidIdentifier(id.to_string()));
    }

    // Check Windows reserved device names
    let upper = id.to_ascii_uppercase();
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    if RESERVED.contains(&upper.as_str()) {
        return Err(StorageError::InvalidIdentifier(format!(
            "'{}' is a reserved operating system device name",
            id
        )));
    }

    Ok(())
}

/// Validates that a lookup query does not contain path traversal characters.
pub fn validate_lookup_target(target: &str) -> Result<(), StorageError> {
    if target.is_empty() || target.len() > 128 {
        return Err(StorageError::InvalidIdentifier(target.to_string()));
    }

    // Reject path separators, null bytes, or directory navigation immediately
    if target.contains('/')
        || target.contains('\\')
        || target.contains('\0')
        || target.contains("..")
    {
        return Err(StorageError::PathTraversal(target.to_string()));
    }

    Ok(())
}

/// PresetStore manages persistent photographic presets on disk and provides access to built-in presets.
pub struct PresetStore {
    root_dir: PathBuf,
    builtin_presets: Vec<PresetFile>,
}

impl PresetStore {
    /// Returns default built-in preset files for all 5 core presets.
    pub fn default_builtins() -> Vec<PresetFile> {
        BuiltinPreset::all()
            .iter()
            .map(|&p| PresetFile::from(p))
            .collect()
    }

    /// Creates a PresetStore rooted at the given directory with specified built-in presets.
    pub fn new(root_dir: PathBuf, builtin_presets: Vec<PresetFile>) -> Result<Self, StorageError> {
        fs::create_dir_all(&root_dir).map_err(|e| StorageError::io(&root_dir, e))?;
        Ok(Self {
            root_dir,
            builtin_presets,
        })
    }

    /// Creates a PresetStore rooted at the given directory using the standard built-in presets.
    pub fn with_default_builtins(root_dir: PathBuf) -> Result<Self, StorageError> {
        Self::new(root_dir, Self::default_builtins())
    }

    /// Initializes a PresetStore using the default OS configuration directory.
    pub fn default_store(builtin_presets: Vec<PresetFile>) -> Result<Self, StorageError> {
        let dir = resolve_default_preset_dir();
        Self::new(dir, builtin_presets)
    }

    /// Returns the active root directory on disk.
    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    /// Lists all presets available in the system (merging built-in and disk presets).
    pub fn list_presets(&self) -> Result<Vec<PresetMetadata>, StorageError> {
        let mut results = Vec::new();

        // 1. Add built-in presets
        for preset in &self.builtin_presets {
            results.push(preset.metadata.clone());
        }

        // 2. Scan physical storage directory
        if self.root_dir.exists() {
            let entries =
                fs::read_dir(&self.root_dir).map_err(|e| StorageError::io(&self.root_dir, e))?;

            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Skipping unreadable directory entry: {}", err);
                        continue;
                    }
                };

                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                // Check extension
                let format = match PresetFormat::from_path(&path) {
                    Ok(f) => f,
                    Err(_) => continue, // Ignore non-preset files
                };

                let content = match fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(err) => {
                        warn!("Failed to read preset file '{}': {}", path.display(), err);
                        continue;
                    }
                };

                match format.deserialize(&content, &path) {
                    Ok(preset) => {
                        // Avoid duplicates if user saved with same ID as a built-in
                        if !results.iter().any(|m| m.id == preset.metadata.id) {
                            results.push(preset.metadata);
                        }
                    }
                    Err(err) => {
                        warn!(
                            "Skipping malformed preset file '{}': {}",
                            path.display(),
                            err
                        );
                    }
                }
            }
        }

        // 3. Sort: built-ins first, then alphabetical by name
        results.sort_by(|a, b| match (a.is_builtin, b.is_builtin) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        Ok(results)
    }

    /// Loads a photographic `Recipe` by ID or display name.
    pub fn load_preset(&self, id_or_name: &str) -> Result<Recipe, StorageError> {
        self.load_preset_file(id_or_name).map(|f| f.recipe)
    }

    /// Loads a full `PresetFile` by ID or display name.
    pub fn load_preset_file(&self, id_or_name: &str) -> Result<PresetFile, StorageError> {
        validate_lookup_target(id_or_name)?;

        // 1. Check in-memory built-in presets
        for preset in &self.builtin_presets {
            if preset.metadata.id.eq_ignore_ascii_case(id_or_name)
                || preset.metadata.name.eq_ignore_ascii_case(id_or_name)
            {
                return Ok(preset.clone());
            }
        }

        // 2. Check direct files on disk if it's a valid identifier slug
        if validate_identifier(id_or_name).is_ok() {
            let toml_path = self.root_dir.join(format!("{}.toml", id_or_name));
            if toml_path.is_file() {
                let content =
                    fs::read_to_string(&toml_path).map_err(|e| StorageError::io(&toml_path, e))?;
                return PresetFormat::Toml.deserialize(&content, &toml_path);
            }

            let json_path = self.root_dir.join(format!("{}.json", id_or_name));
            if json_path.is_file() {
                let content =
                    fs::read_to_string(&json_path).map_err(|e| StorageError::io(&json_path, e))?;
                return PresetFormat::Json.deserialize(&content, &json_path);
            }
        }

        // 3. Scan directory files to match by display name or id
        if self.root_dir.exists() {
            let entries =
                fs::read_dir(&self.root_dir).map_err(|e| StorageError::io(&self.root_dir, e))?;

            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(format) = PresetFormat::from_path(&path) {
                        if let Ok(content) = fs::read_to_string(&path) {
                            if let Ok(file) = format.deserialize(&content, &path) {
                                if file.metadata.name.eq_ignore_ascii_case(id_or_name)
                                    || file.metadata.id.eq_ignore_ascii_case(id_or_name)
                                {
                                    return Ok(file);
                                }
                            }
                        }
                    }
                }
            }
        }

        Err(StorageError::NotFound(id_or_name.to_string()))
    }

    /// Atomically saves a preset to disk using a temporary file rename.
    pub fn save_preset(
        &self,
        preset: &PresetFile,
        format: PresetFormat,
    ) -> Result<PathBuf, StorageError> {
        // Safety checks
        if preset.metadata.is_builtin {
            return Err(StorageError::PermissionDenied(
                "Cannot overwrite factory built-in preset".into(),
            ));
        }

        if self
            .builtin_presets
            .iter()
            .any(|b| b.metadata.id == preset.metadata.id)
        {
            return Err(StorageError::PermissionDenied(
                "Preset ID is reserved by a built-in system preset".into(),
            ));
        }

        validate_identifier(&preset.metadata.id)?;

        let target_path =
            self.root_dir
                .join(format!("{}.{}", preset.metadata.id, format.extension()));

        // Serialize payload
        let payload = format.serialize(preset)?;

        // Atomic write via NamedTempFile in the target directory
        let mut temp_file = tempfile::Builder::new()
            .prefix(".preset_tmp_")
            .tempfile_in(&self.root_dir)
            .map_err(|e| StorageError::io(&self.root_dir, e))?;

        temp_file
            .write_all(payload.as_bytes())
            .map_err(|e| StorageError::io(temp_file.path(), e))?;

        temp_file
            .as_file()
            .sync_all()
            .map_err(|e| StorageError::io(temp_file.path(), e))?;

        temp_file
            .persist(&target_path)
            .map_err(|e| StorageError::io(&target_path, e.error))?;

        debug!("Atomically saved preset to '{}'", target_path.display());
        Ok(target_path)
    }

    /// Deletes a preset from disk. Built-in presets are protected and cannot be deleted.
    pub fn delete_preset(&self, id_or_name: &str) -> Result<(), StorageError> {
        validate_lookup_target(id_or_name)?;

        // Protected check
        for b in &self.builtin_presets {
            if b.metadata.id.eq_ignore_ascii_case(id_or_name)
                || b.metadata.name.eq_ignore_ascii_case(id_or_name)
            {
                return Err(StorageError::PermissionDenied(format!(
                    "Preset '{}' is a built-in preset and cannot be deleted",
                    id_or_name
                )));
            }
        }

        // Check direct files if valid identifier
        if validate_identifier(id_or_name).is_ok() {
            let toml_path = self.root_dir.join(format!("{}.toml", id_or_name));
            if toml_path.is_file() {
                fs::remove_file(&toml_path).map_err(|e| StorageError::io(&toml_path, e))?;
                return Ok(());
            }

            let json_path = self.root_dir.join(format!("{}.json", id_or_name));
            if json_path.is_file() {
                fs::remove_file(&json_path).map_err(|e| StorageError::io(&json_path, e))?;
                return Ok(());
            }
        }

        // Scan files by display name or id
        if self.root_dir.exists() {
            let entries =
                fs::read_dir(&self.root_dir).map_err(|e| StorageError::io(&self.root_dir, e))?;

            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(format) = PresetFormat::from_path(&path) {
                        if let Ok(content) = fs::read_to_string(&path) {
                            if let Ok(file) = format.deserialize(&content, &path) {
                                if file.metadata.name.eq_ignore_ascii_case(id_or_name)
                                    || file.metadata.id.eq_ignore_ascii_case(id_or_name)
                                {
                                    fs::remove_file(&path)
                                        .map_err(|e| StorageError::io(&path, e))?;
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
            }
        }

        Err(StorageError::NotFound(id_or_name.to_string()))
    }

    /// Imports a preset file from an external location, validates and saves it in the store.
    pub fn import_preset(&self, source_path: &Path) -> Result<PresetFile, StorageError> {
        if !source_path.is_file() {
            return Err(StorageError::NotFound(source_path.display().to_string()));
        }

        let format = PresetFormat::from_path(source_path)?;
        let content =
            fs::read_to_string(source_path).map_err(|e| StorageError::io(source_path, e))?;

        let mut preset = format.deserialize(&content, source_path)?;

        // Normalization: imported presets are never built-in
        preset.metadata.is_builtin = false;
        if validate_identifier(&preset.metadata.id).is_err() {
            preset.metadata.id = preset
                .metadata
                .name
                .to_lowercase()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect::<String>();
            if preset.metadata.id.is_empty() {
                preset.metadata.id = format!("import-{}", Utc::now().timestamp());
            }
        }
        preset.metadata.modified_at = Utc::now();

        self.save_preset(&preset, format)?;
        Ok(preset)
    }

    /// Exports an existing preset to an external directory in requested format.
    pub fn export_preset(
        &self,
        id_or_name: &str,
        destination_dir: &Path,
        format: PresetFormat,
    ) -> Result<PathBuf, StorageError> {
        if !destination_dir.is_dir() {
            return Err(StorageError::NotFound(
                destination_dir.display().to_string(),
            ));
        }

        let preset = self.load_preset_file(id_or_name)?;
        let target_path =
            destination_dir.join(format!("{}.{}", preset.metadata.id, format.extension()));

        let payload = format.serialize(&preset)?;

        let mut temp_file = tempfile::Builder::new()
            .prefix(".preset_exp_")
            .tempfile_in(destination_dir)
            .map_err(|e| StorageError::io(destination_dir, e))?;

        temp_file
            .write_all(payload.as_bytes())
            .map_err(|e| StorageError::io(temp_file.path(), e))?;

        temp_file
            .as_file()
            .sync_all()
            .map_err(|e| StorageError::io(temp_file.path(), e))?;

        temp_file
            .persist(&target_path)
            .map_err(|e| StorageError::io(&target_path, e.error))?;

        Ok(target_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_validate_identifier_rules() {
        assert!(validate_identifier("cinema-warm").is_ok());
        assert!(validate_identifier("bw_high_contrast_2").is_ok());
        assert!(validate_identifier("Preset123").is_ok());

        // Traversal / special chars
        assert!(matches!(
            validate_identifier("../escape"),
            Err(StorageError::PathTraversal(_))
        ));
        assert!(matches!(
            validate_identifier("foo/bar"),
            Err(StorageError::PathTraversal(_))
        ));
        assert!(matches!(
            validate_identifier("foo\\bar"),
            Err(StorageError::PathTraversal(_))
        ));

        // Reserved Windows device names
        assert!(matches!(
            validate_identifier("CON"),
            Err(StorageError::InvalidIdentifier(_))
        ));
        assert!(matches!(
            validate_identifier("nul"),
            Err(StorageError::InvalidIdentifier(_))
        ));
        assert!(matches!(
            validate_identifier("COM1"),
            Err(StorageError::InvalidIdentifier(_))
        ));

        // Length
        assert!(matches!(
            validate_identifier(""),
            Err(StorageError::InvalidIdentifier(_))
        ));
        assert!(matches!(
            validate_identifier(&"a".repeat(65)),
            Err(StorageError::InvalidIdentifier(_))
        ));
    }

    #[test]
    fn test_preset_store_crud_lifecycle() {
        let tmp = tempdir().unwrap();
        let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

        // 1. List includes built-ins
        let list = store.list_presets().unwrap();
        assert_eq!(list.len(), 5);
        assert!(list.iter().any(|p| p.id == "natural"));

        // 2. Load built-in
        let natural = store.load_preset("natural").unwrap();
        assert_eq!(natural.exposure, 0.10);

        // 3. Built-in cannot be deleted or overwritten
        assert!(store.delete_preset("natural").is_err());
        let builtin_file = store.load_preset_file("natural").unwrap();
        assert!(store
            .save_preset(&builtin_file, PresetFormat::Toml)
            .is_err());

        // 4. Save custom preset
        let custom_meta = PresetMetadata {
            id: "custom-look".into(),
            name: "Custom Look".into(),
            description: Some("My custom look".into()),
            version: 1,
            author: Some("Photographer".into()),
            tags: vec!["custom".into()],
            created_at: Utc::now(),
            modified_at: Utc::now(),
            is_builtin: false,
            category: Some("Custom".into()),
            favorite: true,
            recommended_intensity: 100.0,
        };
        let mut custom_recipe = Recipe::default();
        custom_recipe.id = "custom-look".into();
        custom_recipe.name = "Custom Look".into();
        custom_recipe.exposure = 0.75;
        let custom_file = PresetFile::new(custom_meta, custom_recipe);

        let saved_path = store.save_preset(&custom_file, PresetFormat::Toml).unwrap();
        assert!(saved_path.exists());

        // 5. List now has 6 presets
        let list2 = store.list_presets().unwrap();
        assert_eq!(list2.len(), 6);

        // 6. Load custom preset
        let loaded = store.load_preset("custom-look").unwrap();
        assert_eq!(loaded.exposure, 0.75);

        // 7. Export custom preset
        let export_dir = tempdir().unwrap();
        let exported_path = store
            .export_preset("custom-look", export_dir.path(), PresetFormat::Json)
            .unwrap();
        assert!(exported_path.exists());

        // 8. Delete custom preset
        store.delete_preset("custom-look").unwrap();
        assert!(!saved_path.exists());
        assert_eq!(store.list_presets().unwrap().len(), 5);

        // 9. Re-import from exported JSON
        let imported = store.import_preset(&exported_path).unwrap();
        assert_eq!(imported.recipe.exposure, 0.75);
        assert_eq!(store.list_presets().unwrap().len(), 6);
    }
}
