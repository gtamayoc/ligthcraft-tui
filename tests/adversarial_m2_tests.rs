#![allow(dead_code)]

//! Milestone M2 Adversarial Stress Test Suite: Recipe Engine & Intensity Scaling Mathematics.
//!
//! Empirically stress-tests:
//! 1. Floating point robustness: NaN, +Inf, -Inf, MIN_POSITIVE, MAX, -0.0, subnormals.
//! 2. Out-of-bounds intensity: Negative (-1.0, -50.0) and excessive (+2.0, +10.0, +100.0) scaling.
//! 3. Monotonicity: Adjustment magnitude strictly non-decreasing across 0.0 to 1.5.
//! 4. Neutral preservation: 0% intensity yields exact neutral / baseline settings for ANY recipe.
//! 5. Built-in presets integrity: All 5 curated presets remain mathematically valid across [0.0, 1.5].
//! 6. Boundary clamping: All 16 sliders strictly respect physical SliderBounds.
//! 7. State robustness: Sanitization and edge case behavior on corrupt recipe data.

mod common;

#[path = "../src/engine/mod.rs"]
pub mod engine;
#[path = "../src/error.rs"]
pub mod error;
#[path = "../src/mcp/mod.rs"]
pub mod mcp;
#[path = "../src/storage/mod.rs"]
pub mod storage;

use common::TestFixture;
use engine::develop::DevelopBridge;
use engine::presets::BuiltinPreset;
use engine::recipe::{DevelopSlider, Recipe};
use mcp::tools::DevelopSettings;

/// Helper: verifies all 16 photographic sliders of a recipe are strictly within their physical SliderBounds,
/// and that no slider contains NaN or Infinity.
fn assert_recipe_within_bounds(r: &Recipe, context: &str) {
    macro_rules! check_slider {
        ($slider:expr, $val:expr, $name:expr) => {
            let bounds = $slider.bounds();
            assert!(
                !$val.is_nan(),
                "[{}] {} must not be NaN, got {}",
                context,
                $name,
                $val
            );
            assert!(
                !$val.is_infinite(),
                "[{}] {} must not be Infinite, got {}",
                context,
                $name,
                $val
            );
            assert!(
                $val >= bounds.min && $val <= bounds.max,
                "[{}] {} = {} outside physical bounds [{}, {}]",
                context,
                $name,
                $val,
                bounds.min,
                bounds.max
            );
        };
    }

    check_slider!(DevelopSlider::Exposure, r.exposure, "exposure");
    check_slider!(DevelopSlider::Contrast, r.contrast, "contrast");
    check_slider!(DevelopSlider::Highlights, r.highlights, "highlights");
    check_slider!(DevelopSlider::Shadows, r.shadows, "shadows");
    check_slider!(DevelopSlider::Whites, r.whites, "whites");
    check_slider!(DevelopSlider::Blacks, r.blacks, "blacks");
    check_slider!(DevelopSlider::Tint, r.tint, "tint");
    check_slider!(DevelopSlider::Vibrance, r.vibrance, "vibrance");
    check_slider!(DevelopSlider::Saturation, r.saturation, "saturation");
    check_slider!(DevelopSlider::Clarity, r.clarity, "clarity");
    check_slider!(DevelopSlider::Dehaze, r.dehaze, "dehaze");
    check_slider!(
        DevelopSlider::SharpenAmount,
        r.sharpen_amount,
        "sharpen_amount"
    );
    check_slider!(DevelopSlider::NrLuminance, r.nr_luminance, "nr_luminance");
    check_slider!(DevelopSlider::NrColor, r.nr_color, "nr_color");
    check_slider!(DevelopSlider::Vignette, r.vignette, "vignette");

    if let Some(temp) = r.temperature {
        let bounds = DevelopSlider::Temperature.bounds();
        assert!(!temp.is_nan(), "[{}] temperature must not be NaN", context);
        assert!(
            !temp.is_infinite(),
            "[{}] temperature must not be Infinite",
            context
        );
        assert!(
            temp >= bounds.min && temp <= bounds.max,
            "[{}] temperature = {} outside physical bounds [{}, {}]",
            context,
            temp,
            bounds.min,
            bounds.max
        );
    }

    // Verify structured blocks
    if let Some(ref m) = r.mixer {
        macro_rules! check_hsl {
            ($color:expr, $name:expr) => {
                assert!(!$color.hue.is_nan() && $color.hue >= -100.0 && $color.hue <= 100.0);
                assert!(!$color.sat.is_nan() && $color.sat >= -100.0 && $color.sat <= 100.0);
                assert!(!$color.lum.is_nan() && $color.lum >= -100.0 && $color.lum <= 100.0);
            };
        }
        check_hsl!(m.red, "mixer.red");
        check_hsl!(m.orange, "mixer.orange");
        check_hsl!(m.yellow, "mixer.yellow");
        check_hsl!(m.green, "mixer.green");
        check_hsl!(m.aqua, "mixer.aqua");
        check_hsl!(m.blue, "mixer.blue");
        check_hsl!(m.purple, "mixer.purple");
        check_hsl!(m.magenta, "mixer.magenta");
    }

    if let Some(ref bw) = r.bw_mix {
        for (val, name) in [
            (bw.red, "bw.red"),
            (bw.orange, "bw.orange"),
            (bw.yellow, "bw.yellow"),
            (bw.green, "bw.green"),
            (bw.aqua, "bw.aqua"),
            (bw.blue, "bw.blue"),
            (bw.purple, "bw.purple"),
            (bw.magenta, "bw.magenta"),
        ] {
            assert!(!val.is_nan(), "[{}] {} must not be NaN", context, name);
            assert!(
                val >= -100.0 && val <= 100.0,
                "[{}] {} outside [-100, 100]",
                context,
                name
            );
        }
    }

    if let Some(ref g) = r.grading {
        for (hsl, name) in [
            (&g.shadows, "grading.shadows"),
            (&g.midtones, "grading.midtones"),
            (&g.highlights, "grading.highlights"),
            (&g.global, "grading.global"),
        ] {
            assert!(
                !hsl.sat.is_nan() && hsl.sat >= 0.0 && hsl.sat <= 100.0,
                "[{}] {} sat out of bounds",
                context,
                name
            );
            assert!(
                !hsl.lum.is_nan() && hsl.lum >= -100.0 && hsl.lum <= 100.0,
                "[{}] {} lum out of bounds",
                context,
                name
            );
        }
    }

    if let Some(ref gr) = r.grain {
        assert!(!gr.amount.is_nan() && gr.amount >= 0.0 && gr.amount <= 100.0);
    }

    for (k, v) in &r.custom_sliders {
        if let Some(num) = v.as_f64() {
            assert!(!num.is_nan(), "[{}] custom slider {} is NaN", context, k);
            assert!(
                !num.is_infinite(),
                "[{}] custom slider {} is Infinite",
                context,
                k
            );
            assert!(
                num >= -100.0 && num <= 100.0,
                "[{}] custom slider {} = {} outside [-100, 100]",
                context,
                k,
                num
            );
        }
    }
}

/// Helper: construct an extreme recipe with all 16 sliders near or at max/min boundaries.
fn make_extreme_recipe() -> Recipe {
    let mut r = Recipe::default();
    r.id = "extreme-stress".to_string();
    r.name = "Extreme Stress Recipe".to_string();
    r.exposure = 4.8;
    r.contrast = -95.0;
    r.highlights = 98.0;
    r.shadows = -90.0;
    r.whites = 85.0;
    r.blacks = -80.0;
    r.temperature = Some(48000.0);
    r.tint = -140.0;
    r.vibrance = 90.0;
    r.saturation = -95.0;
    r.clarity = 95.0;
    r.dehaze = -85.0;
    r.sharpen_amount = 140.0;
    r.nr_luminance = 90.0;
    r.nr_color = 95.0;
    r.vignette = -90.0;
    r
}

// ============================================================================
// 1. FLOATING POINT ROBUSTNESS
// ============================================================================

#[test]
fn test_adv_fp_01_nan_and_infinities_to_apply_intensity() {
    let extreme = make_extreme_recipe();
    let presets = BuiltinPreset::all();

    let adversarial_floats: Vec<(f64, &str)> = vec![
        (f64::NAN, "NaN"),
        (f64::INFINITY, "+Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
        (f64::MIN_POSITIVE, "MIN_POSITIVE (2.225e-308)"),
        (f64::MAX, "MAX (1.797e+308)"),
        (-0.0, "-0.0"),
        (f64::MIN, "MIN (-1.797e+308)"),
        (f64::EPSILON, "EPSILON"),
        (f64::MIN_POSITIVE / 2.0, "subnormal positive"),
        (-f64::MIN_POSITIVE / 2.0, "subnormal negative"),
    ];

    // Test on default recipe, extreme recipe, and all 5 built-in presets
    let mut test_recipes = vec![Recipe::default(), extreme];
    for p in presets {
        test_recipes.push(p.recipe());
    }

    for recipe in &test_recipes {
        for (fp_val, label) in &adversarial_floats {
            let ctx = format!("Recipe '{}' with intensity {}", recipe.id, label);
            // apply_intensity must NEVER panic on any floating point input
            let scaled = recipe.apply_intensity(*fp_val);
            assert_recipe_within_bounds(&scaled, &ctx);

            // In addition, verify evaluate_flat does not panic and contains no NaN
            let flat = recipe.evaluate_flat(*fp_val, Some(5500.0));
            for (k, v) in &flat {
                assert!(!v.is_nan(), "[{}] flat value for {} is NaN", ctx, k);
                assert!(
                    !v.is_infinite(),
                    "[{}] flat value for {} is Infinite",
                    ctx,
                    k
                );
            }
        }
    }
}

#[test]
fn test_adv_fp_02_nan_falls_back_to_nominal_100pct() {
    for p in BuiltinPreset::all() {
        let nominal = p.recipe();
        let nan_scaled = nominal.apply_intensity(f64::NAN);
        let inf_scaled = nominal.apply_intensity(f64::INFINITY);
        let neg_inf_scaled = nominal.apply_intensity(f64::NEG_INFINITY);

        // NaN, +Inf, -Inf must safely fall back to 1.0 (100% nominal)
        assert_eq!(nan_scaled.exposure, nominal.exposure);
        assert_eq!(nan_scaled.contrast, nominal.contrast);
        assert_eq!(nan_scaled.highlights, nominal.highlights);
        assert_eq!(nan_scaled.shadows, nominal.shadows);
        assert_eq!(nan_scaled.temperature, nominal.temperature);

        assert_eq!(inf_scaled.exposure, nominal.exposure);
        assert_eq!(inf_scaled.contrast, nominal.contrast);
        assert_eq!(neg_inf_scaled.exposure, nominal.exposure);
        assert_eq!(neg_inf_scaled.contrast, nominal.contrast);
    }
}

#[test]
fn test_adv_fp_03_negative_zero_preserves_neutral() {
    let cinema = BuiltinPreset::Cinema.recipe();

    let scaled = cinema.apply_intensity(-0.0);
    assert_eq!(scaled.exposure, 0.0);
    assert_eq!(scaled.contrast, 0.0);
    assert_eq!(scaled.highlights, 0.0);
    assert_eq!(scaled.shadows, 0.0);
    assert_eq!(scaled.clarity, 0.0);
    assert_eq!(scaled.temperature, Some(5500.0));
    assert_eq!(scaled.treatment.as_deref(), Some("color"));
}

// ============================================================================
// 2. OUT-OF-BOUNDS INTENSITY
// ============================================================================

#[test]
fn test_adv_oob_01_negative_intensities_clamp_to_zero() {
    let negative_intensities = [-0.0001, -0.5, -1.0, -5.0, -50.0, -100.0, -1e6, -1e15];

    for p in BuiltinPreset::all() {
        let original = p.recipe();
        for &neg_i in &negative_intensities {
            let scaled = original.apply_intensity(neg_i);
            let ctx = format!("Preset '{}' at intensity {}", p.id(), neg_i);

            // Negative intensity must clamp to 0.0 -> exact neutral settings
            assert_recipe_within_bounds(&scaled, &ctx);
            assert_eq!(scaled.exposure, 0.0, "[{}] exposure must be 0.0", ctx);
            assert_eq!(scaled.contrast, 0.0, "[{}] contrast must be 0.0", ctx);
            assert_eq!(scaled.highlights, 0.0, "[{}] highlights must be 0.0", ctx);
            assert_eq!(scaled.shadows, 0.0, "[{}] shadows must be 0.0", ctx);
            assert_eq!(scaled.whites, 0.0, "[{}] whites must be 0.0", ctx);
            assert_eq!(scaled.blacks, 0.0, "[{}] blacks must be 0.0", ctx);
            assert_eq!(scaled.tint, 0.0, "[{}] tint must be 0.0", ctx);
            assert_eq!(scaled.vibrance, 0.0, "[{}] vibrance must be 0.0", ctx);
            assert_eq!(scaled.saturation, 0.0, "[{}] saturation must be 0.0", ctx);
            assert_eq!(scaled.clarity, 0.0, "[{}] clarity must be 0.0", ctx);
            assert_eq!(scaled.dehaze, 0.0, "[{}] dehaze must be 0.0", ctx);
            assert_eq!(
                scaled.sharpen_amount, 0.0,
                "[{}] sharpen_amount must be 0.0",
                ctx
            );
            assert_eq!(
                scaled.nr_luminance, 0.0,
                "[{}] nr_luminance must be 0.0",
                ctx
            );
            assert_eq!(scaled.nr_color, 0.0, "[{}] nr_color must be 0.0", ctx);
            assert_eq!(scaled.vignette, 0.0, "[{}] vignette must be 0.0", ctx);

            if original.temperature.is_some() {
                assert_eq!(
                    scaled.temperature,
                    Some(5500.0),
                    "[{}] temp must be 5500K",
                    ctx
                );
            }
        }
    }
}

#[test]
fn test_adv_oob_02_excessive_intensities_clamp_to_150_percent() {
    let excessive_intensities = [1.500001, 1.51, 1.8, 2.0, 5.0, 10.0, 50.0, 100.0, 1e6, 1e12];

    for p in BuiltinPreset::all() {
        let original = p.recipe();
        let target_150 = original.apply_intensity(1.5);

        for &excess_i in &excessive_intensities {
            let scaled = original.apply_intensity(excess_i);
            let ctx = format!("Preset '{}' at excess intensity {}", p.id(), excess_i);

            assert_recipe_within_bounds(&scaled, &ctx);

            // Any value > 1.5 must produce the exact same clamped result as intensity = 1.5
            assert_eq!(
                scaled.exposure, target_150.exposure,
                "[{}] exposure mismatch",
                ctx
            );
            assert_eq!(
                scaled.contrast, target_150.contrast,
                "[{}] contrast mismatch",
                ctx
            );
            assert_eq!(
                scaled.highlights, target_150.highlights,
                "[{}] highlights mismatch",
                ctx
            );
            assert_eq!(
                scaled.shadows, target_150.shadows,
                "[{}] shadows mismatch",
                ctx
            );
            assert_eq!(
                scaled.whites, target_150.whites,
                "[{}] whites mismatch",
                ctx
            );
            assert_eq!(
                scaled.blacks, target_150.blacks,
                "[{}] blacks mismatch",
                ctx
            );
            assert_eq!(
                scaled.temperature, target_150.temperature,
                "[{}] temperature mismatch",
                ctx
            );
            assert_eq!(
                scaled.clarity, target_150.clarity,
                "[{}] clarity mismatch",
                ctx
            );
            assert_eq!(
                scaled.sharpen_amount, target_150.sharpen_amount,
                "[{}] sharpen mismatch",
                ctx
            );
        }
    }
}

#[test]
fn test_adv_oob_03_extreme_recipe_slider_clamping_at_150() {
    // Construct recipe with sliders that WOULD blow past bounds if not clamped
    let mut r = Recipe::default();
    r.exposure = 4.5; // 4.5 * 1.5 = 6.75 -> must clamp to 5.0
    r.contrast = 80.0; // 80.0 * 1.5 = 120.0 -> must clamp to 100.0
    r.highlights = -90.0; // -90.0 * 1.5 = -135.0 -> must clamp to -100.0
    r.temperature = Some(45000.0); // 5500 + (39500 * 1.5) = 64750 -> must clamp to 50000.0
    r.sharpen_amount = 120.0; // 120.0 * 1.5 = 180.0 -> must clamp to 150.0

    let scaled = r.apply_intensity(1.5);
    assert_eq!(scaled.exposure, 5.0);
    assert_eq!(scaled.contrast, 100.0);
    assert_eq!(scaled.highlights, -100.0);
    assert_eq!(scaled.temperature, Some(50000.0));
    assert_eq!(scaled.sharpen_amount, 150.0);
}

// ============================================================================
// 3. MONOTONICITY
// ============================================================================

#[test]
fn test_adv_monotonicity_01_builtin_presets_sweep() {
    // Continuous sweep across 151 points from 0.0 to 1.5 in 0.01 increments
    let steps = 150;

    for p in BuiltinPreset::all() {
        let nominal = p.recipe();

        let mut prev_intensity = 0.0;
        let mut prev_scaled = nominal.apply_intensity(0.0);

        for step in 1..=steps {
            let intensity = step as f64 * (1.5 / steps as f64);
            let current_scaled = nominal.apply_intensity(intensity);
            let ctx = format!(
                "Preset '{}' from i={} to i={}",
                p.id(),
                prev_intensity,
                intensity
            );

            // Verify bounds at every step
            assert_recipe_within_bounds(&current_scaled, &ctx);

            macro_rules! assert_monotonic_magnitude {
                ($field:ident, $neutral:expr, $name:expr) => {
                    let prev_val = prev_scaled.$field;
                    let curr_val = current_scaled.$field;
                    let prev_mag = (prev_val - $neutral).abs();
                    let curr_mag = (curr_val - $neutral).abs();

                    assert!(
                        curr_mag >= prev_mag - 1e-5,
                        "[{}] Monotonicity violation on {}: prev_mag={} (val={}), curr_mag={} (val={})",
                        ctx,
                        $name,
                        prev_mag,
                        prev_val,
                        curr_mag,
                        curr_val
                    );
                };
            }

            assert_monotonic_magnitude!(exposure, 0.0, "exposure");
            assert_monotonic_magnitude!(contrast, 0.0, "contrast");
            assert_monotonic_magnitude!(highlights, 0.0, "highlights");
            assert_monotonic_magnitude!(shadows, 0.0, "shadows");
            assert_monotonic_magnitude!(whites, 0.0, "whites");
            assert_monotonic_magnitude!(blacks, 0.0, "blacks");
            assert_monotonic_magnitude!(tint, 0.0, "tint");
            assert_monotonic_magnitude!(vibrance, 0.0, "vibrance");
            assert_monotonic_magnitude!(saturation, 0.0, "saturation");
            assert_monotonic_magnitude!(clarity, 0.0, "clarity");
            assert_monotonic_magnitude!(dehaze, 0.0, "dehaze");
            assert_monotonic_magnitude!(sharpen_amount, 0.0, "sharpen_amount");
            assert_monotonic_magnitude!(nr_luminance, 0.0, "nr_luminance");
            assert_monotonic_magnitude!(nr_color, 0.0, "nr_color");
            assert_monotonic_magnitude!(vignette, 0.0, "vignette");

            if nominal.temperature.is_some() {
                let prev_temp = prev_scaled.temperature.unwrap();
                let curr_temp = current_scaled.temperature.unwrap();
                let prev_temp_mag = (prev_temp - 5500.0).abs();
                let curr_temp_mag = (curr_temp - 5500.0).abs();
                assert!(
                    curr_temp_mag >= prev_temp_mag - 1e-4,
                    "[{}] Temp monotonicity violation: prev_mag={}, curr_mag={}",
                    ctx,
                    prev_temp_mag,
                    curr_temp_mag
                );
            }

            prev_intensity = intensity;
            prev_scaled = current_scaled;
        }
    }
}

#[test]
fn test_adv_monotonicity_02_clamped_boundary_saturation() {
    // Slider with extreme target: 4.0 EV exposure.
    // At intensity 1.25, 4.0 * 1.25 = 5.0 (clamped).
    // Above 1.25, it must stay clamped at exactly 5.0 without dropping.
    let mut r = Recipe::default();
    r.exposure = 4.0;

    let r_100 = r.apply_intensity(1.0);
    assert_eq!(r_100.exposure, 4.0);

    let r_125 = r.apply_intensity(1.25);
    assert_eq!(r_125.exposure, 5.0);

    let r_135 = r.apply_intensity(1.35);
    assert_eq!(r_135.exposure, 5.0);

    let r_150 = r.apply_intensity(1.50);
    assert_eq!(r_150.exposure, 5.0);
}

// ============================================================================
// 4. NEUTRAL PRESERVATION
// ============================================================================

#[test]
fn test_adv_neutral_01_all_presets_collapse_to_neutral_at_zero() {
    for p in BuiltinPreset::all() {
        let scaled = p.recipe().apply_intensity(0.0);
        let ctx = format!("Preset '{}' at 0%", p.id());

        assert_eq!(scaled.exposure, 0.0, "{}", ctx);
        assert_eq!(scaled.contrast, 0.0, "{}", ctx);
        assert_eq!(scaled.highlights, 0.0, "{}", ctx);
        assert_eq!(scaled.shadows, 0.0, "{}", ctx);
        assert_eq!(scaled.whites, 0.0, "{}", ctx);
        assert_eq!(scaled.blacks, 0.0, "{}", ctx);
        assert_eq!(scaled.tint, 0.0, "{}", ctx);
        assert_eq!(scaled.vibrance, 0.0, "{}", ctx);
        assert_eq!(scaled.saturation, 0.0, "{}", ctx);
        assert_eq!(scaled.clarity, 0.0, "{}", ctx);
        assert_eq!(scaled.dehaze, 0.0, "{}", ctx);
        assert_eq!(scaled.sharpen_amount, 0.0, "{}", ctx);
        assert_eq!(scaled.nr_luminance, 0.0, "{}", ctx);
        assert_eq!(scaled.nr_color, 0.0, "{}", ctx);
        assert_eq!(scaled.vignette, 0.0, "{}", ctx);

        if p.recipe().temperature.is_some() {
            assert_eq!(scaled.temperature, Some(5500.0), "{}", ctx);
        }

        // Treatment must fall back to "color" at 0% even for B&W presets
        assert_eq!(scaled.treatment.as_deref(), Some("color"), "{}", ctx);

        // Mixer at 0% must have all zero adjustments
        if let Some(ref m) = scaled.mixer {
            assert_eq!(m.green.hue, 0.0);
            assert_eq!(m.green.sat, 0.0);
            assert_eq!(m.green.lum, 0.0);
        }

        // B&W mix at 0% must be all zero
        if let Some(ref bw) = scaled.bw_mix {
            assert_eq!(bw.red, 0.0);
            assert_eq!(bw.blue, 0.0);
        }

        // Grading sat & lum at 0% must be zero
        if let Some(ref g) = scaled.grading {
            assert_eq!(g.shadows.sat, 0.0);
            assert_eq!(g.shadows.lum, 0.0);
            assert_eq!(g.highlights.sat, 0.0);
            assert_eq!(g.highlights.lum, 0.0);
        }

        // Grain amount at 0% must be 0
        if let Some(ref gr) = scaled.grain {
            assert_eq!(gr.amount, 0.0);
        }
    }
}

#[test]
fn test_adv_neutral_02_evaluate_flat_at_zero() {
    for p in BuiltinPreset::all() {
        let flat = p.recipe().evaluate_flat(0.0, Some(6500.0));
        assert_eq!(flat["light.exposure"], 0.0);
        assert_eq!(flat["light.contrast"], 0.0);
        assert_eq!(flat["light.highlights"], 0.0);
        assert_eq!(flat["light.shadows"], 0.0);
        assert_eq!(flat["color.saturation"], 0.0);
        assert_eq!(flat["color.vibrance"], 0.0);
        assert_eq!(flat["effects.clarity"], 0.0);
        assert_eq!(flat["effects.dehaze"], 0.0);
        assert_eq!(flat["detail.sharpenAmount"], 0.0);
        assert_eq!(flat["detail.nrLuminance"], 0.0);
        assert_eq!(flat["detail.nrColor"], 0.0);
        assert_eq!(flat["vignette.amount"], 0.0);

        if p.recipe().temperature.is_some() {
            assert_eq!(flat["wb.temp"], 6500.0);
        }
    }
}

// ============================================================================
// 5. BUILT-IN PRESETS INTEGRITY
// ============================================================================

#[test]
fn test_adv_builtin_01_all_presets_valid_across_intensity_range() {
    let test_intensities = [0.0, 0.1, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5];

    for p in BuiltinPreset::all() {
        let nominal = p.recipe();

        // Nominal checks at 1.0
        assert_recipe_within_bounds(&nominal, &format!("Preset '{}' nominal", p.id()));
        assert!(!nominal.id.is_empty());
        assert!(!nominal.name.is_empty());
        assert!(nominal.description.is_some());

        for &i in &test_intensities {
            let scaled = nominal.apply_intensity(i);
            assert_recipe_within_bounds(&scaled, &format!("Preset '{}' at i={}", p.id(), i));

            // DevelopBridge translation test
            let base_settings = DevelopSettings::default();
            let applied = DevelopBridge::apply_recipe_to_settings(&base_settings, &scaled, 1.0);
            let back = DevelopBridge::settings_to_recipe(&applied, &scaled.name);
            assert_recipe_within_bounds(
                &back,
                &format!("Preset '{}' roundtrip at i={}", p.id(), i),
            );
        }
    }
}

#[test]
fn test_adv_builtin_02_serialization_roundtrip_stability() {
    for p in BuiltinPreset::all() {
        let r = p.recipe();

        // TOML round-trip
        let toml_str = toml::to_string(&r).expect("TOML serialization failed");
        let toml_deserialized: Recipe =
            toml::from_str(&toml_str).expect("TOML deserialization failed");
        assert_eq!(r.id, toml_deserialized.id);
        assert_eq!(r.exposure, toml_deserialized.exposure);
        assert_eq!(r.contrast, toml_deserialized.contrast);
        assert_eq!(r.temperature, toml_deserialized.temperature);

        // JSON round-trip
        let json_str = serde_json::to_string(&r).expect("JSON serialization failed");
        let json_deserialized: Recipe =
            serde_json::from_str(&json_str).expect("JSON deserialization failed");
        assert_eq!(r.id, json_deserialized.id);
        assert_eq!(r.exposure, json_deserialized.exposure);
        assert_eq!(r.contrast, json_deserialized.contrast);
        assert_eq!(r.temperature, json_deserialized.temperature);
    }
}

// ============================================================================
// 6. ADVERSARIAL RECIPE STATE (NAN / INF IN RECIPE SLIDERS)
// ============================================================================

#[test]
fn test_adv_state_01_nan_slider_in_recipe_is_sanitized_by_clamp() {
    // Test SliderBounds::clamp on NaN and Infinities
    let bounds = DevelopSlider::Exposure.bounds();
    assert_eq!(bounds.clamp(f32::NAN), bounds.default);
    assert_eq!(bounds.clamp(f32::INFINITY), bounds.max);
    assert_eq!(bounds.clamp(f32::NEG_INFINITY), bounds.min);

    // If a Recipe has a NaN slider, apply_intensity must sanitize it to default, not propagate NaN
    let mut bad_recipe = Recipe::default();
    bad_recipe.exposure = f32::NAN;
    bad_recipe.contrast = f32::INFINITY;
    bad_recipe.highlights = f32::NEG_INFINITY;

    let sanitized = bad_recipe.apply_intensity(1.0);
    assert!(
        !sanitized.exposure.is_nan(),
        "NaN exposure must be sanitized"
    );
    assert_eq!(sanitized.exposure, 0.0);
    assert_eq!(sanitized.contrast, 100.0);
    assert_eq!(sanitized.highlights, -100.0);
}

#[test]
fn test_adv_state_02_temperature_infinities() {
    let mut inf_temp = Recipe::default();
    inf_temp.temperature = Some(f32::INFINITY);
    let scaled_inf = inf_temp.apply_intensity(1.0);
    assert_eq!(scaled_inf.temperature, Some(50000.0));

    let mut neg_inf_temp = Recipe::default();
    neg_inf_temp.temperature = Some(f32::NEG_INFINITY);
    let scaled_neg = neg_inf_temp.apply_intensity(1.0);
    assert_eq!(scaled_neg.temperature, Some(2000.0));
}

#[test]
fn test_adv_state_03_nan_temperature_behavior() {
    let mut r = Recipe::default();
    r.temperature = Some(f32::NAN);
    let result = std::panic::catch_unwind(move || r.apply_intensity(1.0));
    // Record empirical behavior: apply_intensity does NOT panic when recipe.temperature is Some(NaN)
    assert!(
        result.is_ok(),
        "apply_intensity must not panic on NaN temperature"
    );
    let scaled = result.unwrap();
    assert!(scaled.temperature.is_some());
    // Note: scaled.temperature retains NaN because recipe.rs line 574 uses (base + delta * i).clamp(...)
    // rather than SliderBounds::clamp(...). This is documented as Challenge Finding C-01 (Low Risk).
    assert!(scaled.temperature.unwrap().is_nan());
}

// ============================================================================
// 7. SOURCE RAW CRYPTOGRAPHIC IMMUTABILITY (SHA-256)
// ============================================================================

#[test]
fn test_adv_cr3_raw_immutability() {
    let fixture = TestFixture::new("adv_cr3_immutability");
    // Verify Canon CR3 RAW files are 100% untouched
    fixture.assert_integrity();
}
