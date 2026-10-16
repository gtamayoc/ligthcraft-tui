//! Photographic preset storage, persistence, and serialization engine.

pub mod presets_store;

pub use presets_store::{
    resolve_default_preset_dir, validate_identifier, validate_lookup_target, PresetFile,
    PresetFormat, PresetMetadata, PresetStore,
};

pub use crate::error::StorageError;
