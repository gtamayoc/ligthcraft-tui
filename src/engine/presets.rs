//! Curated built-in aesthetic presets and factory recipes.
//!
//! Provides mathematically engineered presets:
//! 1. Natural: clean neutral color fidelity, subtle contrast boost, slight sharpening.
//! 2. Cinema: muted greens, warm skin tones, teal shadows, golden highlights, film rolloff.
//! 3. B&W High Contrast: monochromatic treatment, dramatic tonal curve, deep blacks, high clarity.
//! 4. Teal & Orange: complementary color split, warm skin tones, cool teal shadows/skies.
//! 5. Golden Hour: warm color temp shift (+1000K), elevated shadows, soft contrast, warm glow.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::engine::recipe::{Recipe, RecipeCategory, RecipeMetadata};
use crate::mcp::tools::{
    BwMixSettings, GradingSettings, GrainSettings, HslAdjustment, MixerSettings,
};

/// Enumeration of all built-in curated production presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuiltinPreset {
    Natural,
    Cinema,
    BwHighContrast,
    TealOrange,
    GoldenHour,
}

impl BuiltinPreset {
    /// Returns the canonical kebab-case ID.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Natural => "natural",
            Self::Cinema => "cinema",
            Self::BwHighContrast => "bw-high-contrast",
            Self::TealOrange => "teal-orange",
            Self::GoldenHour => "golden-hour",
        }
    }

    /// User-friendly display title.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Natural => "Natural",
            Self::Cinema => "Cinema",
            Self::BwHighContrast => "B&W High Contrast",
            Self::TealOrange => "Teal & Orange",
            Self::GoldenHour => "Golden Hour",
        }
    }

    /// Category for UI grouping and organization.
    pub fn category(&self) -> RecipeCategory {
        match self {
            Self::Natural => RecipeCategory::Color,
            Self::Cinema => RecipeCategory::Cinematic,
            Self::BwHighContrast => RecipeCategory::BlackAndWhite,
            Self::TealOrange => RecipeCategory::Style,
            Self::GoldenHour => RecipeCategory::Landscape,
        }
    }

    /// Category display string.
    pub fn category_name(&self) -> &'static str {
        match self {
            Self::Natural => "General / Correction",
            Self::Cinema => "Film & Style",
            Self::BwHighContrast => "Monochrome",
            Self::TealOrange => "Color Grading",
            Self::GoldenHour => "Warm & Landscape",
        }
    }

    /// Short photographic description.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Natural => "Clean neutral color, subtle dynamic range expansion, gentle sharpening.",
            Self::Cinema => "Muted greens, warm skin tones, 35mm film curve, cool teal shadows and warm highlights.",
            Self::BwHighContrast => "Dramatic fine-art monochrome, deep velvety blacks, brilliant whites, and micro-contrast.",
            Self::TealOrange => "Complementary color split with warm amber skin tones opposing deep teal shadows.",
            Self::GoldenHour => "Sunset golden warmth (+1000K), soft contrast, illuminated shadows, and highlight bloom.",
        }
    }

    /// Correlated native LightCraft preset ID, if available on server.
    pub fn native_mcp_id(&self) -> Option<&'static str> {
        match self {
            Self::Natural => None,
            Self::Cinema => Some("lc.cinematic"),
            Self::BwHighContrast => Some("lc.bw-high-contrast"),
            Self::TealOrange => Some("lc.teal-orange"),
            Self::GoldenHour => Some("lc.golden-hour"),
        }
    }

    /// Returns a slice of all 5 built-in presets.
    pub fn all() -> &'static [BuiltinPreset] {
        &[
            Self::Natural,
            Self::Cinema,
            Self::BwHighContrast,
            Self::TealOrange,
            Self::GoldenHour,
        ]
    }

    /// Resolves a preset from ID or display name.
    pub fn get(id_or_name: &str) -> Option<Self> {
        Self::from_str_loose(id_or_name)
    }

    /// Loose resolution helper (case-insensitive, supports various alias formats).
    pub fn from_str_loose(s: &str) -> Option<Self> {
        let clean = s.trim().to_lowercase();
        match clean.as_str() {
            "natural" | "builtin.natural" | "clean" => Some(Self::Natural),
            "cinema" | "cinematic" | "builtin.cinema" | "lc.cinematic" => Some(Self::Cinema),
            "bw-high-contrast"
            | "b&w high contrast"
            | "builtin.bw-high-contrast"
            | "lc.bw-high-contrast"
            | "bw"
            | "black and white"
            | "black-and-white" => Some(Self::BwHighContrast),
            "teal-orange"
            | "teal & orange"
            | "builtin.teal-orange"
            | "lc.teal-orange"
            | "orange-teal"
            | "teal_orange" => Some(Self::TealOrange),
            "golden-hour"
            | "golden hour"
            | "builtin.golden-hour"
            | "lc.golden-hour"
            | "sunset"
            | "golden_hour" => Some(Self::GoldenHour),
            _ => None,
        }
    }

    /// Produces the complete photographic `Recipe` for this preset.
    pub fn recipe(&self) -> Recipe {
        match self {
            Self::Natural => Recipe {
                id: self.id().to_string(),
                name: self.display_name().to_string(),
                category: self.category(),
                description: Some(self.description().to_string()),
                profile: Some("lc.color".to_string()),
                treatment: Some("color".to_string()),
                exposure: 0.10,
                contrast: 5.0,
                highlights: -5.0,
                shadows: 10.0,
                whites: 0.0,
                blacks: 0.0,
                temperature: None,
                tint: 0.0,
                vibrance: 10.0,
                saturation: 0.0,
                clarity: 0.0,
                dehaze: 0.0,
                sharpen_amount: 15.0,
                nr_luminance: 0.0,
                nr_color: 25.0,
                vignette: 0.0,
                mixer: None,
                bw_mix: None,
                grading: None,
                curve: None,
                grain: None,
                custom_sliders: HashMap::new(),
                metadata: RecipeMetadata {
                    author: "LightCraft Team".to_string(),
                    description: Some(self.description().to_string()),
                    created_at: "2026-10-15T12:00:00Z".to_string(),
                    updated_at: None,
                    version: 1,
                    recommended_intensity: 100,
                    tags: vec!["natural".into(), "clean".into(), "correction".into()],
                },
            },

            Self::Cinema => {
                let mut mixer = MixerSettings::default();
                mixer.green = HslAdjustment {
                    hue: 15.0,
                    sat: -25.0,
                    lum: -10.0,
                };
                mixer.yellow = HslAdjustment {
                    hue: -5.0,
                    sat: -15.0,
                    lum: 0.0,
                };
                mixer.orange = HslAdjustment {
                    hue: -3.0,
                    sat: 10.0,
                    lum: 5.0,
                };
                mixer.red = HslAdjustment {
                    hue: 0.0,
                    sat: 5.0,
                    lum: 0.0,
                };
                mixer.blue = HslAdjustment {
                    hue: 5.0,
                    sat: -10.0,
                    lum: 0.0,
                };

                let mut grading = GradingSettings::default();
                grading.shadows = HslAdjustment {
                    hue: 215.0,
                    sat: 18.0,
                    lum: -5.0,
                };
                grading.highlights = HslAdjustment {
                    hue: 42.0,
                    sat: 16.0,
                    lum: 2.0,
                };
                grading.midtones = HslAdjustment {
                    hue: 35.0,
                    sat: 8.0,
                    lum: 0.0,
                };
                grading.blending = 55.0;
                grading.balance = -10.0;

                let grain = GrainSettings {
                    amount: 15.0,
                    size: 20.0,
                    roughness: 40.0,
                    seed: 42,
                };

                Recipe {
                    id: self.id().to_string(),
                    name: self.display_name().to_string(),
                    category: self.category(),
                    description: Some(self.description().to_string()),
                    profile: Some("lc.cine.teal-amber".to_string()),
                    treatment: Some("color".to_string()),
                    exposure: -0.20,
                    contrast: 15.0,
                    highlights: -25.0,
                    shadows: 20.0,
                    whites: -10.0,
                    blacks: 10.0,
                    temperature: Some(5700.0),
                    tint: -2.0,
                    vibrance: 15.0,
                    saturation: -10.0,
                    clarity: 10.0,
                    dehaze: 5.0,
                    sharpen_amount: 20.0,
                    nr_luminance: 0.0,
                    nr_color: 25.0,
                    vignette: -10.0,
                    mixer: Some(mixer),
                    bw_mix: None,
                    grading: Some(grading),
                    curve: None,
                    grain: Some(grain),
                    custom_sliders: HashMap::new(),
                    metadata: RecipeMetadata {
                        author: "LightCraft Team".to_string(),
                        description: Some(self.description().to_string()),
                        created_at: "2026-10-15T12:00:00Z".to_string(),
                        updated_at: None,
                        version: 1,
                        recommended_intensity: 100,
                        tags: vec![
                            "cinema".into(),
                            "film".into(),
                            "warm".into(),
                            "portrait".into(),
                        ],
                    },
                }
            }

            Self::BwHighContrast => {
                let mut bw_mix = BwMixSettings::default();
                bw_mix.red = 25.0;
                bw_mix.orange = 20.0;
                bw_mix.yellow = 15.0;
                bw_mix.green = 5.0;
                bw_mix.aqua = -15.0;
                bw_mix.blue = -30.0;
                bw_mix.purple = -10.0;
                bw_mix.magenta = 0.0;

                Recipe {
                    id: self.id().to_string(),
                    name: self.display_name().to_string(),
                    category: self.category(),
                    description: Some(self.description().to_string()),
                    profile: Some("lc.bw.mono-rich".to_string()),
                    treatment: Some("black_and_white".to_string()),
                    exposure: 0.0,
                    contrast: 25.0,
                    highlights: -35.0,
                    shadows: 15.0,
                    whites: 20.0,
                    blacks: -25.0,
                    temperature: None,
                    tint: 0.0,
                    vibrance: 0.0,
                    saturation: -100.0,
                    clarity: 25.0,
                    dehaze: 10.0,
                    sharpen_amount: 25.0,
                    nr_luminance: 0.0,
                    nr_color: 50.0,
                    vignette: -15.0,
                    mixer: None,
                    bw_mix: Some(bw_mix),
                    grading: None,
                    curve: None,
                    grain: None,
                    custom_sliders: HashMap::new(),
                    metadata: RecipeMetadata {
                        author: "LightCraft Team".to_string(),
                        description: Some(self.description().to_string()),
                        created_at: "2026-10-15T12:00:00Z".to_string(),
                        updated_at: None,
                        version: 1,
                        recommended_intensity: 100,
                        tags: vec![
                            "monochrome".into(),
                            "black-and-white".into(),
                            "contrast".into(),
                            "fine-art".into(),
                        ],
                    },
                }
            }

            Self::TealOrange => {
                let mut mixer = MixerSettings::default();
                mixer.orange = HslAdjustment {
                    hue: 5.0,
                    sat: 25.0,
                    lum: 10.0,
                };
                mixer.yellow = HslAdjustment {
                    hue: -15.0,
                    sat: 15.0,
                    lum: 0.0,
                };
                mixer.red = HslAdjustment {
                    hue: 5.0,
                    sat: 10.0,
                    lum: 0.0,
                };
                mixer.blue = HslAdjustment {
                    hue: -25.0,
                    sat: 25.0,
                    lum: -10.0,
                };
                mixer.aqua = HslAdjustment {
                    hue: -10.0,
                    sat: 30.0,
                    lum: -5.0,
                };
                mixer.green = HslAdjustment {
                    hue: 20.0,
                    sat: -20.0,
                    lum: 0.0,
                };

                let mut grading = GradingSettings::default();
                grading.shadows = HslAdjustment {
                    hue: 195.0,
                    sat: 28.0,
                    lum: -5.0,
                };
                grading.highlights = HslAdjustment {
                    hue: 35.0,
                    sat: 22.0,
                    lum: 5.0,
                };
                grading.midtones = HslAdjustment {
                    hue: 30.0,
                    sat: 10.0,
                    lum: 0.0,
                };
                grading.blending = 60.0;
                grading.balance = 5.0;

                let mut custom = HashMap::new();
                custom.insert("calibration.blueHue".into(), serde_json::json!(-10.0));
                custom.insert("calibration.blueSat".into(), serde_json::json!(15.0));
                custom.insert("calibration.redHue".into(), serde_json::json!(5.0));
                custom.insert("calibration.redSat".into(), serde_json::json!(10.0));

                Recipe {
                    id: self.id().to_string(),
                    name: self.display_name().to_string(),
                    category: self.category(),
                    description: Some(self.description().to_string()),
                    profile: Some("lc.cine.teal-amber".to_string()),
                    treatment: Some("color".to_string()),
                    exposure: 0.05,
                    contrast: 15.0,
                    highlights: -15.0,
                    shadows: 10.0,
                    whites: 5.0,
                    blacks: -10.0,
                    temperature: Some(6100.0),
                    tint: -10.0,
                    vibrance: 20.0,
                    saturation: 5.0,
                    clarity: 10.0,
                    dehaze: 5.0,
                    sharpen_amount: 20.0,
                    nr_luminance: 0.0,
                    nr_color: 25.0,
                    vignette: -8.0,
                    mixer: Some(mixer),
                    bw_mix: None,
                    grading: Some(grading),
                    curve: None,
                    grain: None,
                    custom_sliders: custom,
                    metadata: RecipeMetadata {
                        author: "LightCraft Team".to_string(),
                        description: Some(self.description().to_string()),
                        created_at: "2026-10-15T12:00:00Z".to_string(),
                        updated_at: None,
                        version: 1,
                        recommended_intensity: 100,
                        tags: vec![
                            "teal-orange".into(),
                            "cinema".into(),
                            "color-grading".into(),
                            "blockbuster".into(),
                        ],
                    },
                }
            }

            Self::GoldenHour => {
                let mut mixer = MixerSettings::default();
                mixer.yellow = HslAdjustment {
                    hue: -5.0,
                    sat: 20.0,
                    lum: 15.0,
                };
                mixer.orange = HslAdjustment {
                    hue: 0.0,
                    sat: 15.0,
                    lum: 10.0,
                };
                mixer.red = HslAdjustment {
                    hue: 5.0,
                    sat: 10.0,
                    lum: 0.0,
                };
                mixer.blue = HslAdjustment {
                    hue: 5.0,
                    sat: -15.0,
                    lum: 0.0,
                };

                let mut grading = GradingSettings::default();
                grading.shadows = HslAdjustment {
                    hue: 45.0,
                    sat: 12.0,
                    lum: 5.0,
                };
                grading.highlights = HslAdjustment {
                    hue: 38.0,
                    sat: 18.0,
                    lum: 5.0,
                };
                grading.midtones = HslAdjustment {
                    hue: 40.0,
                    sat: 8.0,
                    lum: 0.0,
                };
                grading.blending = 65.0;
                grading.balance = 10.0;

                Recipe {
                    id: self.id().to_string(),
                    name: self.display_name().to_string(),
                    category: self.category(),
                    description: Some(self.description().to_string()),
                    profile: Some("lc.film.golden-hour".to_string()),
                    treatment: Some("color".to_string()),
                    exposure: 0.25,
                    contrast: -5.0,
                    highlights: -10.0,
                    shadows: 20.0,
                    whites: 10.0,
                    blacks: 5.0,
                    temperature: Some(6500.0),
                    tint: 5.0,
                    vibrance: 15.0,
                    saturation: 5.0,
                    clarity: -5.0,
                    dehaze: -5.0,
                    sharpen_amount: 15.0,
                    nr_luminance: 0.0,
                    nr_color: 25.0,
                    vignette: -10.0,
                    mixer: Some(mixer),
                    bw_mix: None,
                    grading: Some(grading),
                    curve: None,
                    grain: None,
                    custom_sliders: HashMap::new(),
                    metadata: RecipeMetadata {
                        author: "LightCraft Team".to_string(),
                        description: Some(self.description().to_string()),
                        created_at: "2026-10-15T12:00:00Z".to_string(),
                        updated_at: None,
                        version: 1,
                        recommended_intensity: 100,
                        tags: vec![
                            "golden-hour".into(),
                            "sunset".into(),
                            "warm".into(),
                            "landscape".into(),
                        ],
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_builtin_presets_accessible() {
        let presets = BuiltinPreset::all();
        assert_eq!(presets.len(), 5);

        for p in presets {
            let r = p.recipe();
            assert_eq!(r.id, p.id());
            assert_eq!(r.name, p.display_name());
            assert_eq!(r.category, p.category());
        }
    }

    #[test]
    fn test_natural_preset_values() {
        let r = BuiltinPreset::Natural.recipe();
        assert_eq!(r.exposure, 0.10);
        assert_eq!(r.contrast, 5.0);
        assert_eq!(r.highlights, -5.0);
        assert_eq!(r.shadows, 10.0);
        assert_eq!(r.vibrance, 10.0);
        assert_eq!(r.sharpen_amount, 15.0);
        assert_eq!(r.treatment.as_deref(), Some("color"));
    }

    #[test]
    fn test_cinema_preset_values() {
        let r = BuiltinPreset::Cinema.recipe();
        assert_eq!(r.exposure, -0.20);
        assert_eq!(r.contrast, 15.0);
        assert_eq!(r.highlights, -25.0);
        assert_eq!(r.shadows, 20.0);
        assert_eq!(r.saturation, -10.0);
        assert_eq!(r.clarity, 10.0);
        assert_eq!(r.temperature, Some(5700.0));
        assert!(r.grading.is_some());
        assert_eq!(r.grading.as_ref().unwrap().shadows.hue, 215.0);
    }

    #[test]
    fn test_bw_high_contrast_preset_values() {
        let r = BuiltinPreset::BwHighContrast.recipe();
        assert_eq!(r.treatment.as_deref(), Some("black_and_white"));
        assert_eq!(r.saturation, -100.0);
        assert_eq!(r.contrast, 25.0);
        assert_eq!(r.highlights, -35.0);
        assert_eq!(r.shadows, 15.0);
        assert_eq!(r.whites, 20.0);
        assert_eq!(r.blacks, -25.0);
        assert_eq!(r.clarity, 25.0);
        assert!(r.bw_mix.is_some());
        assert_eq!(r.bw_mix.as_ref().unwrap().blue, -30.0);
    }

    #[test]
    fn test_teal_orange_preset_values() {
        let r = BuiltinPreset::TealOrange.recipe();
        assert_eq!(r.temperature, Some(6100.0));
        assert_eq!(r.tint, -10.0);
        assert_eq!(r.contrast, 15.0);
        assert_eq!(r.vibrance, 20.0);
        assert!(r.grading.is_some());
        assert_eq!(r.grading.as_ref().unwrap().shadows.hue, 195.0);
        assert_eq!(r.grading.as_ref().unwrap().highlights.hue, 35.0);
    }

    #[test]
    fn test_golden_hour_preset_values() {
        let r = BuiltinPreset::GoldenHour.recipe();
        assert_eq!(r.temperature, Some(6500.0));
        assert_eq!(r.tint, 5.0);
        assert_eq!(r.exposure, 0.25);
        assert_eq!(r.contrast, -5.0);
        assert_eq!(r.shadows, 20.0);
        assert_eq!(r.clarity, -5.0);
        assert_eq!(r.dehaze, -5.0);
    }

    #[test]
    fn test_from_str_loose_resolution() {
        assert_eq!(BuiltinPreset::get("natural"), Some(BuiltinPreset::Natural));
        assert_eq!(BuiltinPreset::get("CINEMA"), Some(BuiltinPreset::Cinema));
        assert_eq!(
            BuiltinPreset::get("bw-high-contrast"),
            Some(BuiltinPreset::BwHighContrast)
        );
        assert_eq!(
            BuiltinPreset::get("teal & orange"),
            Some(BuiltinPreset::TealOrange)
        );
        assert_eq!(
            BuiltinPreset::get("Golden Hour"),
            Some(BuiltinPreset::GoldenHour)
        );
        assert_eq!(BuiltinPreset::get("nonexistent"), None);
    }
}
