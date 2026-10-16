//! Adversarial stress test suite for Milestone M4 Export Subsystem (`src/export/`).
//!
//! Empirically challenges and stress-tests:
//! 1. Path traversal attacks:
//!    - `../`, `..\`, `/../`, URL-encoded `%2e%2e`, `%2f`, `%5c`, null bytes (`\0`),
//!      and non-existent parent traversal.
//!    - Asserts 100% rejection and zero files written outside sandbox.
//! 2. Filename sanitization torture:
//!    - Illegal Win32 chars (`/:*?"<>|`) and ASCII control characters (< 0x20).
//!    - Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`).
//!    - Trailing dots and spaces.
//!    - Asserts clean sanitization and successful write without OS errors.
//! 3. Collision resolution stress:
//!    - Cascading 100+ collisions under Rename policy.
//!    - Highly concurrent worker reservations race condition test.
//!    - Concurrent pipeline export execution without collisions or file corruption.
//! 4. Non-destructive RAW check:
//!    - Attempt export into `./DRAW/` directory.
//!    - Attempt export targeting `.CR3` / `.cr3` extension.
//!    - Asserts unconditional veto and zero writes.
//!    - Full cryptographic SHA-256 verification of Canon CR3 master files.

mod common;

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::tempdir;

use lightcraft_tui::engine::presets::BuiltinPreset;
use lightcraft_tui::error::IntegrityViolationError;
use lightcraft_tui::export::{
    sanitize_filename_component, validate_export_path, BitDepth, CollisionPolicy,
    CollisionResolution, CollisionResolver, ExportError, ExportJob, ExportOutcome,
    ExportPipeline, ExportProfile, NamingTemplate, OutputFormat,
    ScalingProfile,
};
use lightcraft_tui::mcp::integrity::{find_draw_dir, verify_cr3_baseline, IntegrityEngine};

// ============================================================================
// CHALLENGE 1: Path Traversal Attacks & Sandbox Containment
// ============================================================================

#[test]
fn test_adv_exp01_path_traversal_payload_matrix() {
    let draw_dir = find_draw_dir();
    let source_dirs = [draw_dir.clone()];

    // Comprehensive matrix of adversarial path traversal payloads
    let traversal_payloads = [
        // Standard parent traversal
        "../outside.jpg",
        "../../outside.jpg",
        "../../../escape.jpg",
        "sub/../../escape.jpg",
        "sub/deep/../../../escape.jpg",
        // Windows backslash traversal
        r"..\outside.jpg",
        r"..\..\evil.jpg",
        r"..\..\..\secret.jpg",
        r"sub\..\..\evil.jpg",
        r"sub\deep\..\..\..\secret.jpg",
        // Leading root / slash traversal
        "/../outside.jpg",
        "/../../etc/passwd.jpg",
        r"\..\outside.jpg",
        r"\..\..\secret.jpg",
        r"C:\..\escape.jpg",
        r"C:\fake\..\..\escape.jpg",
        // Mixed slash / backslash traversal
        r"../..\../mixed_escape.jpg",
        r"folder/..\../sub/escape.jpg",
        r"folder\../\..\escape.jpg",
        // URL-encoded traversal sequences (%2e%2e, %2f, %5c)
        "%2e%2e/outside.jpg",
        "%2e%2e%2foutside.jpg",
        "%2e%2e%5coutside.jpg",
        "sub/%2e%2e/%2e%2e/evil.jpg",
        "%2E%2E/upper_case_encoded.jpg",
        "%2E%2E%2Fescape.jpg",
        "sub/%2e%2e/file.jpg",
        // Null byte injections
        "evil\0outside.jpg",
        "folder/sub\0null/test.jpg",
        // Non-existent parent directory traversal
        "nonexistent_parent_dir_xyz/../escape.jpg",
        "deep/fake/nonexistent/path/../../../../escape.jpg",
        "sandbox/nonexistent/../../outside.jpg",
        // Windows device names inside traversal
        "CON/../escape.jpg",
        "NUL/../../escape.jpg",
    ];

    for payload in &traversal_payloads {
        let p = Path::new(payload);

        // 1. Test src/export/mod.rs validate_export_path (100% rejection across all vectors)
        let res = validate_export_path(p, &source_dirs);
        assert!(
            res.is_err(),
            "Payload '{}' MUST be rejected by validate_export_path, but got: {:?}",
            payload,
            res
        );

        match res.unwrap_err() {
            ExportError::PathTraversal(_) => {
                // Expected for traversal sequences and null bytes
            }
            ExportError::ReservedDeviceName(_) => {
                // Also acceptable if device name detected first
            }
            other => {
                panic!(
                    "Payload '{}' returned unexpected error variant: {:?}",
                    payload, other
                );
            }
        }

        // 2. Test IntegrityEngine::validate_destination_path for parent directory traversal
        // Note: IntegrityEngine only performs lexical ParentDir, '..', and %2e%2e checks.
        // It does not inspect for null bytes (which are caught by src/export/mod.rs validate_export_path).
        if !payload.contains('\0') {
            let engine = IntegrityEngine::new(vec![draw_dir.clone()], true);
            let integrity_res = engine.validate_destination_path(p);
            assert!(
                integrity_res.is_err(),
                "Payload '{}' MUST be rejected by IntegrityEngine::validate_destination_path, but got: {:?}",
                payload,
                integrity_res
            );
            assert!(
                matches!(
                    integrity_res.unwrap_err(),
                    IntegrityViolationError::PathTraversal { .. }
                        | IntegrityViolationError::SourceDirectoryContamination { .. }
                        | IntegrityViolationError::ProhibitedTargetExtension { .. }
                ),
                "Payload '{}' should trigger PathTraversal or sandbox violation",
                payload
            );
        }
    }
}

#[tokio::test]
async fn test_adv_exp02_path_traversal_sandbox_escape_zero_writes() {
    let tmp = tempdir().expect("Failed to create root tempdir");
    let sandbox_dir = tmp.path().join("sandbox");
    let outside_dir = tmp.path().join("outside_target");

    fs::create_dir_all(&sandbox_dir).expect("Failed to create sandbox dir");
    fs::create_dir_all(&outside_dir).expect("Failed to create outside dir");

    let pipeline = ExportPipeline::new();
    let profile = ExportProfile {
        name: "Adversarial Profile".into(),
        format: OutputFormat::Jpeg,
        quality: 80,
        bit_depth: BitDepth::Eight,
        scaling: ScalingProfile::FitToBounds {
            max_width: 800,
            max_height: 600,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    // Attack Vectors attempting to write outside sandbox
    let attack_configs: Vec<(PathBuf, NamingTemplate, String)> = vec![
        // Vector 1: output_dir relative parent traversal
        (
            sandbox_dir.join("../outside_target"),
            NamingTemplate::default(),
            "Natural".into(),
        ),
        // Vector 2: non-existent parent traversal in output_dir
        (
            sandbox_dir.join("nonexistent_parent/../../outside_target"),
            NamingTemplate::default(),
            "Natural".into(),
        ),
        // Vector 3: URL-encoded traversal in template
        (
            sandbox_dir.clone(),
            NamingTemplate::parse("%2e%2e/%2e%2e/outside_target/{name}.{ext}"),
            "Natural".into(),
        ),
        // Vector 4: Parent traversal literal in template
        (
            sandbox_dir.clone(),
            NamingTemplate::parse("../../../outside_target/{name}.{ext}"),
            "Natural".into(),
        ),
        // Vector 5: Traversal inside preset_name
        (
            sandbox_dir.clone(),
            NamingTemplate::parse("{preset}__{name}.{ext}"),
            "../../outside_target/hacked".into(),
        ),
        // Vector 6: URL-encoded traversal in preset_name
        (
            sandbox_dir.clone(),
            NamingTemplate::parse("{preset}__{name}.{ext}"),
            "%2e%2e/%2e%2e/outside_target/hacked".into(),
        ),
    ];

    for (idx, (out_dir, template, preset_name)) in attack_configs.into_iter().enumerate() {
        let job = ExportJob {
            job_id: 1000 + idx as u64,
            source_path: PathBuf::from("DRAW/IMG_0588.CR3"),
            photo_id: Some(1000 + idx as i64),
            recipe_name: preset_name,
            recipe: BuiltinPreset::Natural.recipe(),
            profile: profile.clone(),
            output_dir: out_dir,
            template,
            seq_number: idx + 1,
            collision_policy: CollisionPolicy::Overwrite,
        };

        let res = pipeline.execute_export(None, &job).await;
        assert!(
            res.is_err(),
            "Attack vector #{} MUST fail, but succeeded with {:?}",
            idx,
            res
        );

        // Verify zero files written outside sandbox
        let outside_files: Vec<_> = fs::read_dir(&outside_dir)
            .expect("Failed to read outside_dir")
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(
            outside_files.len(),
            0,
            "Attack vector #{} caused files to be written in outside_target: {:?}",
            idx,
            outside_files
        );

        // Verify root temp dir has only sandbox and outside_target
        let root_entries: Vec<_> = fs::read_dir(tmp.path())
            .expect("Failed to read tmp")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            root_entries.len(),
            2,
            "No files leaked to root temp directory: {:?}",
            root_entries
        );
    }
}

// ============================================================================
// CHALLENGE 2: Filename Sanitization Torture
// ============================================================================

#[test]
fn test_adv_exp03_illegal_win32_chars_sanitization() {
    // Torture strings containing every illegal Win32 character
    let torture_inputs = [
        (
            r#"Teal/Orange:*?"<>|Ultra"#,
            "Teal_Orange_______Ultra",
        ),
        (
            "Colon:Asterisk*Question?Quote\"Less<Greater>Pipe|Slash/Backslash\\",
            "Colon_Asterisk_Question_Quote_Less_Greater_Pipe_Slash_Backslash_",
        ),
        (
            "Control\x01Chars\x02In\x1fFilename",
            "Control_Chars_In_Filename",
        ),
        (
            r#":*?"<>|/\/:*?"<>|"#,
            "_________________",
        ),
    ];

    for (raw_input, expected_sanitized) in torture_inputs {
        let sanitized = sanitize_filename_component(raw_input);
        assert_eq!(
            sanitized, expected_sanitized,
            "Sanitization mismatch for input: '{}'",
            raw_input
        );

        // Ensure sanitized output contains none of the forbidden characters
        for forbidden in ['/', '\\', ':', '*', '?', '"', '<', '>', '|'] {
            assert!(
                !sanitized.contains(forbidden),
                "Sanitized string still contains forbidden character '{}': {}",
                forbidden,
                sanitized
            );
        }
    }
}

#[tokio::test]
async fn test_adv_exp04_filename_torture_clean_write_no_os_error() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let pipeline = ExportPipeline::new();

    let profile = ExportProfile {
        name: "Torture Profile".into(),
        format: OutputFormat::Jpeg,
        quality: 75,
        bit_depth: BitDepth::Eight,
        scaling: ScalingProfile::FitToBounds {
            max_width: 640,
            max_height: 480,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    // Preset names and templates loaded with illegal characters
    let torture_scenarios = [
        ("Teal/Orange:Style*Test?\"<Angle>|Pipe", "{name}__{preset}.{ext}"),
        ("Control\x07Bell\x08Back", "{preset}__{seq:2}.{ext}"),
        (r#"Path\Traversal/Like:Tokens"#, "{name}__{preset}.{ext}"),
    ];

    for (idx, (preset_name, template_str)) in torture_scenarios.iter().enumerate() {
        let job = ExportJob {
            job_id: 2000 + idx as u64,
            source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
            photo_id: Some(2000 + idx as i64),
            recipe_name: preset_name.to_string(),
            recipe: BuiltinPreset::TealOrange.recipe(),
            profile: profile.clone(),
            output_dir: tmp.path().to_path_buf(),
            template: NamingTemplate::parse(template_str),
            seq_number: idx + 1,
            collision_policy: CollisionPolicy::Overwrite,
        };

        let res = pipeline.execute_export(None, &job).await;
        assert!(
            res.is_ok(),
            "Export with torture preset '{}' MUST succeed without OS error, but got: {:?}",
            preset_name,
            res.err()
        );

        if let ExportOutcome::Success { output_path, bytes_written, .. } = res.unwrap() {
            assert!(output_path.exists(), "Target file must exist on disk");
            assert!(bytes_written > 0, "Written bytes must be > 0");

            // Verify file can be read and is valid JPEG
            let bytes = fs::read(&output_path).expect("Failed to read output file");
            assert_eq!(&bytes[0..2], &[0xFF, 0xD8], "Valid JPEG header required");
        } else {
            panic!("Expected ExportOutcome::Success");
        }
    }
}

#[tokio::test]
async fn test_adv_exp05_reserved_device_names_torture() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let pipeline = ExportPipeline::new();

    let profile = ExportProfile {
        name: "Reserved Name Profile".into(),
        format: OutputFormat::Jpeg,
        quality: 75,
        bit_depth: BitDepth::Eight,
        scaling: ScalingProfile::FitToBounds {
            max_width: 400,
            max_height: 300,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    // Full list of Win32 reserved device names
    let reserved_names = [
        "CON", "PRN", "AUX", "NUL",
        "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        // Lowercase and mixed-case variants
        "con", "prn", "aux", "nul", "com1", "lpt9",
    ];

    for (idx, &reserved) in reserved_names.iter().enumerate() {
        // 1. Direct unit test of sanitize_filename_component
        let sanitized = sanitize_filename_component(reserved);
        assert!(
            sanitized.starts_with('_'),
            "Reserved device name '{}' must be sanitized with leading '_', got: '{}'",
            reserved,
            sanitized
        );
        assert_eq!(sanitized, format!("_{}", reserved));

        // 2. Full end-to-end export pipeline test
        let job = ExportJob {
            job_id: 3000 + idx as u64,
            source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
            photo_id: Some(3000 + idx as i64),
            recipe_name: reserved.to_string(),
            recipe: BuiltinPreset::Cinema.recipe(),
            profile: profile.clone(),
            output_dir: tmp.path().to_path_buf(),
            template: NamingTemplate::parse("{preset}.{ext}"),
            seq_number: idx + 1,
            collision_policy: CollisionPolicy::Overwrite,
        };

        let res = pipeline.execute_export(None, &job).await;
        assert!(
            res.is_ok(),
            "Export with reserved device name '{}' MUST succeed without Win32 OS error, got: {:?}",
            reserved,
            res.err()
        );

        if let ExportOutcome::Success { output_path, bytes_written, .. } = res.unwrap() {
            assert!(output_path.exists());
            assert!(bytes_written > 0);
            let file_name = output_path.file_name().unwrap().to_string_lossy();
            assert!(
                file_name.starts_with('_'),
                "Exported filename must have leading underscore to guard Win32 reserved name: {}",
                file_name
            );
        } else {
            panic!("Expected ExportOutcome::Success");
        }
    }
}

#[tokio::test]
async fn test_adv_exp06_trailing_dots_and_spaces_torture() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let pipeline = ExportPipeline::new();

    let profile = ExportProfile {
        name: "Trailing Torture".into(),
        format: OutputFormat::Jpeg,
        quality: 75,
        bit_depth: BitDepth::Eight,
        scaling: ScalingProfile::FitToBounds {
            max_width: 400,
            max_height: 300,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    let torture_cases = [
        ("MyPreset.....", "MyPreset"),
        ("MyPreset     ", "MyPreset"),
        ("MyPreset. . . .   ", "MyPreset"),
        ("....    ", "unnamed"),
        ("        ", "unnamed"),
    ];

    for (idx, (raw_preset, expected_clean)) in torture_cases.iter().enumerate() {
        let sanitized = sanitize_filename_component(raw_preset);
        assert_eq!(
            &sanitized, expected_clean,
            "Sanitization failed for trailing dots/spaces: '{}'",
            raw_preset
        );

        let job = ExportJob {
            job_id: 4000 + idx as u64,
            source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
            photo_id: Some(4000 + idx as i64),
            recipe_name: raw_preset.to_string(),
            recipe: BuiltinPreset::Natural.recipe(),
            profile: profile.clone(),
            output_dir: tmp.path().to_path_buf(),
            template: NamingTemplate::parse("{preset}__{seq:2}.{ext}"),
            seq_number: idx + 1,
            collision_policy: CollisionPolicy::Overwrite,
        };

        let res = pipeline.execute_export(None, &job).await;
        assert!(
            res.is_ok(),
            "Export with trailing dots/spaces '{}' MUST succeed without OS errors: {:?}",
            raw_preset,
            res.err()
        );

        if let ExportOutcome::Success { output_path, bytes_written, .. } = res.unwrap() {
            assert!(output_path.exists());
            assert!(bytes_written > 0);
            let filename = output_path.file_name().unwrap().to_string_lossy();
            assert!(
                !filename.ends_with('.') && !filename.ends_with(' '),
                "Filename on disk must not end with trailing dot or space: '{}'",
                filename
            );
        } else {
            panic!("Expected ExportOutcome::Success");
        }
    }
}

// ============================================================================
// CHALLENGE 3: Collision Resolution Stress (Cascading 100+ & Concurrency)
// ============================================================================

#[test]
fn test_adv_exp07_cascading_100_plus_collisions() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let resolver = CollisionResolver::new();
    let file_stem = "cascade_test";
    let ext = "jpg";

    // 1. Pre-create 120 colliding files directly on the filesystem
    let base_file = tmp.path().join(format!("{}.{}", file_stem, ext));
    File::create(&base_file).unwrap().write_all(b"BASE").unwrap();

    for i in 1..=120 {
        let collision_file = tmp.path().join(format!("{}_{}.{}", file_stem, i, ext));
        File::create(&collision_file).unwrap().write_all(b"COLLISION").unwrap();
    }

    // 2. Request resolution for the 121st collision
    let res = resolver.resolve_path(tmp.path(), file_stem, ext, CollisionPolicy::Rename);
    assert!(res.is_ok(), "Resolver failed at 121st collision: {:?}", res.err());

    match res.unwrap() {
        CollisionResolution::Write(path) => {
            assert_eq!(
                path.file_name().unwrap(),
                "cascade_test_121.jpg",
                "Resolver must skip all 120 pre-existing files and assign _121"
            );
        }
        _ => panic!("Expected CollisionResolution::Write"),
    }

    // 3. Request resolution for the 122nd collision (without writing to disk yet, testing in-memory reservation)
    let res2 = resolver.resolve_path(tmp.path(), file_stem, ext, CollisionPolicy::Rename);
    assert!(res2.is_ok());
    match res2.unwrap() {
        CollisionResolution::Write(path) => {
            assert_eq!(
                path.file_name().unwrap(),
                "cascade_test_122.jpg",
                "Resolver must reserve _122 in memory"
            );
        }
        _ => panic!("Expected CollisionResolution::Write"),
    }
}

#[tokio::test]
async fn test_adv_exp08_concurrent_worker_reservation_race_condition() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let resolver = Arc::new(CollisionResolver::new());
    let file_stem = "concurrent_worker_test";
    let ext = "png";

    // Spawn 100 concurrent async tasks all attempting to reserve the exact same filename
    let num_tasks = 100;
    let mut handles = Vec::with_capacity(num_tasks);

    for _ in 0..num_tasks {
        let resolver_clone = Arc::clone(&resolver);
        let out_dir = tmp.path().to_path_buf();
        let stem = file_stem.to_string();
        let extension = ext.to_string();

        let handle = tokio::spawn(async move {
            resolver_clone.resolve_path(&out_dir, &stem, &extension, CollisionPolicy::Rename)
        });
        handles.push(handle);
    }

    let mut resolved_paths = Vec::with_capacity(num_tasks);
    for handle in handles {
        let res = handle.await.expect("Tokio task panicked");
        let resolution = res.expect("resolve_path failed under concurrency");
        match resolution {
            CollisionResolution::Write(p) => resolved_paths.push(p),
            CollisionResolution::Skip(_) => panic!("Rename policy should never yield Skip"),
        }
    }

    // EMPIRICAL ASSERTIONS:
    // 1. Total resolved paths must equal total requests (100)
    assert_eq!(resolved_paths.len(), num_tasks);

    // 2. Every single path MUST be unique (Zero collisions / race condition duplicate reservations)
    let unique_paths: HashSet<_> = resolved_paths.iter().cloned().collect();
    assert_eq!(
        unique_paths.len(),
        num_tasks,
        "Race condition detected! {} duplicate path reservations among 100 concurrent workers",
        num_tasks - unique_paths.len()
    );

    // 3. Verify path names adhere to expected naming pattern
    for path in &resolved_paths {
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            name == "concurrent_worker_test.png" || name.starts_with("concurrent_worker_test_"),
            "Invalid reserved path name: {}",
            name
        );
    }
}

#[tokio::test]
async fn test_adv_exp09_concurrent_pipeline_export_stress() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let pipeline = Arc::new(ExportPipeline::new());

    let profile = ExportProfile {
        name: "Concurrent Export Profile".into(),
        format: OutputFormat::Jpeg,
        quality: 75,
        bit_depth: BitDepth::Eight,
        scaling: ScalingProfile::FitToBounds {
            max_width: 480,
            max_height: 320,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    // 20 concurrent export jobs all targeting the identical destination stem with Rename policy
    let concurrency = 20;
    let mut handles = Vec::with_capacity(concurrency);

    for idx in 0..concurrency {
        let pipeline_clone = Arc::clone(&pipeline);
        let job = ExportJob {
            job_id: 5000 + idx as u64,
            source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
            photo_id: Some(5000 + idx as i64),
            recipe_name: "Natural".into(),
            recipe: BuiltinPreset::Natural.recipe(),
            profile: profile.clone(),
            output_dir: tmp.path().to_path_buf(),
            template: NamingTemplate::parse("concurrent_export.{ext}"),
            seq_number: idx + 1,
            collision_policy: CollisionPolicy::Rename,
        };

        let handle = tokio::spawn(async move {
            pipeline_clone.execute_export(None, &job).await
        });
        handles.push(handle);
    }

    let mut success_paths = Vec::with_capacity(concurrency);
    for handle in handles {
        let res = handle.await.expect("Tokio export worker panicked");
        let outcome = res.expect("Pipeline export failed under concurrency");
        match outcome {
            ExportOutcome::Success { output_path, bytes_written, .. } => {
                assert!(output_path.exists());
                assert!(bytes_written > 0);
                success_paths.push(output_path);
            }
            _ => panic!("Expected ExportOutcome::Success"),
        }
    }

    assert_eq!(success_paths.len(), concurrency);

    // Verify all 20 files are distinct on disk
    let unique_paths: HashSet<_> = success_paths.iter().cloned().collect();
    assert_eq!(
        unique_paths.len(),
        concurrency,
        "Race condition in file generation! Expected 20 unique files on disk, got {}",
        unique_paths.len()
    );

    // Verify all 20 files on disk are valid, non-corrupted JPEGs
    for p in &success_paths {
        let bytes = fs::read(p).expect("Failed to read created export file");
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8], "File {} is corrupted", p.display());
    }
}

// ============================================================================
// CHALLENGE 4: Non-Destructive RAW Check & Extension Veto
// ============================================================================

#[tokio::test]
async fn test_adv_exp10_export_into_draw_directory_unconditional_veto() {
    let draw_dir = find_draw_dir();
    assert!(draw_dir.exists(), "DRAW directory must exist");

    // Capture baseline file list before attack
    let pre_files: Vec<String> = fs::read_dir(&draw_dir)
        .expect("Failed to read DRAW dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    let pipeline = ExportPipeline::new();
    let profile = ExportProfile {
        name: "Malicious Profile".into(),
        format: OutputFormat::Jpeg,
        quality: 90,
        bit_depth: BitDepth::Eight,
        scaling: ScalingProfile::FitToBounds {
            max_width: 800,
            max_height: 600,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    // Attempting export directly into DRAW/
    let attack_dirs = [
        draw_dir.clone(),
        PathBuf::from("DRAW"),
        PathBuf::from("./DRAW"),
        draw_dir.join("sub_contaminate"),
    ];

    for (idx, target_dir) in attack_dirs.iter().enumerate() {
        let job = ExportJob {
            job_id: 6000 + idx as u64,
            source_path: PathBuf::from("DRAW/IMG_0588.CR3"),
            photo_id: Some(6000 + idx as i64),
            recipe_name: "Contaminate".into(),
            recipe: BuiltinPreset::Natural.recipe(),
            profile: profile.clone(),
            output_dir: target_dir.clone(),
            template: NamingTemplate::parse("evil_payload.{ext}"),
            seq_number: idx + 1,
            collision_policy: CollisionPolicy::Overwrite,
        };

        let res = pipeline.execute_export(None, &job).await;
        assert!(
            res.is_err(),
            "Export targeting DRAW directory '{:?}' MUST be vetoed, but succeeded!",
            target_dir
        );

        match res.unwrap_err() {
            ExportError::Integrity(IntegrityViolationError::SourceDirectoryContamination { .. }) => {
                // Expected exact error
            }
            other => {
                panic!(
                    "Expected SourceDirectoryContamination error targeting DRAW, got: {:?}",
                    other
                );
            }
        }
    }

    // Post-attack check: verify DRAW folder was untouched (zero writes, no temporary files)
    let post_files: Vec<String> = fs::read_dir(&draw_dir)
        .expect("Failed to read DRAW dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    assert_eq!(
        pre_files, post_files,
        "Files inside DRAW directory were contaminated! Pre: {:?}, Post: {:?}",
        pre_files, post_files
    );

    // Assert no temporary staging files exist
    for f in &post_files {
        assert!(
            !f.starts_with(".lc_export_tmp_"),
            "Stray temporary export file found in DRAW: {}",
            f
        );
    }
}

#[test]
fn test_adv_exp11_export_targeting_cr3_extension_unconditional_veto() {
    let draw_dir = find_draw_dir();
    let source_dirs = [draw_dir.clone()];

    // Test case variants of prohibited .CR3 extension
    let prohibited_targets = [
        PathBuf::from("target/export/output.cr3"),
        PathBuf::from("target/export/output.CR3"),
        PathBuf::from("target/export/output.Cr3"),
        PathBuf::from("target/export/output.cR3"),
        PathBuf::from("export/sub/IMG_0588.CR3"),
    ];

    for target in &prohibited_targets {
        // 1. validate_export_path
        let res = validate_export_path(target, &source_dirs);
        assert!(
            matches!(res, Err(ExportError::ProhibitedTargetExtension { .. })),
            "validate_export_path must reject prohibited .CR3 extension: {:?}",
            target
        );

        // 2. IntegrityEngine::check_extension_veto
        let engine = IntegrityEngine::new(vec![draw_dir.clone()], true);
        let veto_res = engine.check_extension_veto(target);
        assert!(
            matches!(veto_res, Err(IntegrityViolationError::ProhibitedTargetExtension { .. })),
            "IntegrityEngine must veto target: {:?}",
            target
        );
    }
}

#[test]
fn test_adv_exp12_canon_cr3_master_immutability_baseline() {
    let draw_dir = find_draw_dir();
    assert!(draw_dir.exists(), "DRAW directory must exist");

    let results = verify_cr3_baseline(&draw_dir);
    match results {
        Ok(baseline_results) => {
            for record in &baseline_results {
                assert!(
                    record.is_valid,
                    "CR3 master file {} corrupted! Expected sha256={}, actual={}",
                    record.filename,
                    record.expected_sha256,
                    record.actual_sha256
                );
            }
        }
        Err(missing_err) => {
            let mut checked_any = false;
            for entry in lightcraft_tui::mcp::integrity::CR3_BASELINE {
                let path = draw_dir.join(entry.filename);
                if path.is_file() {
                    let (hash, size) = lightcraft_tui::mcp::integrity::compute_file_sha256(&path)
                        .expect("Failed reading present file");
                    assert_eq!(size, entry.expected_size, "Size mismatch for {}", entry.filename);
                    assert!(
                        hash.eq_ignore_ascii_case(entry.expected_sha256),
                        "SHA-256 mismatch for {}",
                        entry.filename
                    );
                    checked_any = true;
                }
            }
            if !checked_any {
                panic!("Master CR3 baseline cryptographic verification failed: {}", missing_err);
            }
        }
    }
}
