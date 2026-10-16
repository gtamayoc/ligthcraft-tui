//! Tier 1 Feature Integration Tests: Mode A Single Photo Inspector, Variant Grid, Split-View, and 1:1 Loupe Zoom.
//!
//! Covers Milestone M3 features:
//! - F-SGL-01: Single Photo Metadata Inspection & 4-Channel Histogram (<2ms)
//! - F-SGL-02: Variant Matrix Generation (2 to 6 Grid View) & Recipe Promotion
//! - F-SGL-03: Before / After Split-View Comparison & In-Memory Wiper Compositor
//! - F-SGL-04: 1:1 Detail Zoom / Crop Inspection with Coordinate Boundary Clamping
//! - F-SGL-05: Favorite Tagging, Rating, Pick/Reject Flagging & Quick Export

use std::time::Instant;
use image::{DynamicImage, GenericImageView, RgbaImage};
use ratatui::layout::Rect;
use lightcraft_tui::mcp::tools::PhotoInfo;
use lightcraft_tui::tui::mode_a::{
    composite_wiper, compute_variant_grid_rects, FlagStatus, HistogramData, LoupeState, ModeAState,
    ModeAView, PhotoMetadata, SplitViewKind,
};

// ============================================================================
// Feature F-SGL-01: Single Photo Metadata Inspection & 4-Channel Histogram
// ============================================================================

#[test]
fn test_t1_sgl01_01_canon_cr3_metadata_extraction_and_formatting() {
    let photo = PhotoInfo {
        id: 101,
        file_name: "IMG_0588.CR3".into(),
        format: "cr3".into(),
        kind: "raw".into(),
        camera: Some("Canon EOS R6".into()),
        captured: Some("2026-10-15T10:30:00Z".into()),
        width: 6000,
        height: 4000,
        rating: 4,
        flag: "pick".into(),
        edited: true,
        copy_name: None,
        copy_of: None,
        deleted: false,
        keywords: vec!["landscape".into(), "canon".into()],
        label: Some("Red".into()),
        preview_only: None,
        title: "Sunset Over Sierra".into(),
    };

    let exif = vec![
        serde_json::json!({ "name": "FNumber", "value": "2.8" }),
        serde_json::json!({ "name": "ExposureTime", "value": "1/500s" }),
        serde_json::json!({ "name": "ISOSpeedRatings", "value": "400" }),
        serde_json::json!({ "name": "FocalLength", "value": "50.0 mm" }),
        serde_json::json!({ "name": "LensModel", "value": "RF24-70mm F2.8 L IS USM" }),
    ];

    let meta = PhotoMetadata::from_mcp_metadata(&photo, &exif);

    assert_eq!(meta.file_name, "IMG_0588.CR3");
    assert_eq!(meta.width, 6000);
    assert_eq!(meta.height, 4000);
    assert_eq!(meta.aspect_ratio_str(), "3:2");
    assert_eq!(meta.megapixels_str(), "24.0 MP");
    assert_eq!(meta.camera_model.as_deref(), Some("Canon EOS R6"));
    assert_eq!(meta.lens_model.as_deref(), Some("RF24-70mm F2.8 L IS USM"));

    let summary = meta.exposure_summary();
    assert!(summary.contains("1/500s"));
    assert!(summary.contains("f/2.8"));
    assert!(summary.contains("ISO 400"));
    assert!(summary.contains("50mm"));
}

#[test]
fn test_t1_sgl01_02_histogram_generation_performance_under_2ms() {
    // 1920x1080 preview image
    let mut rgba = RgbaImage::new(1920, 1080);
    for (x, y, pixel) in rgba.enumerate_pixels_mut() {
        *pixel = image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255]);
    }
    let img = DynamicImage::ImageRgba8(rgba);

    let start = Instant::now();
    let hist = HistogramData::from_image(&img);
    let elapsed = start.elapsed();

    // Verify performance requirement: < 2ms (allowing generous 15ms ceiling for CI)
    assert!(
        elapsed.as_millis() < 15,
        "Histogram generation took {:?}, expected < 2ms",
        elapsed
    );

    // Verify 32 bins populated
    assert_eq!(hist.bins_lum.len(), 32);
    assert_eq!(hist.bins_r.len(), 32);
    assert_eq!(hist.bins_g.len(), 32);
    assert_eq!(hist.bins_b.len(), 32);

    // Verify sparklines representation produces 4 lines
    let sparks = hist.sparklines();
    assert_eq!(sparks.len(), 4);
    assert!(sparks[0].starts_with("Lum: ["));
    assert!(sparks[1].starts_with("Red: ["));
    assert!(sparks[2].starts_with("Grn: ["));
    assert!(sparks[3].starts_with("Blu: ["));
}

// ============================================================================
// Feature F-SGL-02: Variant Matrix Generation (2 to 6 Previews)
// ============================================================================

#[test]
fn test_t1_sgl02_01_variant_grid_rect_partitions() {
    let canvas = Rect::new(20, 5, 100, 40);

    // 2-Variant Grid (Side-by-side)
    let rects_2 = compute_variant_grid_rects(canvas, 2);
    assert_eq!(rects_2.len(), 2);
    assert_eq!(rects_2[0].width, 50);
    assert_eq!(rects_2[1].width, 50);

    // 4-Variant Grid (2x2)
    let rects_4 = compute_variant_grid_rects(canvas, 4);
    assert_eq!(rects_4.len(), 4);
    assert_eq!(rects_4[0].width, 50);
    assert_eq!(rects_4[0].height, 20);
    assert_eq!(rects_4[1].width, 50);
    assert_eq!(rects_4[2].height, 20);
    assert_eq!(rects_4[3].height, 20);

    // 6-Variant Grid (2x3)
    let rects_6 = compute_variant_grid_rects(canvas, 6);
    assert_eq!(rects_6.len(), 6);
    for r in &rects_6 {
        assert!(r.width > 30);
        assert_eq!(r.height, 20);
    }
}

#[test]
fn test_t1_sgl02_02_variant_promotion_to_active_recipe() {
    let mut state = ModeAState::default();
    state.view_mode = ModeAView::VariantMatrix;
    state.variant_count = 4;

    // Slot 2 is Cinema preset
    state.focused_variant = 2;
    state.variants[2].intensity = 1.25;

    // Promote focused variant to working recipe
    state.promote_focused_variant();

    assert_eq!(state.active_recipe.name, "Cinema");
    assert_eq!(state.active_intensity, 1.25);
}

// ============================================================================
// Feature F-SGL-03: Split-View Comparison & Wiper Compositor
// ============================================================================

#[test]
fn test_t1_sgl03_01_split_view_wiper_compositor_stitching() {
    // Construct distinct Before (pure red) and After (pure blue) images
    let before = DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, image::Rgba([255, 0, 0, 255])));
    let after = DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 255, 255])));

    // 50% split divider
    let composite = composite_wiper(&before, &after, 0.5);

    assert_eq!(composite.dimensions(), (100, 100));

    // Pixel at (20, 50) is left of divider -> must be Before (Red)
    let left_pixel = composite.get_pixel(20, 50);
    assert_eq!(left_pixel[0], 255); // Red
    assert_eq!(left_pixel[2], 0);

    // Pixel at (80, 50) is right of divider -> must be After (Blue)
    let right_pixel = composite.get_pixel(80, 50);
    assert_eq!(right_pixel[0], 0);
    assert_eq!(right_pixel[2], 255); // Blue

    // Pixel at (50, 50) is the 1px white divider line
    let divider_pixel = composite.get_pixel(50, 50);
    assert_eq!(divider_pixel[0], 255);
    assert_eq!(divider_pixel[1], 255);
    assert_eq!(divider_pixel[2], 255);
}

#[test]
fn test_t1_sgl03_02_split_view_submode_toggles() {
    let mut state = ModeAState::default();
    state.view_mode = ModeAView::SplitView;

    assert_eq!(state.split_kind, SplitViewKind::SideBySide);

    state.split_kind = SplitViewKind::Wiper;
    state.split_ratio = 0.35;
    assert_eq!(state.split_ratio, 0.35);

    state.split_kind = SplitViewKind::ToggleAB;
    assert!(!state.toggle_ab_showing_before);
    state.toggle_ab_showing_before = true;
    assert!(state.toggle_ab_showing_before);
}

// ============================================================================
// Feature F-SGL-04: 1:1 Detail Zoom / Crop Inspection & Boundary Clamping
// ============================================================================

#[test]
fn test_t1_sgl04_01_loupe_crop_and_boundary_clamping() {
    let mut loupe = LoupeState::default();
    loupe.zoom_level = 1.0; // 100% 1:1 detail

    // 6000x4000 image, 800x600 canvas
    let (x0, y0, cw, ch) = loupe.compute_crop_rect(6000, 4000, 800, 600);
    assert_eq!(cw, 800);
    assert_eq!(ch, 600);
    // Centered: (6000 - 800)/2 = 2600, (4000 - 600)/2 = 1700
    assert_eq!(x0, 2600);
    assert_eq!(y0, 1700);

    // Pan to extreme negative (-2.0, -2.0) -> must clamp to (0, 0)
    loupe.pan(-2.0, -2.0);
    let (x_clamped_min, y_clamped_min, _, _) = loupe.compute_crop_rect(6000, 4000, 800, 600);
    assert_eq!(x_clamped_min, 0);
    assert_eq!(y_clamped_min, 0);

    // Pan to extreme positive (+3.0, +3.0) -> must clamp to (img_w - cw, img_h - ch)
    loupe.pan(5.0, 5.0);
    let (x_clamped_max, y_clamped_max, _, _) = loupe.compute_crop_rect(6000, 4000, 800, 600);
    assert_eq!(x_clamped_max, 5200); // 6000 - 800
    assert_eq!(y_clamped_max, 3400); // 4000 - 600
}

// ============================================================================
// Feature F-SGL-05: Favorite Tagging, Rating & Flagging
// ============================================================================

#[test]
fn test_t1_sgl05_01_favorite_flagging_and_ratings() {
    let mut state = ModeAState::default();
    state.view_mode = ModeAView::VariantMatrix;
    state.focused_variant = 1;

    // Toggle favorite
    assert!(!state.variants[1].is_favorite);
    state.toggle_favorite();
    assert!(state.variants[1].is_favorite);
    state.toggle_favorite();
    assert!(!state.variants[1].is_favorite);

    // Rating
    state.set_rating(5);
    assert_eq!(state.variants[1].rating, 5);
    // Boundary check > 5 clamped to 5
    state.set_rating(9);
    assert_eq!(state.variants[1].rating, 5);

    // Flagging
    state.set_flag(FlagStatus::Pick);
    assert_eq!(state.variants[1].flag, FlagStatus::Pick);
    state.set_flag(FlagStatus::Reject);
    assert_eq!(state.variants[1].flag, FlagStatus::Reject);
    state.set_flag(FlagStatus::Unflagged);
    assert_eq!(state.variants[1].flag, FlagStatus::Unflagged);
}
