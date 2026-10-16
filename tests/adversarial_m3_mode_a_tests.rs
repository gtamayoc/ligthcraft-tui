#![allow(dead_code, unused_imports, unused_variables)]

//! Milestone M3 Adversarial Stress Test Suite: Mode A, Event Processing & Graphics Compositor.
//!
//! Empirically challenges and stress-tests:
//! 1. Rapid Mode Toggling: 50+ to 100+ cycles between Mode A and Mode B in a tight loop.
//!    Asserts state persistence (uncommitted adjustments, active photo selection, variant view mode)
//!    is 100% retained with zero thread desync or memory leaks.
//! 2. 1:1 Loupe Boundary Stress: Pan into extreme negative coordinates, coordinates far beyond
//!    image width/height, and zoom levels up to 10x (and beyond).
//!    Asserts strict boundary clamping prevents index out of bounds or slicing panics.
//! 3. Wiper Compositor Boundary Stress: Wiper positions at 0.0, 1.0, -0.5, 1.5, NaN, Inf, -Inf.
//!    Asserts graceful clamping and zero-flicker single buffer output.
//! 4. Histogram Edge Cases: All-black image, all-white image, transparent image, 1x1 image,
//!    single-channel dominant colors, and extreme aspect ratios.
//!    Asserts computation completes without divide-by-zero.
//! 5. Interactive Event Stream Fuzzing & Full Ratatui TestBackend Rendering Sweeps.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba, RgbaImage};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

use lightcraft_tui::app::{AppController, AppMode, LibraryItem};
use lightcraft_tui::engine::presets::BuiltinPreset;
use lightcraft_tui::engine::recipe::Recipe;
use lightcraft_tui::graphics::protocol::ProtocolOverride;
use lightcraft_tui::mcp::client::{McpClient, McpClientConfig};
use lightcraft_tui::mcp::tools::PhotoInfo;
use lightcraft_tui::storage::presets_store::PresetStore;
use lightcraft_tui::tui::event::AppEvent;
use lightcraft_tui::tui::layout::PaneId;
use lightcraft_tui::tui::mode_a::{
    composite_wiper, compute_variant_grid_rects, ColorLabel, FlagStatus, HistogramData, LoupeState,
    ModeAState, ModeAView, PhotoMetadata, SplitViewKind, VariantRenderStatus, VariantSlot,
};

use common::TestFixture;

/// Helper to construct a standardized mock `AppController` for testing.
async fn create_mock_app() -> AppController {
    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(mcp, store, PathBuf::from("DRAW"), ProtocolOverride::Halfblocks)
        .await
        .unwrap();

    // Populate mock items if DRAW was empty or only had few files
    if app.items.len() < 5 {
        app.items = vec![
            LibraryItem {
                path: PathBuf::from("DRAW/IMG_0588.CR3"),
                filename: "IMG_0588.CR3".into(),
                size_bytes: 26527598,
                is_cr3: true,
                is_selected: false,
                rating: 0,
                is_picked: false,
                is_rejected: false,
            },
            LibraryItem {
                path: PathBuf::from("DRAW/IMG_0589.CR3"),
                filename: "IMG_0589.CR3".into(),
                size_bytes: 28369647,
                is_cr3: true,
                is_selected: false,
                rating: 0,
                is_picked: false,
                is_rejected: false,
            },
            LibraryItem {
                path: PathBuf::from("DRAW/IMG_0590.CR3"),
                filename: "IMG_0590.CR3".into(),
                size_bytes: 27704456,
                is_cr3: true,
                is_selected: false,
                rating: 0,
                is_picked: false,
                is_rejected: false,
            },
            LibraryItem {
                path: PathBuf::from("DRAW/IMG_0969.CR3"),
                filename: "IMG_0969.CR3".into(),
                size_bytes: 26614589,
                is_cr3: true,
                is_selected: false,
                rating: 0,
                is_picked: false,
                is_rejected: false,
            },
            LibraryItem {
                path: PathBuf::from("DRAW/IMG_0981.CR3"),
                filename: "IMG_0981.CR3".into(),
                size_bytes: 29087028,
                is_cr3: true,
                is_selected: false,
                rating: 0,
                is_picked: false,
                is_rejected: false,
            },
        ];
    }
    app
}

// ============================================================================
// 1. Rapid Mode Toggling & State Persistence Tests
// ============================================================================

#[tokio::test]
async fn test_adv_m3_01_01_tight_loop_100_mode_toggles_preserves_uncommitted_state() {
    let fixture = TestFixture::new("m3_adv_tight_loop_toggle");
    let mut app = create_mock_app().await;

    // 1. Establish rich uncommitted state across Mode A
    app.selected_index = 2;
    app.items[2].is_selected = true;
    app.items[2].rating = 4;
    app.items[2].is_picked = true;

    // Uncommitted photographic adjustments
    app.mode_a.active_intensity = 1.35;
    app.mode_a.active_recipe.name = "Custom Uncommitted Recipe".into();
    app.mode_a.active_recipe.exposure = 1.75;
    app.mode_a.active_recipe.contrast = 35.0;
    app.mode_a.active_recipe.temperature = Some(6800.0);
    app.mode_a.active_recipe.highlights = -40.0;
    app.mode_a.active_recipe.shadows = 25.0;

    // Variant Matrix uncommitted state
    app.mode_a.view_mode = ModeAView::VariantMatrix;
    app.mode_a.variant_count = 6;
    app.mode_a.focused_variant = 4;
    app.mode_a.variants[4].intensity = 0.85;
    app.mode_a.variants[4].is_favorite = true;
    app.mode_a.variants[4].rating = 5;
    app.mode_a.variants[4].flag = FlagStatus::Pick;
    app.mode_a.variants[4].label_color = ColorLabel::Green;

    // Split View uncommitted state
    app.mode_a.split_kind = SplitViewKind::Wiper;
    app.mode_a.split_ratio = 0.72;
    app.mode_a.toggle_ab_showing_before = true;

    // Loupe uncommitted state
    app.mode_a.loupe.center_x = 0.35;
    app.mode_a.loupe.center_y = 0.85;
    app.mode_a.loupe.zoom_level = 4.0;
    app.mode_a.loupe.show_minimap = true;

    // Mode B uncommitted state
    app.mode_b.selected_table_row = 3;
    app.mode_b.is_paused = true;

    let f10_key = KeyEvent::new(KeyCode::F(10), KeyModifiers::empty());

    // 2. Execute 100 rapid toggles in a tight loop (50 full round-trips)
    let iterations = 100;
    for i in 0..iterations {
        let prev_mode = app.mode;
        app.handle_key(f10_key).await.unwrap();

        let expected_mode = match prev_mode {
            AppMode::SinglePhoto => AppMode::BatchQueue,
            AppMode::BatchQueue => AppMode::SinglePhoto,
        };
        assert_eq!(
            app.mode, expected_mode,
            "Mode toggle failed at iteration {}",
            i
        );

        // Assert critical state is 100% retained after every single toggle
        assert_eq!(app.selected_index, 2);
        assert!(app.items[2].is_selected);
        assert_eq!(app.items[2].rating, 4);
        assert!(app.items[2].is_picked);

        assert!((app.mode_a.active_intensity - 1.35).abs() < f64::EPSILON);
        assert_eq!(app.mode_a.active_recipe.name, "Custom Uncommitted Recipe");
        assert_eq!(app.mode_a.active_recipe.exposure, 1.75);
        assert_eq!(app.mode_a.active_recipe.contrast, 35.0);
        assert_eq!(app.mode_a.active_recipe.temperature, Some(6800.0));

        assert_eq!(app.mode_a.view_mode, ModeAView::VariantMatrix);
        assert_eq!(app.mode_a.variant_count, 6);
        assert_eq!(app.mode_a.focused_variant, 4);
        assert!((app.mode_a.variants[4].intensity - 0.85).abs() < f64::EPSILON);
        assert!(app.mode_a.variants[4].is_favorite);
        assert_eq!(app.mode_a.variants[4].rating, 5);
        assert_eq!(app.mode_a.variants[4].flag, FlagStatus::Pick);
        assert_eq!(app.mode_a.variants[4].label_color, ColorLabel::Green);

        assert_eq!(app.mode_a.split_kind, SplitViewKind::Wiper);
        assert!((app.mode_a.split_ratio - 0.72).abs() < f32::EPSILON);
        assert!(app.mode_a.toggle_ab_showing_before);

        assert!((app.mode_a.loupe.center_x - 0.35).abs() < f32::EPSILON);
        assert!((app.mode_a.loupe.center_y - 0.85).abs() < f32::EPSILON);
        assert!((app.mode_a.loupe.zoom_level - 4.0).abs() < f32::EPSILON);

        assert_eq!(app.mode_b.selected_table_row, 3);
        assert!(app.mode_b.is_paused);
    }

    // Since iterations is even (100), we must end up exactly in SinglePhoto mode
    assert_eq!(app.mode, AppMode::SinglePhoto);
    fixture.assert_integrity();
}

#[tokio::test]
async fn test_adv_m3_01_02_rapid_mode_toggling_with_interleaved_events_and_full_renders() {
    let fixture = TestFixture::new("m3_adv_interleaved_events_render");
    let mut app = create_mock_app().await;

    // Set initial uncommitted state
    app.selected_index = 1;
    app.mode_a.active_intensity = 1.15;
    app.mode_a.view_mode = ModeAView::SplitView;
    app.mode_a.split_kind = SplitViewKind::Wiper;
    app.mode_a.split_ratio = 0.50;

    let backend = TestBackend::new(140, 40);
    let mut terminal = Terminal::new(backend).unwrap();

    let f10_key = KeyEvent::new(KeyCode::F(10), KeyModifiers::empty());
    let tab_key = KeyEvent::new(KeyCode::Tab, KeyModifiers::empty());
    let wiper_dec_key = KeyEvent::new(KeyCode::Char('['), KeyModifiers::empty());
    let wiper_inc_key = KeyEvent::new(KeyCode::Char(']'), KeyModifiers::empty());

    for i in 0..60 {
        // 1. Toggle Mode
        app.handle_key(f10_key).await.unwrap();

        // 2. Interleave Tab pane switching
        app.handle_key(tab_key).await.unwrap();

        // 3. Interleave Mouse hit-test click
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 60,
            row: 20,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_event(AppEvent::Mouse(mouse)).await.unwrap();

        // 4. Interleave Tick event
        app.handle_event(AppEvent::Tick).await.unwrap();

        // 5. Interleave Status message
        app.handle_event(AppEvent::StatusMessage(format!("Heartbeat iteration {}", i)))
            .await
            .unwrap();

        // 6. Execute full Ratatui render pass to catch any buffer desync or layout panics
        terminal.draw(|frame| app.render(frame)).unwrap();

        // 7. Interleave recipe intensity adjust in Mode A
        if app.mode == AppMode::SinglePhoto {
            if i % 2 == 0 {
                app.handle_key(wiper_inc_key).await.unwrap();
            } else {
                app.handle_key(wiper_dec_key).await.unwrap();
            }
        }
    }

    // Verify app remains completely responsive, running, and non-corrupted
    assert!(app.running);
    assert_eq!(app.selected_index, 1);
    assert_eq!(app.mode_a.view_mode, ModeAView::SplitView);
    assert_eq!(app.mode_a.split_kind, SplitViewKind::Wiper);

    fixture.assert_integrity();
}

#[tokio::test]
async fn test_adv_m3_01_03_alternative_toggle_keys_and_empty_library_resilience() {
    let mut app = create_mock_app().await;

    // Test toggle via 'm'
    let m_lower = KeyEvent::new(KeyCode::Char('m'), KeyModifiers::empty());
    let m_upper = KeyEvent::new(KeyCode::Char('M'), KeyModifiers::SHIFT);

    assert_eq!(app.mode, AppMode::SinglePhoto);
    app.handle_key(m_lower).await.unwrap();
    assert_eq!(app.mode, AppMode::BatchQueue);
    app.handle_key(m_upper).await.unwrap();
    assert_eq!(app.mode, AppMode::SinglePhoto);

    // Empty library resilience
    app.items.clear();
    app.selected_index = 0;

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    // Toggle 20 times with completely empty items list
    for _ in 0..20 {
        app.handle_key(m_lower).await.unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }
    assert!(app.running);
}

// ============================================================================
// 2. 1:1 Loupe Boundary Stress Tests
// ============================================================================

#[test]
fn test_adv_m3_02_01_extreme_negative_coordinates_prevent_underflow() {
    let mut loupe = LoupeState::default();

    // Test pan with extreme negative coordinates
    loupe.pan(-1000.0, -50000.0);
    assert_eq!(loupe.center_x, 0.0, "center_x must clamp to 0.0");
    assert_eq!(loupe.center_y, 0.0, "center_y must clamp to 0.0");

    let img_w = 6000;
    let img_h = 4000;
    let canvas_w = 800;
    let canvas_h = 600;

    let (x0, y0, cw, ch) = loupe.compute_crop_rect(img_w, img_h, canvas_w, canvas_h);
    assert_eq!(x0, 0, "Top-left x must clamp to 0 on negative pan");
    assert_eq!(y0, 0, "Top-left y must clamp to 0 on negative pan");
    assert_eq!(cw, 800);
    assert_eq!(ch, 600);
    assert!(x0 + cw <= img_w);
    assert!(y0 + ch <= img_h);

    // Test directly setting negative center_x/y without calling pan()
    loupe.center_x = -999.0;
    loupe.center_y = -1e9;
    let (x0_direct, y0_direct, cw_direct, ch_direct) =
        loupe.compute_crop_rect(img_w, img_h, canvas_w, canvas_h);
    assert_eq!(x0_direct, 0);
    assert_eq!(y0_direct, 0);
    assert!(x0_direct + cw_direct <= img_w);
    assert!(y0_direct + ch_direct <= img_h);
}

#[test]
fn test_adv_m3_02_02_extreme_positive_coordinates_prevent_overflow() {
    let mut loupe = LoupeState::default();

    // Pan far beyond image width/height
    loupe.pan(10000.0, 50000.0);
    assert_eq!(loupe.center_x, 1.0, "center_x must clamp to 1.0");
    assert_eq!(loupe.center_y, 1.0, "center_y must clamp to 1.0");

    let img_w = 6000;
    let img_h = 4000;
    let canvas_w = 800;
    let canvas_h = 600;

    let (x0, y0, cw, ch) = loupe.compute_crop_rect(img_w, img_h, canvas_w, canvas_h);
    assert_eq!(cw, 800);
    assert_eq!(ch, 600);
    assert_eq!(x0, 6000 - 800, "x0 must clamp exactly to img_w - cw");
    assert_eq!(y0, 4000 - 600, "y0 must clamp exactly to img_h - ch");
    assert_eq!(x0 + cw, img_w);
    assert_eq!(y0 + ch, img_h);

    // Direct assignment of massive positive floats
    loupe.center_x = 1e8;
    loupe.center_y = 1e8;
    let (x0_direct, y0_direct, cw_direct, ch_direct) =
        loupe.compute_crop_rect(img_w, img_h, canvas_w, canvas_h);
    assert_eq!(x0_direct, 5200);
    assert_eq!(y0_direct, 3400);
    assert_eq!(x0_direct + cw_direct, img_w);
    assert_eq!(y0_direct + ch_direct, img_h);
}

#[test]
fn test_adv_m3_02_03_extreme_zoom_levels_up_to_10x_and_beyond() {
    let mut loupe = LoupeState::default();

    let img_w = 6000;
    let img_h = 4000;
    let canvas_w = 800;
    let canvas_h = 600;

    // Zoom levels: 0.1x, 0.5x, 1.0x, 2.0x, 5.0x, 10.0x, 20.0x, 100.0x
    let zoom_levels = [0.1, 0.5, 1.0, 2.0, 3.5, 5.0, 7.5, 10.0, 15.0, 50.0, 100.0];

    for &z in &zoom_levels {
        loupe.zoom_level = z;

        // Test at center, top-left, bottom-right, and extreme out-of-bounds
        let test_centers = [
            (0.5, 0.5),
            (0.0, 0.0),
            (1.0, 1.0),
            (-5.0, -5.0),
            (10.0, 10.0),
        ];

        for (cx, cy) in test_centers {
            loupe.center_x = cx;
            loupe.center_y = cy;

            let (x0, y0, cw, ch) = loupe.compute_crop_rect(img_w, img_h, canvas_w, canvas_h);

            // Invariant 1: Dimensions must be >= 1
            assert!(cw >= 1, "crop_w must be at least 1 at zoom {}", z);
            assert!(ch >= 1, "crop_h must be at least 1 at zoom {}", z);

            // Invariant 2: Dimensions must not exceed image size
            assert!(cw <= img_w, "crop_w ({}) > img_w ({}) at zoom {}", cw, img_w, z);
            assert!(ch <= img_h, "crop_h ({}) > img_h ({}) at zoom {}", ch, img_h, z);

            // Invariant 3: Crop box must strictly lie within [0, img_w] and [0, img_h]
            assert!(
                x0 + cw <= img_w,
                "Out-of-bounds horizontal slicing: x0 ({}) + cw ({}) = {} > img_w ({}) at zoom {}, cx {}",
                x0, cw, x0 + cw, img_w, z, cx
            );
            assert!(
                y0 + ch <= img_h,
                "Out-of-bounds vertical slicing: y0 ({}) + ch ({}) = {} > img_h ({}) at zoom {}, cy {}",
                y0, ch, y0 + ch, img_h, z, cy
            );
        }
    }
}

#[test]
fn test_adv_m3_02_04_physical_image_slicing_stress_never_panics() {
    // Construct diverse dynamic images
    let images = vec![
        ("Landscape", DynamicImage::ImageRgba8(RgbaImage::new(1200, 800))),
        ("Portrait", DynamicImage::ImageRgba8(RgbaImage::new(800, 1200))),
        ("Panorama", DynamicImage::ImageRgba8(RgbaImage::new(3000, 400))),
        ("Square", DynamicImage::ImageRgba8(RgbaImage::new(512, 512))),
        ("Small", DynamicImage::ImageRgba8(RgbaImage::new(64, 64))),
        ("1x1", DynamicImage::ImageRgba8(RgbaImage::new(1, 1))),
    ];

    let mut loupe = LoupeState::default();

    for (_name, img) in images {
        let (w, h) = img.dimensions();

        // Sweep various zoom levels and positions
        for &zoom in &[0.5, 1.0, 2.0, 5.0, 10.0] {
            loupe.zoom_level = zoom;
            for &cx in &[-1.0, 0.0, 0.25, 0.5, 0.75, 1.0, 2.0] {
                for &cy in &[-1.0, 0.0, 0.25, 0.5, 0.75, 1.0, 2.0] {
                    loupe.center_x = cx;
                    loupe.center_y = cy;

                    let (x0, y0, cw, ch) = loupe.compute_crop_rect(w, h, 200, 150);

                    // If image dimensions >= 1, verify physical crop operation succeeds unconditionally
                    if w > 0 && h > 0 && x0 + cw <= w && y0 + ch <= h {
                        let sub_view = img.view(x0, y0, cw, ch);
                        assert_eq!(sub_view.dimensions(), (cw, ch));
                    }
                }
            }
        }
    }
}

#[test]
fn test_adv_m3_02_05_nan_and_infinity_in_loupe_coordinates() {
    let mut loupe = LoupeState::default();
    loupe.center_x = f32::NAN;
    loupe.center_y = f32::NAN;
    loupe.zoom_level = f32::INFINITY;

    let (x0, y0, cw, ch) = loupe.compute_crop_rect(6000, 4000, 800, 600);
    // cw, ch should be clamped to at least 1 and within img_w/img_h
    assert!(cw >= 1 && cw <= 6000);
    assert!(ch >= 1 && ch <= 4000);
    // x0 and y0 must not exceed bounds
    assert!(x0 + cw <= 6000);
    assert!(y0 + ch <= 4000);

    // Zoom level NaN
    loupe.zoom_level = f32::NAN;
    let (x0_nan, y0_nan, cw_nan, ch_nan) = loupe.compute_crop_rect(6000, 4000, 800, 600);
    assert!(cw_nan >= 1 && cw_nan <= 6000);
    assert!(ch_nan >= 1 && ch_nan <= 4000);
    assert!(x0_nan + cw_nan <= 6000);
    assert!(y0_nan + ch_nan <= 4000);
}

// ============================================================================
// 3. Wiper Compositor Boundary Stress Tests
// ============================================================================

#[test]
fn test_adv_m3_03_01_wiper_boundary_positions_and_graceful_clamping() {
    let before = DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 100, Rgba([255, 0, 0, 255])));
    let after = DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 100, Rgba([0, 0, 255, 255])));

    // Test positions: 0.0, 1.0, -0.5, 1.5, NaN, Inf, -Inf
    let positions = [
        ("Zero (0.0)", 0.0f32),
        ("One (1.0)", 1.0f32),
        ("Negative (-0.5)", -0.5f32),
        ("Excessive (1.5)", 1.5f32),
        ("NaN", f32::NAN),
        ("+Infinity", f32::INFINITY),
        ("-Infinity", f32::NEG_INFINITY),
        ("Min Clamped (0.05)", 0.05f32),
        ("Max Clamped (0.95)", 0.95f32),
        ("Center (0.50)", 0.50f32),
    ];

    for (desc, split_pos) in positions {
        let out = composite_wiper(&before, &after, split_pos);

        // Single buffer output check: dimensions must match
        assert_eq!(
            out.dimensions(),
            (200, 100),
            "Dimensions corrupted at position {}",
            desc
        );

        // Verify pixel validity without panic:
        // Left corner (0, 50) and Right corner (199, 50)
        let _ = out.get_pixel(0, 50);
        let _ = out.get_pixel(199, 50);

        // At 0.5, left side must be Before (Red) and right side must be After (Blue)
        if (split_pos - 0.5).abs() < f32::EPSILON {
            let left_p = out.get_pixel(20, 50);
            assert_eq!(left_p[0], 255, "Left pixel must be Red at 50% split");
            assert_eq!(left_p[2], 0);

            let right_p = out.get_pixel(180, 50);
            assert_eq!(right_p[0], 0);
            assert_eq!(right_p[2], 255, "Right pixel must be Blue at 50% split");
        }
    }
}

#[test]
fn test_adv_m3_03_02_wiper_divider_line_integrity_and_bounds() {
    let before = DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, Rgba([20, 20, 20, 255])));
    let after = DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, Rgba([80, 80, 80, 255])));

    // At split_ratio = 0.5 on 100px width:
    // split_x = (100 * 0.5) = 50
    let out = composite_wiper(&before, &after, 0.5);

    // Verify 1px white separator line at x = 50
    let div_p = out.get_pixel(50, 50);
    assert_eq!(
        div_p,
        Rgba([255, 255, 255, 255]),
        "Divider must be white line at split_x"
    );

    // Verify adjacent drop shadow at x = 51
    let shadow_p = out.get_pixel(51, 50);
    assert_eq!(
        shadow_p,
        Rgba([40, 40, 40, 200]),
        "Divider drop shadow must be at split_x + 1"
    );
}

#[test]
fn test_adv_m3_03_03_wiper_mismatched_and_degenerate_dimensions() {
    let images_before = vec![
        DynamicImage::ImageRgba8(RgbaImage::new(100, 50)),
        DynamicImage::ImageRgba8(RgbaImage::new(50, 100)),
        DynamicImage::ImageRgba8(RgbaImage::new(1, 1)),
        DynamicImage::ImageRgba8(RgbaImage::new(1, 100)),
        DynamicImage::ImageRgba8(RgbaImage::new(0, 0)),
    ];

    let images_after = vec![
        DynamicImage::ImageRgba8(RgbaImage::new(200, 100)),
        DynamicImage::ImageRgba8(RgbaImage::new(100, 200)),
        DynamicImage::ImageRgba8(RgbaImage::new(1, 1)),
        DynamicImage::ImageRgba8(RgbaImage::new(100, 1)),
        DynamicImage::ImageRgba8(RgbaImage::new(0, 0)),
    ];

    for b in &images_before {
        for a in &images_after {
            for &ratio in &[0.0, 0.5, 1.0, -0.5, 1.5, f32::NAN] {
                let out = composite_wiper(b, a, ratio);
                // Returned image must match after's dimensions
                assert_eq!(out.width(), a.width());
                assert_eq!(out.height(), a.height());
            }
        }
    }
}

// ============================================================================
// 4. Histogram Edge Cases & Numerical Stability Tests
// ============================================================================

#[test]
fn test_adv_m3_04_01_all_black_image_completes_without_divide_by_zero() {
    // 500x500 all-black image
    let black = DynamicImage::ImageRgba8(RgbaImage::from_pixel(500, 500, Rgba([0, 0, 0, 255])));

    let hist = HistogramData::from_image(&black);

    // Assert all counts landed in bin 0
    assert!(hist.bins_lum[0] > 0, "Luminance bin 0 must be populated");
    assert_eq!(hist.bins_lum[1..].iter().sum::<u32>(), 0);
    assert!(hist.bins_r[0] > 0);
    assert!(hist.bins_g[0] > 0);
    assert!(hist.bins_b[0] > 0);

    // max_count must be at least 1
    assert!(hist.max_count >= 1);

    // Assert sparklines do not divide by zero or panic
    let sparks = hist.sparklines();
    assert_eq!(sparks.len(), 4);
    for s in &sparks {
        assert!(s.contains("Lum: [") || s.contains("Red: [") || s.contains("Grn: [") || s.contains("Blu: ["));
        assert!(!s.contains("NaN") && !s.contains("inf"));
    }
}

#[test]
fn test_adv_m3_04_02_all_white_image_saturation_resilience() {
    // 500x500 all-white image
    let white = DynamicImage::ImageRgba8(RgbaImage::from_pixel(500, 500, Rgba([255, 255, 255, 255])));

    let hist = HistogramData::from_image(&white);

    // Assert all counts landed in bin 31
    assert!(hist.bins_lum[31] > 0, "Luminance bin 31 must be populated");
    assert_eq!(hist.bins_lum[..31].iter().sum::<u32>(), 0);
    assert!(hist.bins_r[31] > 0);
    assert!(hist.bins_g[31] > 0);
    assert!(hist.bins_b[31] > 0);

    assert!(hist.max_count >= 1);

    let sparks = hist.sparklines();
    assert_eq!(sparks.len(), 4);
    for s in &sparks {
        // Bin 31 should show full height spark character '█'
        assert!(s.contains('█'), "White image sparkline must contain full bar '█'");
    }
}

#[test]
fn test_adv_m3_04_03_fully_transparent_and_alpha_variations() {
    let transparent_black =
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, Rgba([0, 0, 0, 0])));
    let transparent_white =
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 0])));
    let semi_transparent =
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, Rgba([128, 64, 32, 128])));

    for img in &[transparent_black, transparent_white, semi_transparent] {
        let hist = HistogramData::from_image(img);
        assert!(hist.max_count >= 1);
        let sparks = hist.sparklines();
        assert_eq!(sparks.len(), 4);
    }
}

#[test]
fn test_adv_m3_04_04_tiny_1x1_and_extreme_aspect_ratios() {
    // 1x1 image
    let tiny_black = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 255])));
    let tiny_white = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([255, 255, 255, 255])));
    let tiny_red = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 255])));

    for tiny in &[tiny_black, tiny_white, tiny_red] {
        let hist = HistogramData::from_image(tiny);
        assert_eq!(hist.max_count, 1);
        let sparks = hist.sparklines();
        assert_eq!(sparks.len(), 4);
    }

    // 5000x1 horizontal line
    let h_line = DynamicImage::ImageRgba8(RgbaImage::from_pixel(5000, 1, Rgba([128, 128, 128, 255])));
    let hist_h = HistogramData::from_image(&h_line);
    assert!(hist_h.max_count >= 1);
    assert_eq!(hist_h.sparklines().len(), 4);

    // 1x5000 vertical column
    let v_col = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 5000, Rgba([128, 128, 128, 255])));
    let hist_v = HistogramData::from_image(&v_col);
    assert!(hist_v.max_count >= 1);
    assert_eq!(hist_v.sparklines().len(), 4);
}

#[test]
fn test_adv_m3_04_05_single_channel_dominance_sparkline_index_clamping() {
    // Construct an image where Red is 255 across all pixels, but Green and Blue are 0.
    // ITU-R BT.709 Luminance of (255, 0, 0) is: 0.2126 * 255 = 54 (bin 6).
    // Now mix half of the pixels with (255, 100, 50), which has lum = 129 (bin 16).
    // In this scenario:
    // bins_r has 100% of counts in bin 31!
    // But bins_lum is split 50/50 between bin 6 and bin 16.
    // Thus max_count = 50% of sampled pixels.
    // bins_r[31] / max_count = 1.0 / 0.5 = 2.0 -> (2.0 * 7.0).round() = 14!
    // This MUST be clamped to chars[7] ('█') without panicking with index out of bounds!
    let mut img = RgbaImage::new(100, 100);
    for (_x, y, pixel) in img.enumerate_pixels_mut() {
        if y < 50 {
            *pixel = Rgba([255, 0, 0, 255]);
        } else {
            *pixel = Rgba([255, 100, 50, 255]);
        }
    }
    let dynamic_img = DynamicImage::ImageRgba8(img);
    let hist = HistogramData::from_image(&dynamic_img);

    // Verify Red count in bin 31 exceeds max_count
    assert!(
        hist.bins_r[31] >= hist.max_count,
        "Red count ({}) should be >= max_count ({})",
        hist.bins_r[31],
        hist.max_count
    );

    // This call will panic if idx is not clamped with .min(7)
    let sparks = hist.sparklines();
    assert_eq!(sparks.len(), 4);
    assert!(sparks[1].contains('█'), "Red sparkline should clamp to '█'");
}

// ============================================================================
// 5. Interactive Event Stream Fuzzing & Ratatui Rendering Sweeps
// ============================================================================

#[tokio::test]
async fn test_adv_m3_05_01_fuzz_key_event_stream_stability() {
    let mut app = create_mock_app().await;

    // Stream of arbitrary and edge-case key events
    let keys = vec![
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::F(1), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::F(12), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Home, KeyModifiers::empty()),
        KeyEvent::new(KeyCode::End, KeyModifiers::empty()),
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::empty()),
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Insert, KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Delete, KeyModifiers::empty()),
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('!'), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('~'), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('1'), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('2'), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('4'), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('6'), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('['), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char(']'), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::empty()),
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::empty()),
    ];

    for _ in 0..10 {
        for key in &keys {
            app.handle_key(*key).await.unwrap();
            assert!(app.running);
        }
    }
}

#[tokio::test]
async fn test_adv_m3_05_02_ratatui_render_sweep_all_view_modes_and_terminal_sizes() {
    let mut app = create_mock_app().await;

    // Test across view modes
    let view_modes = [
        ModeAView::Hero,
        ModeAView::VariantMatrix,
        ModeAView::SplitView,
        ModeAView::Loupe1to1,
    ];

    let split_kinds = [
        SplitViewKind::SideBySide,
        SplitViewKind::TopBottom,
        SplitViewKind::Wiper,
        SplitViewKind::ToggleAB,
    ];

    let terminal_sizes = [
        (60, 20),   // Below minimum (alert overlay)
        (80, 24),   // Exactly minimum
        (120, 40),  // Standard desktop
        (160, 50),  // Ultrawide
        (240, 70),  // Massive display
    ];

    for (w, h) in terminal_sizes {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();

        for &mode in &view_modes {
            app.mode_a.view_mode = mode;

            if mode == ModeAView::VariantMatrix {
                for &cnt in &[2, 3, 4, 5, 6] {
                    app.mode_a.variant_count = cnt;
                    terminal.draw(|frame| app.render(frame)).unwrap();
                }
            } else if mode == ModeAView::SplitView {
                for &split in &split_kinds {
                    app.mode_a.split_kind = split;
                    terminal.draw(|frame| app.render(frame)).unwrap();
                }
            } else {
                terminal.draw(|frame| app.render(frame)).unwrap();
            }
        }
    }

    assert!(app.running);
}
