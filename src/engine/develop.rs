//! Translation layer and bridge between Recipe engine and LightCraft develop parameters / native presets.

use std::collections::HashMap;

use crate::engine::recipe::Recipe;
use crate::mcp::tools::{
    BwMixSettings, DevelopSettings, GradingSettings, McpPresetInfo, MixerSettings, ProfileSettings,
};

/// Bridge providing bidirectional conversion between `Recipe` and LightCraft `DevelopSettings` / flat maps.
pub struct DevelopBridge;

impl DevelopBridge {
    /// Translates a Recipe at given intensity (0.0..1.5) into a flat map of slider IDs -> values for `set_develop.values`.
    pub fn recipe_to_flat_values(recipe: &Recipe, intensity: f64) -> HashMap<String, f64> {
        recipe.evaluate_flat(intensity, None)
    }

    /// Deep merges a Recipe on top of an existing DevelopSettings tree.
    pub fn apply_recipe_to_settings(
        base: &DevelopSettings,
        recipe: &Recipe,
        intensity: f64,
    ) -> DevelopSettings {
        let i = if intensity.is_nan() || intensity.is_infinite() {
            1.0
        } else {
            intensity.clamp(0.0, 1.5)
        };
        let mut out = base.clone();

        // 1. Treatment
        if let Some(ref tr) = recipe.treatment {
            if i >= 0.5 {
                out.treatment = tr.clone();
            }
        }

        // 2. Profile
        if let Some(ref prof) = recipe.profile {
            out.profile = ProfileSettings {
                id: prof.clone(),
                amount: ((100.0 * i) as f32).clamp(0.0, 200.0),
            };
        }

        // 3. Light & Exposure
        out.light.exposure =
            ((base.light.exposure as f64 + recipe.exposure as f64 * i).clamp(-5.0, 5.0)) as f32;
        out.light.contrast =
            ((base.light.contrast as f64 + recipe.contrast as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.light.highlights = ((base.light.highlights as f64 + recipe.highlights as f64 * i)
            .clamp(-100.0, 100.0)) as f32;
        out.light.shadows =
            ((base.light.shadows as f64 + recipe.shadows as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.light.whites =
            ((base.light.whites as f64 + recipe.whites as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.light.blacks =
            ((base.light.blacks as f64 + recipe.blacks as f64 * i).clamp(-100.0, 100.0)) as f32;

        // 4. White Balance
        if let Some(recipe_temp) = recipe.temperature {
            let temp_delta = (recipe_temp as f64 - 5500.0) * i;
            out.wb.temp = ((base.wb.temp as f64 + temp_delta).clamp(2000.0, 50000.0)) as f32;
        }
        out.wb.tint = ((base.wb.tint as f64 + recipe.tint as f64 * i).clamp(-150.0, 150.0)) as f32;

        // 5. Color
        out.color.vibrance =
            ((base.color.vibrance as f64 + recipe.vibrance as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.color.saturation = ((base.color.saturation as f64 + recipe.saturation as f64 * i)
            .clamp(-100.0, 100.0)) as f32;

        // 6. Effects
        out.effects.clarity =
            ((base.effects.clarity as f64 + recipe.clarity as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.effects.dehaze =
            ((base.effects.dehaze as f64 + recipe.dehaze as f64 * i).clamp(-100.0, 100.0)) as f32;

        // 7. Detail
        out.detail.sharpen_amount = ((base.detail.sharpen_amount as f64
            + recipe.sharpen_amount as f64 * i)
            .clamp(0.0, 150.0)) as f32;
        out.detail.nr_luminance = ((base.detail.nr_luminance as f64
            + recipe.nr_luminance as f64 * i)
            .clamp(0.0, 100.0)) as f32;
        out.detail.nr_color =
            ((base.detail.nr_color as f64 + recipe.nr_color as f64 * i).clamp(0.0, 100.0)) as f32;

        // 8. Vignette
        out.vignette.amount = ((base.vignette.amount as f64 + recipe.vignette as f64 * i)
            .clamp(-100.0, 100.0)) as f32;

        // 9. Structured Blocks
        if let Some(ref m) = recipe.mixer {
            out.mixer = Self::scale_mixer(m, i);
        }
        if let Some(ref bw) = recipe.bw_mix {
            out.bw_mix = Self::scale_bw_mix(bw, i);
        }
        if let Some(ref g) = recipe.grading {
            out.grading = Self::scale_grading(g, i);
        }
        if let Some(ref gr) = recipe.grain {
            let mut sgr = gr.clone();
            sgr.amount = ((sgr.amount as f64 * i).clamp(0.0, 100.0)) as f32;
            out.grain = sgr;
        }

        out
    }

    /// Extracts a normalized `Recipe` from active `DevelopSettings`.
    pub fn settings_to_recipe(settings: &DevelopSettings, name: impl Into<String>) -> Recipe {
        let name_str = name.into();
        let slug = name_str
            .to_lowercase()
            .replace(' ', "-")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect::<String>();

        Recipe {
            id: if slug.is_empty() {
                "custom-recipe".into()
            } else {
                slug
            },
            name: name_str,
            category: Default::default(),
            description: None,
            profile: Some(settings.profile.id.clone()),
            treatment: Some(settings.treatment.clone()),
            exposure: settings.light.exposure,
            contrast: settings.light.contrast,
            highlights: settings.light.highlights,
            shadows: settings.light.shadows,
            whites: settings.light.whites,
            blacks: settings.light.blacks,
            temperature: Some(settings.wb.temp),
            tint: settings.wb.tint,
            vibrance: settings.color.vibrance,
            saturation: settings.color.saturation,
            clarity: settings.effects.clarity,
            dehaze: settings.effects.dehaze,
            sharpen_amount: settings.detail.sharpen_amount,
            nr_luminance: settings.detail.nr_luminance,
            nr_color: settings.detail.nr_color,
            vignette: settings.vignette.amount,
            mixer: Some(settings.mixer.clone()),
            bw_mix: Some(settings.bw_mix.clone()),
            grading: Some(settings.grading.clone()),
            curve: Some(settings.curve.clone()),
            grain: Some(settings.grain.clone()),
            custom_sliders: HashMap::new(),
            metadata: Default::default(),
        }
    }

    fn scale_mixer(m: &MixerSettings, i: f64) -> MixerSettings {
        let mut out = m.clone();
        macro_rules! scale_hsl {
            ($field:ident) => {
                out.$field.hue = ((out.$field.hue as f64 * i).clamp(-100.0, 100.0)) as f32;
                out.$field.sat = ((out.$field.sat as f64 * i).clamp(-100.0, 100.0)) as f32;
                out.$field.lum = ((out.$field.lum as f64 * i).clamp(-100.0, 100.0)) as f32;
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
        out
    }

    fn scale_bw_mix(bw: &BwMixSettings, i: f64) -> BwMixSettings {
        let mut out = bw.clone();
        out.red = ((out.red as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.orange = ((out.orange as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.yellow = ((out.yellow as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.green = ((out.green as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.aqua = ((out.aqua as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.blue = ((out.blue as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.purple = ((out.purple as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.magenta = ((out.magenta as f64 * i).clamp(-100.0, 100.0)) as f32;
        out
    }

    fn scale_grading(g: &GradingSettings, i: f64) -> GradingSettings {
        let mut out = g.clone();
        out.shadows.sat = ((out.shadows.sat as f64 * i).clamp(0.0, 100.0)) as f32;
        out.shadows.lum = ((out.shadows.lum as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.highlights.sat = ((out.highlights.sat as f64 * i).clamp(0.0, 100.0)) as f32;
        out.highlights.lum = ((out.highlights.lum as f64 * i).clamp(-100.0, 100.0)) as f32;
        out.midtones.sat = ((out.midtones.sat as f64 * i).clamp(0.0, 100.0)) as f32;
        out.midtones.lum = ((out.midtones.lum as f64 * i).clamp(-100.0, 100.0)) as f32;
        out
    }
}

/// In-memory catalog of native LightCraft presets discovered via MCP `cmd_presets_list`.
#[derive(Debug, Clone, Default)]
pub struct NativePresetRegistry {
    presets_by_id: HashMap<String, McpPresetInfo>,
    presets_by_name: HashMap<String, McpPresetInfo>,
    groups: HashMap<String, Vec<String>>,
}

impl NativePresetRegistry {
    /// Ingests preset list from MCP `cmd_presets_list` call.
    pub fn populate(&mut self, presets: Vec<McpPresetInfo>) {
        self.presets_by_id.clear();
        self.presets_by_name.clear();
        self.groups.clear();

        for p in presets {
            self.presets_by_id.insert(p.id.clone(), p.clone());
            self.presets_by_name
                .insert(p.name.to_lowercase(), p.clone());
            self.groups
                .entry(p.group.clone())
                .or_default()
                .push(p.id.clone());
        }
    }

    /// Looks up native preset by exact ID (e.g. "lc.teal-orange").
    pub fn find_by_id(&self, id: &str) -> Option<&McpPresetInfo> {
        self.presets_by_id.get(id)
    }

    /// Looks up native preset by loose name (case-insensitive).
    pub fn find_by_name(&self, name: &str) -> Option<&McpPresetInfo> {
        self.presets_by_name.get(&name.trim().to_lowercase())
    }

    /// Enumerate all presets in a group (e.g. "Landscape", "B&W", "Color").
    pub fn get_group(&self, group: &str) -> Vec<&McpPresetInfo> {
        self.groups
            .get(group)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.presets_by_id.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Checks if an ID corresponds to an official native LightCraft preset.
    pub fn is_native(&self, id: &str) -> bool {
        self.presets_by_id.contains_key(id)
    }

    /// Returns all registered presets.
    pub fn all(&self) -> Vec<&McpPresetInfo> {
        self.presets_by_id.values().collect()
    }

    /// Total count of registered presets.
    pub fn len(&self) -> usize {
        self.presets_by_id.len()
    }

    /// Whether registry is empty.
    pub fn is_empty(&self) -> bool {
        self.presets_by_id.is_empty()
    }
}

/// Execution strategy for applying aesthetic variations.
#[derive(Debug, Clone)]
pub enum PresetApplicationStrategy {
    /// Dispatches `apply_preset` directly to LightCraft daemon.
    ServerNative { preset_id: String, amount_pct: f64 },
    /// Renders client-side Recipe and transmits full DevelopSettings / flat values.
    ClientRecipe { recipe: Recipe, intensity: f64 },
    /// Two-phase hybrid: applies native preset as base, then layers custom adjustments.
    HybridMerge {
        native_preset_id: String,
        native_amount: f64,
        recipe_overlay: Recipe,
        overlay_intensity: f64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_develop_bridge_roundtrip() {
        let ds = DevelopSettings::default();
        let r = DevelopBridge::settings_to_recipe(&ds, "Roundtrip Test");
        assert_eq!(r.exposure, ds.light.exposure);
        assert_eq!(r.contrast, ds.light.contrast);
        assert_eq!(r.temperature, Some(ds.wb.temp));

        let merged = DevelopBridge::apply_recipe_to_settings(&ds, &r, 1.0);
        assert_eq!(merged.light.exposure, ds.light.exposure);
        assert_eq!(merged.light.contrast, ds.light.contrast);
    }

    #[test]
    fn test_native_preset_registry() {
        let mut reg = NativePresetRegistry::default();
        reg.populate(vec![
            McpPresetInfo {
                id: "lc.teal-orange".into(),
                name: "Teal & Orange".into(),
                group: "Color".into(),
                builtin: true,
                favorite: true,
            },
            McpPresetInfo {
                id: "lc.bw-high-contrast".into(),
                name: "B&W High Contrast".into(),
                group: "B&W".into(),
                builtin: true,
                favorite: false,
            },
        ]);

        assert_eq!(reg.len(), 2);
        assert!(reg.is_native("lc.teal-orange"));
        assert_eq!(
            reg.find_by_name("teal & orange").unwrap().id,
            "lc.teal-orange"
        );
        assert_eq!(reg.get_group("Color").len(), 1);
    }
}
