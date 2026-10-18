//! Tier 1 Export Pipeline Integration Tests: Multi-format encoding, scaling profiles,
//! template naming tokens, Win32 sanitization, collision handling, and security sandboxing.
//!
//! Covers Milestone M4 features:
//! - F-EXP-01: Multi-Format Export Engine (JPEG, PNG, TIFF)
//! - F-EXP-02: Dimension & Scaling Profiles (Native, LongEdge, FitToBounds, Percentage, dont_enlarge)
//! - F-EXP-03: Template-Based File Naming ({name}, {preset}, {seq:N}, {date:FMT}, {ext})
//! - F-EXP-04: Collision Handling Policies (Rename, Cascading Rename, Skip, Overwrite)
//! - F-SEC-01: Source Immutability Guarantee & CR3 Veto
//! - F-RNF-03: Path Security & Lexical Traversal Rejection

mod common;

use chrono::{TimeZone, Utc};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

use lightcraft_tui::engine::presets::BuiltinPreset;
use lightcraft_tui::error::IntegrityViolationError;
use lightcraft_tui::export::{
    validate_export_path, BitDepth, CollisionPolicy, CollisionResolution, CollisionResolver,
    ExportError, ExportJob, ExportOutcome, ExportPipeline, ExportProfile, NamingContext,
    NamingTemplate, OutputFormat, ScalingProfile,
};
use lightcraft_tui::mcp::integrity::{find_draw_dir, verify_cr3_baseline};

// ============================================================================
// 1. Multi-Format Export Engine (F-EXP-01)
// ============================================================================

#[tokio::test]
async fn test_t1_exp01_multi_format_encoding() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let pipeline = ExportPipeline::new();

    // 1. Test JPEG encoding
    let jpeg_profile = ExportProfile {
        name: "JPEG Test".into(),
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

    let jpeg_job = ExportJob {
        job_id: 1,
        source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
        photo_id: Some(1),
        recipe_name: "Natural".into(),
        recipe: BuiltinPreset::Natural.recipe(),
        profile: jpeg_profile,
        output_dir: tmp.path().to_path_buf(),
        template: NamingTemplate::parse("{name}__{preset}.{ext}"),
        seq_number: 1,
        collision_policy: CollisionPolicy::Overwrite,
    };

    let res = pipeline.execute_export(None, &jpeg_job).await;
    assert!(res.is_ok(), "JPEG export failed: {:?}", res.err());

    let outcome = res.unwrap();
    if let ExportOutcome::Success {
        output_path,
        width,
        height,
        ..
    } = outcome
    {
        assert!(output_path.exists(), "Output file must exist on disk");
        assert_eq!(width, 800);
        assert_eq!(height, 533); // 3:2 aspect ratio preserved

        let bytes = std::fs::read(&output_path).expect("Failed to read output file");
        // Check JPEG SOI marker (0xFF, 0xD8)
        assert!(bytes.len() > 4, "JPEG file too small");
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8], "JPEG magic header mismatch");
    } else {
        panic!("Expected ExportOutcome::Success");
    }

    // 2. Test PNG encoding
    let png_profile = ExportProfile {
        name: "PNG Test".into(),
        format: OutputFormat::Png,
        quality: 100,
        bit_depth: BitDepth::Eight,
        scaling: ScalingProfile::FitToBounds {
            max_width: 600,
            max_height: 400,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    let png_job = ExportJob {
        job_id: 2,
        source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
        photo_id: Some(2),
        recipe_name: "Cinema".into(),
        recipe: BuiltinPreset::Cinema.recipe(),
        profile: png_profile,
        output_dir: tmp.path().to_path_buf(),
        template: NamingTemplate::parse("{name}__{preset}.{ext}"),
        seq_number: 2,
        collision_policy: CollisionPolicy::Overwrite,
    };

    let res = pipeline.execute_export(None, &png_job).await;
    assert!(res.is_ok(), "PNG export failed: {:?}", res.err());

    if let ExportOutcome::Success {
        output_path,
        width,
        height,
        ..
    } = res.unwrap()
    {
        assert!(output_path.exists());
        assert_eq!(width, 600);
        assert_eq!(height, 400);

        let bytes = std::fs::read(&output_path).expect("Failed to read PNG");
        // Check PNG magic bytes: 0x89 0x50 0x4E 0x47 0x0D 0x0A 0x1A 0x0A
        assert!(bytes.len() > 8);
        assert_eq!(
            &bytes[0..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );
    } else {
        panic!("Expected ExportOutcome::Success");
    }

    // 3. Test TIFF encoding
    let tiff_profile = ExportProfile {
        name: "TIFF Test".into(),
        format: OutputFormat::Tiff,
        quality: 100,
        bit_depth: BitDepth::Sixteen,
        scaling: ScalingProfile::FitToBounds {
            max_width: 300,
            max_height: 200,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    let tiff_job = ExportJob {
        job_id: 3,
        source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
        photo_id: Some(3),
        recipe_name: "BW".into(),
        recipe: BuiltinPreset::BwHighContrast.recipe(),
        profile: tiff_profile,
        output_dir: tmp.path().to_path_buf(),
        template: NamingTemplate::parse("{name}__{preset}.{ext}"),
        seq_number: 3,
        collision_policy: CollisionPolicy::Overwrite,
    };

    let res = pipeline.execute_export(None, &tiff_job).await;
    assert!(res.is_ok(), "TIFF export failed: {:?}", res.err());

    if let ExportOutcome::Success {
        output_path,
        width,
        height,
        ..
    } = res.unwrap()
    {
        assert!(output_path.exists());
        assert_eq!(width, 300);
        assert_eq!(height, 200);

        let bytes = std::fs::read(&output_path).expect("Failed to read TIFF");
        // TIFF magic: 'II*' (little-endian) or 'MM*' (big-endian)
        assert!(bytes.len() > 4);
        let is_tiff = (&bytes[0..4] == b"II*\0") || (&bytes[0..4] == b"MM\0*");
        assert!(is_tiff, "TIFF magic header mismatch: {:?}", &bytes[0..4]);
    } else {
        panic!("Expected ExportOutcome::Success");
    }
}

#[tokio::test]
async fn test_t1_exp02_quality_scaling() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let pipeline = ExportPipeline::new();

    // Export quality 50
    let profile_q50 = ExportProfile {
        name: "Q50".into(),
        format: OutputFormat::Jpeg,
        quality: 50,
        scaling: ScalingProfile::FitToBounds {
            max_width: 1200,
            max_height: 800,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    let job_q50 = ExportJob {
        job_id: 10,
        source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
        photo_id: Some(10),
        recipe_name: "Q50".into(),
        recipe: BuiltinPreset::Natural.recipe(),
        profile: profile_q50,
        output_dir: tmp.path().to_path_buf(),
        template: NamingTemplate::parse("{name}__q50.{ext}"),
        seq_number: 1,
        collision_policy: CollisionPolicy::Overwrite,
    };

    let res_q50 = pipeline
        .execute_export(None, &job_q50)
        .await
        .expect("Q50 export failed");
    let path_q50 = match res_q50 {
        ExportOutcome::Success { output_path, .. } => output_path,
        _ => panic!("Expected success"),
    };

    // Export quality 95
    let profile_q95 = ExportProfile {
        name: "Q95".into(),
        format: OutputFormat::Jpeg,
        quality: 95,
        scaling: ScalingProfile::FitToBounds {
            max_width: 1200,
            max_height: 800,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    let job_q95 = ExportJob {
        job_id: 11,
        source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
        photo_id: Some(11),
        recipe_name: "Q95".into(),
        recipe: BuiltinPreset::Natural.recipe(),
        profile: profile_q95,
        output_dir: tmp.path().to_path_buf(),
        template: NamingTemplate::parse("{name}__q95.{ext}"),
        seq_number: 2,
        collision_policy: CollisionPolicy::Overwrite,
    };

    let res_q95 = pipeline
        .execute_export(None, &job_q95)
        .await
        .expect("Q95 export failed");
    let path_q95 = match res_q95 {
        ExportOutcome::Success { output_path, .. } => output_path,
        _ => panic!("Expected success"),
    };

    let size_50 = std::fs::metadata(&path_q50).unwrap().len();
    let size_95 = std::fs::metadata(&path_q95).unwrap().len();

    assert!(
        size_95 > size_50,
        "Quality 95 file ({} bytes) must be demonstrably larger than quality 50 ({} bytes)",
        size_95,
        size_50
    );
}

// ============================================================================
// 2. Dimension & Scaling Profiles (F-EXP-02)
// ============================================================================

#[test]
fn test_t1_exp03_dimension_scaling_profiles() {
    let (src_w, src_h) = (6000u32, 4000u32);

    // 1. Native full resolution
    let native = ScalingProfile::Native;
    assert_eq!(
        native.compute_target_dimensions(src_w, src_h).unwrap(),
        (6000, 4000)
    );

    // 2. Long edge 2048px
    let long_edge = ScalingProfile::LongEdge {
        max_long_edge: 2048,
        dont_enlarge: false,
    };
    let (w, h) = long_edge.compute_target_dimensions(src_w, src_h).unwrap();
    assert_eq!(w, 2048);
    assert_eq!(h, 1365); // 2048 * (4000 / 6000) = 1365.33 -> 1365

    // 3. Fit inside 1920x1080 bounds
    let fit = ScalingProfile::FitToBounds {
        max_width: 1920,
        max_height: 1080,
        dont_enlarge: false,
    };
    let (w, h) = fit.compute_target_dimensions(src_w, src_h).unwrap();
    assert_eq!(w, 1620); // bounded by height 1080: 1080 * 1.5 = 1620
    assert_eq!(h, 1080);

    // 4. Percentage 50%
    let pct = ScalingProfile::Percentage { percent: 50.0 };
    assert_eq!(
        pct.compute_target_dimensions(src_w, src_h).unwrap(),
        (3000, 2000)
    );
}

#[test]
fn test_t1_exp04_dont_enlarge_guard() {
    let (src_w, src_h) = (6000u32, 4000u32);

    // With dont_enlarge = true, requesting 8000px must not upscale beyond 6000px
    let long_edge_guard = ScalingProfile::LongEdge {
        max_long_edge: 8000,
        dont_enlarge: true,
    };
    assert_eq!(
        long_edge_guard
            .compute_target_dimensions(src_w, src_h)
            .unwrap(),
        (6000, 4000)
    );

    // With dont_enlarge = false, requesting 8000px should upscale to 8000x5333
    let long_edge_allow = ScalingProfile::LongEdge {
        max_long_edge: 8000,
        dont_enlarge: false,
    };
    assert_eq!(
        long_edge_allow
            .compute_target_dimensions(src_w, src_h)
            .unwrap(),
        (8000, 5333)
    );

    // FitToBounds with dont_enlarge = true
    let fit_guard = ScalingProfile::FitToBounds {
        max_width: 10000,
        max_height: 10000,
        dont_enlarge: true,
    };
    assert_eq!(
        fit_guard.compute_target_dimensions(src_w, src_h).unwrap(),
        (6000, 4000)
    );
}

// ============================================================================
// 3. Template-Based File Naming (F-EXP-03)
// ============================================================================

#[test]
fn test_t1_exp05_template_token_resolution() {
    let template = NamingTemplate::parse("{date:%Y%m%d}_{name}__{preset}_{seq:2}.{ext}");
    let fixed_date = Utc.with_ymd_and_hms(2026, 10, 15, 12, 0, 0).unwrap();

    let ctx = NamingContext {
        base_filename: "IMG_0588",
        preset_name: "Natural",
        seq_number: 3,
        capture_date: Some(fixed_date),
        extension: "jpg",
    };

    let resolved = template.resolve(&ctx);
    assert_eq!(resolved, "20261015_IMG_0588__Natural_03.jpg");

    // Test sequence padding with {seq:3}
    let t_seq3 = NamingTemplate::parse("{name}_{seq:3}.{ext}");
    assert_eq!(t_seq3.resolve(&ctx), "IMG_0588_003.jpg");

    // Test default template without extension token automatically appending extension
    let t_no_ext = NamingTemplate::parse("{name}_{preset}");
    assert_eq!(t_no_ext.resolve(&ctx), "IMG_0588_Natural.jpg");
}

#[test]
fn test_t1_exp06_template_illegal_char_sanitization() {
    let template = NamingTemplate::parse("{name}_{preset}.jpg");
    let ctx = NamingContext {
        base_filename: "IMG_0588",
        preset_name: "B&W: High/Contrast?",
        seq_number: 1,
        capture_date: None,
        extension: "jpg",
    };

    let resolved = template.resolve(&ctx);
    assert_eq!(resolved, "IMG_0588_B&W_ High_Contrast_.jpg");

    // Test Win32 reserved device name protection (CON -> _CON)
    let ctx_con = NamingContext {
        base_filename: "CON",
        preset_name: "Cinema",
        seq_number: 1,
        capture_date: None,
        extension: "jpg",
    };
    let t_reserved = NamingTemplate::parse("{name}_{preset}.jpg");
    let resolved_con = t_reserved.resolve(&ctx_con);
    assert_eq!(resolved_con, "_CON_Cinema.jpg");
}

// ============================================================================
// 4. Collision Handling Policies (F-EXP-04)
// ============================================================================

#[tokio::test]
async fn test_t1_exp07_collision_policy_rename_cascading() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let resolver = CollisionResolver::new();

    // Pre-create initial file and first collision
    let f1 = tmp.path().join("IMG_0588_Natural.jpg");
    File::create(&f1).unwrap().write_all(b"1").unwrap();

    let f2 = tmp.path().join("IMG_0588_Natural_1.jpg");
    File::create(&f2).unwrap().write_all(b"2").unwrap();

    // Resolution should cascade to _2
    let res = resolver.resolve_path(
        tmp.path(),
        "IMG_0588_Natural",
        "jpg",
        CollisionPolicy::Rename,
    );
    assert!(res.is_ok());

    match res.unwrap() {
        CollisionResolution::Write(path) => {
            assert_eq!(path.file_name().unwrap(), "IMG_0588_Natural_2.jpg");
        }
        _ => panic!("Expected CollisionResolution::Write"),
    }
}

#[tokio::test]
async fn test_t1_exp08_collision_policy_skip() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let pipeline = ExportPipeline::new();

    // Pre-create target file with known content
    let target = tmp.path().join("IMG_0969__Natural__01.jpg");
    File::create(&target)
        .unwrap()
        .write_all(b"ORIGINAL")
        .unwrap();

    let job = ExportJob {
        job_id: 20,
        source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
        photo_id: Some(20),
        recipe_name: "Natural".into(),
        recipe: BuiltinPreset::Natural.recipe(),
        profile: ExportProfile::default(),
        output_dir: tmp.path().to_path_buf(),
        template: NamingTemplate::default(),
        seq_number: 1,
        collision_policy: CollisionPolicy::Skip,
    };

    let res = pipeline
        .execute_export(None, &job)
        .await
        .expect("Export execution failed");
    match res {
        ExportOutcome::Skipped {
            target_path,
            reason,
        } => {
            assert_eq!(target_path, target);
            assert!(reason.contains("already exists"));
        }
        _ => panic!("Expected ExportOutcome::Skipped"),
    }

    // Verify disk content was untouched
    let content = std::fs::read(&target).unwrap();
    assert_eq!(content, b"ORIGINAL");
}

#[tokio::test]
async fn test_t1_exp09_collision_policy_overwrite() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let pipeline = ExportPipeline::new();

    let target = tmp.path().join("IMG_0969__Cinema__01.jpg");
    File::create(&target)
        .unwrap()
        .write_all(b"OLD CONTENT")
        .unwrap();

    let job = ExportJob {
        job_id: 30,
        source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
        photo_id: Some(30),
        recipe_name: "Cinema".into(),
        recipe: BuiltinPreset::Cinema.recipe(),
        profile: ExportProfile::default(),
        output_dir: tmp.path().to_path_buf(),
        template: NamingTemplate::default(),
        seq_number: 1,
        collision_policy: CollisionPolicy::Overwrite,
    };

    let res = pipeline
        .execute_export(None, &job)
        .await
        .expect("Export execution failed");
    match res {
        ExportOutcome::Success { output_path, .. } => {
            assert_eq!(output_path, target);
            let content = std::fs::read(&output_path).unwrap();
            assert_ne!(content, b"OLD CONTENT");
            // Must be valid JPEG
            assert_eq!(&content[0..2], &[0xFF, 0xD8]);
        }
        _ => panic!("Expected ExportOutcome::Success"),
    }
}

// ============================================================================
// 5. Security Sandboxing & Immutability Verification (F-SEC-01, F-RNF-03)
// ============================================================================

#[test]
fn test_t1_exp10_path_traversal_rejection() {
    let source_dirs = [PathBuf::from("DRAW")];

    // Various traversal payloads
    let payloads = [
        "../../Windows/System32/image.jpg",
        r"..\..\evil.jpg",
        "sub/../../DRAW/image.jpg",
        "%2e%2e%2fmalicious.jpg",
    ];

    for payload in &payloads {
        let p = Path::new(payload);
        let res = validate_export_path(p, &source_dirs);
        assert!(
            matches!(res, Err(ExportError::PathTraversal(_))),
            "Payload '{}' should trigger PathTraversal error, got {:?}",
            payload,
            res
        );
    }
}

#[test]
fn test_t1_exp11_source_directory_sandbox_veto() {
    let draw_dir = find_draw_dir();
    let source_dirs = [draw_dir.clone()];

    // Attempting to export directly into ./DRAW/ must be vetoed
    let inside_draw = draw_dir.join("output.jpg");
    let res = validate_export_path(&inside_draw, &source_dirs);
    assert!(
        matches!(
            res,
            Err(ExportError::Integrity(
                IntegrityViolationError::SourceDirectoryContamination { .. }
            ))
        ),
        "Targeting source folder must return SourceDirectoryContamination, got {:?}",
        res
    );
}

#[test]
fn test_t1_exp12_prohibited_target_extension_veto() {
    let source_dirs = [PathBuf::from("DRAW")];

    // Exporting with .CR3 or .cr3 extension must be rejected unconditionally
    let target_upper = Path::new("export/test.CR3");
    let res_upper = validate_export_path(target_upper, &source_dirs);
    assert!(
        matches!(
            res_upper,
            Err(ExportError::ProhibitedTargetExtension { .. })
        ),
        "Target with .CR3 extension must trigger ProhibitedTargetExtension error"
    );

    let target_lower = Path::new("export/test.cr3");
    let res_lower = validate_export_path(target_lower, &source_dirs);
    assert!(
        matches!(
            res_lower,
            Err(ExportError::ProhibitedTargetExtension { .. })
        ),
        "Target with .cr3 extension must trigger ProhibitedTargetExtension error"
    );
}

#[test]
fn test_t1_exp13_source_immutability_baseline() {
    let draw_dir = find_draw_dir();
    assert!(draw_dir.exists(), "DRAW directory must exist");

    let results = verify_cr3_baseline(&draw_dir);
    match results {
        Ok(baseline_results) => {
            for record in &baseline_results {
                assert!(
                    record.is_valid,
                    "CR3 master file {} corrupted! Expected sha256={}, actual={}",
                    record.filename, record.expected_sha256, record.actual_sha256
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
                    assert_eq!(
                        size, entry.expected_size,
                        "Size mismatch for {}",
                        entry.filename
                    );
                    assert!(
                        hash.eq_ignore_ascii_case(entry.expected_sha256),
                        "SHA-256 mismatch for {}",
                        entry.filename
                    );
                    checked_any = true;
                }
            }
            if !checked_any {
                panic!("Baseline CR3 verification failed: {}", missing_err);
            }
        }
    }
}
