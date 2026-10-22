//! Mode A: Single Photo Inspector, Variant Grid, Split-View Comparison, and 1:1 Loupe Zoom.

use image::{DynamicImage, GenericImage, GenericImageView, Rgba};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};
use ratatui::Frame;
use std::sync::Arc;
use std::time::Instant;

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
    pub is_selected: bool,
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
            is_selected: false,
            is_favorite: false,
            rating: 0,
            flag: FlagStatus::Unflagged,
            label_color: ColorLabel::None,
        }
    }
}

pub use crate::storage::metadata::PhotoMetadata;

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

    /// Produce 4-line sparkline text for terminal display using specified glyph set.
    pub fn sparklines_with_glyphs(&self, glyphs: &crate::tui::glyphs::GlyphSet) -> Vec<String> {
        let to_spark = |bins: &[u32; 32]| -> String {
            bins.iter()
                .map(|&count| {
                    if self.max_count == 0 || count == 0 {
                        ' '
                    } else {
                        let idx = ((count as f32 / self.max_count as f32) * 7.0).round() as usize;
                        glyphs.histogram_bar(idx)
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

    /// Produce 4-line sparkline text for terminal display.
    pub fn sparklines(&self) -> Vec<String> {
        self.sparklines_with_glyphs(&crate::tui::glyphs::GlyphSet::Unicode)
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
    let (w, h) = (
        before.width().min(after.width()),
        before.height().min(after.height()),
    );
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

    /// Toggle multi-selection on active variant slot.
    pub fn toggle_selection(&mut self) {
        if let Some(slot) = self.variants.get_mut(self.focused_variant) {
            slot.is_selected = !slot.is_selected;
        }
    }

    /// Return indices of all selected variants, or fallback to the focused variant if none is selected.
    pub fn get_selected_or_focused_indices(&self) -> Vec<usize> {
        let selected: Vec<usize> = self
            .variants
            .iter()
            .take(self.variant_count)
            .filter(|s| s.is_selected)
            .map(|s| s.index)
            .collect();
        if selected.is_empty() {
            vec![self.focused_variant]
        } else {
            selected
        }
    }

    /// Clear all variant multi-selections.
    pub fn clear_selection(&mut self) {
        for slot in &mut self.variants {
            slot.is_selected = false;
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
    glyphs: &crate::tui::glyphs::GlyphSet,
) {
    match state.view_mode {
        ModeAView::Hero => render_hero_view(frame, area, state, is_focused, glyphs),
        ModeAView::VariantMatrix => render_variant_matrix(frame, area, state, is_focused, glyphs),
        ModeAView::SplitView => render_split_view(frame, area, state, is_focused, glyphs),
    }
}

fn render_hero_view(
    frame: &mut Frame,
    area: Rect,
    state: &ModeAState,
    is_focused: bool,
    glyphs: &crate::tui::glyphs::GlyphSet,
) {
    let active_prefix = if is_focused {
        format!("{} [ACTIVO] ", glyphs.step_arrow())
    } else {
        String::new()
    };
    let active_suffix = if is_focused {
        format!(" {}", glyphs.step_left())
    } else {
        String::new()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if is_focused {
            BorderType::Double
        } else {
            BorderType::Rounded
        })
        .border_style(if is_focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        })
        .title(format!(
            " {}{}[2] Visor: {} [{} | {}]{} ",
            active_prefix,
            "",
            state.metadata.file_name,
            state.metadata.aspect_ratio_str(),
            state.metadata.megapixels_str(),
            active_suffix,
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let details = format!(
        "\n  Archivo Maestro RAW:  {}\n  Dimensiones:          {} x {} píxeles ({})\n  Cámara:               {}\n  Exposición:           {}\n  Ajuste Activo:        {}\n  Intensidad:           {:.0}%\n\n  {}\n  Flujo de Trabajo:\n    {} Pulse [Tab] para alternar entre BIBLIOTECA y VISOR\n    {} Pulse [4] para ver Matriz de 4 Variantes de Color\n    {} Pulse [E] para EXPORTAR variante seleccionada en máxima calidad\n    {} Pulse [F1] para ver la Guía Completa de Atajos\n",
        state.metadata.file_name,
        state.metadata.width,
        state.metadata.height,
        state.metadata.megapixels_str(),
        state.metadata.camera_model.as_deref().unwrap_or("Canon EOS R6"),
        state.metadata.exposure_summary(),
        state.active_recipe.name,
        state.active_intensity * 100.0,
        glyphs.bar_horizontal().repeat(58),
        glyphs.bullet(),
        glyphs.bullet(),
        glyphs.bullet(),
        glyphs.bullet(),
    );
    let p = Paragraph::new(details)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
    frame.render_widget(p, inner);
}

fn render_variant_matrix(
    frame: &mut Frame,
    area: Rect,
    state: &ModeAState,
    is_focused: bool,
    glyphs: &crate::tui::glyphs::GlyphSet,
) {
    let rects = compute_variant_grid_rects(area, state.variant_count);

    for (idx, &sub_area) in rects.iter().enumerate() {
        if let Some(slot) = state.variants.get(idx) {
            let is_focused_slot = idx == state.focused_variant;
            let fav_badge = if slot.is_favorite {
                format!(" {}", glyphs.favorite_star())
            } else {
                String::new()
            };
            let sel_badge = if slot.is_selected {
                format!("{} ", glyphs.check_box_checked())
            } else {
                String::new()
            };
            let flag_badge = match slot.flag {
                FlagStatus::Pick => " [P]",
                FlagStatus::Reject => " [REJ]",
                FlagStatus::Unflagged => "",
            };

            // Color sequence logic:
            // 1. If slot.is_selected: White
            // 2. If Rendering: Red (en proceso)
            // 3. If Ready: Green (fin completado, persiste)
            // 4. If focused slot: Orange (en selección)
            // 5. Otherwise: DarkGray
            let border_color = if slot.is_selected {
                Color::White
            } else {
                match &slot.status {
                    VariantRenderStatus::Rendering { .. } => Color::Red,
                    VariantRenderStatus::Ready => Color::Green,
                    VariantRenderStatus::Failed(_) => Color::Magenta,
                    VariantRenderStatus::Pending | VariantRenderStatus::Unloaded => {
                        if is_focused_slot {
                            Color::Rgb(255, 165, 0)
                        } else {
                            Color::DarkGray
                        }
                    }
                }
            };

            // Padding / separación entre cajas de variantes para mejorar legibilidad visual
            let box_area = if sub_area.width >= 16 && sub_area.height >= 6 {
                let v_pad = if sub_area.height >= 10 { 1 } else { 0 };
                Rect {
                    x: sub_area.x.saturating_add(1),
                    y: sub_area.y.saturating_add(v_pad),
                    width: sub_area.width.saturating_sub(2),
                    height: sub_area.height.saturating_sub(v_pad * 2),
                }
            } else {
                sub_area
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(if slot.is_selected || (is_focused_slot && is_focused) {
                    BorderType::Double
                } else {
                    BorderType::Rounded
                })
                .border_style(if is_focused_slot && is_focused {
                    Style::default()
                        .fg(border_color)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(border_color)
                })
                .title(format!(
                    " {}{}{}{} ({}%){}{} ",
                    if is_focused_slot {
                        format!("{} ", glyphs.triangle_right())
                    } else {
                        String::new()
                    },
                    sel_badge,
                    slot.index + 1,
                    if !slot.label.is_empty() {
                        format!(". {}", slot.label)
                    } else {
                        String::new()
                    },
                    (slot.intensity * 100.0) as u32,
                    fav_badge,
                    flag_badge
                ));

            let inner = block.inner(box_area);
            frame.render_widget(block, box_area);

            let status_repr = match &slot.status {
                VariantRenderStatus::Ready => format!("{} Listo", glyphs.ready_mark()),
                VariantRenderStatus::Rendering { .. } => {
                    format!("{} Generando...", glyphs.progress_spinner())
                }
                VariantRenderStatus::Pending => format!("{} Pendiente", glyphs.pending()),
                VariantRenderStatus::Unloaded => format!("{} Sin cargar", glyphs.paused()),
                VariantRenderStatus::Failed(err) => {
                    format!("{} Error: {}", glyphs.error_mark(), err)
                }
            };

            let text = format!(
                "\n  Preset: {}\n  Rating: {}{}\n  Estado: {}",
                slot.recipe.name,
                glyphs.favorite_star().repeat(slot.rating as usize),
                glyphs.star_empty().repeat(5 - slot.rating as usize),
                status_repr
            );
            let p = Paragraph::new(text)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            frame.render_widget(p, inner);
        }
    }
}

fn render_split_view(
    frame: &mut Frame,
    area: Rect,
    state: &ModeAState,
    is_focused: bool,
    glyphs: &crate::tui::glyphs::GlyphSet,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if is_focused {
            BorderType::Double
        } else {
            BorderType::Rounded
        })
        .border_style(if is_focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        })
        .title(format!(
            " {} [2] Visor: Split Antes / Después [{:?}] {} ",
            glyphs.step_arrow(),
            state.split_kind,
            glyphs.step_left(),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    match state.split_kind {
        SplitViewKind::SideBySide => {
            let halves = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(inner);

            let before_block = Block::default()
                .borders(Borders::ALL)
                .title(" Before (Original Baseline) ");
            let after_block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" After ({}) ", state.active_recipe.name));

            let p_before =
                Paragraph::new("\n[ Original Raw Image ]\nIntensity: 0%\nNeutral Balance")
                    .alignment(Alignment::Center)
                    .block(before_block);
            let p_after = Paragraph::new(format!(
                "\n[ Edited Variant ]\nIntensity: {:.0}%\nRecipe: {}",
                state.active_intensity * 100.0,
                state.active_recipe.name
            ))
            .alignment(Alignment::Center)
            .block(after_block);

            frame.render_widget(p_before, halves[0]);
            frame.render_widget(p_after, halves[1]);
        }
        SplitViewKind::TopBottom => {
            let halves = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(inner);

            let p_before =
                Paragraph::new("[ Top: Before / Baseline ]").alignment(Alignment::Center);
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
