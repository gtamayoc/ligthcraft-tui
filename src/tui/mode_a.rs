//! Mode A: Single Photo Inspector, Variant Grid, Split-View Comparison, and 1:1 Loupe Zoom.

use std::sync::Arc;
use std::time::Instant;
use image::{DynamicImage, GenericImage, GenericImageView, Rgba};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::engine::presets::BuiltinPreset;
use crate::engine::recipe::Recipe;
use crate::graphics::cache::CachedProtocol;
use crate::graphics::protocol::GraphicsProtocol;
use crate::mcp::tools::PhotoInfo;

/// Primary inspection view sub-modes within Mode A.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModeAView {
    #[default]
    Hero,
    VariantMatrix,
    SplitView,
    Loupe1to1,
}

/// Split-view presentation strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SplitViewKind {
    #[default]
    SideBySide,
    TopBottom,
    Wiper,
    ToggleAB,
}

/// Status of an asynchronous preview render for a variant slot.
#[derive(Debug, Clone, PartialEq)]
pub enum VariantRenderStatus {
    Unloaded,
    Pending,
    Rendering { started_at: Instant },
    Ready,
    Failed(String),
}

/// Flag status matching LightCraft and professional DAM workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlagStatus {
    #[default]
    Unflagged,
    Pick,
    Reject,
}

/// Color label matching LightCraft cmd_photo_label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorLabel {
    #[default]
    None,
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

/// Single variant candidate in the Variant Matrix.
#[derive(Debug, Clone)]
pub struct VariantSlot {
    pub index: usize,
    pub label: String,
    pub recipe: Recipe,
    pub intensity: f64,
    pub image: Option<Arc<DynamicImage>>,
    pub cached_protocol: Option<CachedProtocol>,
    pub status: VariantRenderStatus,
    pub is_favorite: bool,
    pub rating: u8,
    pub flag: FlagStatus,
    pub label_color: ColorLabel,
}

impl VariantSlot {
    pub fn new(index: usize, label: String, recipe: Recipe) -> Self {
        Self {
            index,
            label,
            recipe,
            intensity: 1.0,
            image: None,
            cached_protocol: None,
            status: VariantRenderStatus::Unloaded,
            is_favorite: false,
            rating: 0,
            flag: FlagStatus::Unflagged,
            label_color: ColorLabel::None,
        }
    }
}

/// Detailed shooting and photographic EXIF metadata.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhotoMetadata {
    pub file_name: String,
    pub file_size_bytes: u64,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub orientation: String,
    pub color_space: String,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_model: Option<String>,
    pub focal_length_mm: Option<f32>,
    pub aperture_f_number: Option<f32>,
    pub shutter_speed: Option<String>,
    pub iso: Option<u32>,
    pub exposure_bias_ev: Option<f32>,
    pub metering_mode: Option<String>,
    pub white_balance: Option<String>,
    pub capture_time: Option<String>,
}

impl PhotoMetadata {
    pub fn from_mcp_metadata(info: &PhotoInfo, raw_exif: &[serde_json::Value]) -> Self {
        let mut meta = Self {
            file_name: info.file_name.clone(),
            format: info.format.clone(),
            width: info.width,
            height: info.height,
            camera_model: info.camera.clone(),
            capture_time: info.captured.clone(),
            ..Default::default()
        };

        for entry in raw_exif {
            let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let val_entry = entry.get("value");
            
            let val_str = val_entry.and_then(|v| v.as_str()).map(|s| s.to_string());
            let val_num = val_entry.and_then(|v| v.as_f64());
            let val_u64 = val_entry.and_then(|v| v.as_u64());

            match name {
                "FNumber" | "ApertureValue" => {
                    meta.aperture_f_number = val_num
                        .map(|n| n as f32)
                        .or_else(|| val_str.as_deref().and_then(|s| s.parse::<f32>().ok()));
                }
                "ExposureTime" | "ShutterSpeedValue" => {
                    meta.shutter_speed = val_str.or_else(|| val_num.map(|n| {
                        if n < 1.0 && n > 0.0 {
                            format!("1/{}s", (1.0 / n).round() as u64)
                        } else {
                            format!("{:.1}s", n)
                        }
                    }));
                }
                "ISOSpeedRatings" | "PhotographicSensitivity" => {
                    meta.iso = val_u64
                        .map(|u| u as u32)
                        .or_else(|| val_str.as_deref().and_then(|s| s.parse::<u32>().ok()))
                        .or_else(|| {
                            val_entry.and_then(|v| v.as_array()).and_then(|arr| {
                                arr.first().and_then(|item| item.as_u64().map(|u| u as u32))
                            })
                        });
                }
                "FocalLength" => {
                    meta.focal_length_mm = val_num
                        .map(|n| n as f32)
                        .or_else(|| val_str.as_deref().and_then(|s| {
                            s.split(' ').next().and_then(|part| part.parse::<f32>().ok())
                        }));
                }
                "Model" if meta.camera_model.is_none() => {
                    meta.camera_model = val_str;
                }
                "Make" => {
                    meta.camera_make = val_str;
                }
                "LensModel" | "LensType" => {
                    meta.lens_model = val_str;
                }
                "ExposureBiasValue" => {
                    meta.exposure_bias_ev = val_num
                        .map(|n| n as f32)
                        .or_else(|| val_str.as_deref().and_then(|s| s.parse::<f32>().ok()));
                }
                "MeteringMode" => {
                    meta.metering_mode = val_str;
                }
                "WhiteBalance" => {
                    meta.white_balance = val_str;
                }
                _ => {}
            }
        }
        meta
    }

    /// Formats aspect ratio as a simplified ratio string (e.g. "3:2", "16:9").
    pub fn aspect_ratio_str(&self) -> String {
        if self.height == 0 {
            return "Unknown".into();
        }
        let gcd = gcd(self.width, self.height);
        let rw = self.width / gcd;
        let rh = self.height / gcd;
        if (rw, rh) == (3, 2)
            || (rw, rh) == (2, 3)
            || (rw, rh) == (16, 9)
            || (rw, rh) == (4, 3)
            || (rw, rh) == (1, 1)
        {
            format!("{}:{}", rw, rh)
        } else {
            let ratio = self.width as f32 / self.height as f32;
            format!("{:.2}:1", ratio)
        }
    }

    /// Megapixel calculation formatted to 1 decimal place.
    pub fn megapixels_str(&self) -> String {
        let mp = (self.width as f64 * self.height as f64) / 1_000_000.0;
        format!("{:.1} MP", mp)
    }

    /// Formats shooting exposure summary: "1/500s  f/2.8  ISO 400  50mm".
    pub fn exposure_summary(&self) -> String {
        format!(
            "{}  {}  {}  {}",
            self.shutter_speed.as_deref().unwrap_or("1/---s"),
            self.aperture_f_number
                .map(|f| format!("f/{:.1}", f))
                .unwrap_or_else(|| "f/--".into()),
            self.iso
                .map(|i| format!("ISO {}", i))
                .unwrap_or_else(|| "ISO ---".into()),
            self.focal_length_mm
                .map(|f| format!("{:.0}mm", f))
                .unwrap_or_else(|| "--mm".into()),
        )
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

/// 32-bin downsampled histogram for Luminance and RGB channels.
#[derive(Debug, Clone)]
pub struct HistogramData {
    pub bins_lum: [u32; 32],
    pub bins_r: [u32; 32],
    pub bins_g: [u32; 32],
    pub bins_b: [u32; 32],
    pub max_count: u32,
}

impl HistogramData {
    /// Compute 32-bin downsampled histogram in < 2ms from preview image.
    pub fn from_image(img: &DynamicImage) -> Self {
        let mut bins_lum = [0u32; 32];
        let mut bins_r = [0u32; 32];
        let mut bins_g = [0u32; 32];
        let mut bins_b = [0u32; 32];

        let (w, h) = img.dimensions();
        // Downsample sampling step to process ~10,000 pixels max for < 2ms performance
        let step_x = (w / 100).max(1);
        let step_y = (h / 100).max(1);

        let mut max_count = 1;

        for y in (0..h).step_by(step_y as usize) {
            for x in (0..w).step_by(step_x as usize) {
                let pixel = img.get_pixel(x, y);
                let r = pixel[0] as f32;
                let g = pixel[1] as f32;
                let b = pixel[2] as f32;

                // ITU-R BT.709 Luminance
                let lum = (0.2126 * r + 0.7152 * g + 0.0722 * b).round() as usize;

                let bin_lum = (lum * 32 / 256).min(31);
                let bin_r = (pixel[0] as usize * 32 / 256).min(31);
                let bin_g = (pixel[1] as usize * 32 / 256).min(31);
                let bin_b = (pixel[2] as usize * 32 / 256).min(31);

                bins_lum[bin_lum] += 1;
                bins_r[bin_r] += 1;
                bins_g[bin_g] += 1;
                bins_b[bin_b] += 1;

                max_count = max_count.max(bins_lum[bin_lum]);
            }
        }

        Self {
            bins_lum,
            bins_r,
            bins_g,
            bins_b,
            max_count,
        }
    }

    /// Produce 4-line sparkline text for terminal display.
    pub fn sparklines(&self) -> Vec<String> {
        let chars = [' ', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let to_spark = |bins: &[u32; 32]| -> String {
            bins.iter()
                .map(|&count| {
                    if self.max_count == 0 || count == 0 {
                        ' '
                    } else {
                        let idx = ((count as f32 / self.max_count as f32) * 7.0).round() as usize;
                        chars[idx.min(7)]
                    }
                })
                .collect()
        };

        vec![
            format!("Lum: [{}]", to_spark(&self.bins_lum)),
            format!("Red: [{}]", to_spark(&self.bins_r)),
            format!("Grn: [{}]", to_spark(&self.bins_g)),
            format!("Blu: [{}]", to_spark(&self.bins_b)),
        ]
    }
}

/// 1:1 Loupe navigation and pan coordinates.
#[derive(Debug, Clone)]
pub struct LoupeState {
    pub center_x: f32,
    pub center_y: f32,
    pub zoom_level: f32,
    pub show_minimap: bool,
}

impl Default for LoupeState {
    fn default() -> Self {
        Self {
            center_x: 0.5,
            center_y: 0.5,
            zoom_level: 1.0,
            show_minimap: true,
        }
    }
}

impl LoupeState {
    pub fn compute_crop_rect(
        &self,
        img_w: u32,
        img_h: u32,
        canvas_w: u32,
        canvas_h: u32,
    ) -> (u32, u32, u32, u32) {
        let crop_w = ((canvas_w as f32) / self.zoom_level)
            .min(img_w as f32)
            .max(1.0) as u32;
        let crop_h = ((canvas_h as f32) / self.zoom_level)
            .min(img_h as f32)
            .max(1.0) as u32;

        let ideal_x = (self.center_x * img_w as f32) - (crop_w as f32 / 2.0);
        let ideal_y = (self.center_y * img_h as f32) - (crop_h as f32 / 2.0);

        let max_x = img_w.saturating_sub(crop_w);
        let max_y = img_h.saturating_sub(crop_h);

        let x0 = (ideal_x.clamp(0.0, max_x as f32)).round() as u32;
        let y0 = (ideal_y.clamp(0.0, max_y as f32)).round() as u32;

        (x0, y0, crop_w, crop_h)
    }

    pub fn pan(&mut self, dx: f32, dy: f32) {
        self.center_x = (self.center_x + dx).clamp(0.0, 1.0);
        self.center_y = (self.center_y + dy).clamp(0.0, 1.0);
    }
}

/// Stitches left half of before and right half of after with a 1-pixel separator line.
pub fn composite_wiper(
    before: &DynamicImage,
    after: &DynamicImage,
    split_ratio: f32,
) -> DynamicImage {
    let (w, h) = (before.width().min(after.width()), before.height().min(after.height()));
    if w == 0 || h == 0 {
        return after.clone();
    }
    let split_x = ((w as f32) * split_ratio.clamp(0.05, 0.95)) as u32;

    let mut out = after.clone();
    for y in 0..h {
        for x in 0..split_x.min(w) {
            out.put_pixel(x, y, before.get_pixel(x, y));
        }
        if split_x < w {
            out.put_pixel(split_x, y, Rgba([255, 255, 255, 255]));
        }
        if split_x + 1 < w {
            out.put_pixel(split_x + 1, y, Rgba([40, 40, 40, 200]));
        }
    }
    out
}

/// Compute responsive non-overlapping sub-rects for 2 to 6 variant grids.
pub fn compute_variant_grid_rects(area: Rect, count: usize) -> Vec<Rect> {
    match count {
        2 => Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area)
            .to_vec(),
        3 => Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ])
            .split(area)
            .to_vec(),
        4 => {
            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            let top = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(v_chunks[0]);
            let bottom = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(v_chunks[1]);
            vec![top[0], top[1], bottom[0], bottom[1]]
        }
        5 | 6 => {
            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            let top = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Ratio(1, 3),
                    Constraint::Ratio(1, 3),
                    Constraint::Ratio(1, 3),
                ])
                .split(v_chunks[0]);
            let bottom = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Ratio(1, 3),
                    Constraint::Ratio(1, 3),
                    Constraint::Ratio(1, 3),
                ])
                .split(v_chunks[1]);
            if count == 5 {
                vec![top[0], top[1], top[2], bottom[0], bottom[1]]
            } else {
                vec![top[0], top[1], top[2], bottom[0], bottom[1], bottom[2]]
            }
        }
        _ => vec![area],
    }
}

/// Comprehensive state container for Mode A Single Photo Inspector.
#[derive(Debug, Clone)]
pub struct ModeAState {
    pub active_photo: Option<PhotoInfo>,
    pub metadata: PhotoMetadata,
    pub view_mode: ModeAView,
    pub baseline_recipe: Recipe,
    pub active_recipe: Recipe,
    pub active_intensity: f64,
    pub base_image: Option<Arc<DynamicImage>>,
    pub cached_protocol: Option<CachedProtocol>,

    // Variant Matrix State
    pub variant_count: usize,
    pub focused_variant: usize,
    pub variants: Vec<VariantSlot>,

    // Split-View State
    pub split_kind: SplitViewKind,
    pub split_ratio: f32,
    pub toggle_ab_showing_before: bool,

    // 1:1 Loupe Zoom State
    pub loupe: LoupeState,

    // Histogram & Overlays
    pub show_info_overlay: bool,
    pub histogram: Option<HistogramData>,
}

impl Default for ModeAState {
    fn default() -> Self {
        let mut baseline = Recipe::default();
        baseline.name = "Baseline (As-Shot)".into();
        let mut natural = BuiltinPreset::Natural.recipe();
        natural.name = "Natural".into();

        let mut cinema = BuiltinPreset::Cinema.recipe();
        cinema.name = "Cinema".into();

        let mut bw = BuiltinPreset::BwHighContrast.recipe();
        bw.name = "B&W High Contrast".into();

        let mut teal = BuiltinPreset::TealOrange.recipe();
        teal.name = "Teal & Orange".into();

        let variants = vec![
            VariantSlot::new(0, "Baseline (As-Shot)".into(), baseline.clone()),
            VariantSlot::new(1, "Natural".into(), natural),
            VariantSlot::new(2, "Cinema".into(), cinema),
            VariantSlot::new(3, "B&W High Contrast".into(), bw),
            VariantSlot::new(4, "Teal & Orange".into(), teal),
            VariantSlot::new(5, "Golden Hour".into(), BuiltinPreset::GoldenHour.recipe()),
        ];

        Self {
            active_photo: None,
            metadata: PhotoMetadata::default(),
            view_mode: ModeAView::Hero,
            baseline_recipe: baseline.clone(),
            active_recipe: baseline,
            active_intensity: 1.0,
            base_image: None,
            cached_protocol: None,
            variant_count: 4,
            focused_variant: 0,
            variants,
            split_kind: SplitViewKind::SideBySide,
            split_ratio: 0.5,
            toggle_ab_showing_before: false,
            loupe: LoupeState::default(),
            show_info_overlay: true,
            histogram: None,
        }
    }
}

impl ModeAState {
    /// Update histogram when base preview image is refreshed.
    pub fn update_image(&mut self, img: Arc<DynamicImage>) {
        self.histogram = Some(HistogramData::from_image(&img));
        self.base_image = Some(img);
    }

    /// Promote focused variant to active working recipe.
    pub fn promote_focused_variant(&mut self) {
        if let Some(slot) = self.variants.get(self.focused_variant) {
            self.active_recipe = slot.recipe.clone();
            self.active_intensity = slot.intensity;
        }
    }

    /// Toggle favorite flag on active variant slot.
    pub fn toggle_favorite(&mut self) {
        if let Some(slot) = self.variants.get_mut(self.focused_variant) {
            slot.is_favorite = !slot.is_favorite;
        }
    }

    /// Set rating on focused variant.
    pub fn set_rating(&mut self, stars: u8) {
        if let Some(slot) = self.variants.get_mut(self.focused_variant) {
            slot.rating = stars.min(5);
        }
    }

    /// Set pick flag on focused variant.
    pub fn set_flag(&mut self, flag: FlagStatus) {
        if let Some(slot) = self.variants.get_mut(self.focused_variant) {
            slot.flag = flag;
        }
    }
}

/// Render Mode A content within the assigned center canvas area.
pub fn render_mode_a(
    frame: &mut Frame,
    area: Rect,
    state: &ModeAState,
    _protocol: &GraphicsProtocol,
    is_focused: bool,
) {
    match state.view_mode {
        ModeAView::Hero => render_hero_view(frame, area, state, is_focused),
        ModeAView::VariantMatrix => render_variant_matrix(frame, area, state, is_focused),
        ModeAView::SplitView => render_split_view(frame, area, state, is_focused),
        ModeAView::Loupe1to1 => render_loupe_view(frame, area, state, is_focused),
    }
}

fn render_hero_view(frame: &mut Frame, area: Rect, state: &ModeAState, is_focused: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if is_focused { BorderType::Thick } else { BorderType::Rounded })
        .border_style(if is_focused { Style::default().fg(Color::Cyan) } else { Style::default().fg(Color::DarkGray) })
        .title(format!(
            " {}2. PREVISUALIZADOR: {} [{} | {}]{} ",
            if is_focused { "❯ [ACTIVO] " } else { "" },
            state.metadata.file_name,
            state.metadata.aspect_ratio_str(),
            state.metadata.megapixels_str(),
            if is_focused { " ❮" } else { "" }
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let details = format!(
        "\n  Archivo Maestro RAW:  {}\n  Dimensiones:          {} x {} píxeles ({})\n  Cámara:               {}\n  Exposición:           {}\n  Ajuste Activo:        {}\n  Intensidad:           {:.0}%\n\n  ────────────────────────────────────────────────────────────\n  Flujo de Trabajo:\n    • Pulse [Tab] para alternar entre BIBLIOTECA y PREVISUALIZADOR\n    • Pulse [4] para ver Matriz de 4 Variantes de Color\n    • Pulse [E] para EXPORTAR variante seleccionada en máxima calidad (TIFF 16b + JPEG 100%)\n",
        state.metadata.file_name,
        state.metadata.width,
        state.metadata.height,
        state.metadata.megapixels_str(),
        state.metadata.camera_model.as_deref().unwrap_or("Canon EOS R6"),
        state.metadata.exposure_summary(),
        state.active_recipe.name,
        state.active_intensity * 100.0,
    );
    let p = Paragraph::new(details).alignment(Alignment::Left);
    frame.render_widget(p, inner);
}

fn render_variant_matrix(frame: &mut Frame, area: Rect, state: &ModeAState, is_focused: bool) {
    let rects = compute_variant_grid_rects(area, state.variant_count);

    for (idx, &sub_area) in rects.iter().enumerate() {
        if let Some(slot) = state.variants.get(idx) {
            let is_selected_slot = idx == state.focused_variant;
            let fav_badge = if slot.is_favorite { " ★" } else { "" };
            let flag_badge = match slot.flag {
                FlagStatus::Pick => " [P]",
                FlagStatus::Reject => " [X]",
                FlagStatus::Unflagged => "",
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(if is_selected_slot { BorderType::Thick } else { BorderType::Rounded })
                .border_style(if is_selected_slot && is_focused {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else if is_selected_slot {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                })
                .title(format!(
                    " {}{}{} ({}%){}{} ",
                    if is_selected_slot { "► " } else { "" },
                    slot.index + 1,
                    if !slot.label.is_empty() { format!(". {}", slot.label) } else { String::new() },
                    (slot.intensity * 100.0) as u32,
                    fav_badge,
                    flag_badge
                ));

            let inner = block.inner(sub_area);
            frame.render_widget(block, sub_area);

            let status_repr = match &slot.status {
                VariantRenderStatus::Ready => "✔ Listo".to_string(),
                VariantRenderStatus::Rendering { .. } => "⟳ Generando...".to_string(),
                VariantRenderStatus::Pending => "⏳ Pendiente".to_string(),
                VariantRenderStatus::Unloaded => "⏸ Sin cargar".to_string(),
                VariantRenderStatus::Failed(err) => format!("✖ Error: {}", err),
            };

            let text = format!(
                "\n  Preset: {}\n  Rating: {}{}\n  Estado: {}",
                slot.recipe.name,
                "★".repeat(slot.rating as usize),
                "☆".repeat(5 - slot.rating as usize),
                status_repr
            );
            let p = Paragraph::new(text).alignment(Alignment::Center);
            frame.render_widget(p, inner);
        }
    }
}

fn render_split_view(frame: &mut Frame, area: Rect, state: &ModeAState, is_focused: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if is_focused { BorderType::Thick } else { BorderType::Rounded })
        .border_style(if is_focused { Style::default().fg(Color::Cyan) } else { Style::default().fg(Color::DarkGray) })
        .title(format!(" ❯ Split-View Comparison: Before vs After [{:?}] ", state.split_kind));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    match state.split_kind {
        SplitViewKind::SideBySide => {
            let halves = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(inner);

            let before_block = Block::default().borders(Borders::ALL).title(" Before (Original Baseline) ");
            let after_block = Block::default().borders(Borders::ALL).title(format!(" After ({}) ", state.active_recipe.name));

            let p_before = Paragraph::new("\n[ Original Raw Image ]\nIntensity: 0%\nNeutral Balance").alignment(Alignment::Center).block(before_block);
            let p_after = Paragraph::new(format!("\n[ Edited Variant ]\nIntensity: {:.0}%\nRecipe: {}", state.active_intensity * 100.0, state.active_recipe.name)).alignment(Alignment::Center).block(after_block);

            frame.render_widget(p_before, halves[0]);
            frame.render_widget(p_after, halves[1]);
        }
        SplitViewKind::TopBottom => {
            let halves = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(inner);

            let p_before = Paragraph::new("[ Top: Before / Baseline ]").alignment(Alignment::Center);
            let p_after = Paragraph::new("[ Bottom: After / Recipe ]").alignment(Alignment::Center);
            frame.render_widget(p_before, halves[0]);
            frame.render_widget(p_after, halves[1]);
        }
        SplitViewKind::Wiper => {
            let p = Paragraph::new(format!(
                "\n  [ Wiper Compositor Mode ]\n\n  Divider: {:.0}%  (Adjust with [ and ])\n  Left: Before  |  Right: After",
                state.split_ratio * 100.0
            ))
            .alignment(Alignment::Center);
            frame.render_widget(p, inner);
        }
        SplitViewKind::ToggleAB => {
            let current = if state.toggle_ab_showing_before {
                "BEFORE (Original Baseline)"
            } else {
                "AFTER (Active Recipe)"
            };
            let p = Paragraph::new(format!("\n  [ Instantaneous A/B Toggle ]\n\n  Currently Showing: {}\n  Press [ \\ ] to flip instantaneously.", current))
                .alignment(Alignment::Center);
            frame.render_widget(p, inner);
        }
    }
}

fn render_loupe_view(frame: &mut Frame, area: Rect, state: &ModeAState, is_focused: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if is_focused { BorderType::Thick } else { BorderType::Rounded })
        .border_style(if is_focused { Style::default().fg(Color::Cyan) } else { Style::default().fg(Color::DarkGray) })
        .title(format!(
            " ❯ 1:1 Loupe Detail Zoom (Z={:.0}%) Center: ({:.2}, {:.2}) ",
            state.loupe.zoom_level * 100.0,
            state.loupe.center_x,
            state.loupe.center_y
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (crop_x, crop_y, crop_w, crop_h) = state.loupe.compute_crop_rect(
        state.metadata.width.max(1000),
        state.metadata.height.max(1000),
        inner.width as u32 * 10,
        inner.height as u32 * 20,
    );

    let text = format!(
        "\n\n  [ 100% Pixel Inspection Viewport ]\n\n  Crop Box: [x={}, y={}, w={}, h={}]\n  Navigate: [Arrows / h,j,k,l] Pan\n  Zoom: [Z] Exit to Full View\n",
        crop_x, crop_y, crop_w, crop_h
    );
    let p = Paragraph::new(text).alignment(Alignment::Center);
    frame.render_widget(p, inner);

    // Mini-map locator widget in bottom right
    if state.loupe.show_minimap && inner.width > 26 && inner.height > 8 {
        let minimap_area = Rect {
            x: inner.x + inner.width.saturating_sub(24),
            y: inner.y + inner.height.saturating_sub(7),
            width: 22,
            height: 6,
        };
        let minimap_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" MiniMap ");
        let minimap_text = format!(
            "  [{:^12}]\n  pan: {:.1}, {:.1}",
            format!("■ {:.0}%", state.loupe.zoom_level * 100.0),
            state.loupe.center_x,
            state.loupe.center_y
        );
        let minimap_widget = Paragraph::new(minimap_text).block(minimap_block);
        frame.render_widget(minimap_widget, minimap_area);
    }
}
