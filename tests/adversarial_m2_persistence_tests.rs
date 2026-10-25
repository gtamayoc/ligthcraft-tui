#![allow(dead_code)]

//! Milestone M2 Adversarial Stress Test Suite: Preset Persistence, Atomic Writes, and Security.
//!
//! Empirically tests:
//! 1. Path traversal attacks: loading, saving, deleting with traversal paths and reserved names.
//! 2. Built-in preset protection: attempting to overwrite or delete factory built-in presets.
//! 3. Atomic write integrity: NamedTempFile atomicity, cleanup on failure, concurrency, no orphan files.
//! 4. Cryptographic immutability: Canon CR3 RAW source file SHA-256 verification before and after testing.

mod common;

#[path = "../src/engine/mod.rs"]
pub mod engine;
#[path = "../src/error.rs"]
pub mod error;
#[path = "../src/mcp/mod.rs"]
pub mod mcp;
#[path = "../src/storage/mod.rs"]
pub mod storage;

use chrono::Utc;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use tempfile::tempdir;

use common::cr3_integrity::{compute_file_sha256, find_draw_dir, CR3_BASELINE};
use engine::recipe::Recipe;
use error::StorageError;
use storage::presets_store::{
    validate_identifier, validate_lookup_target, PresetFile, PresetFormat, PresetMetadata,
    PresetStore,
};

/// Helper: creates a test PresetFile with a specified id, name, and recipe.
fn make_test_preset(id: &str, name: &str, is_builtin: bool) -> PresetFile {
    let meta = PresetMetadata {
        id: id.to_string(),
        name: name.to_string(),
        description: Some(format!("Test preset for {}", id)),
        version: 1,
        author: Some("Adversarial Harness".to_string()),
        tags: vec!["test".to_string(), "adversarial".to_string()],
        created_at: Utc::now(),
        modified_at: Utc::now(),
        is_builtin,
        category: Some("Test".to_string()),
        favorite: false,
        recommended_intensity: 100.0,
    };
    let mut recipe = Recipe::default();
    recipe.id = id.to_string();
    recipe.name = name.to_string();
    recipe.exposure = 0.42;
    recipe.contrast = 15.0;
    PresetFile::new(meta, recipe)
}

// ============================================================================
// 1. PATH TRAVERSAL ATTACKS & MALICIOUS IDENTIFIERS (MISSION ITEM 1)
// ============================================================================

#[test]
fn test_adv_sec_01_path_traversal_malicious_ids_validate_identifier() {
    let malicious_ids = [
        "../evil",
        "..\\evil",
        "foo/../../bar",
        "/etc/passwd",
        "C:\\Windows\\System32\\calc",
        "C:/Windows/System32/calc",
        "../../../../../../Windows/System32/drivers/etc/hosts",
        "..\\..\\..\\..\\Windows\\System32\\calc.exe",
        "/root/.ssh/id_rsa",
        "subdir/preset",
        "subdir\\preset",
        "..",
        ".",
        "../",
        "..\\",
        "evil\0name",
        "id/with/slash",
        "id\\with\\backslash",
        "foo/bar",
        "foo\\bar",
    ];

    for &malicious in &malicious_ids {
        let res = validate_identifier(malicious);
        assert!(
            res.is_err(),
            "validate_identifier MUST reject malicious ID '{}'",
            malicious
        );
        match res.unwrap_err() {
            StorageError::PathTraversal(path) => {
                assert!(
                    path.contains(malicious) || malicious.contains(&path),
                    "PathTraversal error should reference malicious path: {}",
                    malicious
                );
            }
            StorageError::InvalidIdentifier(_) => {
                // Also acceptable for identifiers violating format
            }
            other => panic!(
                "Expected PathTraversal or InvalidIdentifier for '{}', got {:?}",
                malicious, other
            ),
        }
    }
}

#[test]
fn test_adv_sec_02_path_traversal_malicious_ids_validate_lookup_target() {
    let malicious_lookup_targets = [
        "../evil",
        "..\\evil",
        "foo/../../bar",
        "/etc/passwd",
        "C:\\Windows\\System32\\calc",
        "C:/Windows/System32/calc",
        "../../../../../../Windows/System32/drivers/etc/hosts",
        "..\\..\\..\\..\\Windows\\System32\\calc.exe",
        "/root/.ssh/id_rsa",
        "subdir/preset",
        "subdir\\preset",
        "..",
        "../",
        "..\\",
        "target\0null",
    ];

    for &target in &malicious_lookup_targets {
        let res = validate_lookup_target(target);
        assert!(
            res.is_err(),
            "validate_lookup_target MUST reject malicious target '{}'",
            target
        );
        match res.unwrap_err() {
            StorageError::PathTraversal(_) => {
                // Expected
            }
            StorageError::InvalidIdentifier(_) => {
                // Also acceptable
            }
            other => panic!(
                "Expected PathTraversal or InvalidIdentifier for lookup target '{}', got {:?}",
                target, other
            ),
        }
    }
}

#[test]
fn test_adv_sec_03_store_save_rejects_path_traversal_ids() {
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    let malicious_ids = [
        "../evil",
        "..\\evil",
        "foo/../../bar",
        "/etc/passwd",
        "C:\\Windows\\System32\\calc",
        "CON",
        "NUL",
        "PRN",
        "AUX",
    ];

    for &id in &malicious_ids {
        let p = make_test_preset(id, &format!("Attack {}", id), false);

        // Attempt TOML save
        let toml_res = store.save_preset(&p, PresetFormat::Toml);
        assert!(
            toml_res.is_err(),
            "save_preset(Toml) MUST reject malicious ID '{}'",
            id
        );
        assert!(
            matches!(
                toml_res.unwrap_err(),
                StorageError::PathTraversal(_) | StorageError::InvalidIdentifier(_)
            ),
            "Expected PathTraversal or InvalidIdentifier on save for '{}'",
            id
        );

        // Attempt JSON save
        let json_res = store.save_preset(&p, PresetFormat::Json);
        assert!(
            json_res.is_err(),
            "save_preset(Json) MUST reject malicious ID '{}'",
            id
        );
        assert!(
            matches!(
                json_res.unwrap_err(),
                StorageError::PathTraversal(_) | StorageError::InvalidIdentifier(_)
            ),
            "Expected PathTraversal or InvalidIdentifier on save for '{}'",
            id
        );
    }
}

#[test]
fn test_adv_sec_04_store_load_and_delete_rejects_path_traversal_ids() {
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    let traversal_ids = [
        "../evil",
        "..\\evil",
        "foo/../../bar",
        "/etc/passwd",
        "C:\\Windows\\System32\\calc",
    ];

    for &id in &traversal_ids {
        // Attempt load_preset
        let load_res = store.load_preset(id);
        assert!(
            load_res.is_err(),
            "load_preset MUST reject traversal ID '{}'",
            id
        );
        assert!(
            matches!(load_res.unwrap_err(), StorageError::PathTraversal(_)),
            "load_preset('{}') must return PathTraversal",
            id
        );

        // Attempt load_preset_file
        let load_file_res = store.load_preset_file(id);
        assert!(
            load_file_res.is_err(),
            "load_preset_file MUST reject traversal ID '{}'",
            id
        );
        assert!(
            matches!(load_file_res.unwrap_err(), StorageError::PathTraversal(_)),
            "load_preset_file('{}') must return PathTraversal",
            id
        );

        // Attempt delete_preset
        let del_res = store.delete_preset(id);
        assert!(
            del_res.is_err(),
            "delete_preset MUST reject traversal ID '{}'",
            id
        );
        assert!(
            matches!(del_res.unwrap_err(), StorageError::PathTraversal(_)),
            "delete_preset('{}') must return PathTraversal",
            id
        );
    }
}

#[test]
fn test_adv_sec_05_windows_reserved_device_names() {
    let reserved_names = [
        "CON", "con", "Con", "PRN", "prn", "Prn", "AUX", "aux", "Aux", "NUL", "nul", "Nul", "COM1",
        "com1", "COM9", "com9", "LPT1", "lpt1", "LPT9", "lpt9",
    ];

    for &name in &reserved_names {
        let res = validate_identifier(name);
        assert!(
            res.is_err(),
            "validate_identifier MUST reject Windows reserved device name '{}'",
            name
        );
        assert!(
            matches!(res.unwrap_err(), StorageError::InvalidIdentifier(_)),
            "Expected InvalidIdentifier for reserved name '{}'",
            name
        );
    }
}

#[test]
fn test_adv_sec_06_reserved_device_names_store_load_and_delete_behavior() {
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    let reserved = ["CON", "NUL", "PRN", "AUX"];

    for &id in &reserved {
        // Saving must strictly return InvalidIdentifier
        let p = make_test_preset(id, id, false);
        let save_res = store.save_preset(&p, PresetFormat::Toml);
        assert!(
            matches!(save_res, Err(StorageError::InvalidIdentifier(_))),
            "Saving with reserved name '{}' must return InvalidIdentifier",
            id
        );

        // Test empirical behavior on load and delete
        let load_res = store.load_preset(id);
        assert!(load_res.is_err(), "Loading '{}' must fail", id);
        // Record empirical error type (either InvalidIdentifier, PathTraversal, or NotFound)
        match load_res.unwrap_err() {
            StorageError::InvalidIdentifier(_)
            | StorageError::PathTraversal(_)
            | StorageError::NotFound(_) => {
                // Safe: does not open device
            }
            other => panic!(
                "Unexpected error loading reserved device name '{}': {:?}",
                id, other
            ),
        }

        let del_res = store.delete_preset(id);
        assert!(del_res.is_err(), "Deleting '{}' must fail", id);
        match del_res.unwrap_err() {
            StorageError::InvalidIdentifier(_)
            | StorageError::PathTraversal(_)
            | StorageError::NotFound(_) => {
                // Safe: does not delete/lock device
            }
            other => panic!(
                "Unexpected error deleting reserved device name '{}': {:?}",
                id, other
            ),
        }
    }
}

#[test]
fn test_adv_sec_07_identifier_boundary_lengths_and_special_chars() {
    // Empty identifier
    assert!(matches!(
        validate_identifier(""),
        Err(StorageError::InvalidIdentifier(_))
    ));
    assert!(matches!(
        validate_lookup_target(""),
        Err(StorageError::InvalidIdentifier(_))
    ));

    // Exactly 64 chars -> valid
    let id_64 = "a".repeat(64);
    assert!(validate_identifier(&id_64).is_ok());

    // 65 chars -> exceeds 64 limit
    let id_65 = "a".repeat(65);
    assert!(matches!(
        validate_identifier(&id_65),
        Err(StorageError::InvalidIdentifier(_))
    ));

    // 128 chars lookup target -> valid
    let target_128 = "b".repeat(128);
    assert!(validate_lookup_target(&target_128).is_ok());

    // 129 chars lookup target -> exceeds 128 limit
    let target_129 = "b".repeat(129);
    assert!(matches!(
        validate_lookup_target(&target_129),
        Err(StorageError::InvalidIdentifier(_))
    ));

    // Special characters rejected by identifier whitelist
    let special_chars = [
        "preset name",
        "preset\tname",
        "preset\nname",
        "preset*test",
        "preset?test",
        "preset<test",
        "preset>test",
        "preset|test",
        "preset:test",
        "preset\"test",
        "preset;rm",
        "preset$var",
        "preset`cmd`",
        "preset&bg",
        "preset#hash",
        "preset@at",
        "prëset",
        "пресет",
        "✨preset",
    ];

    for &s in &special_chars {
        assert!(
            matches!(
                validate_identifier(s),
                Err(StorageError::InvalidIdentifier(_) | StorageError::PathTraversal(_))
            ),
            "validate_identifier must reject special characters in '{}'",
            s
        );
    }
}

// ============================================================================
// 2. BUILT-IN PRESET PROTECTION (MISSION ITEM 2)
// ============================================================================

#[test]
fn test_adv_builtin_01_all_five_builtins_cannot_be_overwritten() {
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    let builtins = [
        "cinema",
        "natural",
        "bw-high-contrast",
        "teal-orange",
        "golden-hour",
    ];

    for &id in &builtins {
        // Case A: PresetFile has is_builtin: true
        let mut loaded = store
            .load_preset_file(id)
            .expect("Builtin should be loadable");
        assert!(loaded.metadata.is_builtin);
        loaded.recipe.exposure = 4.99; // Attempt to tamper

        let save_res = store.save_preset(&loaded, PresetFormat::Toml);
        assert!(
            save_res.is_err(),
            "save_preset MUST NOT allow overwriting built-in '{}'",
            id
        );
        match save_res.unwrap_err() {
            StorageError::PermissionDenied(msg) => {
                assert!(
                    msg.to_lowercase().contains("built-in")
                        || msg.to_lowercase().contains("builtin")
                );
            }
            other => panic!("Expected PermissionDenied for '{}', got {:?}", id, other),
        }

        // Case B: PresetFile has is_builtin: false, but id is a built-in id
        let mut unflagged = loaded.clone();
        unflagged.metadata.is_builtin = false;
        let save_unflagged = store.save_preset(&unflagged, PresetFormat::Toml);
        assert!(
            save_unflagged.is_err(),
            "save_preset MUST reject reserved built-in ID '{}' even if is_builtin=false",
            id
        );
        assert!(
            matches!(
                save_unflagged.unwrap_err(),
                StorageError::PermissionDenied(_)
            ),
            "Expected PermissionDenied for built-in ID '{}'",
            id
        );

        // Case C: JSON format save attempt
        let save_json = store.save_preset(&unflagged, PresetFormat::Json);
        assert!(
            matches!(save_json.unwrap_err(), StorageError::PermissionDenied(_)),
            "Expected PermissionDenied for built-in ID '{}' under JSON format",
            id
        );
    }
}

#[test]
fn test_adv_builtin_02_all_five_builtins_cannot_be_deleted() {
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    let builtins = [
        "cinema",
        "natural",
        "bw-high-contrast",
        "teal-orange",
        "golden-hour",
    ];

    for &id in &builtins {
        // Delete by exact slug ID
        let del_res = store.delete_preset(id);
        assert!(
            del_res.is_err(),
            "delete_preset MUST reject deleting built-in ID '{}'",
            id
        );
        match del_res.unwrap_err() {
            StorageError::PermissionDenied(msg) => {
                assert!(msg.contains("built-in"));
            }
            other => panic!(
                "Expected PermissionDenied for delete '{}', got {:?}",
                id, other
            ),
        }

        // Delete by uppercase ID
        let upper_id = id.to_ascii_uppercase();
        let del_upper = store.delete_preset(&upper_id);
        assert!(
            matches!(del_upper.unwrap_err(), StorageError::PermissionDenied(_)),
            "delete_preset MUST reject uppercase built-in ID '{}'",
            upper_id
        );
    }

    // Delete by human-readable display names
    let display_names = [
        "Cinema",
        "Natural",
        "B&W High Contrast",
        "Teal & Orange",
        "Golden Hour",
    ];

    for &name in &display_names {
        let del_name = store.delete_preset(name);
        assert!(
            matches!(del_name.unwrap_err(), StorageError::PermissionDenied(_)),
            "delete_preset MUST reject deleting built-in by display name '{}'",
            name
        );
    }
}

#[test]
fn test_adv_builtin_03_builtins_remain_completely_unmodified() {
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    // Verify all 5 are present in list_presets
    let list = store.list_presets().unwrap();
    assert_eq!(list.len(), 5);
    for item in &list {
        assert!(item.is_builtin);
    }

    // Check recipe values match nominal standards
    let natural = store.load_preset("natural").unwrap();
    assert_eq!(natural.exposure, 0.10);
    assert_eq!(natural.contrast, 5.0);

    let cinema = store.load_preset("cinema").unwrap();
    assert_eq!(cinema.exposure, -0.20);
    assert_eq!(cinema.contrast, 15.0);

    let bw = store.load_preset("bw-high-contrast").unwrap();
    assert_eq!(bw.saturation, -100.0);
    assert_eq!(bw.contrast, 25.0);

    let to = store.load_preset("teal-orange").unwrap();
    assert_eq!(to.temperature, Some(6100.0));

    let gh = store.load_preset("golden-hour").unwrap();
    assert_eq!(gh.temperature, Some(6500.0));
    assert_eq!(gh.exposure, 0.25);
}

#[test]
fn test_adv_builtin_04_case_sensitivity_shadowing_empirical_check() {
    // Challenge test: What happens if an adversary attempts to save "Cinema" (capital C)?
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    let p = make_test_preset("Cinema", "Hacked Cinema", false);
    let save_res = store.save_preset(&p, PresetFormat::Toml);

    // Record empirical behavior: Does save_preset check case-insensitively against builtins?
    if save_res.is_ok() {
        // If it succeeded, verify whether "load_preset('cinema')" is shadowed or protected
        let loaded = store.load_preset("cinema").unwrap();
        assert_eq!(
            loaded.exposure, -0.20,
            "Built-in cinema preset must NEVER be shadowed by a disk file!"
        );
        let loaded_cap = store.load_preset("Cinema").unwrap();
        assert_eq!(
            loaded_cap.exposure, -0.20,
            "Built-in cinema must take precedence even for case variation 'Cinema'!"
        );
    }
}

#[test]
fn test_adv_builtin_05_case_insensitive_reservation_desync_finding() {
    // EMPIRICAL CHALLENGE FINDING (C-01):
    // Demonstrates that save_preset uses case-sensitive check against built-in IDs:
    // `self.builtin_presets.iter().any(|b| b.metadata.id == preset.metadata.id)`
    // On Windows or case-folding systems, saving "Cinema" creates "Cinema.toml" on disk.
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    let p = make_test_preset("Cinema", "Custom Look Cinema", false);
    let save_res = store.save_preset(&p, PresetFormat::Toml);

    // 1. Empirical verification: save_preset permits saving "Cinema"
    assert!(
        save_res.is_ok(),
        "save_preset permits case-variant ID 'Cinema' due to exact string comparison"
    );
    let saved_path = save_res.unwrap();
    assert!(saved_path.is_file());

    // 2. Empirical verification: list_presets lists both "cinema" and "Cinema"
    let list = store.list_presets().unwrap();
    assert_eq!(
        list.len(),
        6,
        "list_presets has 6 entries because deduplication is case-sensitive"
    );
    assert!(list.iter().any(|m| m.id == "cinema" && m.is_builtin));
    assert!(list.iter().any(|m| m.id == "Cinema" && !m.is_builtin));

    // 3. Empirical verification: loading "Cinema" returns the built-in, NOT the custom one!
    let loaded = store.load_preset("Cinema").unwrap();
    assert_eq!(
        loaded.exposure, -0.20,
        "load_preset('Cinema') falls back to built-in 'cinema' because load uses eq_ignore_ascii_case"
    );

    // 4. Empirical verification: user cannot delete "Cinema.toml" via delete_preset!
    let del_res = store.delete_preset("Cinema");
    assert!(
        matches!(del_res, Err(StorageError::PermissionDenied(_))),
        "delete_preset('Cinema') returns PermissionDenied because delete uses eq_ignore_ascii_case!"
    );
    // The file remains trapped on disk:
    assert!(
        saved_path.is_file(),
        "File 'Cinema.toml' remains trapped on disk"
    );
}

#[test]
fn test_adv_import_01_import_preset_validation() {
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    let ext_dir = tempdir().unwrap();

    // 1. Test importing file claiming is_builtin = true
    let spoof_file = ext_dir.path().join("spoof.toml");
    let spoof_preset = make_test_preset("spoof-look", "Spoofed Builtin", true);
    let spoof_payload = PresetFormat::Toml.serialize(&spoof_preset).unwrap();
    fs::write(&spoof_file, spoof_payload).unwrap();

    let imported = store.import_preset(&spoof_file).unwrap();
    assert!(
        !imported.metadata.is_builtin,
        "Imported preset MUST NOT retain is_builtin: true flag"
    );

    // 2. Test importing file with reserved built-in ID "natural"
    let natural_file = ext_dir.path().join("natural.toml");
    let natural_spoof = make_test_preset("natural", "Fake Natural", false);
    let natural_payload = PresetFormat::Toml.serialize(&natural_spoof).unwrap();
    fs::write(&natural_file, natural_payload).unwrap();

    let import_natural_res = store.import_preset(&natural_file);
    assert!(
        matches!(import_natural_res, Err(StorageError::PermissionDenied(_))),
        "Importing preset with reserved ID 'natural' must fail with PermissionDenied"
    );

    // 3. Test importing file with path traversal ID
    let traversal_file = ext_dir.path().join("traversal.json");
    let mut traversal_preset = make_test_preset("../../../evil_escape", "Clean Name", false);
    traversal_preset.metadata.id = "../../../evil_escape".to_string();
    let traversal_payload = PresetFormat::Json.serialize(&traversal_preset).unwrap();
    fs::write(&traversal_file, traversal_payload).unwrap();

    let imported_traversal = store.import_preset(&traversal_file).unwrap();
    assert!(
        !imported_traversal.metadata.id.contains(".."),
        "Imported preset ID must be sanitized against path traversal"
    );
    assert_eq!(imported_traversal.metadata.id, "cleanname");

    // 4. Test importing nonexistent file
    let nonexistent = ext_dir.path().join("missing.toml");
    assert!(matches!(
        store.import_preset(&nonexistent),
        Err(StorageError::NotFound(_))
    ));
}

#[test]
fn test_adv_export_01_export_preset_validation() {
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    let export_dir = tempdir().unwrap();

    // 1. Export built-in preset to JSON
    let exported = store
        .export_preset("golden-hour", export_dir.path(), PresetFormat::Json)
        .unwrap();
    assert!(exported.is_file());
    assert_eq!(exported.extension().unwrap(), "json");

    let content = fs::read_to_string(&exported).unwrap();
    let parsed: PresetFile = PresetFormat::Json.deserialize(&content, &exported).unwrap();
    assert_eq!(parsed.metadata.id, "golden-hour");
    assert_eq!(parsed.recipe.exposure, 0.25);

    // 2. Export nonexistent preset
    let nonexist_res = store.export_preset("nonexistent", export_dir.path(), PresetFormat::Toml);
    assert!(matches!(nonexist_res, Err(StorageError::NotFound(_))));

    // 3. Export to nonexistent directory
    let bad_dir = export_dir.path().join("does_not_exist");
    let bad_dir_res = store.export_preset("golden-hour", &bad_dir, PresetFormat::Toml);
    assert!(matches!(bad_dir_res, Err(StorageError::NotFound(_))));
}

// ============================================================================
// 3. ATOMIC WRITE INTEGRITY (MISSION ITEM 3)
// ============================================================================

#[test]
fn test_adv_atomic_01_clean_save_leaves_no_tempfiles() {
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    // Save 10 distinct presets
    for i in 0..10 {
        let id = format!("clean-preset-{}", i);
        let p = make_test_preset(&id, &format!("Clean Preset {}", i), false);
        let fmt = if i % 2 == 0 {
            PresetFormat::Toml
        } else {
            PresetFormat::Json
        };
        let path = store.save_preset(&p, fmt).expect("Save should succeed");
        assert!(path.is_file());
    }

    // Inspect directory: there must be exactly 10 preset files, and ZERO .preset_tmp_ files
    let entries: Vec<PathBuf> = fs::read_dir(store.root_dir())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();

    assert_eq!(entries.len(), 10);
    for entry in &entries {
        let fname = entry.file_name().unwrap().to_string_lossy();
        assert!(
            !fname.starts_with(".preset_tmp_"),
            "Orphaned temporary file detected: {}",
            fname
        );
        assert!(fname.ends_with(".toml") || fname.ends_with(".json"));
    }
}

#[test]
fn test_adv_atomic_02_atomic_overwrite_preserves_validity() {
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    let id = "evolving-preset";
    let mut p = make_test_preset(id, "Evolving Preset", false);

    // Initial save (v1)
    p.recipe.exposure = 0.50;
    let path = store.save_preset(&p, PresetFormat::Toml).unwrap();
    assert!(path.is_file());
    assert_eq!(store.load_preset(id).unwrap().exposure, 0.50);

    // Overwrite save (v2)
    p.recipe.exposure = 1.25;
    store.save_preset(&p, PresetFormat::Toml).unwrap();
    assert_eq!(store.load_preset(id).unwrap().exposure, 1.25);

    // Overwrite save (v3)
    p.recipe.exposure = -0.75;
    store.save_preset(&p, PresetFormat::Toml).unwrap();
    assert_eq!(store.load_preset(id).unwrap().exposure, -0.75);

    // Verify file is clean and parseable TOML
    let content = fs::read_to_string(&path).unwrap();
    let parsed: PresetFile = toml::from_str(&content).unwrap();
    assert_eq!(parsed.recipe.exposure, -0.75);
}

#[test]
fn test_adv_atomic_03_failed_write_unwritable_target_leaves_no_orphans() {
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    // Create a directory where the target file would be: "blocked.toml"
    let target_dir_path = store.root_dir().join("blocked.toml");
    fs::create_dir(&target_dir_path).unwrap();

    let p = make_test_preset("blocked", "Blocked Preset", false);
    // Persist must fail because target_path is a directory
    let res = store.save_preset(&p, PresetFormat::Toml);
    assert!(
        res.is_err(),
        "save_preset MUST fail when target is an existing directory"
    );
    assert!(matches!(res.unwrap_err(), StorageError::Io { .. }));

    // Check directory: temporary file MUST be automatically cleaned up by tempfile drop
    let entries: Vec<PathBuf> = fs::read_dir(store.root_dir())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();

    // Only "blocked.toml" directory should exist, no .preset_tmp_* files
    for entry in &entries {
        let fname = entry.file_name().unwrap().to_string_lossy();
        assert!(
            !fname.starts_with(".preset_tmp_"),
            "Tempfile leaked after failed persist: {}",
            fname
        );
    }
}

#[test]
fn test_adv_atomic_04_concurrent_saves_no_corruption() {
    let tmp = tempdir().unwrap();
    let store = Arc::new(PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap());

    let thread_count = 16;
    let mut handles = Vec::new();

    // Spawn 16 concurrent threads:
    // 8 threads saving distinct presets
    // 8 threads racing to save the same preset "contested"
    for i in 0..thread_count {
        let store_clone = Arc::clone(&store);
        let handle = thread::spawn(move || {
            if i < 8 {
                let id = format!("concurrent-preset-{}", i);
                let p = make_test_preset(&id, &format!("Concurrent {}", i), false);
                let fmt = if i % 2 == 0 {
                    PresetFormat::Toml
                } else {
                    PresetFormat::Json
                };
                store_clone
                    .save_preset(&p, fmt)
                    .expect("Concurrent distinct save failed");
            } else {
                let mut p = make_test_preset("contested", "Contested Preset", false);
                p.recipe.exposure = (i as f32) * 0.1;
                let _ = store_clone.save_preset(&p, PresetFormat::Toml);
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().expect("Thread panicked");
    }

    // Verify all distinct presets are loadable and valid
    for i in 0..8 {
        let id = format!("concurrent-preset-{}", i);
        let loaded = store
            .load_preset(&id)
            .expect("Should load concurrent preset");
        assert_eq!(loaded.exposure, 0.42);
    }

    // Verify contested preset is valid, uncorrupted TOML
    let contested = store
        .load_preset("contested")
        .expect("Contested preset should be loadable");
    assert!(contested.exposure >= 0.8 && contested.exposure <= 1.6);

    // Verify zero orphaned temporary files in directory
    for entry in fs::read_dir(store.root_dir()).unwrap().flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        assert!(
            !fname.starts_with(".preset_tmp_"),
            "Found orphaned temporary file after concurrent execution: {}",
            fname
        );
    }
}

#[test]
fn test_adv_atomic_05_tempfile_isolation_in_list_presets() {
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    // Save a legitimate preset
    let p = make_test_preset("legit", "Legitimate Preset", false);
    store.save_preset(&p, PresetFormat::Toml).unwrap();

    // Intentionally inject an abandoned temp file with corrupted content
    let corrupted_temp = store.root_dir().join(".preset_tmp_abandoned.toml");
    fs::write(&corrupted_temp, b"<<<CORRUPTED GARBAGE DATA>>>").unwrap();

    // Also inject a non-preset file
    let readme = store.root_dir().join("README.txt");
    fs::write(&readme, b"Notes about presets").unwrap();

    // Call list_presets
    let list = store.list_presets().unwrap();

    // Should include 5 builtins + 1 legit preset = 6 total
    assert_eq!(list.len(), 6);
    assert!(!list.iter().any(|m| m.id.starts_with(".preset_tmp_")));
    assert!(!list.iter().any(|m| m.name.contains("GARBAGE")));
}

// ============================================================================
// 4. CRYPTOGRAPHIC IMMUTABILITY OF CANON CR3 RAW FILES (MISSION ITEM 4)
// ============================================================================

#[test]
fn test_adv_cr3_01_authoritative_sha256_verification_before_and_after() {
    let draw_dir = find_draw_dir();
    if !draw_dir.is_dir() {
        eprintln!("Skipping CR3 immutability test: DRAW directory does not exist.");
        return;
    }

    // PRE-TEST HASHING: directly compute SHA-256 and size on present Canon CR3 files
    let mut pre_hashes = Vec::new();
    for entry in CR3_BASELINE {
        let file_path = draw_dir.join(entry.filename);
        if file_path.is_file() {
            let (hash, size) = compute_file_sha256(&file_path)
                .unwrap_or_else(|e| panic!("Failed to hash {}: {}", file_path.display(), e));

            assert_eq!(
                size, entry.expected_size,
                "Pre-flight size mismatch on {}",
                entry.filename
            );
            assert!(
                hash.eq_ignore_ascii_case(entry.expected_sha256),
                "Pre-flight SHA-256 mismatch on {}: expected {}, got {}",
                entry.filename,
                entry.expected_sha256,
                hash
            );

            pre_hashes.push((entry.filename, hash, size));
        }
    }
    if pre_hashes.is_empty() {
        eprintln!("Skipping CR3 immutability test: no source RAW files present.");
        return;
    }

    // PERFORM MASSIVE PRESET OPERATIONS (simulate high filesystem activity)
    let tmp = tempdir().unwrap();
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).unwrap();

    for i in 0..50 {
        let id = format!("stress-preset-{}", i);
        let p = make_test_preset(&id, &format!("Stress {}", i), false);
        store.save_preset(&p, PresetFormat::Toml).unwrap();
        let _ = store.load_preset(&id).unwrap();
        if i % 5 == 0 {
            store.delete_preset(&id).unwrap();
        }
    }

    // POST-TEST HASHING: re-verify byte-for-byte SHA-256 and size
    for (filename, pre_hash, pre_size) in pre_hashes {
        let file_path = draw_dir.join(filename);
        let (post_hash, post_size) = compute_file_sha256(&file_path)
            .unwrap_or_else(|e| panic!("Failed to re-hash {}: {}", file_path.display(), e));

        assert_eq!(
            post_size, pre_size,
            "CR3 MUTATION: size changed on {}! pre={}, post={}",
            filename, pre_size, post_size
        );
        assert_eq!(
            post_hash, pre_hash,
            "CR3 MUTATION: SHA-256 hash changed on {}! pre={}, post={}",
            filename, pre_hash, post_hash
        );
    }
}
