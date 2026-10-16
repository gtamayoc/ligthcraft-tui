#![allow(dead_code)]

//! Tier 1 Feature Integration Tests: Photographic Recipe Model, Intensity Scaling (0-150%),
//! Curated Aesthetic Presets, TOML/JSON Persistence, and LightCraft Develop Bridge.
//!
//! Covers Milestone M2 features:
//! - F-REC-01: Multi-Slider Photographic Recipe Model
//! - F-REC-02: Preset Intensity Scaling (0% to 150%)
//! - F-REC-03: Recipe & Preset Persistence (TOML & JSON)
//! - F-REC-04: Built-in Aesthetic Presets (Natural, Cinema, B&W, Teal & Orange, Golden Hour)
//! - F-REC-05: Native MCP Presets Bridge & Develop Translation
//! - F-SEC-01: Cryptographic Non-Destructive Source RAW Immutability

mod common;

#[path = "../src/error.rs"]
pub mod error;
#[path = "../src/mcp/mod.rs"]
pub mod mcp;
#[path = "../src/engine/mod.rs"]
pub mod engine;
#[path = "../src/storage/mod.rs"]
pub mod storage;

use chrono::Utc;
use tempfile::tempdir;

use common::TestFixture;
use engine::develop::{DevelopBridge, NativePresetRegistry, PresetApplicationStrategy};
use engine::presets::BuiltinPreset;
use engine::recipe::{DevelopSlider, Recipe, RecipeCategory, TrackKind};
use error::StorageError;
use mcp::tools::{DevelopSettings, McpPresetInfo};
use storage::presets_store::{validate_identifier, PresetFile, PresetFormat, PresetMetadata, PresetStore};

// ============================================================================
// Feature F-REC-01: Multi-Slider Photographic Recipe Model
// ============================================================================

#[test]
fn test_t1_rec01_01_nominal_recipe_construction() {
    let fixture = TestFixture::new("rec01_nominal_construction");

    let mut recipe = Recipe::default();
    recipe.id = "portrait-soft".to_string();
    recipe.name = "Portrait Soft Glow".to_string();
    recipe.category = RecipeCategory::Portrait;
    recipe.exposure = 0.35;
    recipe.contrast = -10.0;
    recipe.highlights = -15.0;
    recipe.shadows = 20.0;
    recipe.whites = 5.0;
    recipe.blacks = -5.0;
    recipe.temperature = Some(5800.0);
    recipe.tint = 3.0;
    recipe.vibrance = 12.0;
    recipe.saturation = -5.0;
    recipe.clarity = -8.0;
    recipe.dehaze = 0.0;
    recipe.sharpen_amount = 25.0;
    recipe.nr_luminance = 15.0;
    recipe.nr_color = 25.0;
    recipe.vignette = -5.0;

    assert_eq!(recipe.id, "portrait-soft");
    assert_eq!(recipe.name, "Portrait Soft Glow");
    assert_eq!(recipe.category, RecipeCategory::Portrait);
    assert_eq!(recipe.exposure, 0.35);
    assert_eq!(recipe.contrast, -10.0);
    assert_eq!(recipe.temperature, Some(5800.0));
    assert_eq!(recipe.sharpen_amount, 25.0);
    assert_eq!(recipe.sharpening(), 25.0);
    assert_eq!(recipe.luminance_nr(), 15.0);
    assert_eq!(recipe.color_nr(), 25.0);

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec01_02_develop_schema_mapping() {
    let fixture = TestFixture::new("rec01_schema_mapping");

    let mut recipe = Recipe::default();
    recipe.exposure = 0.50;
    recipe.contrast = 20.0;
    recipe.highlights = -30.0;
    recipe.shadows = 25.0;
    recipe.whites = 10.0;
    recipe.blacks = -15.0;
    recipe.tint = 4.0;
    recipe.vibrance = 15.0;
    recipe.saturation = 5.0;
    recipe.clarity = 12.0;
    recipe.dehaze = 8.0;
    recipe.sharpen_amount = 35.0;
    recipe.nr_luminance = 10.0;
    recipe.nr_color = 20.0;
    recipe.vignette = -12.0;
    recipe.temperature = Some(6200.0);

    let flat = recipe.evaluate_flat(1.0, None);

    assert_eq!(flat.get("light.exposure").copied(), Some(0.50));
    assert_eq!(flat.get("light.contrast").copied(), Some(20.0));
    assert_eq!(flat.get("light.highlights").copied(), Some(-30.0));
    assert_eq!(flat.get("light.shadows").copied(), Some(25.0));
    assert_eq!(flat.get("light.whites").copied(), Some(10.0));
    assert_eq!(flat.get("light.blacks").copied(), Some(-15.0));
    assert_eq!(flat.get("wb.temp").copied(), Some(6200.0));
    assert_eq!(flat.get("wb.tint").copied(), Some(4.0));
    assert_eq!(flat.get("color.vibrance").copied(), Some(15.0));
    assert_eq!(flat.get("color.saturation").copied(), Some(5.0));
    assert_eq!(flat.get("effects.clarity").copied(), Some(12.0));
    assert_eq!(flat.get("effects.dehaze").copied(), Some(8.0));
    assert_eq!(flat.get("detail.sharpenAmount").copied(), Some(35.0));
    assert_eq!(flat.get("detail.nrLuminance").copied(), Some(10.0));
    assert_eq!(flat.get("detail.nrColor").copied(), Some(20.0));
    assert_eq!(flat.get("vignette.amount").copied(), Some(-12.0));

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec01_03_white_balance_translation() {
    let fixture = TestFixture::new("rec01_wb_translation");

    // As-shot temperature preservation: temperature is None
    let as_shot = Recipe::default();
    assert!(as_shot.temperature.is_none());
    let flat_as_shot = as_shot.evaluate_flat(1.0, None);
    assert!(!flat_as_shot.contains_key("wb.temp"), "Preserved as-shot temperature must not inject wb.temp");

    // Explicit Kelvin bounds verification
    let mut extreme_warm = Recipe::default();
    extreme_warm.temperature = Some(55000.0);
    let flat_warm = extreme_warm.evaluate_flat(1.0, None);
    assert_eq!(flat_warm["wb.temp"], 50000.0, "Kelvin upper bound clamped at 50,000K");

    let mut extreme_cold = Recipe::default();
    extreme_cold.temperature = Some(1000.0);
    let flat_cold = extreme_cold.evaluate_flat(1.0, None);
    assert_eq!(flat_cold["wb.temp"], 2000.0, "Kelvin lower bound clamped at 2,000K");

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec01_04_effects_and_clarity_mapping() {
    let fixture = TestFixture::new("rec01_effects_mapping");

    let bounds_clarity = DevelopSlider::Clarity.bounds();
    assert_eq!(bounds_clarity.min, -100.0);
    assert_eq!(bounds_clarity.max, 100.0);
    assert_eq!(bounds_clarity.unit, "%");
    assert_eq!(bounds_clarity.track, TrackKind::Centered);

    let bounds_dehaze = DevelopSlider::Dehaze.bounds();
    assert_eq!(bounds_dehaze.min, -100.0);
    assert_eq!(bounds_dehaze.max, 100.0);

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec01_05_recipe_clone_and_mutation() {
    let fixture = TestFixture::new("rec01_clone_mutation");

    let original = Recipe {
        id: "orig".into(),
        name: "Original Recipe".into(),
        exposure: 0.25,
        contrast: 10.0,
        ..Default::default()
    };

    let mut clone = original.clone();
    clone.exposure = 1.0;
    clone.contrast = 45.0;

    assert_eq!(original.exposure, 0.25, "Original must remain completely untouched");
    assert_eq!(original.contrast, 10.0, "Original must remain completely untouched");
    assert_eq!(clone.exposure, 1.0);
    assert_eq!(clone.contrast, 45.0);

    fixture.assert_integrity();
}

// ============================================================================
// Feature F-REC-02: Preset Intensity Scaling (0% to 150%)
// ============================================================================

#[test]
fn test_t1_rec02_01_zero_neutral_intensity() {
    let fixture = TestFixture::new("rec02_zero_neutral");

    let mut recipe = Recipe::default();
    recipe.exposure = 2.50;
    recipe.contrast = 50.0;
    recipe.highlights = -60.0;
    recipe.shadows = 40.0;
    recipe.whites = 30.0;
    recipe.blacks = -35.0;
    recipe.tint = 25.0;
    recipe.vibrance = 45.0;
    recipe.saturation = -20.0;
    recipe.clarity = 30.0;
    recipe.dehaze = 15.0;
    recipe.sharpen_amount = 50.0;
    recipe.nr_luminance = 20.0;
    recipe.nr_color = 30.0;
    recipe.vignette = -25.0;
    recipe.temperature = Some(7500.0);
    recipe.treatment = Some("black_and_white".into());

    let scaled = recipe.apply_intensity(0.0);

    assert_eq!(scaled.exposure, 0.0);
    assert_eq!(scaled.contrast, 0.0);
    assert_eq!(scaled.highlights, 0.0);
    assert_eq!(scaled.shadows, 0.0);
    assert_eq!(scaled.whites, 0.0);
    assert_eq!(scaled.blacks, 0.0);
    assert_eq!(scaled.tint, 0.0);
    assert_eq!(scaled.vibrance, 0.0);
    assert_eq!(scaled.saturation, 0.0);
    assert_eq!(scaled.clarity, 0.0);
    assert_eq!(scaled.dehaze, 0.0);
    assert_eq!(scaled.sharpen_amount, 0.0);
    assert_eq!(scaled.nr_luminance, 0.0);
    assert_eq!(scaled.nr_color, 0.0);
    assert_eq!(scaled.vignette, 0.0);
    assert_eq!(scaled.temperature, Some(5500.0), "0% intensity evaluates temperature to 5500K baseline");
    assert_eq!(scaled.treatment, Some("color".into()), "0% intensity restores color baseline");

    let flat = recipe.evaluate_flat(0.0, Some(6500.0));
    assert_eq!(flat["light.exposure"], 0.0);
    assert_eq!(flat["light.contrast"], 0.0);
    assert_eq!(flat["wb.temp"], 6500.0);

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec02_02_nominal_100_intensity() {
    let fixture = TestFixture::new("rec02_nominal_100");

    let mut recipe = Recipe::default();
    recipe.exposure = -0.75;
    recipe.contrast = 25.0;
    recipe.highlights = -40.0;
    recipe.shadows = 30.0;
    recipe.whites = 15.0;
    recipe.blacks = -20.0;
    recipe.temperature = Some(6200.0);
    recipe.tint = -6.0;
    recipe.vibrance = 18.0;
    recipe.saturation = 8.0;
    recipe.clarity = 15.0;
    recipe.dehaze = 10.0;
    recipe.sharpen_amount = 30.0;
    recipe.nr_luminance = 12.0;
    recipe.nr_color = 25.0;
    recipe.vignette = -15.0;

    let scaled = recipe.apply_intensity(1.0);

    assert!((scaled.exposure - (-0.75)).abs() < 1e-6);
    assert!((scaled.contrast - 25.0).abs() < 1e-6);
    assert!((scaled.highlights - (-40.0)).abs() < 1e-6);
    assert!((scaled.shadows - 30.0).abs() < 1e-6);
    assert!((scaled.whites - 15.0).abs() < 1e-6);
    assert!((scaled.blacks - (-20.0)).abs() < 1e-6);
    assert!((scaled.temperature.unwrap() - 6200.0).abs() < 1e-6);
    assert!((scaled.tint - (-6.0)).abs() < 1e-6);
    assert!((scaled.vibrance - 18.0).abs() < 1e-6);
    assert!((scaled.saturation - 8.0).abs() < 1e-6);
    assert!((scaled.clarity - 15.0).abs() < 1e-6);
    assert!((scaled.dehaze - 10.0).abs() < 1e-6);
    assert!((scaled.sharpen_amount - 30.0).abs() < 1e-6);
    assert!((scaled.nr_luminance - 12.0).abs() < 1e-6);
    assert!((scaled.nr_color - 25.0).abs() < 1e-6);
    assert!((scaled.vignette - (-15.0)).abs() < 1e-6);

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec02_03_half_50_intensity() {
    let fixture = TestFixture::new("rec02_half_50");

    let mut recipe = Recipe::default();
    recipe.exposure = 1.0;
    recipe.contrast = 40.0;
    recipe.highlights = -50.0;
    recipe.sharpen_amount = 40.0;
    recipe.temperature = Some(6500.0); // delta = 1000K; at 50% delta = 500K -> 6000K

    let scaled = recipe.apply_intensity(0.5);

    assert_eq!(scaled.exposure, 0.5);
    assert_eq!(scaled.contrast, 20.0);
    assert_eq!(scaled.highlights, -25.0);
    assert_eq!(scaled.sharpen_amount, 20.0);
    assert_eq!(scaled.temperature, Some(6000.0));

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec02_04_amplified_150_clamped_intensity() {
    let fixture = TestFixture::new("rec02_amplified_150");

    let mut recipe = Recipe::default();
    recipe.contrast = 80.0;   // 80 * 1.5 = 120 -> clamped to 100.0
    recipe.exposure = -4.0;   // -4 * 1.5 = -6.0 -> clamped to -5.0
    recipe.highlights = 80.0; // 80 * 1.5 = 120 -> clamped to 100.0
    recipe.sharpen_amount = 120.0; // 120 * 1.5 = 180 -> clamped to 150.0
    recipe.temperature = Some(40000.0); // delta = 34500K; 5500 + 1.5*34500 = 57250 -> clamped to 50000.0

    let scaled = recipe.apply_intensity(1.5);

    assert_eq!(scaled.contrast, 100.0, "Contrast clamped at max 100.0");
    assert_eq!(scaled.exposure, -5.0, "Exposure clamped at min -5.0");
    assert_eq!(scaled.highlights, 100.0, "Highlights clamped at max 100.0");
    assert_eq!(scaled.sharpen_amount, 150.0, "Sharpening clamped at max 150.0");
    assert_eq!(scaled.temperature, Some(50000.0), "Temperature clamped at max 50,000K");

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec02_05_extreme_and_fractional_intensity() {
    let fixture = TestFixture::new("rec02_extreme_fractional");

    let mut recipe = Recipe::default();
    recipe.contrast = 20.0;

    // Fractional intensity 33.7% (0.337)
    let frac = recipe.apply_intensity(0.337);
    let expected = 20.0 * 0.337_f32;
    assert!((frac.contrast - expected).abs() < 1e-4);

    // Negative intensity: clamped to 0.0
    let neg = recipe.apply_intensity(-0.50);
    assert_eq!(neg.contrast, 0.0);

    // Excess intensity: clamped to 1.5
    let excess = recipe.apply_intensity(3.50);
    assert_eq!(excess.contrast, 30.0); // 20 * 1.5

    // NaN / Infinity guard: zero panic guarantee
    let nan_intensity = recipe.apply_intensity(f64::NAN);
    assert_eq!(nan_intensity.contrast, 20.0); // defaults to nominal 1.0

    let inf_intensity = recipe.apply_intensity(f64::INFINITY);
    assert_eq!(inf_intensity.contrast, 20.0);

    fixture.assert_integrity();
}

// ============================================================================
// Feature F-REC-03: Recipe Persistence (TOML & JSON)
// ============================================================================

#[test]
fn test_t1_rec03_01_toml_roundtrip_serialization() {
    let fixture = TestFixture::new("rec03_toml_roundtrip");

    let preset = PresetFile::new(
        PresetMetadata {
            id: "cinematic-dusk".into(),
            name: "Cinematic Dusk".into(),
            description: Some("Mood sunset cinema grade".into()),
            version: 1,
            author: Some("LightCraft Team".into()),
            tags: vec!["cinema".into(), "dusk".into()],
            created_at: Utc::now(),
            modified_at: Utc::now(),
            is_builtin: false,
            category: Some("Cinematic".into()),
            favorite: true,
            recommended_intensity: 100.0,
        },
        BuiltinPreset::Cinema.recipe(),
    );

    let serialized = PresetFormat::Toml.serialize(&preset).unwrap();
    assert!(serialized.contains("id = \"cinematic-dusk\""));
    assert!(serialized.contains("exposure = -0.2"));
    assert!(serialized.contains("contrast = 15.0"));

    let deserialized = PresetFormat::Toml.deserialize(&serialized, std::path::Path::new("dummy.toml")).unwrap();
    assert_eq!(deserialized.metadata.id, preset.metadata.id);
    assert_eq!(deserialized.metadata.name, preset.metadata.name);
    assert_eq!(deserialized.recipe.exposure, preset.recipe.exposure);
    assert_eq!(deserialized.recipe.contrast, preset.recipe.contrast);
    assert_eq!(deserialized.recipe.temperature, preset.recipe.temperature);

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec03_02_json_roundtrip_serialization() {
    let fixture = TestFixture::new("rec03_json_roundtrip");

    let preset = PresetFile::new(
        PresetMetadata {
            id: "teal-amber-style".into(),
            name: "Teal & Amber Style".into(),
            description: Some("Blockbuster color grade".into()),
            version: 1,
            author: Some("Author".into()),
            tags: vec!["style".into()],
            created_at: Utc::now(),
            modified_at: Utc::now(),
            is_builtin: false,
            category: Some("Style".into()),
            favorite: false,
            recommended_intensity: 100.0,
        },
        BuiltinPreset::TealOrange.recipe(),
    );

    let serialized = PresetFormat::Json.serialize(&preset).unwrap();
    assert!(serialized.contains("\"id\": \"teal-amber-style\""));
    assert!(serialized.contains("\"tint\": -10.0"));

    let deserialized = PresetFormat::Json.deserialize(&serialized, std::path::Path::new("dummy.json")).unwrap();
    assert_eq!(deserialized.metadata.id, preset.metadata.id);
    assert_eq!(deserialized.recipe.tint, preset.recipe.tint);

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec03_03_store_save_and_load_crud() {
    let fixture = TestFixture::new("rec03_store_crud");
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    let custom = PresetFile::new(
        PresetMetadata {
            id: "my-custom-recipe".into(),
            name: "My Custom Recipe".into(),
            description: Some("A unique look".into()),
            version: 1,
            author: Some("Photographer".into()),
            tags: vec!["custom".into()],
            created_at: Utc::now(),
            modified_at: Utc::now(),
            is_builtin: false,
            category: Some("Custom".into()),
            favorite: true,
            recommended_intensity: 90.0,
        },
        Recipe {
            id: "my-custom-recipe".into(),
            name: "My Custom Recipe".into(),
            exposure: 0.65,
            contrast: 18.0,
            clarity: 14.0,
            sharpen_amount: 30.0,
            ..Default::default()
        },
    );

    // Save
    let path = store.save_preset(&custom, PresetFormat::Toml).unwrap();
    assert!(path.is_file());

    // Load by ID
    let loaded = store.load_preset("my-custom-recipe").unwrap();
    assert_eq!(loaded.exposure, 0.65);
    assert_eq!(loaded.contrast, 18.0);

    // Load by Name
    let loaded_by_name = store.load_preset("My Custom Recipe").unwrap();
    assert_eq!(loaded_by_name.exposure, 0.65);

    // Load file with metadata
    let loaded_file = store.load_preset_file("my-custom-recipe").unwrap();
    assert_eq!(loaded_file.metadata.author.as_deref(), Some("Photographer"));
    assert_eq!(loaded_file.metadata.recommended_intensity, 90.0);

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec03_04_store_directory_enumeration() {
    let fixture = TestFixture::new("rec03_store_enum");
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    // 5 built-ins present initially
    let list1 = store.list_presets().unwrap();
    assert_eq!(list1.len(), 5);

    // Save 2 custom presets
    for idx in 1..=2 {
        let p = PresetFile::new(
            PresetMetadata {
                id: format!("custom-preset-{}", idx),
                name: format!("Custom Preset {}", idx),
                description: None,
                version: 1,
                author: None,
                tags: vec![],
                created_at: Utc::now(),
                modified_at: Utc::now(),
                is_builtin: false,
                category: None,
                favorite: false,
                recommended_intensity: 100.0,
            },
            Recipe::default(),
        );
        store.save_preset(&p, PresetFormat::Toml).unwrap();
    }

    let list2 = store.list_presets().unwrap();
    assert_eq!(list2.len(), 7);
    assert!(list2.iter().any(|p| p.id == "custom-preset-1"));
    assert!(list2.iter().any(|p| p.id == "custom-preset-2"));

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec03_05_store_delete_preset() {
    let fixture = TestFixture::new("rec03_store_delete");
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    // Built-ins cannot be deleted
    assert!(matches!(store.delete_preset("natural"), Err(StorageError::PermissionDenied(_))));
    assert!(matches!(store.delete_preset("cinema"), Err(StorageError::PermissionDenied(_))));

    // Save custom preset then delete it
    let p = PresetFile::new(
        PresetMetadata {
            id: "temp-preset".into(),
            name: "Temporary Preset".into(),
            description: None,
            version: 1,
            author: None,
            tags: vec![],
            created_at: Utc::now(),
            modified_at: Utc::now(),
            is_builtin: false,
            category: None,
            favorite: false,
            recommended_intensity: 100.0,
        },
        Recipe::default(),
    );
    let path = store.save_preset(&p, PresetFormat::Toml).unwrap();
    assert!(path.exists());

    store.delete_preset("temp-preset").unwrap();
    assert!(!path.exists());

    // Deleting again produces NotFound
    assert!(matches!(store.delete_preset("temp-preset"), Err(StorageError::NotFound(_))));

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec03_06_store_import_and_export() {
    let fixture = TestFixture::new("rec03_import_export");
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    let export_dir = tempdir().unwrap();

    // Export Cinema preset as JSON
    let exported_path = store.export_preset("cinema", export_dir.path(), PresetFormat::Json).unwrap();
    assert!(exported_path.is_file());
    assert_eq!(exported_path.extension().unwrap(), "json");

    // Create a new fresh store in another folder
    let store2_dir = tempdir().unwrap();
    let store2 = PresetStore::new(store2_dir.path().to_path_buf(), vec![]).unwrap();

    // Import the exported file
    let imported = store2.import_preset(&exported_path).unwrap();
    assert_eq!(imported.recipe.exposure, -0.20);
    assert!(!imported.metadata.is_builtin, "Imported presets must not claim built-in immunity");

    let loaded = store2.load_preset(&imported.metadata.id).unwrap();
    assert_eq!(loaded.exposure, -0.20);

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec03_07_store_atomic_write_and_path_traversal() {
    let fixture = TestFixture::new("rec03_security_and_traversal");
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    // Path traversal rejection
    assert!(matches!(store.load_preset("../windows/system32"), Err(StorageError::PathTraversal(_))));
    assert!(matches!(store.load_preset("sub/dir/preset"), Err(StorageError::PathTraversal(_))));
    assert!(matches!(store.load_preset("sub\\dir\\preset"), Err(StorageError::PathTraversal(_))));

    // Reserved Windows names
    assert!(matches!(validate_identifier("CON"), Err(StorageError::InvalidIdentifier(_))));
    assert!(matches!(validate_identifier("NUL"), Err(StorageError::InvalidIdentifier(_))));
    assert!(matches!(validate_identifier("AUX"), Err(StorageError::InvalidIdentifier(_))));
    assert!(matches!(validate_identifier("COM3"), Err(StorageError::InvalidIdentifier(_))));

    // Built-in overwrite rejection
    let natural_file = store.load_preset_file("natural").unwrap();
    assert!(matches!(store.save_preset(&natural_file, PresetFormat::Toml), Err(StorageError::PermissionDenied(_))));

    fixture.assert_integrity();
}

// ============================================================================
// Feature F-REC-04: Built-in Aesthetic Presets
// ============================================================================

#[test]
fn test_t1_rec04_01_natural_preset() {
    let fixture = TestFixture::new("rec04_natural");

    let r = BuiltinPreset::Natural.recipe();
    assert_eq!(r.exposure, 0.10);
    assert_eq!(r.contrast, 5.0);
    assert_eq!(r.highlights, -5.0);
    assert_eq!(r.shadows, 10.0);
    assert_eq!(r.whites, 0.0);
    assert_eq!(r.blacks, 0.0);
    assert_eq!(r.vibrance, 10.0);
    assert_eq!(r.saturation, 0.0);
    assert_eq!(r.sharpen_amount, 15.0);
    assert_eq!(r.treatment.as_deref(), Some("color"));
    assert_eq!(r.profile.as_deref(), Some("lc.color"));

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec04_02_cinema_preset() {
    let fixture = TestFixture::new("rec04_cinema");

    let r = BuiltinPreset::Cinema.recipe();
    assert_eq!(r.exposure, -0.20);
    assert_eq!(r.contrast, 15.0);
    assert_eq!(r.highlights, -25.0);
    assert_eq!(r.shadows, 20.0);
    assert_eq!(r.whites, -10.0);
    assert_eq!(r.blacks, 10.0);
    assert_eq!(r.temperature, Some(5700.0));
    assert_eq!(r.tint, -2.0);
    assert_eq!(r.saturation, -10.0);
    assert_eq!(r.vibrance, 15.0);
    assert_eq!(r.clarity, 10.0);
    assert_eq!(r.dehaze, 5.0);
    assert_eq!(r.sharpen_amount, 20.0);
    assert_eq!(r.vignette, -10.0);

    let grading = r.grading.as_ref().expect("Cinema must have color grading");
    assert_eq!(grading.shadows.hue, 215.0, "Teal shadows");
    assert_eq!(grading.highlights.hue, 42.0, "Warm golden highlights");

    let grain = r.grain.as_ref().expect("Cinema must have 35mm film grain");
    assert_eq!(grain.amount, 15.0);

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec04_03_bw_high_contrast_preset() {
    let fixture = TestFixture::new("rec04_bw_high_contrast");

    let r = BuiltinPreset::BwHighContrast.recipe();
    assert_eq!(r.treatment.as_deref(), Some("black_and_white"));
    assert_eq!(r.saturation, -100.0);
    assert_eq!(r.contrast, 25.0);
    assert_eq!(r.highlights, -35.0);
    assert_eq!(r.shadows, 15.0);
    assert_eq!(r.whites, 20.0);
    assert_eq!(r.blacks, -25.0);
    assert_eq!(r.clarity, 25.0);
    assert_eq!(r.dehaze, 10.0);
    assert_eq!(r.sharpen_amount, 25.0);
    assert_eq!(r.vignette, -15.0);

    let bw_mix = r.bw_mix.as_ref().expect("B&W must have panchromatic channel weights");
    assert_eq!(bw_mix.red, 25.0);
    assert_eq!(bw_mix.blue, -30.0, "Red filter / dark dramatic skies");

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec04_04_teal_orange_preset() {
    let fixture = TestFixture::new("rec04_teal_orange");

    let r = BuiltinPreset::TealOrange.recipe();
    assert_eq!(r.temperature, Some(6100.0));
    assert_eq!(r.tint, -10.0);
    assert_eq!(r.contrast, 15.0);
    assert_eq!(r.vibrance, 20.0);
    assert_eq!(r.saturation, 5.0);
    assert_eq!(r.clarity, 10.0);

    let grading = r.grading.as_ref().expect("Teal & Orange grading");
    assert_eq!(grading.shadows.hue, 195.0, "Cyan / Teal shadows");
    assert_eq!(grading.highlights.hue, 35.0, "Golden orange highlights");

    assert!(r.custom_sliders.contains_key("calibration.blueHue"));

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec04_05_golden_hour_preset() {
    let fixture = TestFixture::new("rec04_golden_hour");

    let r = BuiltinPreset::GoldenHour.recipe();
    assert_eq!(r.temperature, Some(6500.0), "+1000K warm shift");
    assert_eq!(r.tint, 5.0);
    assert_eq!(r.exposure, 0.25);
    assert_eq!(r.contrast, -5.0);
    assert_eq!(r.shadows, 20.0);
    assert_eq!(r.clarity, -5.0, "Bloom / soft atmospheric diffusion");
    assert_eq!(r.dehaze, -5.0);

    fixture.assert_integrity();
}

// ============================================================================
// Feature F-REC-05: Native MCP Presets Bridge & Develop Translation
// ============================================================================

#[test]
fn test_t1_rec05_01_native_registry_query() {
    let fixture = TestFixture::new("rec05_registry_query");

    let mut reg = NativePresetRegistry::default();
    assert!(reg.is_empty());

    let presets = vec![
        McpPresetInfo {
            id: "lc.natural".into(),
            name: "Natural".into(),
            group: "Color".into(),
            builtin: true,
            favorite: false,
        },
        McpPresetInfo {
            id: "lc.cinematic".into(),
            name: "Cinematic".into(),
            group: "Film".into(),
            builtin: true,
            favorite: true,
        },
        McpPresetInfo {
            id: "lc.bw-high-contrast".into(),
            name: "B&W High Contrast".into(),
            group: "B&W".into(),
            builtin: true,
            favorite: false,
        },
    ];

    reg.populate(presets);
    assert_eq!(reg.len(), 3);
    assert!(reg.is_native("lc.cinematic"));
    assert!(!reg.is_native("lc.nonexistent"));

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec05_02_native_preset_grouping() {
    let fixture = TestFixture::new("rec05_preset_grouping");

    let mut reg = NativePresetRegistry::default();
    reg.populate(vec![
        McpPresetInfo { id: "p1".into(), name: "P1".into(), group: "Color".into(), builtin: true, favorite: false },
        McpPresetInfo { id: "p2".into(), name: "P2".into(), group: "Color".into(), builtin: true, favorite: false },
        McpPresetInfo { id: "p3".into(), name: "P3".into(), group: "B&W".into(), builtin: true, favorite: false },
    ]);

    assert_eq!(reg.get_group("Color").len(), 2);
    assert_eq!(reg.get_group("B&W").len(), 1);
    assert_eq!(reg.get_group("Unknown").len(), 0);

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec05_03_native_preset_lookup_case_insensitive() {
    let fixture = TestFixture::new("rec05_preset_lookup");

    let mut reg = NativePresetRegistry::default();
    reg.populate(vec![
        McpPresetInfo {
            id: "lc.teal-orange".into(),
            name: "Teal & Orange".into(),
            group: "Color".into(),
            builtin: true,
            favorite: true,
        },
    ]);

    assert!(reg.find_by_name("teal & orange").is_some());
    assert!(reg.find_by_name("TEAL & ORANGE").is_some());
    assert_eq!(reg.find_by_name("Teal & Orange").unwrap().id, "lc.teal-orange");

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec05_04_develop_bridge_recipe_to_settings() {
    let fixture = TestFixture::new("rec05_develop_bridge");

    let mut base_settings = DevelopSettings::default();
    base_settings.light.exposure = 0.10;
    base_settings.light.contrast = 5.0;

    let recipe = BuiltinPreset::Cinema.recipe();

    // At 100% intensity: nominal addition clamped
    let merged = DevelopBridge::apply_recipe_to_settings(&base_settings, &recipe, 1.0);
    assert!((merged.light.exposure - (-0.10)).abs() < 1e-5); // 0.10 + (-0.20) = -0.10
    assert!((merged.light.contrast - 20.0).abs() < 1e-5);   // 5.0 + 15.0 = 20.0
    assert_eq!(merged.light.highlights, -25.0);

    // Extract recipe back from settings
    let extracted = DevelopBridge::settings_to_recipe(&merged, "Extracted Cinema");
    assert_eq!(extracted.name, "Extracted Cinema");
    assert_eq!(extracted.exposure, merged.light.exposure);
    assert_eq!(extracted.contrast, merged.light.contrast);

    fixture.assert_integrity();
}

#[test]
fn test_t1_rec05_05_preset_application_strategies() {
    let fixture = TestFixture::new("rec05_application_strategies");

    let strat1 = PresetApplicationStrategy::ServerNative {
        preset_id: "lc.cinematic".into(),
        amount_pct: 100.0,
    };
    match &strat1 {
        PresetApplicationStrategy::ServerNative { preset_id, amount_pct } => {
            assert_eq!(preset_id, "lc.cinematic");
            assert_eq!(*amount_pct, 100.0);
        }
        _ => panic!("Expected ServerNative"),
    }

    let strat2 = PresetApplicationStrategy::ClientRecipe {
        recipe: BuiltinPreset::Natural.recipe(),
        intensity: 0.75,
    };
    match &strat2 {
        PresetApplicationStrategy::ClientRecipe { recipe, intensity } => {
            assert_eq!(recipe.name, "Natural");
            assert_eq!(*intensity, 0.75);
        }
        _ => panic!("Expected ClientRecipe"),
    }

    let strat3 = PresetApplicationStrategy::HybridMerge {
        native_preset_id: "lc.cinematic".into(),
        native_amount: 100.0,
        recipe_overlay: Recipe { exposure: 0.20, ..Default::default() },
        overlay_intensity: 1.0,
    };
    match &strat3 {
        PresetApplicationStrategy::HybridMerge { native_preset_id, recipe_overlay, .. } => {
            assert_eq!(native_preset_id, "lc.cinematic");
            assert_eq!(recipe_overlay.exposure, 0.20);
        }
        _ => panic!("Expected HybridMerge"),
    }

    fixture.assert_integrity();
}

// ============================================================================
// Feature F-SEC-01: Cryptographic Non-Destructive Source RAW Immutability
// ============================================================================

#[test]
fn test_t1_sec01_raw_immutability_throughout_m2_operations() {
    let fixture = TestFixture::new("sec01_raw_immutability_m2");

    // Perform intensive M2 recipe scaling, persistence, and conversion operations
    let presets = BuiltinPreset::all();
    for p in presets {
        let r = p.recipe();
        for &intensity in &[0.0, 0.25, 0.50, 0.75, 1.0, 1.25, 1.50] {
            let _scaled = r.apply_intensity(intensity);
            let _flat = r.evaluate_flat(intensity, None);
        }
    }

    // Explicit cryptographic SHA-256 baseline assertion
    fixture.assert_integrity();
}
