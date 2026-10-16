//! Test suite verifying autonomous and native metadata extraction in LightCraft TUI.

use tempfile::NamedTempFile;

use lightcraft_tui::storage::metadata::read_native_metadata;
use lightcraft_tui::tui::mode_a::PhotoMetadata;

#[test]
fn test_native_metadata_formats_summary_without_panics() {
    let mut meta = PhotoMetadata {
        file_name: "test_shot.raw".to_string(),
        file_size_bytes: 24_500_123,
        format: "RAW".to_string(),
        width: 6000,
        height: 4000,
        orientation: "Normal".to_string(),
        color_space: "sRGB".to_string(),
        camera_make: Some("Sony".to_string()),
        camera_model: Some("ILCE-7RM4".to_string()),
        lens_model: Some("FE 24-70mm F2.8 GM".to_string()),
        focal_length_mm: Some(35.0),
        aperture_f_number: Some(2.8),
        shutter_speed: Some("1/250s".to_string()),
        iso: Some(100),
        exposure_bias_ev: Some(0.0),
        metering_mode: Some("Multi-segment".to_string()),
        white_balance: Some("Auto".to_string()),
        capture_time: Some("2026-10-16 12:00:00".to_string()),
    };

    assert_eq!(meta.aspect_ratio_str(), "3:2");
    assert_eq!(meta.megapixels_str(), "24.0 MP");
    assert_eq!(meta.exposure_summary(), "1/250s  f/2.8  ISO 100  35mm");

    // Test with missing fields
    meta.camera_model = None;
    meta.shutter_speed = None;
    meta.aperture_f_number = None;
    meta.iso = None;
    meta.focal_length_mm = None;
    meta.width = 0;
    meta.height = 0;

    assert_eq!(meta.aspect_ratio_str(), "Unknown");
    assert_eq!(meta.megapixels_str(), "0.0 MP");
    assert_eq!(meta.exposure_summary(), "1/---s  f/--  ISO ---  --mm");
}

#[test]
fn test_native_metadata_extraction_on_generated_image() {
    // Generate a valid 400x300 image to verify dimensions fallback
    let mut tmp = NamedTempFile::new().unwrap();
    let img = image::RgbaImage::new(400, 300);
    img.write_to(&mut tmp, image::ImageFormat::Png).unwrap();

    let meta = read_native_metadata(tmp.path());
    assert_eq!(meta.width, 400);
    assert_eq!(meta.height, 300);
    assert_eq!(meta.aspect_ratio_str(), "4:3");
    assert_eq!(meta.megapixels_str(), "0.1 MP");
}

#[test]
fn test_from_mcp_metadata_heterogeneous_json_types() {
    let photo = lightcraft_tui::mcp::tools::PhotoInfo {
        id: 42,
        file_name: "hetero_test.jpg".to_string(),
        format: "jpeg".to_string(),
        kind: "raster".to_string(),
        camera: None,
        captured: None,
        width: 1920,
        height: 1080,
        rating: 5,
        flag: "pick".to_string(),
        edited: false,
        copy_name: None,
        copy_of: None,
        deleted: false,
        keywords: vec![],
        label: None,
        preview_only: None,
        title: "Test".to_string(),
    };

    let raw_exif = vec![
        // Numeric float instead of string
        serde_json::json!({ "name": "FNumber", "value": 1.8 }),
        // Numeric exposure time
        serde_json::json!({ "name": "ExposureTime", "value": 0.005 }),
        // Array of ISO ratings
        serde_json::json!({ "name": "ISOSpeedRatings", "value": [800] }),
        // Numeric focal length
        serde_json::json!({ "name": "FocalLength", "value": 85.0 }),
        // String model
        serde_json::json!({ "name": "Model", "value": "Fujifilm X-T5" }),
    ];

    let meta = PhotoMetadata::from_mcp_metadata(&photo, &raw_exif);
    assert_eq!(meta.camera_model.as_deref(), Some("Fujifilm X-T5"));
    assert_eq!(meta.aperture_f_number, Some(1.8));
    assert_eq!(meta.iso, Some(800));
    assert_eq!(meta.focal_length_mm, Some(85.0));
    assert_eq!(meta.shutter_speed.as_deref(), Some("1/200s"));
}
