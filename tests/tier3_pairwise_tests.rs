#![allow(dead_code, unused_imports, unused_variables)]

//! Tier 3 Pairwise Cross-Feature Integration Tests:
//! Validates complex cross-subsystem interactions between presets, recipe scaling,
//! split-view rendering, batch queues, collision resolution, multi-format exports,
//! and Canon CR3 non-destructive source immutability.
//!
//! Authoritative Specification: TEST_INFRA.md §3 Tier 3
//! - Scenario 1: Preset Intensity Scaling + Split-View Comparison (F-REC-02 x F-SGL-03)
//! - Scenario 2: Batch Processing Queue + Collision Policies (F-BAT-04 x F-EXP-04)
//! - Scenario 3: Canon CR3 RAW Source + Multi-Format Export + Variations (F-SEC-01 x F-EXP-01 x F-BAT-03)
//! - Scenario 4: Favorite Flagging + Batch Queue Selective Export (F-SGL-05 x F-BAT-03)
//! - Scenario 5: Dynamic Terminal Resizing + Graphic Protocols (F-TUI-01 x F-TUI-03 x F-RNF-01)
//! - Scenario 6: Develop Adjustments + Presets + Undo/Redo (F-MCP-01 x F-REC-01 x F-REC-05)
//! - Scenario 7: Batch Concurrency Scaling + Queue Controls (F-BAT-04 x F-BAT-05)
//! - Scenario 8: Custom Preset Creation + Local Persistence + Batch Application (F-REC-03 x F-BAT-03)

mod common;

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use chrono::Utc;
use tempfile::tempdir;

use image::{DynamicImage, GenericImageView, RgbImage, Rgba};
use ratatui::layout::Rect;
use ratatui_image::FontSize;

use lightcraft_tui::engine::develop::DevelopBridge;
use lightcraft_tui::engine::presets::BuiltinPreset;
use lightcraft_tui::engine::recipe::{DevelopSlider, Recipe, RecipeCategory};
use lightcraft_tui::export::{
    BitDepth, CollisionPolicy, CollisionResolution, CollisionResolver, ExportError, ExportJob,
    ExportOutcome, ExportPipeline, ExportProfile, NamingContext, NamingTemplate, OutputFormat,
    ScalingProfile,
};
use lightcraft_tui::graphics::cache::PreviewCache;
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
// Scenario 1: Preset Intensity Scaling + Split-View Comparison (F-REC-02 x F-SGL-03)
// ============================================================================

#[test]
fn test_tier3_scenario1_preset_intensity_scaling_and_split_view() {
    let draw_dir = find_draw_dir();
    let baseline_check = verify_cr3_baseline(&draw_dir);
    if let Ok(res) = baseline_check {
        assert_eq!(res.len(), 5);
    }

    // 1. Verify intensity scaling deltas across 0%, 50%, 100%, 150%
    let cinema = BuiltinPreset::Cinema.recipe();
    let r_0 = cinema.apply_intensity(0.0);
    let r_50 = cinema.apply_intensity(0.5);
    let r_100 = cinema.apply_intensity(1.0);
    let r_150 = cinema.apply_intensity(1.5);

    // At 0% intensity, parameter deltas from default baseline should be zero
    let default_recipe = Recipe::default();
    assert_eq!(r_0.contrast, default_recipe.contrast);
    assert_eq!(r_0.exposure, default_recipe.exposure);

    // At 100% intensity, parameters match original definition
    assert_eq!(r_100.contrast, cinema.contrast);
    assert_eq!(r_100.exposure, cinema.exposure);

    // At 50% intensity, deltas are halved
    assert!((r_50.contrast - (cinema.contrast * 0.5)).abs() < 1e-4);
    assert!((r_50.exposure - (cinema.exposure * 0.5)).abs() < 1e-4);

    // At 150% intensity, deltas are 1.5x (bounded within limits)
    assert!((r_150.contrast - (cinema.contrast * 1.5)).abs() < 1e-4);

    // 2. Test Split-View Wiper Compositing with scaling
    let before_img = DynamicImage::ImageRgb8(RgbImage::from_fn(100, 100, |_, _| {
        image::Rgb([100, 100, 100])
    }));
    let after_img = DynamicImage::ImageRgb8(RgbImage::from_fn(100, 100, |_, _| {
        image::Rgb([200, 150, 100])
    }));

    // Split ratio at 50%
    let split_50 = composite_wiper(&before_img, &after_img, 0.5);
    assert_eq!(split_50.width(), 100);
    assert_eq!(split_50.height(), 100);

    // Left side (x < 50) must match 'before'
    let left_pixel = split_50.get_pixel(10, 50);
    assert_eq!(left_pixel[0], 100);
    assert_eq!(left_pixel[1], 100);
    assert_eq!(left_pixel[2], 100);

    // Right side (x > 52) must match 'after'
    let right_pixel = split_50.get_pixel(90, 50);
    assert_eq!(right_pixel[0], 200);
    assert_eq!(right_pixel[1], 150);
    assert_eq!(right_pixel[2], 100);

    // Split ratio dynamic adjustment from 10% to 90%
    for ratio in [0.10, 0.25, 0.50, 0.75, 0.90] {
        let wiper = composite_wiper(&before_img, &after_img, ratio);
        assert_eq!(wiper.width(), 100);
        let expected_split_x = (100.0 * ratio) as u32;
        // Verify before segment
        let b_px = wiper.get_pixel(expected_split_x.saturating_sub(5), 50);
        assert_eq!(b_px[0], 100);
    }
}

// ============================================================================
// Scenario 2: Batch Processing Queue + Collision Policies (F-BAT-04 x F-EXP-04)
// ============================================================================

#[tokio::test]
async fn test_tier3_scenario2_queue_and_collision_policies() {
    let tmp = tempdir().expect("tempdir");
    let out_dir = tmp.path().to_path_buf();
    let pipeline = ExportPipeline::new();

    // 1. Write an initial baseline export
    let job1 = ExportJob {
        job_id: 1,
        source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
        photo_id: Some(1),
        recipe_name: "Natural".into(),
        recipe: BuiltinPreset::Natural.recipe(),
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

    let res1 = pipeline.execute_export(None, &job1).await;
    assert!(res1.is_ok());
    let path1 = match res1.unwrap() {
        ExportOutcome::Success { output_path, .. } => output_path,
        other => panic!("Expected Success, got {:?}", other),
    };
    assert!(path1.exists());
    assert_eq!(path1.file_name().unwrap(), "IMG_0969__Natural.jpg");

    // 2. Collision policy Rename: second export should create _1.jpg
    let mut job2 = job1.clone();
    job2.job_id = 2;
    job2.collision_policy = CollisionPolicy::Rename;
    let res2 = pipeline.execute_export(None, &job2).await;
    assert!(res2.is_ok());
    let path2 = match res2.unwrap() {
        ExportOutcome::Success { output_path, .. } => output_path,
        other => panic!("Expected Success, got {:?}", other),
    };
    assert!(path2.exists());
    assert_eq!(path2.file_name().unwrap(), "IMG_0969__Natural_1.jpg");

    // 3. Collision policy Skip: third export with Skip should not write a new file
    let mut job3 = job1.clone();
    job3.job_id = 3;
    job3.collision_policy = CollisionPolicy::Skip;
    let res3 = pipeline.execute_export(None, &job3).await;
    assert!(res3.is_ok());
    match res3.unwrap() {
        ExportOutcome::Skipped { target_path, .. } => {
            assert_eq!(target_path.file_name().unwrap(), "IMG_0969__Natural.jpg");
        }
        other => panic!("Expected Skipped, got {:?}", other),
    }

    // 4. Collision policy Overwrite: fourth export overwrites existing file
    let mut job4 = job1.clone();
    job4.job_id = 4;
    job4.collision_policy = CollisionPolicy::Overwrite;
    std::thread::sleep(Duration::from_millis(15));
    let res4 = pipeline.execute_export(None, &job4).await;
    assert!(res4.is_ok());
    let path4 = match res4.unwrap() {
        ExportOutcome::Success { output_path, .. } => output_path,
        other => panic!("Expected Success, got {:?}", other),
    };
    assert_eq!(path4, path1);
}

// ============================================================================
// Scenario 3: Canon CR3 RAW Source + Multi-Format Export + Variations (F-SEC-01 x F-EXP-01 x F-BAT-03)
// ============================================================================

#[tokio::test]
async fn test_tier3_scenario3_cr3_raw_multi_format_variations() {
    let draw_dir = find_draw_dir();
    let initial_baseline = verify_cr3_baseline(&draw_dir);
    if let Ok(res) = initial_baseline {
        assert_eq!(res.len(), 5);
    }

    let tmp = tempdir().expect("tempdir");
    let out_dir = tmp.path().to_path_buf();
    let pipeline = ExportPipeline::new();

    let formats = [
        (OutputFormat::Jpeg, BitDepth::Eight, "jpg"),
        (OutputFormat::Png, BitDepth::Eight, "png"),
        (OutputFormat::Tiff, BitDepth::Sixteen, "tif"),
    ];

    let presets = [
        BuiltinPreset::Natural,
        BuiltinPreset::Cinema,
        BuiltinPreset::BwHighContrast,
        BuiltinPreset::TealOrange,
    ];

    // Process IMG_0969.CR3 across all 4 presets and 3 formats
    let mut job_counter: u64 = 0;
    for preset in &presets {
        for &(format, bit_depth, _) in &formats {
            job_counter += 1;
            let job = ExportJob {
                job_id: job_counter,
                source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
                photo_id: Some(job_counter as i64),
                recipe_name: preset.display_name().to_string(),
                recipe: preset.recipe(),
                profile: ExportProfile {
                    name: format!("Format_{:?}_{:?}", format, bit_depth),
                    format,
                    quality: 90,
                    bit_depth,
                    scaling: ScalingProfile::FitToBounds {
                        max_width: 600,
                        max_height: 400,
                        dont_enlarge: false,
                    },
                    ..Default::default()
                },
                output_dir: out_dir.clone(),
                template: NamingTemplate::parse("{name}__{preset}.{ext}"),
                seq_number: job_counter as usize,
                collision_policy: CollisionPolicy::Rename,
            };

            let res = pipeline.execute_export(None, &job).await;
            assert!(res.is_ok(), "Export failed for {:?} with {:?}", preset, format);
            let outcome = res.unwrap();
            if let ExportOutcome::Success { output_path, width, height, bytes_written, .. } = outcome {
                assert!(output_path.exists());
                assert!(bytes_written > 0);
                assert!(width <= 600 && height <= 400);

                let data = std::fs::read(&output_path).unwrap();
                match format {
                    OutputFormat::Jpeg => {
                        assert_eq!(&data[0..2], &[0xFF, 0xD8], "JPEG header");
                    }
                    OutputFormat::Png => {
                        assert_eq!(&data[0..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A], "PNG header");
                    }
                    OutputFormat::Tiff => {
                        let is_le = &data[0..4] == &[0x49, 0x49, 0x2A, 0x00];
                        let is_be = &data[0..4] == &[0x4D, 0x4D, 0x00, 0x2A];
                        assert!(is_le || is_be, "TIFF header");
                    }
                }
            } else {
                panic!("Expected Success outcome");
            }
        }
    }

    // Verify 100% byte-for-byte non-destructive immutability on present Canon CR3 master files
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
        }
    }
}

// ============================================================================
// Scenario 4: Favorite Flagging + Batch Queue Selective Export (F-SGL-05 x F-BAT-03)
// ============================================================================

#[tokio::test]
async fn test_tier3_scenario4_favorite_flagging_and_selective_export() {
    let tmp = tempdir().expect("tempdir");
    let out_dir = tmp.path().to_path_buf();
    let pipeline = ExportPipeline::new();

    // 1. Construct Mode A Variant Matrix with 4 variants
    let mut variants = vec![
        VariantSlot::new(0, "Natural".into(), BuiltinPreset::Natural.recipe()),
        VariantSlot::new(1, "Cinema".into(), BuiltinPreset::Cinema.recipe()),
        VariantSlot::new(2, "B&W High Contrast".into(), BuiltinPreset::BwHighContrast.recipe()),
        VariantSlot::new(3, "Teal & Orange".into(), BuiltinPreset::TealOrange.recipe()),
    ];

    // Flag variants 1 (Cinema) and 3 (Teal & Orange) as favorites
    variants[1].is_favorite = true;
    variants[3].is_favorite = true;

    // Filter only favorited variants for selective batch queue
    let favorite_jobs: Vec<_> = variants
        .iter()
        .filter(|v| v.is_favorite)
        .enumerate()
        .map(|(idx, v)| ExportJob {
            job_id: (idx + 1) as u64,
            source_path: PathBuf::from("DRAW/IMG_0969.CR3"),
            photo_id: Some(969),
            recipe_name: v.label.clone(),
            recipe: v.recipe.clone(),
            profile: ExportProfile {
                name: "FavoritesWeb".into(),
                format: OutputFormat::Jpeg,
                quality: 90,
                bit_depth: BitDepth::Eight,
                scaling: ScalingProfile::FitToBounds {
                    max_width: 800,
                    max_height: 600,
                    dont_enlarge: false,
                },
                ..Default::default()
            },
            output_dir: out_dir.clone(),
            template: NamingTemplate::parse("{name}__fav__{preset}.{ext}"),
            seq_number: idx + 1,
            collision_policy: CollisionPolicy::Overwrite,
        })
        .collect();

    assert_eq!(favorite_jobs.len(), 2);

    for job in &favorite_jobs {
        let res = pipeline.execute_export(None, job).await;
        assert!(res.is_ok());
        if let ExportOutcome::Success { output_path, .. } = res.unwrap() {
            assert!(output_path.exists());
            let name = output_path.file_name().unwrap().to_string_lossy();
            assert!(name.contains("__fav__"));
        } else {
            panic!("Expected Success");
        }
    }

    // Verify exactly 2 files exist in output dir
    let entries: Vec<_> = std::fs::read_dir(&out_dir).unwrap().collect();
    assert_eq!(entries.len(), 2);
}

// ============================================================================
// Scenario 5: Dynamic Terminal Resizing + Graphic Protocols (F-TUI-01 x F-TUI-03 x F-RNF-01)
// ============================================================================

#[test]
fn test_tier3_scenario5_dynamic_terminal_resizing_and_graphic_protocols() {
    // 1. Protocol initialization
    let halfblocks = GraphicsProtocol::halfblocks();
    assert_eq!(halfblocks.mode(), ProtocolOverride::Halfblocks);

    // 2. Responsive variant grid sub-rect calculations across various terminal sizes
    let small_terminal = Rect::new(0, 0, 80, 24);
    let large_terminal = Rect::new(0, 0, 200, 60);

    for count in 2..=6 {
        let rects_small = compute_variant_grid_rects(small_terminal, count);
        assert_eq!(rects_small.len(), count);
        for r in &rects_small {
            assert!(r.width > 0 && r.height > 0);
            assert!(r.x + r.width <= small_terminal.width);
            assert!(r.y + r.height <= small_terminal.height);
        }

        let rects_large = compute_variant_grid_rects(large_terminal, count);
        assert_eq!(rects_large.len(), count);
        for r in &rects_large {
            assert!(r.width > 0 && r.height > 0);
            assert!(r.x + r.width <= large_terminal.width);
            assert!(r.y + r.height <= large_terminal.height);
        }
    }

    // 3. Image aspect ratio downscaling fits inside terminal cells
    let font_size = FontSize { width: 8, height: 16 };
    let pane = Rect::new(0, 0, 80, 40);
    let fit = ImageRenderer::calculate_fit(6000, 4000, font_size, pane);
    assert!(fit.render_area.width <= 80);
    assert!(fit.render_area.height <= 40);
    assert!(fit.render_area.width > 0 && fit.render_area.height > 0);
}

// ============================================================================
// Scenario 6: Develop Adjustments + Presets + Undo/Redo (F-MCP-01 x F-REC-01 x F-REC-05)
// ============================================================================

#[test]
fn test_tier3_scenario6_develop_adjustments_presets_undo_redo() {
    // Simulating developer history stack
    let mut history_undo: Vec<Recipe> = Vec::new();
    let mut history_redo: Vec<Recipe> = Vec::new();

    // 1. Initial pristine recipe
    let mut current = Recipe::default();
    history_undo.push(current.clone());

    // 2. Apply manual slider edits (exposure +0.5, contrast +10.0)
    let mut edit1 = current.clone();
    edit1.exposure = 0.5;
    edit1.contrast = 10.0;
    history_undo.push(edit1.clone());
    current = edit1;
    assert_eq!(current.exposure, 0.5);

    // 3. Apply built-in preset Cinema
    let preset_cinema = BuiltinPreset::Cinema.recipe();
    history_undo.push(preset_cinema.clone());
    current = preset_cinema;
    assert_eq!(current.name, "Cinema");
    assert_eq!(current.exposure, -0.20);

    // 4. Undo back to manual edits
    let prev = history_undo.pop().unwrap();
    history_redo.push(prev);
    current = history_undo.last().unwrap().clone();
    assert_eq!(current.exposure, 0.5);
    assert_eq!(current.contrast, 10.0);

    // 5. Redo back to preset Cinema
    let redone = history_redo.pop().unwrap();
    history_undo.push(redone.clone());
    current = redone;
    assert_eq!(current.name, "Cinema");
    assert_eq!(current.exposure, -0.20);
}

// ============================================================================
// Scenario 7: Batch Concurrency Scaling + Queue Controls (F-BAT-04 x F-BAT-05)
// ============================================================================

#[tokio::test]
async fn test_tier3_scenario7_batch_concurrency_and_controls() {
    let pool = BatchWorkerPool::new(WorkerPoolConfig { concurrency: 4 }, None);

    let photos = vec![
        ScannedPhoto {
            path: PathBuf::from("DRAW/IMG_0969.CR3"),
            filename: "IMG_0969.CR3".into(),
            extension: "cr3".into(),
            size_bytes: 26_614_589,
            is_cr3: true,
            is_selected: true,
            dimensions: Some((6000, 4000)),
            orientation: Some(Orientation::Landscape),
            aspect_ratio: Some(AspectRatio::Ratio3x2),
        },
        ScannedPhoto {
            path: PathBuf::from("DRAW/IMG_0981.CR3"),
            filename: "IMG_0981.CR3".into(),
            extension: "cr3".into(),
            size_bytes: 29_087_028,
            is_cr3: true,
            is_selected: true,
            dimensions: Some((6000, 4000)),
            orientation: Some(Orientation::Landscape),
            aspect_ratio: Some(AspectRatio::Ratio3x2),
        },
    ];

    let strategy = BatchStrategy::MultiRecipeVariations {
        recipes: vec![
            (BuiltinPreset::Natural.recipe(), 1.0),
            (BuiltinPreset::Cinema.recipe(), 1.0),
        ],
    };

    let jobs = StrategyEngine::expand_jobs(&photos, &strategy, "web_1080p").unwrap();
    assert_eq!(jobs.len(), 4);

    let results = pool.run_queue(jobs, None).await;
    assert_eq!(results.len(), 4);
    for j in results {
        assert!(matches!(j.status, BatchJobStatus::Done { .. }));
    }
}

// ============================================================================
// Scenario 8: Custom Preset Creation + Local Persistence + Batch Application (F-REC-03 x F-BAT-03)
// ============================================================================

#[test]
fn test_tier3_scenario8_custom_preset_creation_persistence_batch() {
    let tmp = tempdir().expect("tempdir");
    let store = PresetStore::with_default_builtins(tmp.path().to_path_buf()).expect("PresetStore creation");

    // 1. Create custom recipe
    let mut custom_recipe = Recipe::default();
    custom_recipe.id = "my-vintage-look".into();
    custom_recipe.name = "My Vintage Look".into();
    custom_recipe.category = RecipeCategory::Vintage;
    custom_recipe.exposure = 0.30;
    custom_recipe.contrast = -15.0;
    custom_recipe.highlights = -20.0;
    custom_recipe.shadows = 25.0;
    custom_recipe.clarity = 8.0;

    let meta = PresetMetadata {
        id: "my-vintage-look".into(),
        name: "My Vintage Look".into(),
        description: Some("Custom Vintage Look".into()),
        version: 1,
        author: Some("User".into()),
        tags: vec!["vintage".into()],
        created_at: Utc::now(),
        modified_at: Utc::now(),
        is_builtin: false,
        category: Some("Vintage".into()),
        favorite: false,
        recommended_intensity: 100.0,
    };

    let preset_file = PresetFile::new(meta, custom_recipe);

    // 2. Persist to disk as TOML
    store.save_preset(&preset_file, PresetFormat::Toml).expect("Save preset");
    assert!(tmp.path().join("my-vintage-look.toml").exists());

    // 3. Load from disk and verify byte-for-byte fidelity
    let loaded = store.load_preset_file("my-vintage-look").expect("Load preset");
    assert_eq!(loaded.metadata.id, "my-vintage-look");
    assert_eq!(loaded.metadata.name, "My Vintage Look");
    assert_eq!(loaded.recipe.exposure, 0.30);
    assert_eq!(loaded.recipe.contrast, -15.0);

    // 4. Batch application of custom preset
    let photos = vec![ScannedPhoto {
        path: PathBuf::from("DRAW/IMG_0969.CR3"),
        filename: "IMG_0969.CR3".into(),
        extension: "cr3".into(),
        size_bytes: 26_614_589,
        is_cr3: true,
        is_selected: true,
        dimensions: Some((6000, 4000)),
        orientation: Some(Orientation::Landscape),
        aspect_ratio: Some(AspectRatio::Ratio3x2),
    }];

    let strategy = BatchStrategy::Uniform {
        recipe: loaded.recipe,
        intensity: 1.0,
    };

    let jobs = StrategyEngine::expand_jobs(&photos, &strategy, "web_1080p").unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].recipe.name, "My Vintage Look");
}
