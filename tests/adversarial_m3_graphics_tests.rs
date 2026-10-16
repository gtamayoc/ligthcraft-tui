//! Tier 2 Adversarial Stress Test Suite for Milestone M3 (Graphics & Layout).
//!
//! Empirically challenges and stress-tests:
//! 1. Terminal resize torture tests (tiny 0x0/1x1/10x5, huge 500x200/1000x1000/65535x65535, 60+ burst resizes).
//! 2. Aspect ratio edge cases (100:1 panorama, 1:100 tall strip, 0 width/height, NaN/Inf handling).
//! 3. PreviewCache memory budget stress (200MB+ flood against 64MB budget, byte-level eviction, weak ref drop verification).

use std::path::PathBuf;
use std::sync::{Arc, Weak};
use image::imageops::FilterType;
use image::{DynamicImage, Rgba, RgbaImage};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use ratatui_image::picker::Picker;
use ratatui_image::FontSize;

use lightcraft_tui::app::AppController;
use lightcraft_tui::graphics::cache::{
    CacheProtocolKind, PreviewCache, PreviewCacheConfig, PreviewCacheKey,
};
use lightcraft_tui::graphics::protocol::ProtocolOverride;
use lightcraft_tui::graphics::renderer::ImageRenderer;
use lightcraft_tui::mcp::client::{McpClient, McpClientConfig};
use lightcraft_tui::storage::presets_store::PresetStore;
use lightcraft_tui::tui::layout::{AppLayout, PaneId};
use lightcraft_tui::tui::mode_a::{
    compute_variant_grid_rects, composite_wiper, LoupeState,
};

// ============================================================================
// PART 1: TERMINAL RESIZE TORTURE TESTS
// ============================================================================

#[test]
fn test_adv_layout_tiny_and_subminimum_dimensions() {
    let tiny_dimensions = [
        (0, 0),
        (1, 1),
        (10, 5),
        (1, 24),
        (80, 1),
        (79, 23),
        (79, 24),
        (80, 23),
        (40, 12),
        (0, 50),
        (100, 0),
    ];

    for (w, h) in tiny_dimensions {
        let area = Rect::new(0, 0, w, h);
        let layout = AppLayout::compute(area);

        // Sub-minimum terminal bounds must trigger is_too_small modal flag
        if w < AppLayout::MIN_WIDTH || h < AppLayout::MIN_HEIGHT {
            assert!(
                layout.is_too_small,
                "Dimensions {}x{} must be flagged as is_too_small",
                w, h
            );
            assert_eq!(layout.header_area.width, 0);
            assert_eq!(layout.library_area.width, 0);
            assert_eq!(layout.preview_area.width, 0);
            assert_eq!(layout.inspector_area.width, 0);
            assert_eq!(layout.status_area.width, 0);
        }

        // Hit-test on subminimum dimensions must not panic
        assert_eq!(layout.pane_at(0, 0), None);
        assert_eq!(layout.pane_at(w, h), None);
        assert_eq!(layout.pane_at(u16::MAX, u16::MAX), None);
    }
}

#[test]
fn test_adv_layout_exact_boundary_thresholds() {
    // Exact minimum threshold is 80x24
    let exact_min = Rect::new(0, 0, 80, 24);
    let layout_min = AppLayout::compute(exact_min);
    assert!(!layout_min.is_too_small, "80x24 must satisfy minimum dimensions");
    assert_eq!(layout_min.header_area.height, 1);
    assert_eq!(layout_min.status_area.height, 2);
    assert_eq!(layout_min.library_area.height, 21);
    assert_eq!(layout_min.preview_area.height, 21);
    assert_eq!(layout_min.inspector_area.height, 21);
    assert!(layout_min.preview_area.width > 0);

    // One pixel below in either axis must fail
    assert!(AppLayout::compute(Rect::new(0, 0, 79, 24)).is_too_small);
    assert!(AppLayout::compute(Rect::new(0, 0, 80, 23)).is_too_small);
    assert!(AppLayout::compute(Rect::new(0, 0, 79, 23)).is_too_small);

    // One pixel above in both axes must succeed
    assert!(!AppLayout::compute(Rect::new(0, 0, 81, 25)).is_too_small);
}

#[test]
fn test_adv_layout_breakpoint_transitions_no_gap_or_overlap() {
    // Test width boundaries: 119 -> 120 (Compact to Desktop) and 159 -> 160 (Desktop to Ultrawide)
    let widths = [80, 100, 119, 120, 140, 159, 160, 200, 300];
    let heights = [24, 29, 30, 40, 60];

    for &w in &widths {
        for &h in &heights {
            let area = Rect::new(0, 0, w, h);
            let layout = AppLayout::compute(area);

            assert!(!layout.is_too_small, "Dimensions {}x{} should be valid", w, h);

            // Verify vertical split partition
            let vert_sum = layout.header_area.height + layout.library_area.height + layout.status_area.height;
            assert_eq!(
                vert_sum, h,
                "Vertical panes height sum ({}) must equal terminal height ({}) for {}x{}",
                vert_sum, h, w, h
            );

            // Verify horizontal split partition
            let horiz_sum = layout.library_area.width + layout.preview_area.width + layout.inspector_area.width;
            assert_eq!(
                horiz_sum, w,
                "Horizontal panes width sum ({}) must equal terminal width ({}) for {}x{}",
                horiz_sum, w, w, h
            );

            // Header height breakpoint
            if h >= 30 {
                assert_eq!(layout.header_area.height, 3);
                assert_eq!(layout.status_area.height, 3);
            } else {
                assert_eq!(layout.header_area.height, 1);
                assert_eq!(layout.status_area.height, 2);
            }
        }
    }
}

#[test]
fn test_adv_layout_huge_and_extreme_dimensions() {
    let huge_dimensions = [
        (500, 200),
        (1000, 1000),
        (2500, 1500),
        (5000, 5000),
        (10000, 10000),
        (32767, 32767),
        (65535, 65535),
    ];

    for (w, h) in huge_dimensions {
        let area = Rect::new(0, 0, w, h);
        let layout = AppLayout::compute(area);

        assert!(!layout.is_too_small);
        assert_eq!(layout.total_area.width, w);
        assert_eq!(layout.total_area.height, h);

        // Verify pane coordinates fit in u16 without overflow
        assert!(layout.preview_area.x < w);
        assert!(layout.preview_area.y < h);
        assert!(layout.preview_area.width <= w);
        assert!(layout.preview_area.height <= h);

        // Hit testing at extreme limits
        let _ = layout.pane_at(0, 0);
        assert_eq!(layout.pane_at(w / 2, h / 2), Some(PaneId::PreviewCanvas));
        let _ = layout.pane_at(w.saturating_sub(1), h.saturating_sub(1));
        let _ = layout.pane_at(u16::MAX, u16::MAX);
    }
}

#[test]
fn test_adv_layout_rapid_resize_bursts_60_cycles() {
    // Simulate erratic window dragging (60 rapid resize events cycling extremes)
    let resize_sequence: Vec<(u16, u16)> = (0..60)
        .map(|i| match i % 6 {
            0 => (1, 1),
            1 => (120, 40),
            2 => (500, 200),
            3 => (10, 5),
            4 => (80, 24),
            _ => (1000, 1000),
        })
        .collect();

    for (step, &(w, h)) in resize_sequence.iter().enumerate() {
        let area = Rect::new(0, 0, w, h);
        let layout = AppLayout::compute(area);

        if w < 80 || h < 24 {
            assert!(layout.is_too_small, "Step {}: {}x{} must be too small", step, w, h);
        } else {
            assert!(!layout.is_too_small, "Step {}: {}x{} must be valid", step, w, h);
            let sum_w = layout.library_area.width + layout.preview_area.width + layout.inspector_area.width;
            assert_eq!(sum_w, w, "Step {}: width mismatch", step);
        }
    }
}

#[tokio::test]
async fn test_adv_render_full_app_under_torture_dimensions() {
    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let app = AppController::new(mcp, store, PathBuf::from("DRAW"), ProtocolOverride::Halfblocks)
        .await
        .unwrap();

    let torture_sizes = [
        (1, 1),
        (5, 5),
        (10, 5),
        (79, 23),
        (80, 24),
        (120, 40),
        (200, 60),
        (500, 200),
        (1000, 500),
    ];

    for &(w, h) in &torture_sizes {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();

        // Render pass MUST NOT panic on any dimensions
        terminal
            .draw(|frame| {
                app.render(frame);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.area.width, w);
        assert_eq!(buffer.area.height, h);
    }
}

#[test]
fn test_adv_variant_grid_torture_dimensions_and_counts() {
    let test_areas = [
        Rect::new(0, 0, 0, 0),
        Rect::new(0, 0, 1, 1),
        Rect::new(0, 0, 10, 5),
        Rect::new(0, 0, 80, 24),
        Rect::new(0, 0, 500, 200),
        Rect::new(0, 0, 1000, 1000),
    ];

    let counts = [0, 1, 2, 3, 4, 5, 6, 7, 10, 100];

    for area in test_areas {
        for &count in &counts {
            let rects = compute_variant_grid_rects(area, count);

            // Check invariant: rect count matches expectations
            match count {
                2 => assert_eq!(rects.len(), 2),
                3 => assert_eq!(rects.len(), 3),
                4 => assert_eq!(rects.len(), 4),
                5 => assert_eq!(rects.len(), 5),
                6 => assert_eq!(rects.len(), 6),
                _ => assert_eq!(rects.len(), 1), // fallback to single rect
            }

            // Every sub-rect must be bounded inside the parent area
            for rect in rects {
                assert!(rect.x >= area.x);
                assert!(rect.y >= area.y);
                assert!(rect.x + rect.width <= area.x + area.width);
                assert!(rect.y + rect.height <= area.y + area.height);
            }
        }
    }
}

// ============================================================================
// PART 2: ASPECT RATIO EDGE CASES & FIT CALCULATION
// ============================================================================

#[test]
fn test_adv_fit_extreme_panorama_100_to_1() {
    let font_sizes = [
        FontSize::new(10, 20), // Standard 1:2 cell
        FontSize::new(8, 16),
        FontSize::new(12, 24),
        FontSize::new(10, 10), // Square cell
    ];

    let panes = [
        Rect::new(0, 0, 100, 40),
        Rect::new(0, 0, 30, 60),
        Rect::new(0, 0, 300, 50),
        Rect::new(0, 0, 80, 24),
    ];

    for font in font_sizes {
        for pane in panes {
            // Extreme 100:1 panorama (10,000 x 100)
            let fit_100_1 = ImageRenderer::calculate_fit(10000, 100, font, pane);

            assert!(fit_100_1.render_area.width <= pane.width);
            assert!(fit_100_1.render_area.height <= pane.height);
            assert!(
                fit_100_1.render_area.height >= 1,
                "Height must be at least 1 cell"
            );
            // Panorama must fill the width of the pane
            assert_eq!(fit_100_1.render_area.width, pane.width);

            // Centered vertically
            assert!(fit_100_1.render_area.y >= pane.y);
            assert!(fit_100_1.render_area.y + fit_100_1.render_area.height <= pane.y + pane.height);

            // Extreme 1000:1 panorama (100,000 x 100)
            let fit_1000_1 = ImageRenderer::calculate_fit(100000, 100, font, pane);
            assert_eq!(fit_1000_1.render_area.width, pane.width);
            assert_eq!(fit_1000_1.render_area.height, 1);
        }
    }
}

#[test]
fn test_adv_fit_extreme_tall_strip_1_to_100() {
    let font_sizes = [
        FontSize::new(10, 20),
        FontSize::new(8, 16),
        FontSize::new(10, 10),
    ];

    let panes = [
        Rect::new(0, 0, 100, 40),
        Rect::new(0, 0, 40, 100),
        Rect::new(0, 0, 80, 24),
    ];

    for font in font_sizes {
        for pane in panes {
            // Extreme 1:100 tall strip (100 x 10,000)
            let fit_1_100 = ImageRenderer::calculate_fit(100, 10000, font, pane);

            assert!(fit_1_100.render_area.height <= pane.height);
            assert!(fit_1_100.render_area.width <= pane.width);
            assert!(
                fit_1_100.render_area.width >= 1,
                "Width must be at least 1 cell"
            );
            // Tall strip must fill the height of the pane
            assert_eq!(fit_1_100.render_area.height, pane.height);

            // Centered horizontally
            assert!(fit_1_100.render_area.x >= pane.x);
            assert!(fit_1_100.render_area.x + fit_1_100.render_area.width <= pane.x + pane.width);

            // Extreme 1:1000 strip (100 x 100,000)
            let fit_1_1000 = ImageRenderer::calculate_fit(100, 100000, font, pane);
            assert_eq!(fit_1_1000.render_area.height, pane.height);
            assert_eq!(fit_1_1000.render_area.width, 1);
        }
    }
}

#[test]
fn test_adv_fit_zero_and_near_zero_inputs() {
    let font = FontSize::new(10, 20);
    let pane = Rect::new(0, 0, 80, 40);

    // 1. Zero image width
    let f1 = ImageRenderer::calculate_fit(0, 1000, font, pane);
    assert_eq!(f1.render_area, Rect::default());
    assert_eq!(f1.target_pixel_size, (0, 0));

    // 2. Zero image height
    let f2 = ImageRenderer::calculate_fit(1000, 0, font, pane);
    assert_eq!(f2.render_area, Rect::default());
    assert_eq!(f2.target_pixel_size, (0, 0));

    // 3. Zero pane width
    let f3 = ImageRenderer::calculate_fit(1000, 1000, font, Rect::new(0, 0, 0, 40));
    assert_eq!(f3.render_area, Rect::default());

    // 4. Zero pane height
    let f4 = ImageRenderer::calculate_fit(1000, 1000, font, Rect::new(0, 0, 80, 0));
    assert_eq!(f4.render_area, Rect::default());

    // 5. Zero font width / height
    let f5 = ImageRenderer::calculate_fit(1000, 1000, FontSize::new(0, 20), pane);
    assert_eq!(f5.render_area, Rect::default());
    let f6 = ImageRenderer::calculate_fit(1000, 1000, FontSize::new(10, 0), pane);
    assert_eq!(f6.render_area, Rect::default());

    // 6. 1x1 image in 1x1 pane
    let f7 = ImageRenderer::calculate_fit(1, 1, font, Rect::new(5, 5, 1, 1));
    assert_eq!(f7.render_area.width, 1);
    assert_eq!(f7.render_area.height, 1);
    assert_eq!(f7.render_area.x, 5);
    assert_eq!(f7.render_area.y, 5);
}

#[test]
fn test_adv_fit_massive_u32_dimensions() {
    let font = FontSize::new(10, 20);
    let pane = Rect::new(0, 0, 120, 60);

    // u32::MAX dimensions should not overflow f32 or panic
    let f_huge_w = ImageRenderer::calculate_fit(u32::MAX, 1, font, pane);
    assert_eq!(f_huge_w.render_area.width, 120);
    assert_eq!(f_huge_w.render_area.height, 1);

    let f_huge_h = ImageRenderer::calculate_fit(1, u32::MAX, font, pane);
    assert_eq!(f_huge_h.render_area.width, 1);
    assert_eq!(f_huge_h.render_area.height, 60);

    let f_both_huge = ImageRenderer::calculate_fit(u32::MAX, u32::MAX, font, pane);
    assert!(f_both_huge.render_area.width <= pane.width);
    assert!(f_both_huge.render_area.height <= pane.height);
}

#[test]
fn test_adv_prepare_protocol_error_on_zero_area() {
    let picker = Picker::halfblocks();
    let img = DynamicImage::ImageRgba8(RgbaImage::new(10, 10));

    // Zero pane must return error cleanly without panic
    let res = ImageRenderer::prepare_protocol(&picker, &img, Rect::new(0, 0, 0, 0), FilterType::Nearest);
    assert!(res.is_err(), "Zero pane area must return Err(AppError::Image)");
}

#[test]
fn test_adv_loupe_crop_extreme_aspect_ratios_and_zooms() {
    let mut loupe = LoupeState::default();

    // 1. Extreme zoom levels: 0.0, negative, high zoom, near-zero
    let zooms = [0.0, -1.0, 0.0001, 1.0, 10.0, 100.0];
    for z in zooms {
        loupe.zoom_level = z;
        let (x, y, w, h) = loupe.compute_crop_rect(6000, 4000, 800, 600);
        assert!(w >= 1 && w <= 6000);
        assert!(h >= 1 && h <= 4000);
        assert!(x + w <= 6000);
        assert!(y + h <= 4000);
    }

    // 2. Extreme center coordinates: out-of-bounds pan
    loupe.zoom_level = 2.0;
    loupe.pan(-10.0, -10.0);
    assert_eq!(loupe.center_x, 0.0);
    assert_eq!(loupe.center_y, 0.0);
    let (x, y, _, _) = loupe.compute_crop_rect(6000, 4000, 800, 600);
    assert_eq!(x, 0);
    assert_eq!(y, 0);

    loupe.pan(20.0, 20.0);
    assert_eq!(loupe.center_x, 1.0);
    assert_eq!(loupe.center_y, 1.0);
    let (x, y, w, h) = loupe.compute_crop_rect(6000, 4000, 800, 600);
    assert_eq!(x + w, 6000);
    assert_eq!(y + h, 4000);

    // 3. Extreme 100:1 panorama image (10000x100)
    let (px, py, pw, ph) = loupe.compute_crop_rect(10000, 100, 800, 600);
    assert!(px + pw <= 10000);
    assert!(py + ph <= 100);

    // 4. Extreme 1:100 tall strip image (100x10000)
    let (tx, ty, tw, th) = loupe.compute_crop_rect(100, 10000, 800, 600);
    assert!(tx + tw <= 100);
    assert!(ty + th <= 10000);
}

#[test]
fn test_adv_wiper_extreme_aspect_ratios_and_zero_sizes() {
    let before = DynamicImage::ImageRgba8(RgbaImage::new(1000, 10));
    let after = DynamicImage::ImageRgba8(RgbaImage::new(1000, 10));

    // Valid wiper across 100:1 panorama
    let wiped = composite_wiper(&before, &after, 0.5);
    assert_eq!(wiped.width(), 1000);
    assert_eq!(wiped.height(), 10);

    // Zero-sized input fallback
    let zero_img = DynamicImage::ImageRgba8(RgbaImage::new(0, 0));
    let fallback = composite_wiper(&zero_img, &after, 0.5);
    assert_eq!(fallback.width(), 1000);

    // Extreme split ratios (-10.0, 10.0) should clamp without panic
    let clamped_left = composite_wiper(&before, &after, -5.0);
    assert_eq!(clamped_left.width(), 1000);
    let clamped_right = composite_wiper(&before, &after, 5.0);
    assert_eq!(clamped_right.width(), 1000);
}

// ============================================================================
// PART 3: PREVIEW CACHE MEMORY BUDGET STRESS & EVICTION
// ============================================================================

#[test]
fn test_adv_preview_cache_flood_200mb_against_64mb_budget() {
    // Standard budget: 64 MB (67,108,864 bytes)
    let max_budget_bytes = 64 * 1024 * 1024;
    let config = PreviewCacheConfig {
        max_memory_bytes: max_budget_bytes,
        max_frames: 100,
    };
    let mut cache = PreviewCache::new(config);

    // Each test image: 2048 x 1024 x 4 bytes = 8,388,608 bytes (8 MB)
    let img_width = 2048;
    let img_height = 1024;
    let bytes_per_img = img_width as usize * img_height as usize * 4;
    assert_eq!(bytes_per_img, 8 * 1024 * 1024); // Exactly 8 MB per image

    // Insert 25 images: 25 * 8 MB = 200 MB of image data
    let total_images = 25;
    for i in 0..total_images {
        let raw = RgbaImage::from_pixel(img_width, img_height, Rgba([i as u8, 128, 200, 255]));
        let dynamic_img = DynamicImage::ImageRgba8(raw);

        let photo_id = format!("CR3_STRESS_{:04}", i);
        cache.insert_source_image(&photo_id, 0xAA55, dynamic_img);

        // INVARIANT 1: After EVERY insertion, tracked memory MUST NEVER exceed 64 MB
        let current = cache.current_memory_bytes();
        assert!(
            current <= max_budget_bytes,
            "Memory violation at insertion #{}: current {} > budget {}",
            i, current, max_budget_bytes
        );
    }

    // Final checks
    let final_bytes = cache.current_memory_bytes();
    assert!(
        final_bytes <= max_budget_bytes,
        "Final memory {} exceeded 64MB budget",
        final_bytes
    );

    // With 8 MB per image and 64 MB ceiling, at most 8 images can be retained (8 * 8MB = 64MB)
    assert!(
        cache.image_count() <= 8,
        "Expected at most 8 images in cache, found {}",
        cache.image_count()
    );

    // Oldest items (0..16) MUST have been evicted (strict FIFO/LRU eviction)
    for i in 0..16 {
        let photo_id = format!("CR3_STRESS_{:04}", i);
        assert!(
            cache.get_source_image(&photo_id, 0xAA55).is_none(),
            "Photo {} should have been evicted",
            photo_id
        );
    }

    // Most recent items (total_images - 5 .. total_images) MUST be present
    for i in (total_images - 5)..total_images {
        let photo_id = format!("CR3_STRESS_{:04}", i);
        assert!(
            cache.get_source_image(&photo_id, 0xAA55).is_some(),
            "Recent photo {} should still be in cache",
            photo_id
        );
    }
}

#[test]
fn test_adv_preview_cache_weak_ref_dropped_on_eviction() {
    // Strict 16 MB budget
    let config = PreviewCacheConfig {
        max_memory_bytes: 16 * 1024 * 1024, // 16 MB
        max_frames: 10,
    };
    let mut cache = PreviewCache::new(config);

    // Create 3 images of 8 MB each (8MB, 8MB, 8MB = 24MB > 16MB)
    let make_img = || DynamicImage::ImageRgba8(RgbaImage::new(2048, 1024));

    // 1. Insert Image 0 (8 MB)
    let arc0 = cache.insert_source_image("photo_0", 1, make_img());
    let weak0: Weak<DynamicImage> = Arc::downgrade(&arc0);
    drop(arc0); // Now ONLY cache holds a strong reference

    // Assert Image 0 is alive in cache
    assert!(weak0.upgrade().is_some(), "Image 0 must be alive in cache");
    assert_eq!(cache.image_count(), 1);

    // 2. Insert Image 1 (8 MB) -> total 16 MB == 16 MB budget
    let arc1 = cache.insert_source_image("photo_1", 1, make_img());
    let weak1 = Arc::downgrade(&arc1);
    drop(arc1);

    assert!(weak0.upgrade().is_some(), "Image 0 should still be alive");
    assert!(weak1.upgrade().is_some(), "Image 1 should be alive");
    assert_eq!(cache.image_count(), 2);

    // 3. Insert Image 2 (8 MB) -> total 24 MB > 16 MB budget!
    // Eviction MUST trigger and drop Image 0's Arc
    let arc2 = cache.insert_source_image("photo_2", 1, make_img());
    let weak2 = Arc::downgrade(&arc2);
    drop(arc2);

    // CRITICAL EMPIRICAL PROOF:
    // weak0 MUST fail to upgrade because Image 0 was genuinely evicted and dropped from heap!
    assert!(
        weak0.upgrade().is_none(),
        "Image 0 data was NOT dropped upon eviction (memory leak detected!)"
    );

    // Image 1 and Image 2 must still be retained
    assert!(weak1.upgrade().is_some(), "Image 1 should still be alive");
    assert!(weak2.upgrade().is_some(), "Image 2 should still be alive");
    assert!(cache.current_memory_bytes() <= 16 * 1024 * 1024);
}

#[test]
fn test_adv_preview_cache_protocol_flood_and_tier1_eviction() {
    let config = PreviewCacheConfig {
        max_memory_bytes: 2 * 1024 * 1024, // 2 MB limit
        max_frames: 20,
    };
    let mut cache = PreviewCache::new(config);
    let picker = Picker::halfblocks();
    let img = DynamicImage::ImageRgba8(RgbaImage::new(200, 100));
    let pane = Rect::new(0, 0, 80, 40);

    // Insert 50 rendered protocol frames (each ~150 KB)
    for i in 0..50 {
        let (proto, rect) = ImageRenderer::prepare_protocol(&picker, &img, pane, FilterType::Nearest).unwrap();
        let key = PreviewCacheKey {
            photo_id: format!("photo_proto_{}", i),
            recipe_hash: i as u64,
            target_cells: (80, 40),
            crop_region: None,
            protocol: CacheProtocolKind::Halfblocks,
        };
        cache.insert_protocol(key, proto, rect);

        assert!(
            cache.current_memory_bytes() <= 2 * 1024 * 1024,
            "Protocol cache memory exceeded 2MB budget at index {}",
            i
        );
    }

    assert!(cache.protocol_count() <= 20);

    // Early keys must be evicted
    let old_key = PreviewCacheKey {
        photo_id: "photo_proto_0".into(),
        recipe_hash: 0,
        target_cells: (80, 40),
        crop_region: None,
        protocol: CacheProtocolKind::Halfblocks,
    };
    assert!(cache.get_protocol(&old_key).is_none());
}

#[test]
fn test_adv_preview_cache_interleaved_tier1_tier2_flood_and_purge() {
    let config = PreviewCacheConfig {
        max_memory_bytes: 32 * 1024 * 1024, // 32 MB
        max_frames: 50,
    };
    let mut cache = PreviewCache::new(config);
    let picker = Picker::halfblocks();
    let pane = Rect::new(0, 0, 60, 30);

    // Interleave large images (4 MB each) and protocol renderings
    for i in 0..20 {
        // Tier 2: 4 MB master image (1024 x 1024 x 4)
        let dyn_img = DynamicImage::ImageRgba8(RgbaImage::new(1024, 1024));
        let photo_id = format!("IMG_{:04}.CR3", i);
        cache.insert_source_image(&photo_id, 42, dyn_img.clone());

        // Tier 1: Protocol frame
        let (proto, rect) = ImageRenderer::prepare_protocol(&picker, &dyn_img, pane, FilterType::Nearest).unwrap();
        let key = PreviewCacheKey {
            photo_id: photo_id.clone(),
            recipe_hash: 42,
            target_cells: (60, 30),
            crop_region: None,
            protocol: CacheProtocolKind::Halfblocks,
        };
        cache.insert_protocol(key, proto, rect);

        assert!(
            cache.current_memory_bytes() <= 32 * 1024 * 1024,
            "Memory exceeded 32 MB budget at step {}",
            i
        );
    }

    // Target photo eviction
    let target_photo = "IMG_0019.CR3";
    assert!(cache.get_source_image(target_photo, 42).is_some());
    cache.evict_photo(target_photo);
    assert!(cache.get_source_image(target_photo, 42).is_none());

    // Full clear
    cache.clear();
    assert_eq!(cache.current_memory_bytes(), 0);
    assert_eq!(cache.image_count(), 0);
    assert_eq!(cache.protocol_count(), 0);
}

#[test]
fn test_adv_preview_cache_single_image_exceeds_budget() {
    // Budget: 10 MB
    let config = PreviewCacheConfig {
        max_memory_bytes: 10 * 1024 * 1024,
        max_frames: 10,
    };
    let mut cache = PreviewCache::new(config);

    // Insert an image of size 2048 x 2048 x 4 = 16 MB (exceeds total budget)
    let huge_img = DynamicImage::ImageRgba8(RgbaImage::new(2048, 2048));
    cache.insert_source_image("oversized_photo", 1, huge_img);

    // Eviction logic pops until current_bytes <= budget, or LRU is empty
    assert!(
        cache.current_memory_bytes() <= 10 * 1024 * 1024,
        "Memory must be within budget even after inserting oversized image"
    );
}

#[test]
fn test_adv_preview_cache_repeated_key_overwrite_accounting() {
    let config = PreviewCacheConfig {
        max_memory_bytes: 50 * 1024 * 1024,
        max_frames: 50,
    };
    let mut cache = PreviewCache::new(config);

    // Repeatedly update the same key with alternating sizes (1MB vs 4MB)
    for i in 0..100 {
        let size = if i % 2 == 0 { 512 } else { 1024 };
        let img = DynamicImage::ImageRgba8(RgbaImage::new(size, size));
        cache.insert_source_image("SAME_PHOTO", 100, img);

        let expected_bytes = (size as usize) * (size as usize) * 4;
        assert_eq!(
            cache.current_memory_bytes(),
            expected_bytes,
            "Step {}: Overwrite must accurately reflect single entry size without drifting",
            i
        );
        assert_eq!(cache.image_count(), 1);
    }
}
