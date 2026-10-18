//! Aspect ratio geometry, center-crop calculator, and interactive wizard models (US-001).

use serde::{Deserialize, Serialize};

/// Standard photographic and digital aspect ratio formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AspectRatio {
    /// 1:1 Square (Instagram Feed, album art)
    Square1x1,
    /// 3:2 Traditional 35mm DSLR / Mirrorless standard
    Photo3x2,
    /// 4:3 Classic digital / Medium format standard
    Classic4x3,
    /// 16:9 High-definition widescreen / Video / Banners
    Widescreen16x9,
    /// 4:5 Vertical portrait / Social networks
    Social4x5,
    /// Original uncropped geometry
    Original,
    /// Custom user-specified width-to-height ratio (w, h)
    Custom(u32, u32),
}

impl Default for AspectRatio {
    fn default() -> Self {
        Self::Original
    }
}

impl AspectRatio {
    /// Short token label suitable for filename sanitization and template expansion.
    pub fn aspect_label(&self) -> &'static str {
        match self {
            Self::Square1x1 => "1x1",
            Self::Photo3x2 => "3x2",
            Self::Classic4x3 => "4x3",
            Self::Widescreen16x9 => "16x9",
            Self::Social4x5 => "4x5",
            Self::Original => "orig",
            Self::Custom(_, _) => "custom",
        }
    }

    /// Descriptive human-readable name for TUI menus and summary tables.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Square1x1 => "1:1 Cuadrado (Redes / Portadas)",
            Self::Photo3x2 => "3:2 Fotografía tradicional 35mm",
            Self::Classic4x3 => "4:3 Clásico digital / Formato medio",
            Self::Widescreen16x9 => "16:9 Panorámico / Pantalla completa",
            Self::Social4x5 => "4:5 Retrato vertical / Social",
            Self::Original => "Original (Sin recorte)",
            Self::Custom(_, _) => "Personalizado",
        }
    }

    /// Numerical aspect ratio as (width_ratio, height_ratio) if non-original.
    pub fn ratio_dimensions(&self) -> Option<(u32, u32)> {
        match self {
            Self::Square1x1 => Some((1, 1)),
            Self::Photo3x2 => Some((3, 2)),
            Self::Classic4x3 => Some((4, 3)),
            Self::Widescreen16x9 => Some((16, 9)),
            Self::Social4x5 => Some((4, 5)),
            Self::Original => None,
            Self::Custom(w, h) => {
                if *w > 0 && *h > 0 {
                    Some((*w, *h))
                } else {
                    None
                }
            }
        }
    }

    /// Standard list of all supported presets for interactive selection.
    pub const ALL: [AspectRatio; 6] = [
        AspectRatio::Square1x1,
        AspectRatio::Photo3x2,
        AspectRatio::Classic4x3,
        AspectRatio::Widescreen16x9,
        AspectRatio::Social4x5,
        AspectRatio::Original,
    ];
}

/// Compute symmetric center-crop box (x, y, crop_w, crop_h) preserving maximum usable area.
///
/// Guaranteed properties:
/// 1. Zero pixel distortion: the crop adheres exactly to the target aspect ratio.
/// 2. Maximal area: crops only along the axis that exceeds the target proportion.
/// 3. Symmetric centering: `x = (orig_w - crop_w) / 2` and `y = (orig_h - crop_h) / 2`.
/// 4. Boundaries: `x + crop_w <= orig_w` and `y + crop_h <= orig_h`.
pub fn calculate_center_crop(orig_w: u32, orig_h: u32, ratio: AspectRatio) -> (u32, u32, u32, u32) {
    if orig_w == 0 || orig_h == 0 {
        return (0, 0, orig_w, orig_h);
    }

    let Some((rw, rh)) = ratio.ratio_dimensions() else {
        return (0, 0, orig_w, orig_h);
    };

    let target_ratio = rw as f64 / rh as f64;
    let source_ratio = orig_w as f64 / orig_h as f64;

    // Epsilon to prevent unnecessary 1-pixel rounding crops on matching ratios
    const EPSILON: f64 = 1e-4;

    if (source_ratio - target_ratio).abs() < EPSILON {
        return (0, 0, orig_w, orig_h);
    }

    if source_ratio > target_ratio {
        // Source is wider than target: constrain height, crop width symmetrically
        let crop_h = orig_h;
        let crop_w = ((orig_h as f64 * target_ratio).round() as u32)
            .min(orig_w)
            .max(1);
        let x = (orig_w.saturating_sub(crop_w)) / 2;
        let y = 0;
        (x, y, crop_w, crop_h)
    } else {
        // Source is taller than target: constrain width, crop height symmetrically
        let crop_w = orig_w;
        let crop_h = ((orig_w as f64 / target_ratio).round() as u32)
            .min(orig_h)
            .max(1);
        let x = 0;
        let y = (orig_h.saturating_sub(crop_h)) / 2;
        (x, y, crop_w, crop_h)
    }
}

/// Active step within the top interactive wizard flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WizardStep {
    Photos,
    #[default]
    Variants,
    AspectRatios,
    FileFormats,
    Summary,
    Exporting,
    SavePreset,
}

impl WizardStep {
    pub fn step_number(&self) -> usize {
        match self {
            Self::Photos => 1,
            Self::Variants => 2,
            Self::AspectRatios => 3,
            Self::FileFormats => 4,
            Self::Summary => 5,
            Self::Exporting => 6,
            Self::SavePreset => 5,
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Photos => "PASO 1/6: ENTRADA DE FOTOS",
            Self::Variants => "PASO 2/6: VARIANTES Y REVELADO",
            Self::AspectRatios => "PASO 3/6: PROPORCIONES (ASPECT RATIOS)",
            Self::FileFormats => "PASO 4/6: FORMATOS DE SALIDA",
            Self::Summary => "PASO 5/6: RESUMEN Y CONFIRMACIÓN",
            Self::Exporting => "PASO 6/6: RENDERIZADO / EXPORTACIÓN EN PROCESO",
            Self::SavePreset => "GUARDAR CONFIGURACIÓN COMO PRESET",
        }
    }
}

/// Runtime state for the top-banner interactive wizard flow.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractiveFlowState {
    pub is_active: bool,
    pub step: WizardStep,
    pub selected_variants: Vec<usize>,
    pub selected_ratios: Vec<AspectRatio>,
    pub selected_formats: Vec<crate::export::OutputFormat>,
    pub focused_ratio_idx: usize,
    pub focused_format_idx: usize,
    pub focused_summary_idx: usize,
    pub export_completed: usize,
    pub export_total: usize,
    pub export_current_label: String,
    pub export_percent: f32,
    pub export_start_time: Option<std::time::Instant>,
    pub is_confirming_cancel: bool,
}

impl Eq for InteractiveFlowState {}

impl Default for InteractiveFlowState {
    fn default() -> Self {
        Self {
            is_active: false,
            step: WizardStep::Variants,
            selected_variants: Vec::new(),
            selected_ratios: vec![AspectRatio::Original],
            selected_formats: vec![crate::export::OutputFormat::Tiff],
            focused_ratio_idx: 0,
            focused_format_idx: 0,
            focused_summary_idx: 0,
            export_completed: 0,
            export_total: 0,
            export_current_label: String::new(),
            export_percent: 0.0,
            export_start_time: None,
            is_confirming_cancel: false,
        }
    }
}

impl InteractiveFlowState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start_with_variants(&mut self, variants: Vec<usize>) {
        self.is_active = true;
        self.step = WizardStep::AspectRatios;
        self.selected_variants = variants;
        self.focused_ratio_idx = 0;
        self.focused_format_idx = 0;
        self.focused_summary_idx = 0;
        if self.selected_ratios.is_empty() {
            self.selected_ratios = vec![AspectRatio::Original];
        }
        if self.selected_formats.is_empty() {
            self.selected_formats = vec![crate::export::OutputFormat::Tiff];
        }
    }

    pub fn toggle_ratio(&mut self, ratio: AspectRatio) {
        if let Some(pos) = self.selected_ratios.iter().position(|r| *r == ratio) {
            if self.selected_ratios.len() > 1 {
                self.selected_ratios.remove(pos);
            }
        } else {
            self.selected_ratios.push(ratio);
        }
    }

    pub fn prev_ratio(&mut self) {
        // 7 options: 6 AspectRatio variants + 1 "All"
        if self.focused_ratio_idx > 0 {
            self.focused_ratio_idx -= 1;
        } else {
            self.focused_ratio_idx = 6;
        }
    }

    pub fn next_ratio(&mut self) {
        if self.focused_ratio_idx < 6 {
            self.focused_ratio_idx += 1;
        } else {
            self.focused_ratio_idx = 0;
        }
    }

    pub fn toggle_focused_ratio(&mut self) {
        match self.focused_ratio_idx {
            0 => self.toggle_ratio(AspectRatio::Square1x1),
            1 => self.toggle_ratio(AspectRatio::Photo3x2),
            2 => self.toggle_ratio(AspectRatio::Classic4x3),
            3 => self.toggle_ratio(AspectRatio::Widescreen16x9),
            4 => self.toggle_ratio(AspectRatio::Social4x5),
            5 => self.toggle_ratio(AspectRatio::Original),
            6 => {
                if self.selected_ratios.len() == AspectRatio::ALL.len() {
                    self.selected_ratios = vec![AspectRatio::Original];
                } else {
                    self.selected_ratios = AspectRatio::ALL.to_vec();
                }
            }
            _ => {}
        }
    }

    pub fn prev_format(&mut self) {
        // 5 options: 0=TIFF, 1=JPEG, 2=PNG, 3=TIFF+JPEG, 4=Todos
        if self.focused_format_idx > 0 {
            self.focused_format_idx -= 1;
        } else {
            self.focused_format_idx = 4;
        }
    }

    pub fn next_format(&mut self) {
        if self.focused_format_idx < 4 {
            self.focused_format_idx += 1;
        } else {
            self.focused_format_idx = 0;
        }
    }

    pub fn toggle_focused_format(&mut self) {
        match self.focused_format_idx {
            0 => self.toggle_format(crate::export::OutputFormat::Tiff),
            1 => self.toggle_format(crate::export::OutputFormat::Jpeg),
            2 => self.toggle_format(crate::export::OutputFormat::Png),
            3 => {
                let has_both = self
                    .selected_formats
                    .contains(&crate::export::OutputFormat::Tiff)
                    && self
                        .selected_formats
                        .contains(&crate::export::OutputFormat::Jpeg);
                if has_both && self.selected_formats.len() == 2 {
                    self.selected_formats = vec![crate::export::OutputFormat::Tiff];
                } else {
                    self.selected_formats = vec![
                        crate::export::OutputFormat::Tiff,
                        crate::export::OutputFormat::Jpeg,
                    ];
                }
            }
            4 => {
                if self.selected_formats.len() == 3 {
                    self.selected_formats = vec![crate::export::OutputFormat::Tiff];
                } else {
                    self.selected_formats = vec![
                        crate::export::OutputFormat::Tiff,
                        crate::export::OutputFormat::Jpeg,
                        crate::export::OutputFormat::Png,
                    ];
                }
            }
            _ => {}
        }
    }

    pub fn prev_summary(&mut self) {
        // 6 options: 0=Confirm, 1=Edit Var, 2=Edit Ratio, 3=Edit Fmt, 4=Preset, 5=Cancel
        if self.focused_summary_idx > 0 {
            self.focused_summary_idx -= 1;
        } else {
            self.focused_summary_idx = 5;
        }
    }

    pub fn next_summary(&mut self) {
        if self.focused_summary_idx < 5 {
            self.focused_summary_idx += 1;
        } else {
            self.focused_summary_idx = 0;
        }
    }

    pub fn set_export_progress(&mut self, completed: usize, total: usize, label: String, pct: f32) {
        self.export_completed = completed;
        self.export_total = total;
        self.export_current_label = label;
        self.export_percent = pct;
        if self.export_start_time.is_none() {
            self.export_start_time = Some(std::time::Instant::now());
        }
    }

    pub fn toggle_format(&mut self, format: crate::export::OutputFormat) {
        if let Some(pos) = self.selected_formats.iter().position(|f| *f == format) {
            if self.selected_formats.len() > 1 {
                self.selected_formats.remove(pos);
            }
        } else {
            self.selected_formats.push(format);
        }
    }

    pub fn close(&mut self) {
        self.is_active = false;
        self.step = WizardStep::Variants;
        self.export_start_time = None;
        self.is_confirming_cancel = false;
    }
}

/// Evaluated task plan calculating the matrix of jobs to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobPlan {
    pub photos_count: usize,
    pub variants_count: usize,
    pub ratios_count: usize,
    pub formats_count: usize,
}

impl JobPlan {
    pub fn new(photos: usize, variants: usize, ratios: usize, formats: usize) -> Self {
        Self {
            photos_count: photos.max(1),
            variants_count: variants.max(1),
            ratios_count: ratios.max(1),
            formats_count: formats.max(1),
        }
    }

    pub fn base_combinations(&self) -> usize {
        self.photos_count * self.variants_count * self.ratios_count
    }

    pub fn total_output_files(&self) -> usize {
        self.base_combinations() * self.formats_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_center_crop_square_on_landscape_3x2() {
        // 6000x4000 source with 1:1 square crop -> 4000x4000 centered at x=1000, y=0
        let (x, y, w, h) = calculate_center_crop(6000, 4000, AspectRatio::Square1x1);
        assert_eq!(w, 4000);
        assert_eq!(h, 4000);
        assert_eq!(x, 1000);
        assert_eq!(y, 0);
    }

    #[test]
    fn test_center_crop_widescreen_16x9_on_3x2() {
        // 3:2 is 1.5, 16:9 is ~1.777. Source is taller than target -> crop height symmetrically
        // 6000 width -> height = 6000 * 9 / 16 = 3375. y = (4000 - 3375) / 2 = 312
        let (x, y, w, h) = calculate_center_crop(6000, 4000, AspectRatio::Widescreen16x9);
        assert_eq!(w, 6000);
        assert_eq!(h, 3375);
        assert_eq!(x, 0);
        assert_eq!(y, 312);
    }

    #[test]
    fn test_center_crop_matching_aspect_is_identity() {
        let (x, y, w, h) = calculate_center_crop(6000, 4000, AspectRatio::Photo3x2);
        assert_eq!(w, 6000);
        assert_eq!(h, 4000);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
    }

    #[test]
    fn test_center_crop_original_is_identity() {
        let (x, y, w, h) = calculate_center_crop(5472, 3648, AspectRatio::Original);
        assert_eq!(w, 5472);
        assert_eq!(h, 3648);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
    }

    #[test]
    fn test_job_plan_calculation() {
        // 2 photos, 3 variants, 2 ratios, 2 formats
        let plan = JobPlan::new(2, 3, 2, 2);
        assert_eq!(plan.base_combinations(), 12);
        assert_eq!(plan.total_output_files(), 24);
    }

    #[test]
    fn test_interactive_flow_state_toggle_and_guard() {
        let mut state = InteractiveFlowState::new();
        state.start_with_variants(vec![0, 1]);
        assert_eq!(state.step, WizardStep::AspectRatios);
        assert_eq!(state.selected_variants, vec![0, 1]);

        // Toggle ratio 1:1 on
        state.toggle_ratio(AspectRatio::Square1x1);
        assert!(state.selected_ratios.contains(&AspectRatio::Square1x1));

        // Toggling off when multiple are present works
        state.toggle_ratio(AspectRatio::Original);
        assert!(!state.selected_ratios.contains(&AspectRatio::Original));
        assert_eq!(state.selected_ratios, vec![AspectRatio::Square1x1]);

        // Guard: cannot toggle off the last remaining ratio
        state.toggle_ratio(AspectRatio::Square1x1);
        assert_eq!(state.selected_ratios, vec![AspectRatio::Square1x1]);
    }
}
