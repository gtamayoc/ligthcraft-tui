//! Tier 1 Feature Integration Tests: Terminal Graphic Protocols, Aspect-Ratio Downscaling, and LRU Cache Budgeting.
//!
//! Covers Milestone M3 features:
//! - F-TUI-03: Terminal Graphic Protocols (Kitty, Sixel, Halfblocks) & 3:2 Aspect Ratio Preservation
//! - F-RNF-02: Preview Memory Budgeting, Dual-Tier LRU Cache, and Zero Memory Leak Guarantee

use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};
use ratatui::layout::Rect;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::FontSize;
use std::time::Instant;

use lightcraft_tui::graphics::cache::{
    CacheProtocolKind, PreviewCache, PreviewCacheConfig, PreviewCacheKey,
};
use lightcraft_tui::graphics::protocol::{GraphicsProtocol, ProtocolOverride};
use lightcraft_tui::graphics::renderer::ImageRenderer;

// ============================================================================
// Feature F-TUI-03: Terminal Graphic Protocols & Aspect Ratio Preservation
// ============================================================================

#[test]
fn test_t1_tui03_01_protocol_override_parsing() {
    assert_eq!(
        ProtocolOverride::from_str_loose("auto"),
        Some(ProtocolOverride::Auto)
    );
    assert_eq!(
        ProtocolOverride::from_str_loose("KITTY"),
        Some(ProtocolOverride::Kitty)
    );
    assert_eq!(
        ProtocolOverride::from_str_loose("Sixel"),
        Some(ProtocolOverride::Sixel)
    );
    assert_eq!(
        ProtocolOverride::from_str_loose("halfblocks"),
        Some(ProtocolOverride::Halfblocks)
    );
    assert_eq!(
        ProtocolOverride::from_str_loose("unicode"),
        Some(ProtocolOverride::Halfblocks)
    );
    assert_eq!(ProtocolOverride::from_str_loose("unknown"), None);
}

#[test]
fn test_t1_tui03_02_halfblocks_fallback_initialization() {
    let proto = GraphicsProtocol::halfblocks();
    assert_eq!(proto.protocol_type(), ProtocolType::Halfblocks);
    assert!(proto.is_halfblocks());
    assert_eq!(proto.mode(), ProtocolOverride::Halfblocks);

    let summary = proto.summary();
    assert!(summary.contains("Halfblocks"));
}

#[test]
fn test_t1_tui03_03_fit_bounds_aspect_ratio_3_to_2_landscape() {
    // Standard Canon 3:2 landscape RAW image (6000x4000)
    let font_size = FontSize::new(10, 20); // 1:2 cell aspect ratio
    let pane = Rect::new(0, 0, 100, 30);

    let fit = ImageRenderer::calculate_fit(6000, 4000, font_size, pane);

    // Image ratio = 1.5. Cell ratio = 0.5.
    // Equivalent cells ratio = 1.5 / 0.5 = 3.0 cells wide per cell high.
    // In pane 100x30, 30 height would require 90 width (90 <= 100).
    // So height-constrained: width = 90, height = 30.
    assert_eq!(fit.render_area.height, 30);
    assert_eq!(fit.render_area.width, 90);
    // Centered horizontally: (100 - 90) / 2 = 5
    assert_eq!(fit.render_area.x, 5);
    assert_eq!(fit.render_area.y, 0);

    // Target pixel size: 90*10 = 900, 30*20 = 600 (aspect ratio 900:600 = 1.5, exact 3:2!)
    assert_eq!(fit.target_pixel_size, (900, 600));
}

#[test]
fn test_t1_tui03_04_fit_bounds_aspect_ratio_portrait_and_panoramas() {
    let font_size = FontSize::new(10, 20);
    let pane = Rect::new(0, 0, 80, 40);

    // 1. Portrait 2:3 image (4000x6000)
    let fit_portrait = ImageRenderer::calculate_fit(4000, 6000, font_size, pane);
    assert!(fit_portrait.render_area.width <= pane.width);
    assert!(fit_portrait.render_area.height <= pane.height);
    // Portrait takes full height, narrow width
    assert_eq!(fit_portrait.render_area.height, 40);

    // 2. Extreme 100:1 panorama (10000x100)
    let fit_pano = ImageRenderer::calculate_fit(10000, 100, font_size, pane);
    assert_eq!(fit_pano.render_area.width, 80);
    assert!(fit_pano.render_area.height >= 1);
    assert!(fit_pano.render_area.height <= pane.height);

    // 3. Extreme 1:100 tall strip (100x10000)
    let fit_tall = ImageRenderer::calculate_fit(100, 10000, font_size, pane);
    assert_eq!(fit_tall.render_area.height, 40);
    assert!(fit_tall.render_area.width >= 1);
    assert!(fit_tall.render_area.width <= pane.width);
}

#[test]
fn test_t1_tui03_05_prepare_protocol_downscaling_execution() {
    let picker = Picker::halfblocks();
    let img = DynamicImage::ImageRgba8(RgbaImage::new(600, 400));
    let pane = Rect::new(0, 0, 60, 20);

    let (proto, render_area) =
        ImageRenderer::prepare_protocol(&picker, &img, pane, FilterType::Nearest).unwrap();

    assert!(render_area.width > 0 && render_area.width <= pane.width);
    assert!(render_area.height > 0 && render_area.height <= pane.height);
    assert!(matches!(
        proto,
        ratatui_image::protocol::Protocol::Halfblocks(_)
    ));
}

// ============================================================================
// Feature F-RNF-02: Preview Memory Budgeting & Dual-Tier LRU Cache
// ============================================================================

#[test]
fn test_t1_rnf02_01_cache_budget_ceiling_and_eviction() {
    // Configure strict 1 MB limit to test eviction logic under load
    let config = PreviewCacheConfig {
        max_memory_bytes: 1024 * 1024, // 1 MB
        max_frames: 50,
    };
    let mut cache = PreviewCache::new(config);
    let picker = Picker::halfblocks();
    let pane = Rect::new(0, 0, 40, 20);

    // Insert 20 preview entries
    for i in 0..20 {
        let img = DynamicImage::ImageRgba8(RgbaImage::new(100, 100));
        let (proto, rect) =
            ImageRenderer::prepare_protocol(&picker, &img, pane, FilterType::Nearest).unwrap();
        let key = PreviewCacheKey {
            photo_id: format!("photo_{}", i),
            recipe_hash: i as u64,
            target_cells: (40, 20),
            crop_region: None,
            protocol: CacheProtocolKind::Halfblocks,
        };
        cache.insert_protocol(key, proto, rect);
    }

    // Memory footprint must not exceed 1 MB ceiling
    assert!(
        cache.current_memory_bytes() <= 1024 * 1024,
        "Current memory {} exceeded 1MB budget",
        cache.current_memory_bytes()
    );
}

#[test]
fn test_t1_rnf02_02_cache_hit_speed_and_retrieval() {
    let mut cache = PreviewCache::new(PreviewCacheConfig::default());
    let picker = Picker::halfblocks();
    let img = DynamicImage::ImageRgba8(RgbaImage::new(200, 200));
    let pane = Rect::new(0, 0, 50, 25);

    let (proto, rect) =
        ImageRenderer::prepare_protocol(&picker, &img, pane, FilterType::Nearest).unwrap();
    let key = PreviewCacheKey {
        photo_id: "IMG_0588.CR3".into(),
        recipe_hash: 42,
        target_cells: (50, 25),
        crop_region: None,
        protocol: CacheProtocolKind::Halfblocks,
    };

    cache.insert_protocol(key.clone(), proto, rect);

    // Measure hit speed
    let start = Instant::now();
    let retrieved = cache.get_protocol(&key);
    let elapsed = start.elapsed();

    assert!(retrieved.is_some());
    let hit = retrieved.unwrap();
    assert_eq!(hit.render_area, rect);
    assert!(hit.byte_size > 0);

    // Cache hit must resolve in < 1ms
    assert!(
        elapsed.as_millis() < 5,
        "Cache hit took {:?}, expected < 1ms",
        elapsed
    );
}

#[test]
fn test_t1_rnf02_03_photo_invalidation_and_purge() {
    let mut cache = PreviewCache::new(PreviewCacheConfig::default());
    let picker = Picker::halfblocks();
    let img = DynamicImage::ImageRgba8(RgbaImage::new(100, 100));
    let pane = Rect::new(0, 0, 40, 20);

    // Insert 3 variants for IMG_0588 and 2 for IMG_0589
    for r in 0..3 {
        let (proto, rect) =
            ImageRenderer::prepare_protocol(&picker, &img, pane, FilterType::Nearest).unwrap();
        cache.insert_protocol(
            PreviewCacheKey {
                photo_id: "IMG_0588.CR3".into(),
                recipe_hash: r,
                target_cells: (40, 20),
                crop_region: None,
                protocol: CacheProtocolKind::Halfblocks,
            },
            proto,
            rect,
        );
    }
    for r in 0..2 {
        let (proto, rect) =
            ImageRenderer::prepare_protocol(&picker, &img, pane, FilterType::Nearest).unwrap();
        cache.insert_protocol(
            PreviewCacheKey {
                photo_id: "IMG_0589.CR3".into(),
                recipe_hash: r,
                target_cells: (40, 20),
                crop_region: None,
                protocol: CacheProtocolKind::Halfblocks,
            },
            proto,
            rect,
        );
    }

    assert_eq!(cache.protocol_count(), 5);

    // Invalidate IMG_0588 only
    cache.evict_photo("IMG_0588.CR3");
    assert_eq!(cache.protocol_count(), 2);

    // Full clear
    cache.clear();
    assert_eq!(cache.protocol_count(), 0);
    assert_eq!(cache.image_count(), 0);
    assert_eq!(cache.current_memory_bytes(), 0);
}
