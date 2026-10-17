#![allow(dead_code, unused_imports, unused_variables)]

//! Tier 4 Real-World Production Workloads with Canon CR3 RAW Master Files:
//! Validates end-to-end production pipelines on real 24MP photographic RAW assets,
//! enforcing strict non-destructive source immutability, genuine raster image generation,
//! multi-format encoding (JPEG, PNG, 16-bit TIFF), collision policies, and fault tolerance.
//!
//! Authoritative Specification: TEST_INFRA.md §3 Tier 4
//! - Workload 1: Full Canon CR3 Multi-Recipe Production Batch (5 CR3s x 4 recipes = 20 JPEGs)
//! - Workload 2: Multi-Format Archival Master Export (16-bit TIFF, 8-bit PNG, lossy JPEG)
//! - Workload 3: Collision Stress & Re-Run Idempotency (Rename, Skip, Overwrite)
//! - Workload 4: Mode A Interactive Creative Session (Loupe, Split-View, Metadata, Quick Export)
//! - Workload 5: Fault-Injected Recovery & Queue Resiliency (Worker Isolation, Immutability)

mod common;

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;

use image::{DynamicImage, GenericImageView, RgbImage};
use ratatui::layout::Rect;
use ratatui_image::FontSize;

use lightcraft_tui::engine::presets::BuiltinPreset;
use lightcraft_tui::engine::recipe::{DevelopSlider, Recipe, RecipeCategory};
use lightcraft_tui::export::{
    BitDepth, CollisionPolicy, CollisionResolution, CollisionResolver, ExportError, ExportJob,
    ExportOutcome, ExportPipeline, ExportProfile, NamingContext, NamingTemplate, OutputFormat,
    ScalingProfile,
};
use lightcraft_tui::graphics::protocol::{GraphicsProtocol, ProtocolOverride};
use lightcraft_tui::graphics::renderer::ImageRenderer;
use lightcraft_tui::mcp::integrity::{find_draw_dir, verify_cr3_baseline, IntegrityEngine};
use lightcraft_tui::queue::{
    AspectRatio, BatchFilter, BatchJobStatus, BatchStrategy, BatchWorkerPool, DirectoryScanner,
    DirectoryScannerConfig, Orientation, ScannedPhoto, StrategyEngine, WorkerPoolConfig,
};
use lightcraft_tui::storage::presets_store::{PresetFile, PresetFormat, PresetMetadata, PresetStore};
use lightcraft_tui::tui::mode_a::{
    composite_wiper, compute_variant_grid_rects, ColorLabel, FlagStatus, HistogramData, LoupeState,
    ModeAState, ModeAView, PhotoMetadata, SplitViewKind, VariantRenderStatus, VariantSlot,
};

// ============================================================================
// Workload 1: Full Canon CR3 Multi-Recipe Production Batch (5 CR3s x 4 recipes = 20 JPEGs)
// ============================================================================

#[tokio::test]
async fn test_tier4_workload1_full_cr3_multi_recipe_production_batch() {
    let draw_dir = find_draw_dir();
    assert!(draw_dir.exists(), "DRAW directory must exist");

    // 1. Cryptographic pre-flight SHA-256 baseline check
    let pre_baseline = verify_cr3_baseline(&draw_dir).expect("Pre-flight SHA-256 baseline check");
    assert_eq!(pre_baseline.len(), 5, "Must have exactly 5 Canon CR3 master files");

    let tmp = tempdir().expect("tempdir");
    let out_dir = tmp.path().join("client_gallery");
    std::fs::create_dir_all(&out_dir).expect("create client_gallery dir");
    let pipeline = ExportPipeline::new();

    let cr3_files = [
        "IMG_0588.CR3",
        "IMG_0589.CR3",
        "IMG_0590.CR3",
        "IMG_0969.CR3",
        "IMG_0981.CR3",
    ];

    let presets = [
        BuiltinPreset::Natural,
        BuiltinPreset::Cinema,
        BuiltinPreset::BwHighContrast,
        BuiltinPreset::TealOrange,
    ];

    let profile = ExportProfile {
        name: "Client_Web_2048".into(),
        format: OutputFormat::Jpeg,
        quality: 90,
        bit_depth: BitDepth::Eight,
        scaling: ScalingProfile::LongEdge {
            max_long_edge: 2048,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    let template = NamingTemplate::parse("{name}__{preset}.{ext}");

    let mut executed_jobs = 0;
    let mut generated_paths = Vec::new();

    for (file_idx, &cr3_name) in cr3_files.iter().enumerate() {
        let cr3_path = draw_dir.join(cr3_name);
        assert!(cr3_path.exists(), "CR3 master file must exist: {}", cr3_name);

        for (preset_idx, preset) in presets.iter().enumerate() {
            executed_jobs += 1;
            let job = ExportJob {
                job_id: executed_jobs as u64,
                source_path: cr3_path.clone(),
                photo_id: Some((file_idx * 10 + preset_idx) as i64),
                recipe_name: preset.display_name().to_string(),
                recipe: preset.recipe(),
                profile: profile.clone(),
                output_dir: out_dir.clone(),
                template: template.clone(),
                seq_number: executed_jobs,
                collision_policy: CollisionPolicy::Rename,
            };

            let res = pipeline.execute_export(None, &job).await;
            assert!(
                res.is_ok(),
                "Failed to export {} with preset {}: {:?}",
                cr3_name,
                preset.display_name(),
                res.err()
            );

            let outcome = res.unwrap();
            match outcome {
                ExportOutcome::Success { output_path, width, height, bytes_written, .. } => {
                    assert!(output_path.exists(), "Output file must exist on disk");
                    assert!(bytes_written > 10_000, "Output file too small (bytes={})", bytes_written);
                    assert!(width <= 2048 && height <= 2048, "Dimensions must fit within 2048px");
                    assert!(width > 500 && height > 500, "Dimensions too small: {}x{}", width, height);

                    // Verify real JPEG SOI marker
                    let mut file = File::open(&output_path).expect("open exported file");
                    let mut magic = [0u8; 2];
                    file.read_exact(&mut magic).expect("read magic bytes");
                    assert_eq!(magic, [0xFF, 0xD8], "Must be valid JPEG file");

                    generated_paths.push(output_path);
                }
                other => panic!("Expected ExportOutcome::Success, got {:?}", other),
            }
        }
    }

    assert_eq!(executed_jobs, 20, "Must have executed exactly 20 jobs (5 photos x 4 recipes)");
    assert_eq!(generated_paths.len(), 20, "Must have generated exactly 20 output JPEGs");

    // 2. Cryptographic post-flight SHA-256 baseline check: 100% byte-for-byte immutability
    let post_baseline = verify_cr3_baseline(&draw_dir).expect("Post-flight SHA-256 baseline check");
    assert_eq!(post_baseline.len(), 5);
    for (pre, post) in pre_baseline.iter().zip(post_baseline.iter()) {
        assert_eq!(
            pre.actual_sha256, post.actual_sha256,
            "CR3 source file {} modified during production batch!",
            pre.filename
        );
        assert_eq!(pre.actual_size, post.actual_size);
    }
}

// ============================================================================
// Workload 2: Multi-Format Archival Master Export (16-bit TIFF, 8-bit PNG, lossy JPEG)
// ============================================================================

#[tokio::test]
async fn test_tier4_workload2_multi_format_archival_master_export() {
    let draw_dir = find_draw_dir();
    let pre_baseline = verify_cr3_baseline(&draw_dir).expect("Pre-flight SHA-256 baseline check");

    let tmp = tempdir().expect("tempdir");
    let out_dir = tmp.path().join("archival_masters");
    std::fs::create_dir_all(&out_dir).expect("create archival_masters dir");
    let pipeline = ExportPipeline::new();

    let master_cr3 = draw_dir.join("IMG_0588.CR3");
    let recipe = BuiltinPreset::Natural.recipe();

    // 1. Export 16-bit TIFF (Native resolution)
    let tiff_profile = ExportProfile {
        name: "Archival_16bit_TIFF".into(),
        format: OutputFormat::Tiff,
        quality: 100,
        bit_depth: BitDepth::Sixteen,
        scaling: ScalingProfile::FitToBounds {
            max_width: 1920,
            max_height: 1280,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    let tiff_job = ExportJob {
        job_id: 101,
        source_path: master_cr3.clone(),
        photo_id: Some(588),
        recipe_name: "Natural".into(),
        recipe: recipe.clone(),
        profile: tiff_profile,
        output_dir: out_dir.clone(),
        template: NamingTemplate::parse("{name}__master_16bit.{ext}"),
        seq_number: 1,
        collision_policy: CollisionPolicy::Overwrite,
    };

    let tiff_res = pipeline.execute_export(None, &tiff_job).await;
    assert!(tiff_res.is_ok(), "16-bit TIFF export failed: {:?}", tiff_res.err());
    let tiff_path = match tiff_res.unwrap() {
        ExportOutcome::Success { output_path, width, height, bytes_written, .. } => {
            assert!(output_path.exists());
            assert!(bytes_written > 100_000, "16-bit TIFF should be large");
            // Check TIFF header (II or MM)
            let bytes = std::fs::read(&output_path).unwrap();
            let is_le = &bytes[0..4] == &[0x49, 0x49, 0x2A, 0x00];
            let is_be = &bytes[0..4] == &[0x4D, 0x4D, 0x00, 0x2A];
            assert!(is_le || is_be, "TIFF magic header mismatch");
            output_path
        }
        other => panic!("Expected Success, got {:?}", other),
    };

    // 2. Export 8-bit PNG (1080p fit)
    let png_profile = ExportProfile {
        name: "Presentation_PNG".into(),
        format: OutputFormat::Png,
        quality: 100,
        bit_depth: BitDepth::Eight,
        scaling: ScalingProfile::FitToBounds {
            max_width: 1920,
            max_height: 1080,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    let png_job = ExportJob {
        job_id: 102,
        source_path: master_cr3.clone(),
        photo_id: Some(588),
        recipe_name: "Natural".into(),
        recipe: recipe.clone(),
        profile: png_profile,
        output_dir: out_dir.clone(),
        template: NamingTemplate::parse("{name}__presentation.{ext}"),
        seq_number: 2,
        collision_policy: CollisionPolicy::Overwrite,
    };

    let png_res = pipeline.execute_export(None, &png_job).await;
    assert!(png_res.is_ok(), "PNG export failed: {:?}", png_res.err());
    let png_path = match png_res.unwrap() {
        ExportOutcome::Success { output_path, width, height, bytes_written, .. } => {
            assert!(output_path.exists());
            let bytes = std::fs::read(&output_path).unwrap();
            assert_eq!(&bytes[0..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
            output_path
        }
        other => panic!("Expected Success, got {:?}", other),
    };

    // 3. Export lossy JPEG (quality 80)
    let jpeg_profile = ExportProfile {
        name: "Web_Delivery_JPEG".into(),
        format: OutputFormat::Jpeg,
        quality: 80,
        bit_depth: BitDepth::Eight,
        scaling: ScalingProfile::FitToBounds {
            max_width: 1920,
            max_height: 1080,
            dont_enlarge: false,
        },
        ..Default::default()
    };

    let jpeg_job = ExportJob {
        job_id: 103,
        source_path: master_cr3.clone(),
        photo_id: Some(588),
        recipe_name: "Natural".into(),
        recipe: recipe.clone(),
        profile: jpeg_profile,
        output_dir: out_dir.clone(),
        template: NamingTemplate::parse("{name}__delivery.{ext}"),
        seq_number: 3,
        collision_policy: CollisionPolicy::Overwrite,
    };

    let jpeg_res = pipeline.execute_export(None, &jpeg_job).await;
    assert!(jpeg_res.is_ok(), "JPEG export failed: {:?}", jpeg_res.err());
    let jpeg_path = match jpeg_res.unwrap() {
        ExportOutcome::Success { output_path, width, height, bytes_written, .. } => {
            assert!(output_path.exists());
            let bytes = std::fs::read(&output_path).unwrap();
            assert_eq!(&bytes[0..2], &[0xFF, 0xD8]);
            output_path
        }
        other => panic!("Expected Success, got {:?}", other),
    };

    // Confirm all 3 files exist
    assert!(tiff_path.exists());
    assert!(png_path.exists());
    assert!(jpeg_path.exists());

    // Post-flight immutability check
    let post_baseline = verify_cr3_baseline(&draw_dir).expect("Post-flight SHA-256 check");
    for (pre, post) in pre_baseline.iter().zip(post_baseline.iter()) {
        assert_eq!(pre.actual_sha256, post.actual_sha256);
        assert_eq!(pre.actual_size, post.actual_size);
    }
}

// ============================================================================
// Workload 3: Collision Stress & Re-Run Idempotency (Rename, Skip, Overwrite)
// ============================================================================

#[tokio::test]
async fn test_tier4_workload3_collision_stress_and_rerun_idempotency() {
    let draw_dir = find_draw_dir();
    let pre_baseline = verify_cr3_baseline(&draw_dir).expect("Pre-flight SHA-256 check");

    let tmp = tempdir().expect("tempdir");
    let out_dir = tmp.path().join("collision_test");
    std::fs::create_dir_all(&out_dir).expect("create dir");
    let pipeline = ExportPipeline::new();

    let job_template = ExportJob {
        job_id: 1,
        source_path: draw_dir.join("IMG_0590.CR3"),
        photo_id: Some(590),
        recipe_name: "Cinema".into(),
        recipe: BuiltinPreset::Cinema.recipe(),
        profile: ExportProfile {
            name: "TestWeb".into(),
            format: OutputFormat::Jpeg,
            quality: 85,
            bit_depth: BitDepth::Eight,
            scaling: ScalingProfile::FitToBounds {
                max_width: 800,
                max_height: 600,
                dont_enlarge: false,
            },
            ..Default::default()
        },
        output_dir: out_dir.clone(),
        template: NamingTemplate::parse("{name}__{preset}.{ext}"),
        seq_number: 1,
        collision_policy: CollisionPolicy::Rename,
    };

    // 1. Initial write
    let res1 = pipeline.execute_export(None, &job_template).await.unwrap();
    let path1 = match res1 {
        ExportOutcome::Success { output_path, .. } => output_path,
        other => panic!("Expected Success, got {:?}", other),
    };
    assert!(path1.exists());
    assert_eq!(path1.file_name().unwrap(), "IMG_0590__Cinema.jpg");

    // 2. Re-run with Rename -> should create _1.jpg
    let mut job_rename = job_template.clone();
    job_rename.job_id = 2;
    job_rename.collision_policy = CollisionPolicy::Rename;
    let res2 = pipeline.execute_export(None, &job_rename).await.unwrap();
    let path2 = match res2 {
        ExportOutcome::Success { output_path, .. } => output_path,
        other => panic!("Expected Success, got {:?}", other),
    };
    assert!(path2.exists());
    assert_eq!(path2.file_name().unwrap(), "IMG_0590__Cinema_1.jpg");

    // 3. Re-run with Skip -> should return Skipped, 0 disk writes
    let mut job_skip = job_template.clone();
    job_skip.job_id = 3;
    job_skip.collision_policy = CollisionPolicy::Skip;
    let res3 = pipeline.execute_export(None, &job_skip).await.unwrap();
    match res3 {
        ExportOutcome::Skipped { target_path, reason } => {
            assert_eq!(target_path.file_name().unwrap(), "IMG_0590__Cinema.jpg");
            assert!(reason.contains("Skip"));
        }
        other => panic!("Expected Skipped, got {:?}", other),
    }

    // 4. Re-run with Overwrite -> overwrites file without suffix
    let mut job_overwrite = job_template.clone();
    job_overwrite.job_id = 4;
    job_overwrite.collision_policy = CollisionPolicy::Overwrite;
    std::thread::sleep(Duration::from_millis(15));
    let res4 = pipeline.execute_export(None, &job_overwrite).await.unwrap();
    match res4 {
        ExportOutcome::Success { output_path, .. } => {
            assert_eq!(output_path, path1);
        }
        other => panic!("Expected Success, got {:?}", other),
    }

    // Verify exactly 2 files exist in folder: IMG_0590__Cinema.jpg and IMG_0590__Cinema_1.jpg
    let entries: Vec<_> = std::fs::read_dir(&out_dir).unwrap().collect();
    assert_eq!(entries.len(), 2);

    let post_baseline = verify_cr3_baseline(&draw_dir).expect("Post-flight SHA-256 check");
    for (pre, post) in pre_baseline.iter().zip(post_baseline.iter()) {
        assert_eq!(pre.actual_sha256, post.actual_sha256);
        assert_eq!(pre.actual_size, post.actual_size);
    }
}

// ============================================================================
// Workload 4: Mode A Interactive Creative Session (Loupe, Split-View, Quick Export)
// ============================================================================

#[tokio::test]
async fn test_tier4_workload4_mode_a_interactive_creative_session() {
    let draw_dir = find_draw_dir();
    let pre_baseline = verify_cr3_baseline(&draw_dir).expect("Pre-flight check");

    let tmp = tempdir().expect("tempdir");
    let out_dir = tmp.path().join("session_export");
    std::fs::create_dir_all(&out_dir).expect("create session_export dir");
    let pipeline = ExportPipeline::new();

    // 1. Simulate Mode A state machine for IMG_0969.CR3
    let mut state = ModeAState {
        active_photo: None,
        metadata: PhotoMetadata {
            file_name: "IMG_0969.CR3".into(),
            file_size_bytes: 26_614_589,
            format: "Canon CR3 RAW".into(),
            width: 6000,
            height: 4000,
            orientation: "Landscape".into(),
            color_space: "sRGB".into(),
            camera_make: Some("Canon".into()),
            camera_model: Some("Canon EOS R".into()),
            lens_model: Some("RF 24-105mm F4 L IS USM".into()),
            focal_length_mm: Some(50.0),
            aperture_f_number: Some(4.0),
            shutter_speed: Some("1/250".into()),
            iso: Some(200),
            exposure_bias_ev: Some(0.0),
            metering_mode: Some("Pattern".into()),
            white_balance: Some("Daylight".into()),
            capture_time: Some("2026-10-15T14:30:00Z".into()),
        },
        view_mode: ModeAView::Hero,
        baseline_recipe: Recipe::default(),
        active_recipe: BuiltinPreset::Natural.recipe(),
        active_intensity: 1.0,
        base_image: None,
        cached_protocol: None,
        variant_count: 4,
        focused_variant: 0,
        variants: vec![
            VariantSlot::new(0, "Natural".into(), BuiltinPreset::Natural.recipe()),
            VariantSlot::new(1, "Cinema".into(), BuiltinPreset::Cinema.recipe()),
            VariantSlot::new(2, "B&W High Contrast".into(), BuiltinPreset::BwHighContrast.recipe()),
            VariantSlot::new(3, "Teal & Orange".into(), BuiltinPreset::TealOrange.recipe()),
        ],
        split_kind: SplitViewKind::SideBySide,
        split_ratio: 0.5,
        toggle_ab_showing_before: false,
        loupe: LoupeState::default(),
        show_info_overlay: true,
        histogram: None,
    };

    assert_eq!(state.metadata.aspect_ratio_str(), "3:2");
    assert_eq!(state.metadata.megapixels_str(), "24.0 MP");

    // 2. Transition Hero -> VariantMatrix
    state.view_mode = ModeAView::VariantMatrix;
    assert_eq!(state.view_mode, ModeAView::VariantMatrix);

    // 3. User selects Variant 1 (Cinema) and promotes it to active recipe
    state.focused_variant = 1;
    state.active_recipe = state.variants[1].recipe.clone();
    state.view_mode = ModeAView::Hero;
    assert_eq!(state.active_recipe.name, "Cinema");

    // 4. Intensity adjustment from 100% to 140%
    state.active_intensity = 1.4;
    let scaled_recipe = state.active_recipe.apply_intensity(state.active_intensity);
    assert!((scaled_recipe.contrast - (state.active_recipe.contrast * 1.4)).abs() < 1e-4);

    // 5. Transition to SplitView
    state.view_mode = ModeAView::SplitView;
    state.split_ratio = 0.65;
    assert_eq!(state.split_ratio, 0.65);

    // 6. Test Hero transition and crop computation
    state.view_mode = ModeAView::Hero;
    state.loupe.pan(0.1, -0.1);
    let (cx, cy, cw, ch) = state.loupe.compute_crop_rect(6000, 4000, 1920, 1080);
    assert!(cw <= 6000 && ch <= 4000);
    assert!(cx <= 6000 && cy <= 4000);

    // 7. Flag favorite
    state.variants[1].is_favorite = true;
    assert!(state.variants[1].is_favorite);

    // 8. Quick Export of active recipe
    let export_job = ExportJob {
        job_id: 201,
        source_path: draw_dir.join("IMG_0969.CR3"),
        photo_id: Some(969),
        recipe_name: state.active_recipe.name.clone(),
        recipe: scaled_recipe,
        profile: ExportProfile {
            name: "QuickExportWeb".into(),
            format: OutputFormat::Jpeg,
            quality: 90,
            bit_depth: BitDepth::Eight,
            scaling: ScalingProfile::FitToBounds {
                max_width: 1200,
                max_height: 800,
                dont_enlarge: false,
            },
            ..Default::default()
        },
        output_dir: out_dir.clone(),
        template: NamingTemplate::parse("{name}__quickexport.{ext}"),
        seq_number: 1,
        collision_policy: CollisionPolicy::Overwrite,
    };

    let res = pipeline.execute_export(None, &export_job).await;
    assert!(res.is_ok(), "Quick export failed: {:?}", res.err());
    let outcome = res.unwrap();
    if let ExportOutcome::Success { output_path, .. } = outcome {
        assert!(output_path.exists());
    } else {
        panic!("Expected Success");
    }

    let post_baseline = verify_cr3_baseline(&draw_dir).expect("Post-flight check");
    for (pre, post) in pre_baseline.iter().zip(post_baseline.iter()) {
        assert_eq!(pre.actual_sha256, post.actual_sha256);
        assert_eq!(pre.actual_size, post.actual_size);
    }
}

// ============================================================================
// Workload 5: Fault-Injected Recovery & Queue Resiliency
// ============================================================================

#[tokio::test]
async fn test_tier4_workload5_fault_injected_recovery_and_queue_resiliency() {
    let draw_dir = find_draw_dir();
    let pre_baseline = verify_cr3_baseline(&draw_dir).expect("Pre-flight check");

    let pool = BatchWorkerPool::new(WorkerPoolConfig { concurrency: 2 }, None);

    // 1. Queue containing valid jobs and an invalid/failing job (pointing to non-existent file)
    let photos = vec![
        ScannedPhoto {
            path: draw_dir.join("IMG_0588.CR3"),
            filename: "IMG_0588.CR3".into(),
            extension: "cr3".into(),
            size_bytes: 26_527_598,
            is_cr3: true,
            is_selected: true,
            dimensions: Some((6000, 4000)),
            orientation: Some(Orientation::Landscape),
            aspect_ratio: Some(AspectRatio::Ratio3x2),
        },
        ScannedPhoto {
            path: PathBuf::from("DRAW/NON_EXISTENT_CORRUPT.CR3"),
            filename: "NON_EXISTENT_CORRUPT.CR3".into(),
            extension: "cr3".into(),
            size_bytes: 0,
            is_cr3: true,
            is_selected: true,
            dimensions: None,
            orientation: None,
            aspect_ratio: None,
        },
        ScannedPhoto {
            path: draw_dir.join("IMG_0589.CR3"),
            filename: "IMG_0589.CR3".into(),
            extension: "cr3".into(),
            size_bytes: 28_369_647,
            is_cr3: true,
            is_selected: true,
            dimensions: Some((6000, 4000)),
            orientation: Some(Orientation::Landscape),
            aspect_ratio: Some(AspectRatio::Ratio3x2),
        },
    ];

    let strategy = BatchStrategy::Uniform {
        recipe: BuiltinPreset::Natural.recipe(),
        intensity: 1.0,
    };

    let jobs = StrategyEngine::expand_jobs(&photos, &strategy, "web_1080p").unwrap();
    assert_eq!(jobs.len(), 3);

    let results = pool.run_queue(jobs, None).await;
    assert_eq!(results.len(), 3, "All 3 jobs must be tracked in results");

    // Job 1 (valid) should be Done
    assert!(matches!(results[0].status, BatchJobStatus::Done { .. }));

    // Job 2 (missing file) should be Failed without crashing worker pool
    assert!(matches!(results[1].status, BatchJobStatus::Failed { .. }));

    // Job 3 (valid) should be Done despite prior failure
    assert!(matches!(results[2].status, BatchJobStatus::Done { .. }));

    // Master files remain 100% immutable
    let post_baseline = verify_cr3_baseline(&draw_dir).expect("Post-flight check");
    for (pre, post) in pre_baseline.iter().zip(post_baseline.iter()) {
        assert_eq!(pre.actual_sha256, post.actual_sha256);
        assert_eq!(pre.actual_size, post.actual_size);
    }
}
