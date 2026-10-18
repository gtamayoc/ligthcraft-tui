//! Photographic recipe data model, slider definitions, and intensity scaling mathematics.
//!
//! Complies with LightCraft 0.8.0 develop parameter specifications (122 controls, 14 sections).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::mcp::tools::{
    BwMixSettings, CurveSettings, GradingSettings, GrainSettings, MixerSettings,
};

/// Standard photographic category for organizing and filtering recipes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecipeCategory {
    Color,
    Portrait,
    Landscape,
    Film,
    BlackAndWhite,
    Cinematic,
    Urban,
    Food,
    Vintage,
    Style,
    Custom,
}

impl RecipeCategory {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Color => "Color",
            Self::Portrait => "Portrait",
            Self::Landscape => "Landscape",
            Self::Film => "Film Emulation",
            Self::BlackAndWhite => "Black & White",
            Self::Cinematic => "Cinematic",
            Self::Urban => "Urban / Street",
            Self::Food => "Food & Still Life",
            Self::Vintage => "Vintage / Retro",
            Self::Style => "Creative Style",
            Self::Custom => "Custom Presets",
        }
    }
}

impl Default for RecipeCategory {
    fn default() -> Self {
        Self::Custom
    }
}

/// Enumeration of the 14 functional sections comprising LightCraft's develop pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SliderSection {
    Profile,
    Color,
    Light,
    Curve,
    Detail,
    Effects,
    Vignette,
    Grain,
    Grading,
    Mixer,
    BwMix,
    Optics,
    Geometry,
    Calibration,
}

/// Visual track presentation type for TUI rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    Centered,
    Gradient,
    Plain,
    Temp,
    Tint,
    Rainbow,
    Hue,
    Sat,
    Lum,
}

/// Physical and operational bounds of a develop slider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SliderBounds {
    /// Physical minimum allowed by the engine.
    pub min: f32,
    /// Physical maximum allowed by the engine.
    pub max: f32,
    /// Neutral / baseline default value.
    pub default: f32,
    /// Ergonomic adjustment step in TUI.
    pub step: f32,
    /// Precision in decimal digits for display formatting.
    pub precision: usize,
    /// Unit label (e.g. "EV", "K", "%", "").
    pub unit: &'static str,
    /// Track visualization style in TUI.
    pub track: TrackKind,
}

impl SliderBounds {
    pub const fn new(
        min: f32,
        max: f32,
        default: f32,
        step: f32,
        precision: usize,
        unit: &'static str,
        track: TrackKind,
    ) -> Self {
        Self {
            min,
            max,
            default,
            step,
            precision,
            unit,
            track,
        }
    }

    /// Clamps value to strictly within [min, max], guarding against NaN and Infinity.
    pub fn clamp(&self, val: f32) -> f32 {
        if val.is_nan() {
            return self.default;
        }
        val.clamp(self.min, self.max)
    }

    /// Snaps value to the nearest discrete step increment.
    pub fn snap(&self, val: f32) -> f32 {
        if self.step <= 0.0 || val.is_nan() {
            return self.clamp(val);
        }
        let stepped = (val / self.step).round() * self.step;
        self.clamp(stepped)
    }
}

/// The 16 core photographic sliders identified for primary TUI adjustment,
/// plus an extensibility variant for any arbitrary LightCraft control ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DevelopSlider {
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Temperature,
    Tint,
    Vibrance,
    Saturation,
    Clarity,
    Dehaze,
    SharpenAmount,
    Sharpening,
    NrLuminance,
    LuminanceNr,
    NrColor,
    ColorNr,
    Vignette,
    Custom(String),
}

impl DevelopSlider {
    /// Canonical LightCraft control ID string.
    pub fn control_id(&self) -> &str {
        match self {
            Self::Exposure => "light.exposure",
            Self::Contrast => "light.contrast",
            Self::Highlights => "light.highlights",
            Self::Shadows => "light.shadows",
            Self::Whites => "light.whites",
            Self::Blacks => "light.blacks",
            Self::Temperature => "wb.temp",
            Self::Tint => "wb.tint",
            Self::Vibrance => "color.vibrance",
            Self::Saturation => "color.saturation",
            Self::Clarity => "effects.clarity",
            Self::Dehaze => "effects.dehaze",
            Self::SharpenAmount | Self::Sharpening => "detail.sharpenAmount",
            Self::NrLuminance | Self::LuminanceNr => "detail.nrLuminance",
            Self::NrColor | Self::ColorNr => "detail.nrColor",
            Self::Vignette => "vignette.amount",
            Self::Custom(id) => id.as_str(),
        }
    }

    /// Human-readable UI label.
    pub fn label(&self) -> &str {
        match self {
            Self::Exposure => "Exposure",
            Self::Contrast => "Contrast",
            Self::Highlights => "Highlights",
            Self::Shadows => "Shadows",
            Self::Whites => "Whites",
            Self::Blacks => "Blacks",
            Self::Temperature => "Temperature",
            Self::Tint => "Tint",
            Self::Vibrance => "Vibrance",
            Self::Saturation => "Saturation",
            Self::Clarity => "Clarity",
            Self::Dehaze => "Dehaze",
            Self::SharpenAmount | Self::Sharpening => "Sharpening",
            Self::NrLuminance | Self::LuminanceNr => "Luminance NR",
            Self::NrColor | Self::ColorNr => "Color NR",
            Self::Vignette => "Vignette",
            Self::Custom(id) => id.as_str(),
        }
    }

    /// Parent section in LightCraft develop pipeline.
    pub fn section(&self) -> SliderSection {
        match self {
            Self::Exposure
            | Self::Contrast
            | Self::Highlights
            | Self::Shadows
            | Self::Whites
            | Self::Blacks => SliderSection::Light,
            Self::Temperature | Self::Tint | Self::Vibrance | Self::Saturation => {
                SliderSection::Color
            }
            Self::Clarity | Self::Dehaze => SliderSection::Effects,
            Self::SharpenAmount
            | Self::Sharpening
            | Self::NrLuminance
            | Self::LuminanceNr
            | Self::NrColor
            | Self::ColorNr => SliderSection::Detail,
            Self::Vignette => SliderSection::Vignette,
            Self::Custom(id) => {
                if id.starts_with("light.") || id.starts_with("hdr.") {
                    SliderSection::Light
                } else if id.starts_with("wb.") || id.starts_with("color.") {
                    SliderSection::Color
                } else if id.starts_with("curve.") {
                    SliderSection::Curve
                } else if id.starts_with("detail.") || id.starts_with("enhance.") {
                    SliderSection::Detail
                } else if id.starts_with("effects.") {
                    SliderSection::Effects
                } else if id.starts_with("vignette.") {
                    SliderSection::Vignette
                } else if id.starts_with("grain.") {
                    SliderSection::Grain
                } else if id.starts_with("grading.") {
                    SliderSection::Grading
                } else if id.starts_with("mixer.") {
                    SliderSection::Mixer
                } else if id.starts_with("bw.") {
                    SliderSection::BwMix
                } else if id.starts_with("optics.") {
                    SliderSection::Optics
                } else if id.starts_with("geometry.") || id.starts_with("crop.") {
                    SliderSection::Geometry
                } else if id.starts_with("calibration.") {
                    SliderSection::Calibration
                } else if id.starts_with("profile.") {
                    SliderSection::Profile
                } else {
                    SliderSection::Light
                }
            }
        }
    }

    /// Whether this slider represents a delta relative to 0.0 (true) or an absolute magnitude (false).
    pub fn is_relative(&self) -> bool {
        match self {
            Self::Temperature => false,
            Self::Custom(id) => {
                id != "profile.amount"
                    && id != "geometry.scale"
                    && !id.starts_with("curve.split")
                    && !id.contains("Midpoint")
                    && !id.contains("Detail")
                    && !id.contains("Roughness")
            }
            _ => true,
        }
    }

    /// Default mathematical neutral baseline value for this slider.
    pub fn neutral_baseline(&self) -> f32 {
        match self {
            Self::Temperature => 5500.0,
            _ => 0.0,
        }
    }

    /// Physical slider bounds and presentation parameters.
    pub fn bounds(&self) -> SliderBounds {
        match self {
            Self::Exposure => {
                SliderBounds::new(-5.0, 5.0, 0.0, 0.05, 2, " EV", TrackKind::Centered)
            }
            Self::Contrast => {
                SliderBounds::new(-100.0, 100.0, 0.0, 1.0, 0, "%", TrackKind::Centered)
            }
            Self::Highlights => {
                SliderBounds::new(-100.0, 100.0, 0.0, 1.0, 0, "%", TrackKind::Centered)
            }
            Self::Shadows => {
                SliderBounds::new(-100.0, 100.0, 0.0, 1.0, 0, "%", TrackKind::Centered)
            }
            Self::Whites => SliderBounds::new(-100.0, 100.0, 0.0, 1.0, 0, "%", TrackKind::Centered),
            Self::Blacks => SliderBounds::new(-100.0, 100.0, 0.0, 1.0, 0, "%", TrackKind::Centered),
            Self::Temperature => {
                SliderBounds::new(2000.0, 50000.0, 5500.0, 50.0, 0, " K", TrackKind::Temp)
            }
            Self::Tint => SliderBounds::new(-150.0, 150.0, 0.0, 1.0, 0, "", TrackKind::Tint),
            Self::Vibrance => {
                SliderBounds::new(-100.0, 100.0, 0.0, 1.0, 0, "%", TrackKind::Gradient)
            }
            Self::Saturation => {
                SliderBounds::new(-100.0, 100.0, 0.0, 1.0, 0, "%", TrackKind::Gradient)
            }
            Self::Clarity => {
                SliderBounds::new(-100.0, 100.0, 0.0, 1.0, 0, "%", TrackKind::Centered)
            }
            Self::Dehaze => SliderBounds::new(-100.0, 100.0, 0.0, 1.0, 0, "%", TrackKind::Centered),
            Self::SharpenAmount | Self::Sharpening => {
                SliderBounds::new(0.0, 150.0, 0.0, 1.0, 0, "", TrackKind::Plain)
            }
            Self::NrLuminance | Self::LuminanceNr => {
                SliderBounds::new(0.0, 100.0, 0.0, 1.0, 0, "", TrackKind::Plain)
            }
            Self::NrColor | Self::ColorNr => {
                SliderBounds::new(0.0, 100.0, 0.0, 1.0, 0, "", TrackKind::Plain)
            }
            Self::Vignette => {
                SliderBounds::new(-100.0, 100.0, 0.0, 1.0, 0, "%", TrackKind::Centered)
            }
            Self::Custom(_) => SliderBounds::new(-100.0, 100.0, 0.0, 1.0, 0, "", TrackKind::Plain),
        }
    }
}

/// Metadata describing a preset / recipe author and version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeMetadata {
    #[serde(default = "default_author")]
    pub author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default = "default_intensity_pct")]
    pub recommended_intensity: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

fn default_author() -> String {
    "User".to_string()
}

fn default_version() -> u32 {
    1
}

fn default_intensity_pct() -> u32 {
    100
}

impl Default for RecipeMetadata {
    fn default() -> Self {
        Self {
            author: default_author(),
            description: None,
            created_at: String::new(),
            updated_at: None,
            version: default_version(),
            recommended_intensity: default_intensity_pct(),
            tags: Vec::new(),
        }
    }
}

/// A complete photographic recipe encompassing the 16 core photographic sliders,
/// target profile, treatment, tone curves, and an extensible map for any of LightCraft's 122 sliders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    /// Unique identifier slug (e.g. "teal-orange", "bw-film-1960").
    pub id: String,
    /// User-visible display title (e.g. "Teal & Orange Cinematic").
    pub name: String,
    /// Categorization for navigation and filtering.
    #[serde(default)]
    pub category: RecipeCategory,
    /// Detailed description of the visual mood and intended lighting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Target color lookup profile (e.g. "lc.color", "lc.cine.teal-amber").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Image treatment pipeline ("color" or "black_and_white").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub treatment: Option<String>,

    // ========================================================================
    // 16 Core Photographic Sliders
    // ========================================================================
    /// Exposure compensation in EV [-5.0, +5.0]
    #[serde(default)]
    pub exposure: f32,
    /// Contrast [-100.0, +100.0]
    #[serde(default)]
    pub contrast: f32,
    /// Highlights recovery / compression [-100.0, +100.0]
    #[serde(default)]
    pub highlights: f32,
    /// Shadows recovery / deepening [-100.0, +100.0]
    #[serde(default)]
    pub shadows: f32,
    /// White clipping adjustment [-100.0, +100.0]
    #[serde(default)]
    pub whites: f32,
    /// Black clipping adjustment [-100.0, +100.0]
    #[serde(default)]
    pub blacks: f32,
    /// Absolute color temperature in Kelvin [2000.0, 50000.0].
    /// `None` indicates preserving the photo's as-shot temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Tint balance along magenta/green axis [-150.0, +150.0]
    #[serde(default)]
    pub tint: f32,
    /// Smart selective saturation for skin/subtle tones [-100.0, +100.0]
    #[serde(default)]
    pub vibrance: f32,
    /// Global color saturation [-100.0, +100.0]
    #[serde(default)]
    pub saturation: f32,
    /// Midtone local contrast [-100.0, +100.0]
    #[serde(default)]
    pub clarity: f32,
    /// Atmospheric haze reduction [-100.0, +100.0]
    #[serde(default)]
    pub dehaze: f32,
    /// High-frequency edge sharpening [0.0, 150.0]
    #[serde(default, alias = "sharpening", alias = "sharpenAmount")]
    pub sharpen_amount: f32,
    /// Luminance noise reduction [0.0, 100.0]
    #[serde(default, alias = "luminance_nr", alias = "nrLuminance")]
    pub nr_luminance: f32,
    /// Chrominance noise reduction [0.0, 100.0]
    #[serde(default, alias = "color_nr", alias = "nrColor")]
    pub nr_color: f32,
    /// Lens vignetting compensation or artistic falloff [-100.0, +100.0]
    #[serde(default)]
    pub vignette: f32,

    // ========================================================================
    // Structured Processing Blocks
    // ========================================================================
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mixer: Option<MixerSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bw_mix: Option<BwMixSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grading: Option<GradingSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<CurveSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<GrainSettings>,

    // ========================================================================
    // Extensibility Map (Captures any of LightCraft's 122 sliders / 14 sections)
    // ========================================================================
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub custom_sliders: HashMap<String, serde_json::Value>,

    /// Recipe metadata, authorship, and version.
    #[serde(default)]
    pub metadata: RecipeMetadata,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            id: "neutral".to_string(),
            name: "Neutral (Zeroed)".to_string(),
            category: RecipeCategory::Custom,
            description: Some("Neutral baseline with zero photographic adjustments".to_string()),
            profile: None,
            treatment: Some("color".to_string()),
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            temperature: None,
            tint: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            sharpen_amount: 0.0,
            nr_luminance: 0.0,
            nr_color: 0.0,
            vignette: 0.0,
            mixer: None,
            bw_mix: None,
            grading: None,
            curve: None,
            grain: None,
            custom_sliders: HashMap::new(),
            metadata: RecipeMetadata::default(),
        }
    }
}

impl Recipe {
    // Convenience field aliases for interoperability
    pub fn sharpening(&self) -> f32 {
        self.sharpen_amount
    }

    pub fn set_sharpening(&mut self, val: f32) {
        self.sharpen_amount = val;
    }

    pub fn luminance_nr(&self) -> f32 {
        self.nr_luminance
    }

    pub fn set_luminance_nr(&mut self, val: f32) {
        self.nr_luminance = val;
    }

    pub fn color_nr(&self) -> f32 {
        self.nr_color
    }

    pub fn set_nr_color(&mut self, val: f32) {
        self.nr_color = val;
    }

    pub fn temp(&self) -> f32 {
        self.temperature.unwrap_or(5500.0)
    }

    pub fn set_temp(&mut self, val: f32) {
        self.temperature = Some(val);
    }

    /// Evaluates the recipe and returns a new Recipe with adjustments scaled by `intensity` (0.0 to 1.5, representing 0% to 150%).
    ///
    /// Monotonicity guarantee:
    /// - At `intensity = 0.0`: All relative sliders evaluate strictly to 0.0; temperature is neutral (5500K) or None; curves collapse to identity.
    /// - At `intensity = 1.0`: Nominal recipe values are preserved without drift.
    /// - At `intensity = 1.5`: Proportional 1.5x amplification clamped strictly within physical engine bounds.
    /// - Negative intensity is clamped to 0.0. Values > 1.5 are clamped to 1.5. NaN/Inf inputs default safely to 1.0.
    pub fn apply_intensity(&self, intensity: f64) -> Recipe {
        let i = if intensity.is_nan() || intensity.is_infinite() {
            1.0
        } else {
            intensity.clamp(0.0, 1.5)
        };

        let mut out = self.clone();

        // 1. Scale 15 relative core sliders
        out.exposure = DevelopSlider::Exposure
            .bounds()
            .clamp((self.exposure as f64 * i) as f32);
        out.contrast = DevelopSlider::Contrast
            .bounds()
            .clamp((self.contrast as f64 * i) as f32);
        out.highlights = DevelopSlider::Highlights
            .bounds()
            .clamp((self.highlights as f64 * i) as f32);
        out.shadows = DevelopSlider::Shadows
            .bounds()
            .clamp((self.shadows as f64 * i) as f32);
        out.whites = DevelopSlider::Whites
            .bounds()
            .clamp((self.whites as f64 * i) as f32);
        out.blacks = DevelopSlider::Blacks
            .bounds()
            .clamp((self.blacks as f64 * i) as f32);
        out.tint = DevelopSlider::Tint
            .bounds()
            .clamp((self.tint as f64 * i) as f32);
        out.vibrance = DevelopSlider::Vibrance
            .bounds()
            .clamp((self.vibrance as f64 * i) as f32);
        out.saturation = DevelopSlider::Saturation
            .bounds()
            .clamp((self.saturation as f64 * i) as f32);
        out.clarity = DevelopSlider::Clarity
            .bounds()
            .clamp((self.clarity as f64 * i) as f32);
        out.dehaze = DevelopSlider::Dehaze
            .bounds()
            .clamp((self.dehaze as f64 * i) as f32);
        out.sharpen_amount = DevelopSlider::SharpenAmount
            .bounds()
            .clamp((self.sharpen_amount as f64 * i) as f32);
        out.nr_luminance = DevelopSlider::NrLuminance
            .bounds()
            .clamp((self.nr_luminance as f64 * i) as f32);
        out.nr_color = DevelopSlider::NrColor
            .bounds()
            .clamp((self.nr_color as f64 * i) as f32);
        out.vignette = DevelopSlider::Vignette
            .bounds()
            .clamp((self.vignette as f64 * i) as f32);

        // 2. Absolute Temperature (Kelvin): base standard daylight is 5500K
        if let Some(target_temp) = self.temperature {
            let base = 5500.0_f64;
            let delta = (target_temp as f64) - base;
            let scaled_temp = (base + delta * i).clamp(2000.0, 50000.0);
            out.temperature = Some(scaled_temp as f32);
        }

        // 3. Treatment: if monochrome, only activate at intensity >= 0.5; at 0.0, fall back to "color"
        if let Some(ref tr) = self.treatment {
            if tr == "black_and_white" && i < 0.5 {
                out.treatment = Some("color".to_string());
            }
        }

        // 4. Color Mixer scaling
        if let Some(ref m) = self.mixer {
            let mut sm = m.clone();
            macro_rules! scale_hsl {
                ($field:ident) => {
                    sm.$field.hue = ((sm.$field.hue as f64 * i).clamp(-100.0, 100.0)) as f32;
                    sm.$field.sat = ((sm.$field.sat as f64 * i).clamp(-100.0, 100.0)) as f32;
                    sm.$field.lum = ((sm.$field.lum as f64 * i).clamp(-100.0, 100.0)) as f32;
                };
            }
            scale_hsl!(red);
            scale_hsl!(orange);
            scale_hsl!(yellow);
            scale_hsl!(green);
            scale_hsl!(aqua);
            scale_hsl!(blue);
            scale_hsl!(purple);
            scale_hsl!(magenta);
            out.mixer = Some(sm);
        }

        // 5. B&W Panchromatic mix scaling
        if let Some(ref bw) = self.bw_mix {
            let mut sbw = bw.clone();
            macro_rules! scale_bw {
                ($field:ident) => {
                    sbw.$field = ((sbw.$field as f64 * i).clamp(-100.0, 100.0)) as f32;
                };
            }
            scale_bw!(red);
            scale_bw!(orange);
            scale_bw!(yellow);
            scale_bw!(green);
            scale_bw!(aqua);
            scale_bw!(blue);
            scale_bw!(purple);
            scale_bw!(magenta);
            out.bw_mix = Some(sbw);
        }

        // 6. Grading split-toning scaling (hue held constant; sat & lum scale)
        if let Some(ref g) = self.grading {
            let mut sg = g.clone();
            sg.shadows.sat = ((sg.shadows.sat as f64 * i).clamp(0.0, 100.0)) as f32;
            sg.shadows.lum = ((sg.shadows.lum as f64 * i).clamp(-100.0, 100.0)) as f32;
            sg.midtones.sat = ((sg.midtones.sat as f64 * i).clamp(0.0, 100.0)) as f32;
            sg.midtones.lum = ((sg.midtones.lum as f64 * i).clamp(-100.0, 100.0)) as f32;
            sg.highlights.sat = ((sg.highlights.sat as f64 * i).clamp(0.0, 100.0)) as f32;
            sg.highlights.lum = ((sg.highlights.lum as f64 * i).clamp(-100.0, 100.0)) as f32;
            sg.global.sat = ((sg.global.sat as f64 * i).clamp(0.0, 100.0)) as f32;
            sg.global.lum = ((sg.global.lum as f64 * i).clamp(-100.0, 100.0)) as f32;
            out.grading = Some(sg);
        }

        // 7. Grain scaling
        if let Some(ref gr) = self.grain {
            let mut sgr = gr.clone();
            sgr.amount = ((sgr.amount as f64 * i).clamp(0.0, 100.0)) as f32;
            out.grain = Some(sgr);
        }

        // 8. Custom sliders scaling
        let mut scaled_custom = HashMap::new();
        for (k, v) in &self.custom_sliders {
            if let Some(num) = v.as_f64() {
                let scaled = (num * i).clamp(-100.0, 100.0);
                scaled_custom.insert(k.clone(), serde_json::json!(scaled));
            } else {
                scaled_custom.insert(k.clone(), v.clone());
            }
        }
        out.custom_sliders = scaled_custom;

        out
    }

    /// Evaluates the recipe at a given intensity percentage (0.0 to 1.5, representing 0% to 150%)
    /// against an optional photo baseline temperature, returning flat control values for LightCraft `set_develop.values`.
    pub fn evaluate_flat(
        &self,
        intensity: f64,
        baseline_temp: Option<f64>,
    ) -> HashMap<String, f64> {
        let i = if intensity.is_nan() || intensity.is_infinite() {
            1.0
        } else {
            intensity.clamp(0.0, 1.5)
        };
        let mut map = HashMap::with_capacity(32);

        macro_rules! scale_relative {
            ($slider:expr, $val:expr) => {
                let bounds = $slider.bounds();
                let scaled = bounds.clamp(($val as f64 * i) as f32);
                map.insert($slider.control_id().to_string(), scaled as f64);
            };
        }

        scale_relative!(DevelopSlider::Exposure, self.exposure);
        scale_relative!(DevelopSlider::Contrast, self.contrast);
        scale_relative!(DevelopSlider::Highlights, self.highlights);
        scale_relative!(DevelopSlider::Shadows, self.shadows);
        scale_relative!(DevelopSlider::Whites, self.whites);
        scale_relative!(DevelopSlider::Blacks, self.blacks);
        scale_relative!(DevelopSlider::Tint, self.tint);
        scale_relative!(DevelopSlider::Vibrance, self.vibrance);
        scale_relative!(DevelopSlider::Saturation, self.saturation);
        scale_relative!(DevelopSlider::Clarity, self.clarity);
        scale_relative!(DevelopSlider::Dehaze, self.dehaze);
        scale_relative!(DevelopSlider::SharpenAmount, self.sharpen_amount);
        scale_relative!(DevelopSlider::NrLuminance, self.nr_luminance);
        scale_relative!(DevelopSlider::NrColor, self.nr_color);
        scale_relative!(DevelopSlider::Vignette, self.vignette);

        // Absolute slider: Temperature
        if let Some(target_temp) = self.temperature {
            let base = baseline_temp.unwrap_or(6500.0);
            let delta = (target_temp as f64) - base;
            let scaled_temp = (base + delta * i).clamp(2000.0, 50000.0);
            map.insert(
                DevelopSlider::Temperature.control_id().to_string(),
                scaled_temp,
            );
        }

        // Extensibility map scaling
        for (key, val) in &self.custom_sliders {
            if let Some(num) = val.as_f64() {
                let slider = DevelopSlider::Custom(key.clone());
                let bounds = slider.bounds();
                let scaled = bounds.clamp((num * i) as f32);
                map.insert(key.clone(), scaled as f64);
            }
        }

        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nominal_recipe_construction() {
        let mut r = Recipe::default();
        r.exposure = 0.5;
        r.contrast = 20.0;
        assert_eq!(r.exposure, 0.5);
        assert_eq!(r.contrast, 20.0);
        assert_eq!(r.sharpen_amount, 0.0);
    }

    #[test]
    fn test_intensity_0_yields_neutral() {
        let mut r = Recipe::default();
        r.exposure = 1.5;
        r.contrast = 30.0;
        r.highlights = -50.0;
        r.temperature = Some(7000.0);

        let scaled = r.apply_intensity(0.0);
        assert_eq!(scaled.exposure, 0.0);
        assert_eq!(scaled.contrast, 0.0);
        assert_eq!(scaled.highlights, 0.0);
        assert_eq!(scaled.temperature, Some(5500.0));

        let flat = r.evaluate_flat(0.0, Some(6500.0));
        assert_eq!(flat["light.exposure"], 0.0);
        assert_eq!(flat["light.contrast"], 0.0);
        assert_eq!(flat["wb.temp"], 6500.0);
    }

    #[test]
    fn test_intensity_100_fidelity() {
        let mut r = Recipe::default();
        r.exposure = -0.5;
        r.contrast = 15.0;
        r.temperature = Some(6000.0);

        let scaled = r.apply_intensity(1.0);
        assert!((scaled.exposure - (-0.5)).abs() < 1e-6);
        assert!((scaled.contrast - 15.0).abs() < 1e-6);
        assert!((scaled.temperature.unwrap() - 6000.0).abs() < 1e-6);
    }

    #[test]
    fn test_intensity_150_scaling_and_clamping() {
        let mut r = Recipe::default();
        r.contrast = 80.0; // 80 * 1.5 = 120 -> clamped to 100.0
        r.exposure = -4.0; // -4 * 1.5 = -6.0 -> clamped to -5.0

        let scaled = r.apply_intensity(1.5);
        assert_eq!(scaled.contrast, 100.0);
        assert_eq!(scaled.exposure, -5.0);
    }

    #[test]
    fn test_intensity_clamping_negative_and_excess() {
        let mut r = Recipe::default();
        r.contrast = 20.0;

        let neg = r.apply_intensity(-0.5);
        assert_eq!(neg.contrast, 0.0);

        let excess = r.apply_intensity(3.0);
        assert_eq!(excess.contrast, 30.0); // 20 * 1.5
    }
}
