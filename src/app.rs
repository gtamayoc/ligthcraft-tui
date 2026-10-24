//! Central application controller, state coordinator, and 60 FPS non-blocking render loop.

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use image::DynamicImage;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap};
use ratatui::Frame;
use std::path::PathBuf;
use std::sync::Arc;

use crate::engine::presets::BuiltinPreset;
use crate::engine::recipe::Recipe;
use crate::error::AppError;
use crate::export::aspect::{AspectRatio, InteractiveFlowState, JobPlan, WizardStep};
use crate::graphics::cache::{PreviewCache, PreviewCacheConfig};
use crate::graphics::protocol::{GraphicsProtocol, ProtocolOverride};
use crate::mcp::client::McpClient;
use crate::storage::metadata::read_native_metadata;
use crate::storage::presets_store::PresetStore;
use crate::tui::event::{AppEvent, EventManager};
use crate::tui::glyphs::GlyphSet;
use crate::tui::layout::{AppLayout, PaneId};
use crate::tui::mode_a::{render_mode_a, FlagStatus, ModeAState, ModeAView};
use crate::tui::mode_b::{render_mode_b, ModeBState};
use crate::tui::terminal::TerminalGuard;
use tokio_util::sync::CancellationToken;

/// Dual operating modes (Requirement F-TUI-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    /// Mode A: Single Photo inspection, variant grid, split view before/after.
    SinglePhoto,
    /// Mode B: Batch Queue for uniform or multi-recipe directory processing.
    BatchQueue,
}

/// Photographic item in the library explorer pane.
#[derive(Debug, Clone)]
pub struct LibraryItem {
    pub path: PathBuf,
    pub filename: String,
    pub size_bytes: u64,
    pub is_cr3: bool,
    pub is_selected: bool,
    pub rating: u8,
    pub is_picked: bool,
    pub is_rejected: bool,
}

/// Central application controller owning all UI and domain subsystem states.
pub struct AppController {
    pub mode: AppMode,
    pub focused_pane: PaneId,
    pub items: Vec<LibraryItem>,
    pub selected_index: usize,
    pub active_recipe: Recipe,
    pub mode_a: ModeAState,
    pub mode_b: ModeBState,
    pub graphics: GraphicsProtocol,
    pub cache: PreviewCache,
    pub mcp_client: Arc<McpClient>,
    pub preset_store: PresetStore,
    pub status_message: String,
    pub terminal_size: Rect,
    pub needs_render: bool,
    pub running: bool,
    pub force_clear: bool,
    pub export_prompt: bool,
    pub flow_state: InteractiveFlowState,
    pub event_sender: Option<tokio::sync::mpsc::Sender<AppEvent>>,
    pub flow_cancel_token: Option<CancellationToken>,
    pub glyph_set: GlyphSet,
    pub show_help_modal: bool,
    pub help_active_tab: usize,
}

impl AppController {
    /// Initialize application state, discover local photos in draw_dir, and prepare TUI components.
    pub async fn new(
        mcp_client: Arc<McpClient>,
        preset_store: PresetStore,
        draw_dir: PathBuf,
        graphics_override: ProtocolOverride,
    ) -> Result<Self, AppError> {
        let mut items = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&draw_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(ext) = p.extension() {
                    let ext_str = ext.to_string_lossy().to_lowercase();
                    if ["cr3", "jpg", "jpeg", "png", "tiff", "tif"].contains(&ext_str.as_str()) {
                        let meta = entry.metadata().ok();
                        let size = meta.map(|m| m.len()).unwrap_or(0);
                        items.push(LibraryItem {
                            filename: p
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                            is_cr3: ext_str == "cr3",
                            size_bytes: size,
                            path: p,
                            is_selected: false,
                            rating: 0,
                            is_picked: false,
                            is_rejected: false,
                        });
                    }
                }
            }
        }

        items.sort_by(|a, b| a.filename.cmp(&b.filename));

        let graphics = GraphicsProtocol::new(graphics_override);
        let cache = PreviewCache::new(PreviewCacheConfig::default());
        let mut mode_a = ModeAState::default();

        if let Some(first) = items.first_mut() {
            first.is_selected = true;
            mode_a.metadata = read_native_metadata(&first.path);
            mode_a.active_photo = Some(crate::mcp::tools::PhotoInfo {
                id: 1,
                file_name: first.filename.clone(),
                format: if first.is_cr3 {
                    "cr3".into()
                } else {
                    "image".into()
                },
                kind: if first.is_cr3 {
                    "raw".into()
                } else {
                    "raster".into()
                },
                width: mode_a.metadata.width,
                height: mode_a.metadata.height,
                rating: 0,
                flag: "none".into(),
                camera: mode_a.metadata.camera_model.clone(),
                captured: mode_a.metadata.capture_time.clone(),
                edited: false,
                copy_name: None,
                copy_of: None,
                deleted: false,
                keywords: Vec::new(),
                label: None,
                preview_only: None,
                title: String::new(),
            });
        }

        let mode_b = ModeBState::default();

        Ok(Self {
            mode: AppMode::SinglePhoto,
            focused_pane: PaneId::Library,
            items,
            selected_index: 0,
            active_recipe: BuiltinPreset::Natural.recipe(),
            mode_a,
            mode_b,
            graphics,
            cache,
            mcp_client,
            preset_store,
            status_message: "Listo. Navegue por fotos o pulse [F1] para ver atajos".into(),
            terminal_size: Rect::new(0, 0, 120, 30),
            needs_render: true,
            running: true,
            force_clear: false,
            export_prompt: false,
            flow_state: InteractiveFlowState::default(),
            event_sender: None,
            flow_cancel_token: None,
            glyph_set: GlyphSet::detect(),
            show_help_modal: false,
            help_active_tab: 0,
        })
    }

    /// Primary event loop running at 60 FPS non-blocking cadence (Requirement F-RNF-01).
    pub async fn run(&mut self, mut terminal: TerminalGuard) -> Result<(), AppError> {
        let mut event_manager = EventManager::new(16);
        self.event_sender = Some(event_manager.sender());

        // Kick off variant preview generation for initially selected photo
        self.trigger_variant_previews();

        while self.running {
            // Continuously detect terminal window resize (e.g. OS maximize or manual resize)
            if let Ok(size) = terminal.size() {
                let current_rect = Rect::new(0, 0, size.width, size.height);
                if current_rect != self.terminal_size {
                    self.terminal_size = current_rect;
                    self.force_clear = true;
                    self.needs_render = true;
                }
            }

            if self.needs_render {
                if self.force_clear {
                    let _ = terminal.clear();
                    self.force_clear = false;
                }
                terminal.draw(|frame| self.render(frame))?;
                self.needs_render = false;
            }

            if let Some(event) = event_manager.next().await {
                self.handle_event(event).await?;
            } else {
                break;
            }
        }

        event_manager.shutdown();
        terminal.restore()?;
        Ok(())
    }

    /// Spawns asynchronous worker tasks to generate preview thumbnails for all variant slots.
    pub fn trigger_variant_previews(&mut self) {
        let Some(item) = self.items.get(self.selected_index) else {
            return;
        };
        let source_path = item.path.clone();
        let photo_id = item.filename.clone();
        let count = self.mode_a.variant_count;

        for (idx, slot) in self.mode_a.variants.iter_mut().enumerate().take(count) {
            slot.status = crate::tui::mode_a::VariantRenderStatus::Rendering {
                started_at: std::time::Instant::now(),
            };

            let Some(ref tx) = self.event_sender else {
                continue;
            };
            let tx_clone = tx.clone();
            let source_clone = source_path.clone();
            let photo_id_clone = photo_id.clone();
            let recipe = slot.recipe.clone();

            tokio::spawn(async move {
                // Timeout watchdog: enforce max 10s per variant to avoid infinite hang
                let render_fut =
                    tokio::task::spawn_blocking(move || -> Result<DynamicImage, String> {
                        let dyn_img = crate::export::ExportPipeline::load_source_image(
                            &source_clone,
                            Some(800),
                        )
                        .map_err(|e| e.to_string())?;
                        let mut rgb = dyn_img.to_rgb8();
                        crate::export::apply_recipe_to_rgb8(&mut rgb, &recipe);
                        Ok(DynamicImage::ImageRgb8(rgb))
                    });

                match tokio::time::timeout(std::time::Duration::from_secs(10), render_fut).await {
                    Ok(Ok(Ok(rendered))) => {
                        let _ = tx_clone
                            .send(AppEvent::VariantReady {
                                photo_id: photo_id_clone,
                                slot_index: idx,
                                image: Arc::new(rendered),
                            })
                            .await;
                    }
                    Ok(Ok(Err(err))) => {
                        let _ = tx_clone
                            .send(AppEvent::VariantFailed {
                                photo_id: photo_id_clone,
                                slot_index: idx,
                                error: err,
                            })
                            .await;
                    }
                    Ok(Err(panic_err)) => {
                        let _ = tx_clone
                            .send(AppEvent::VariantFailed {
                                photo_id: photo_id_clone,
                                slot_index: idx,
                                error: format!("Thread error: {}", panic_err),
                            })
                            .await;
                    }
                    Err(_) => {
                        let _ = tx_clone
                            .send(AppEvent::VariantFailed {
                                photo_id: photo_id_clone,
                                slot_index: idx,
                                error: "Tiempo de espera agotado (>10s)".into(),
                            })
                            .await;
                    }
                }
            });
        }
    }

    /// Dispatch event to handlers and mark dirty if state changes.
    pub async fn handle_event(&mut self, event: AppEvent) -> Result<(), AppError> {
        match event {
            AppEvent::Key(key) => {
                self.handle_key(key).await?;
                self.needs_render = true;
            }
            AppEvent::Mouse(mouse) => {
                self.handle_mouse(mouse);
                self.needs_render = true;
            }
            AppEvent::Resize(w, h) => {
                self.terminal_size = Rect::new(0, 0, w, h);
                self.force_clear = true;
                self.needs_render = true;
            }
            AppEvent::Tick => {
                if self.is_animating() || self.flow_state.step == WizardStep::Exporting {
                    self.needs_render = true;
                }
            }
            AppEvent::FlowExportProgress {
                completed,
                total,
                current_label,
                percent,
            } => {
                if self.flow_state.is_active {
                    self.flow_state.set_export_progress(
                        completed,
                        total,
                        current_label.clone(),
                        percent,
                    );
                    if !self.flow_state.is_confirming_cancel {
                        self.status_message = format!(
                            "Renderizado en proceso: {}/{} ({:.0}%) - {}",
                            completed, total, percent, current_label
                        );
                    }
                    self.needs_render = true;
                }
            }
            AppEvent::FlowExportFinished {
                successful,
                total,
                output_dir,
            } => {
                self.flow_cancel_token = None;
                self.flow_state.close();
                self.mode_a.clear_selection();
                self.status_message = format!(
                    "Exportación completada con éxito. Guardado OK: {}/{} archivo(s) en {}",
                    successful, total, output_dir
                );
                self.needs_render = true;
            }
            AppEvent::FlowExportCancelled {
                processed,
                total,
                cancelled,
            } => {
                self.flow_cancel_token = None;
                self.flow_state.close();
                self.mode_a.clear_selection();
                self.status_message = format!(
                    "Exportación cancelada por el usuario. {} de {} fotos guardadas, {} omitidas.",
                    processed, total, cancelled
                );
                self.needs_render = true;
            }
            AppEvent::PreviewReady {
                photo_id, image, ..
            } => {
                if let Some(item) = self.items.get(self.selected_index) {
                    if item.filename == photo_id {
                        self.mode_a.update_image(image);
                    }
                }
                self.needs_render = true;
            }
            AppEvent::VariantReady {
                photo_id,
                slot_index,
                image,
            } => {
                if let Some(item) = self.items.get(self.selected_index) {
                    if item.filename == photo_id {
                        if let Some(slot) = self.mode_a.variants.get_mut(slot_index) {
                            slot.image = Some(image);
                            slot.status = crate::tui::mode_a::VariantRenderStatus::Ready;
                        }
                    }
                }
                self.needs_render = true;
            }
            AppEvent::VariantFailed {
                photo_id,
                slot_index,
                error,
            } => {
                if let Some(item) = self.items.get(self.selected_index) {
                    if item.filename == photo_id {
                        if let Some(slot) = self.mode_a.variants.get_mut(slot_index) {
                            slot.status = crate::tui::mode_a::VariantRenderStatus::Failed(error);
                        }
                    }
                }
                self.needs_render = true;
            }
            AppEvent::BatchProgress {
                completed,
                total,
                current_file,
                percent,
            } => {
                self.mode_b.completed_count = completed;
                self.mode_b.total_jobs = total;
                if let Some(ref mut queue_item) = self
                    .mode_b
                    .queue
                    .iter_mut()
                    .find(|j| j.status == crate::tui::mode_b::BatchJobStatus::Queued)
                {
                    queue_item.status = crate::tui::mode_b::BatchJobStatus::Processing {
                        progress_pct: percent as u8,
                    };
                }
                self.status_message = format!(
                    "Batch: {} ({:.1}%) - [{}/{}]",
                    current_file, percent, completed, total
                );
                self.needs_render = true;
            }
            AppEvent::StatusMessage(msg) => {
                self.status_message = msg;
                self.needs_render = true;
            }
            AppEvent::Error(err) => {
                self.status_message = format!("Error: {}", err);
                self.needs_render = true;
            }
            AppEvent::Quit => {
                self.running = false;
            }
            _ => {}
        }
        Ok(())
    }

    /// Process keyboard inputs.
    pub async fn handle_key(&mut self, key: KeyEvent) -> Result<(), AppError> {
        // Discard key release events (prevent double triggers on Windows console)
        if key.kind == KeyEventKind::Release {
            return Ok(());
        }

        // F1 or ? toggles Help Modal
        if key.code == KeyCode::F(1) || key.code == KeyCode::Char('?') {
            self.show_help_modal = !self.show_help_modal;
            self.needs_render = true;
            return Ok(());
        }

        // If Help Modal is open, handle tabs and dismiss keys, blocking background events
        if self.show_help_modal {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                    self.show_help_modal = false;
                    self.needs_render = true;
                }
                KeyCode::Tab | KeyCode::Right => {
                    self.help_active_tab = (self.help_active_tab + 1) % 4;
                    self.needs_render = true;
                }
                KeyCode::BackTab | KeyCode::Left => {
                    self.help_active_tab = if self.help_active_tab == 0 {
                        3
                    } else {
                        self.help_active_tab - 1
                    };
                    self.needs_render = true;
                }
                KeyCode::Char('1') => {
                    self.help_active_tab = 0;
                    self.needs_render = true;
                }
                KeyCode::Char('2') => {
                    self.help_active_tab = 1;
                    self.needs_render = true;
                }
                KeyCode::Char('3') => {
                    self.help_active_tab = 2;
                    self.needs_render = true;
                }
                KeyCode::Char('4') => {
                    self.help_active_tab = 3;
                    self.needs_render = true;
                }
                _ => {}
            }
            return Ok(());
        }

        // Active interactive top wizard flow
        if self.flow_state.is_active {
            self.handle_flow_key(key).await?;
            return Ok(());
        }

        // Export format selection prompt active in status bar
        if self.export_prompt {
            match key.code {
                KeyCode::Esc => {
                    self.export_prompt = false;
                    self.status_message = "Exportación cancelada.".into();
                }
                KeyCode::Char('1') => {
                    self.export_prompt = false;
                    self.execute_variant_export_batch(1).await?;
                }
                KeyCode::Char('2') => {
                    self.export_prompt = false;
                    self.execute_variant_export_batch(2).await?;
                }
                KeyCode::Char('3') | KeyCode::Enter => {
                    self.export_prompt = false;
                    self.execute_variant_export_batch(3).await?;
                }
                _ => {}
            }
            return Ok(());
        }

        // Global hotkeys
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(ref token) = self.flow_cancel_token {
                    token.cancel();
                }
                self.running = false;
                return Ok(());
            }
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                if let Some(ref token) = self.flow_cancel_token {
                    token.cancel();
                }
                self.running = false;
                return Ok(());
            }
            KeyCode::F(10) | KeyCode::Char('m') | KeyCode::Char('M') => {
                // Seamless dual-mode switch with zero state loss
                self.mode = match self.mode {
                    AppMode::SinglePhoto => AppMode::BatchQueue,
                    AppMode::BatchQueue => AppMode::SinglePhoto,
                };
                self.status_message = format!("Switched to {:?}", self.mode);
                return Ok(());
            }
            KeyCode::F(5) => {
                self.force_clear = true;
                self.needs_render = true;
                self.status_message = "Vista y buffer del terminal restablecidos (F5)".into();
                return Ok(());
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.force_clear = true;
                self.needs_render = true;
                self.status_message = "Vista y buffer del terminal restablecidos (Ctrl+L)".into();
                return Ok(());
            }
            KeyCode::Char('r') | KeyCode::Char('R')
                if self.mode == AppMode::SinglePhoto && !self.export_prompt =>
            {
                self.force_clear = true;
                self.needs_render = true;
                self.status_message = "Vista y buffer del terminal restablecidos (R)".into();
                return Ok(());
            }
            KeyCode::Tab => {
                self.focused_pane = self.focused_pane.next();
                return Ok(());
            }
            KeyCode::BackTab => {
                self.focused_pane = self.focused_pane.prev();
                return Ok(());
            }
            _ => {}
        }

        // Mode-specific and pane-specific navigation
        match self.mode {
            AppMode::SinglePhoto => self.handle_mode_a_key(key).await?,
            AppMode::BatchQueue => self.handle_mode_b_key(key).await?,
        }

        Ok(())
    }

    async fn handle_mode_a_key(&mut self, key: KeyEvent) -> Result<(), AppError> {
        match key.code {
            // Photo library navigation: strictly isolated to Panel 1
            KeyCode::Up | KeyCode::Left | KeyCode::Char('k') | KeyCode::Char('h')
                if self.focused_pane == PaneId::Library =>
            {
                self.navigate_photo_prev().await?;
            }
            KeyCode::Down | KeyCode::Right | KeyCode::Char('j') | KeyCode::Char('l')
                if self.focused_pane == PaneId::Library =>
            {
                self.navigate_photo_next().await?;
            }
            KeyCode::Enter if self.focused_pane == PaneId::Library => {
                if let Some(cur) = self.items.get_mut(self.selected_index) {
                    cur.is_selected = true;
                }
                self.focused_pane = PaneId::PreviewCanvas;
                self.mode_a.view_mode = ModeAView::VariantMatrix;
                self.status_message = "Paso 1 confirmado. Ahora en Paso 2: Variantes. Pulsa [Space] para seleccionar variantes o [E] para continuar.".into();
            }
            KeyCode::Char(' ') if self.focused_pane == PaneId::Library => {
                if let Some(item) = self.items.get_mut(self.selected_index) {
                    item.is_selected = !item.is_selected;
                    let is_sel = item.is_selected;
                    self.status_message = format!(
                        "Foto {} {}",
                        item.filename,
                        if is_sel {
                            format!("seleccionada {}", self.glyph_set.check_box_checked())
                        } else {
                            format!("deseleccionada {}", self.glyph_set.check_box_unchecked())
                        }
                    );
                }
            }

            // Photo navigation from PreviewCanvas (Hero and SplitView only)
            KeyCode::Up | KeyCode::Left | KeyCode::Char('k') | KeyCode::Char('h')
                if self.focused_pane == PaneId::PreviewCanvas
                    && self.mode_a.view_mode != ModeAView::VariantMatrix =>
            {
                self.navigate_photo_prev().await?;
            }
            KeyCode::Down | KeyCode::Right | KeyCode::Char('j') | KeyCode::Char('l')
                if self.focused_pane == PaneId::PreviewCanvas
                    && self.mode_a.view_mode != ModeAView::VariantMatrix =>
            {
                self.navigate_photo_next().await?;
            }

            // View modes (Hero, 2x2 Matrix, Split-View, 1:1 Loupe)
            KeyCode::Char('1') if self.focused_pane == PaneId::PreviewCanvas => {
                self.mode_a.view_mode = ModeAView::Hero;
                self.status_message = "Mode A: Hero Preview View".into();
            }
            KeyCode::Char('2') if self.focused_pane == PaneId::PreviewCanvas => {
                self.mode_a.view_mode = ModeAView::VariantMatrix;
                self.mode_a.variant_count = 2;
                self.status_message = "Mode A: 2-Variant Matrix".into();
                self.trigger_variant_previews();
            }
            KeyCode::Char('4') | KeyCode::Char('v') | KeyCode::Char('V')
                if self.focused_pane == PaneId::PreviewCanvas =>
            {
                self.mode_a.view_mode = ModeAView::VariantMatrix;
                self.mode_a.variant_count = 4;
                self.status_message = "Mode A: 4-Way 2x2 Variant Matrix".into();
                self.trigger_variant_previews();
            }
            KeyCode::Char('6') if self.focused_pane == PaneId::PreviewCanvas => {
                self.mode_a.view_mode = ModeAView::VariantMatrix;
                self.mode_a.variant_count = 6;
                self.status_message = "Mode A: 6-Variant Matrix".into();
                self.trigger_variant_previews();
            }
            KeyCode::Char('s') | KeyCode::Char('S')
                if self.focused_pane == PaneId::PreviewCanvas =>
            {
                self.mode_a.view_mode = match self.mode_a.view_mode {
                    ModeAView::SplitView => ModeAView::Hero,
                    _ => ModeAView::SplitView,
                };
                self.status_message = format!("Mode A View: {:?}", self.mode_a.view_mode);
            }
            KeyCode::Char('\\') => {
                // Instantaneous A/B flip toggle
                self.mode_a.toggle_ab_showing_before = !self.mode_a.toggle_ab_showing_before;
                self.status_message = if self.mode_a.toggle_ab_showing_before {
                    "Split Toggle: BEFORE (Baseline)".into()
                } else {
                    "Split Toggle: AFTER (Recipe)".into()
                };
            }

            // Variant grid navigation (Horizontal & Vertical)
            KeyCode::Left | KeyCode::Char('h')
                if self.mode_a.view_mode == ModeAView::VariantMatrix
                    && self.focused_pane == PaneId::PreviewCanvas =>
            {
                if self.mode_a.focused_variant > 0 {
                    self.mode_a.focused_variant -= 1;
                } else if self.mode_a.variant_count > 0 {
                    self.mode_a.focused_variant = self.mode_a.variant_count - 1;
                }
            }
            KeyCode::Right | KeyCode::Char('l')
                if self.mode_a.view_mode == ModeAView::VariantMatrix
                    && self.focused_pane == PaneId::PreviewCanvas =>
            {
                if self.mode_a.focused_variant + 1 < self.mode_a.variant_count {
                    self.mode_a.focused_variant += 1;
                } else {
                    self.mode_a.focused_variant = 0;
                }
            }
            KeyCode::Up | KeyCode::Char('k')
                if self.mode_a.view_mode == ModeAView::VariantMatrix
                    && self.focused_pane == PaneId::PreviewCanvas =>
            {
                let cols = match self.mode_a.variant_count {
                    2 | 4 => 2,
                    _ => 3,
                };
                if self.mode_a.focused_variant >= cols {
                    self.mode_a.focused_variant -= cols;
                }
            }
            KeyCode::Down | KeyCode::Char('j')
                if self.mode_a.view_mode == ModeAView::VariantMatrix
                    && self.focused_pane == PaneId::PreviewCanvas =>
            {
                let cols = match self.mode_a.variant_count {
                    2 | 4 => 2,
                    _ => 3,
                };
                if self.mode_a.focused_variant + cols < self.mode_a.variant_count {
                    self.mode_a.focused_variant += cols;
                }
            }
            KeyCode::Enter
                if self.mode_a.view_mode == ModeAView::VariantMatrix
                    && self.focused_pane == PaneId::PreviewCanvas =>
            {
                self.mode_a.promote_focused_variant();
                self.status_message = format!(
                    "Promoted variant to active recipe: {}",
                    self.mode_a.active_recipe.name
                );
            }

            // Multi-selection (Space) & Favorite tagging (F) in PreviewCanvas
            KeyCode::Char(' ') if self.focused_pane == PaneId::PreviewCanvas => {
                if self.mode_a.view_mode == ModeAView::VariantMatrix {
                    self.mode_a.toggle_selection();
                    let is_sel = self
                        .mode_a
                        .variants
                        .get(self.mode_a.focused_variant)
                        .map(|s| s.is_selected)
                        .unwrap_or(false);
                    self.status_message = format!(
                        "Variante {} {}",
                        self.mode_a.focused_variant + 1,
                        if is_sel {
                            "marcada para lote [X]"
                        } else {
                            "desmarcada"
                        }
                    );
                } else {
                    self.mode_a.toggle_favorite();
                    self.status_message = "Toggled favorite on active variant".into();
                }
            }
            KeyCode::Char('f') | KeyCode::Char('F')
                if self.focused_pane == PaneId::PreviewCanvas =>
            {
                self.mode_a.toggle_favorite();
                self.status_message = format!(
                    "Toggled favorite ({}) on active variant",
                    self.glyph_set.favorite_star()
                );
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                self.mode_a.set_flag(FlagStatus::Pick);
                if let Some(item) = self.items.get_mut(self.selected_index) {
                    item.is_picked = true;
                    item.is_rejected = false;
                }
                self.status_message = "Flagged as PICK".into();
            }
            KeyCode::Char('x') | KeyCode::Char('X') => {
                self.mode_a.set_flag(FlagStatus::Reject);
                if let Some(item) = self.items.get_mut(self.selected_index) {
                    item.is_picked = false;
                    item.is_rejected = true;
                }
                self.status_message = "Flagged as REJECT".into();
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                self.mode_a.set_flag(FlagStatus::Unflagged);
                if let Some(item) = self.items.get_mut(self.selected_index) {
                    item.is_picked = false;
                    item.is_rejected = false;
                }
                self.status_message = "Cleared flag (UNFLAGGED)".into();
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                if self.focused_pane == PaneId::PreviewCanvas
                    && self.mode_a.view_mode == ModeAView::VariantMatrix
                {
                    let targets = self.mode_a.get_selected_or_focused_indices();
                    self.flow_state.start_with_variants(targets);
                    self.status_message =
                        "Asistente: Paso 3/5 - Selecciona proporciones de imagen en la cabecera superior".into();
                    self.needs_render = true;
                } else if let Some(item) = self.items.get(self.selected_index) {
                    let source_path = item.path.clone();
                    let file_name = item.filename.clone();
                    let (recipe, target_label) = if self.mode_a.view_mode
                        == ModeAView::VariantMatrix
                    {
                        if let Some(slot) = self.mode_a.variants.get(self.mode_a.focused_variant) {
                            (
                                slot.recipe.clone(),
                                format!("Variante {} ({})", slot.index + 1, slot.recipe.name),
                            )
                        } else {
                            (
                                self.mode_a.active_recipe.clone(),
                                self.mode_a.active_recipe.name.clone(),
                            )
                        }
                    } else {
                        (
                            self.mode_a.active_recipe.clone(),
                            self.mode_a.active_recipe.name.clone(),
                        )
                    };

                    let preset_name = recipe.name.clone();
                    let output_dir = PathBuf::from("export").join("interactive");

                    self.status_message = format!(
                        "Exportando {} [{}] en máxima calidad...",
                        file_name, target_label
                    );
                    self.needs_render = true;

                    let pipeline = crate::export::ExportPipeline::new();

                    // 1. Exportar versión maestra TIFF 16-bit archival
                    let tiff_job = crate::export::ExportJob {
                        job_id: 1,
                        source_path: source_path.clone(),
                        photo_id: None,
                        recipe_name: preset_name.clone(),
                        recipe: recipe.clone(),
                        profile: crate::export::ExportProfile {
                            name: format!("{}_TIFF16", preset_name),
                            format: crate::export::OutputFormat::Tiff,
                            quality: 100,
                            bit_depth: crate::export::BitDepth::Sixteen,
                            scaling: crate::export::ScalingProfile::Native,
                            ..Default::default()
                        },
                        output_dir: output_dir.clone(),
                        template: crate::export::NamingTemplate::parse("{name}__{preset}.{ext}"),
                        seq_number: 1,
                        collision_policy: crate::export::CollisionPolicy::Overwrite,
                    };

                    // 2. Exportar versión JPEG 100% de calidad
                    let jpeg_job = crate::export::ExportJob {
                        job_id: 2,
                        source_path: source_path.clone(),
                        photo_id: None,
                        recipe_name: preset_name.clone(),
                        recipe: recipe.clone(),
                        profile: crate::export::ExportProfile {
                            name: format!("{}_JPEG100", preset_name),
                            format: crate::export::OutputFormat::Jpeg,
                            quality: 100,
                            bit_depth: crate::export::BitDepth::Eight,
                            scaling: crate::export::ScalingProfile::Native,
                            ..Default::default()
                        },
                        output_dir: output_dir.clone(),
                        template: crate::export::NamingTemplate::parse("{name}__{preset}.{ext}"),
                        seq_number: 2,
                        collision_policy: crate::export::CollisionPolicy::Overwrite,
                    };

                    let res_tiff = pipeline.execute_export(None, &tiff_job).await;
                    let res_jpeg = pipeline.execute_export(None, &jpeg_job).await;

                    match (res_tiff, res_jpeg) {
                        (
                            Ok(crate::export::ExportOutcome::Success {
                                bytes_written: b1, ..
                            }),
                            Ok(crate::export::ExportOutcome::Success {
                                bytes_written: b2, ..
                            }),
                        ) => {
                            self.status_message = format!(
                                "Guardado OK: {} [{}] (TIFF 16b: {:.1}MB, JPEG 100%: {:.1}MB)",
                                file_name,
                                target_label,
                                b1 as f64 / 1_048_576.0,
                                b2 as f64 / 1_048_576.0,
                            );
                        }
                        (Err(e), _) | (_, Err(e)) => {
                            self.status_message = format!(
                                "Error al exportar {} [{}]: {}",
                                file_name, target_label, e
                            );
                        }
                        _ => {
                            self.status_message = format!(
                                "Exportación completada con avisos para {} [{}]",
                                file_name, target_label
                            );
                        }
                    }
                }
            }

            // Recipe intensity adjustments
            KeyCode::Char('[') => {
                self.mode_a.active_intensity =
                    (self.mode_a.active_intensity - 0.05).clamp(0.0, 1.5);
                self.status_message =
                    format!("Intensity: {:.0}%", self.mode_a.active_intensity * 100.0);
            }
            KeyCode::Char(']') => {
                self.mode_a.active_intensity =
                    (self.mode_a.active_intensity + 0.05).clamp(0.0, 1.5);
                self.status_message =
                    format!("Intensity: {:.0}%", self.mode_a.active_intensity * 100.0);
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_mode_b_key(&mut self, key: KeyEvent) -> Result<(), AppError> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.mode_b.prev_row();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.mode_b.next_row();
            }
            KeyCode::Char(' ') => {
                self.mode_b.toggle_pause();
                self.status_message = if self.mode_b.is_paused {
                    "Batch queue PAUSED".into()
                } else {
                    "Batch queue RUNNING".into()
                };
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.mode_b.cancel();
                self.status_message = "Batch queue CANCELLED".into();
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                let count = self.mode_b.retry_failed();
                self.status_message = format!("Retrying {} failed jobs", count);
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.show_help_modal {
            // Block background clicks when help modal is open
            return;
        }
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
            let layout = AppLayout::compute(self.terminal_size);
            if let Some(clicked) = layout.pane_at(mouse.column, mouse.row) {
                if [PaneId::Library, PaneId::PreviewCanvas].contains(&clicked) {
                    self.focused_pane = clicked;
                }
            }
        }
    }

    async fn navigate_photo_prev(&mut self) -> Result<(), AppError> {
        if !self.items.is_empty() {
            let old_idx = self.selected_index;
            let multi_selected = self.items.iter().filter(|i| i.is_selected).count() > 1;
            if self.selected_index > 0 {
                self.selected_index -= 1;
            } else {
                self.selected_index = self.items.len() - 1;
            }
            if !multi_selected {
                if let Some(old) = self.items.get_mut(old_idx) {
                    old.is_selected = false;
                }
                if let Some(cur) = self.items.get_mut(self.selected_index) {
                    cur.is_selected = true;
                }
            }
            self.sync_selected_photo().await?;
        }
        Ok(())
    }

    async fn navigate_photo_next(&mut self) -> Result<(), AppError> {
        if !self.items.is_empty() {
            let old_idx = self.selected_index;
            let multi_selected = self.items.iter().filter(|i| i.is_selected).count() > 1;
            if self.selected_index < self.items.len() - 1 {
                self.selected_index += 1;
            } else {
                self.selected_index = 0;
            }
            if !multi_selected {
                if let Some(old) = self.items.get_mut(old_idx) {
                    old.is_selected = false;
                }
                if let Some(cur) = self.items.get_mut(self.selected_index) {
                    cur.is_selected = true;
                }
            }
            self.sync_selected_photo().await?;
        }
        Ok(())
    }

    async fn sync_selected_photo(&mut self) -> Result<(), AppError> {
        if let Some(item) = self.items.get(self.selected_index) {
            self.mode_a.clear_selection();
            self.mode_a.metadata = read_native_metadata(&item.path);
            if let Some(ref mut photo) = self.mode_a.active_photo {
                photo.file_name = item.filename.clone();
                photo.format = if item.is_cr3 {
                    "cr3".into()
                } else {
                    "image".into()
                };
                photo.width = self.mode_a.metadata.width;
                photo.height = self.mode_a.metadata.height;
                photo.camera = self.mode_a.metadata.camera_model.clone();
                photo.captured = self.mode_a.metadata.capture_time.clone();
            }
            self.status_message = format!("Selected: {}", item.filename);
            self.trigger_variant_previews();
        }
        Ok(())
    }

    /// Execute batch export for all multi-selected (or currently focused) variants in Mode A.
    pub async fn execute_variant_export_batch(&mut self, format_opt: u8) -> Result<(), AppError> {
        let Some(item) = self.items.get(self.selected_index) else {
            self.status_message = "No hay foto seleccionada para exportar".into();
            return Ok(());
        };

        let source_path = item.path.clone();
        let file_name = item.filename.clone();
        let target_indices = self.mode_a.get_selected_or_focused_indices();
        let output_dir = PathBuf::from("export").join("interactive");
        let pipeline = crate::export::ExportPipeline::new();

        let format_label = match format_opt {
            1 => "TIFF 16-bit",
            2 => "JPEG 100%",
            _ => "TIFF 16-bit + JPEG 100%",
        };

        self.status_message = format!(
            "Exportando {} variante(s) de {} en formato {}...",
            target_indices.len(),
            file_name,
            format_label
        );
        self.needs_render = true;

        let mut success_count = 0usize;
        let mut job_seq = 1usize;

        for &slot_idx in &target_indices {
            let (recipe, preset_name) = if let Some(slot) = self.mode_a.variants.get(slot_idx) {
                (slot.recipe.clone(), slot.recipe.name.clone())
            } else {
                (
                    self.mode_a.active_recipe.clone(),
                    self.mode_a.active_recipe.name.clone(),
                )
            };

            let variant_tag = format!("var{}_{}", slot_idx + 1, preset_name.replace(' ', "_"));

            // TIFF export job
            if format_opt == 1 || format_opt == 3 {
                let tiff_job = crate::export::ExportJob {
                    job_id: job_seq as u64,
                    source_path: source_path.clone(),
                    photo_id: None,
                    recipe_name: variant_tag.clone(),
                    recipe: recipe.clone(),
                    profile: crate::export::ExportProfile {
                        name: format!("{}_TIFF16", variant_tag),
                        format: crate::export::OutputFormat::Tiff,
                        quality: 100,
                        bit_depth: crate::export::BitDepth::Sixteen,
                        scaling: crate::export::ScalingProfile::Native,
                        ..Default::default()
                    },
                    output_dir: output_dir.clone(),
                    template: crate::export::NamingTemplate::parse("{name}__{preset}.{ext}"),
                    seq_number: job_seq,
                    collision_policy: crate::export::CollisionPolicy::Overwrite,
                };
                job_seq += 1;
                let _ = pipeline.execute_export(None, &tiff_job).await;
            }

            // JPEG export job
            if format_opt == 2 || format_opt == 3 {
                let jpeg_job = crate::export::ExportJob {
                    job_id: job_seq as u64,
                    source_path: source_path.clone(),
                    photo_id: None,
                    recipe_name: variant_tag.clone(),
                    recipe: recipe.clone(),
                    profile: crate::export::ExportProfile {
                        name: format!("{}_JPEG100", variant_tag),
                        format: crate::export::OutputFormat::Jpeg,
                        quality: 100,
                        bit_depth: crate::export::BitDepth::Eight,
                        scaling: crate::export::ScalingProfile::Native,
                        ..Default::default()
                    },
                    output_dir: output_dir.clone(),
                    template: crate::export::NamingTemplate::parse("{name}__{preset}.{ext}"),
                    seq_number: job_seq,
                    collision_policy: crate::export::CollisionPolicy::Overwrite,
                };
                job_seq += 1;
                let _ = pipeline.execute_export(None, &jpeg_job).await;
            }

            success_count += 1;
        }

        self.mode_a.clear_selection();
        self.status_message = format!(
            "Exportación completada. Guardado OK: {} variante(s) exportadas en {}",
            success_count, format_label
        );
        self.needs_render = true;
        Ok(())
    }

    /// Execute batch export using the configured InteractiveFlowState matrix (US-006 & US-007).
    pub async fn execute_flow_export_batch(&mut self) -> Result<(), AppError> {
        let Some(item) = self.items.get(self.selected_index) else {
            self.status_message = "No hay foto seleccionada para exportar".into();
            self.flow_state.close();
            return Ok(());
        };

        let source_path = item.path.clone();
        let file_name = item.filename.clone();
        let target_variants = if self.flow_state.selected_variants.is_empty() {
            self.mode_a.get_selected_or_focused_indices()
        } else {
            self.flow_state.selected_variants.clone()
        };
        let target_ratios = if self.flow_state.selected_ratios.is_empty() {
            vec![AspectRatio::Original]
        } else {
            self.flow_state.selected_ratios.clone()
        };
        let target_formats = if self.flow_state.selected_formats.is_empty() {
            vec![crate::export::OutputFormat::Tiff]
        } else {
            self.flow_state.selected_formats.clone()
        };

        let output_dir = PathBuf::from("export").join("interactive");
        let pipeline = crate::export::ExportPipeline::new();

        let total_jobs = target_variants.len() * target_ratios.len() * target_formats.len();
        self.flow_state.step = WizardStep::Exporting;
        self.flow_state.set_export_progress(
            0,
            total_jobs,
            format!("Iniciando renderizado de {} archivo(s)...", total_jobs),
            0.0,
        );

        self.status_message = format!(
            "Exportando matriz en segundo plano: {} archivo(s) para {}...",
            total_jobs, file_name
        );
        self.needs_render = true;

        let mut jobs = Vec::new();
        let mut job_seq = 1usize;

        for &slot_idx in &target_variants {
            let (recipe, preset_name) = if let Some(slot) = self.mode_a.variants.get(slot_idx) {
                (slot.recipe.clone(), slot.recipe.name.clone())
            } else {
                (
                    self.mode_a.active_recipe.clone(),
                    self.mode_a.active_recipe.name.clone(),
                )
            };

            for &ratio in &target_ratios {
                for &format in &target_formats {
                    let aspect_tag = ratio.aspect_label();
                    let variant_tag = format!(
                        "var{}_{}__{}",
                        slot_idx + 1,
                        preset_name.replace(' ', "_"),
                        aspect_tag
                    );

                    let (bit_depth, quality) = match format {
                        crate::export::OutputFormat::Tiff => {
                            (crate::export::BitDepth::Sixteen, 100)
                        }
                        crate::export::OutputFormat::Png => (crate::export::BitDepth::Eight, 100),
                        crate::export::OutputFormat::Jpeg => (crate::export::BitDepth::Eight, 95),
                    };

                    let job = crate::export::ExportJob {
                        job_id: job_seq as u64,
                        source_path: source_path.clone(),
                        photo_id: None,
                        recipe_name: variant_tag.clone(),
                        recipe: recipe.clone(),
                        profile: crate::export::ExportProfile {
                            name: format!("{}_{:?}", variant_tag, format),
                            format,
                            quality,
                            bit_depth,
                            scaling: crate::export::ScalingProfile::AspectCrop { ratio },
                            ..Default::default()
                        },
                        output_dir: output_dir.clone(),
                        template: crate::export::NamingTemplate::parse("{name}__{preset}.{ext}"),
                        seq_number: job_seq,
                        collision_policy: crate::export::CollisionPolicy::Overwrite,
                    };
                    jobs.push((job, format, ratio, variant_tag));
                    job_seq += 1;
                }
            }
        }

        // Initialize cooperative cancellation token for this batch run
        let cancel_token = CancellationToken::new();
        self.flow_cancel_token = Some(cancel_token.clone());

        // Check if event sender is available for async streaming
        if let Some(ref tx) = self.event_sender {
            let tx_clone = tx.clone();
            tokio::spawn(async move {
                let mut success_count = 0usize;
                let total = jobs.len();
                for (idx, (job, format, ratio, variant_tag)) in jobs.into_iter().enumerate() {
                    // Check cancellation before starting next job
                    if cancel_token.is_cancelled() {
                        let _ = tx_clone
                            .send(AppEvent::FlowExportCancelled {
                                processed: success_count,
                                total,
                                cancelled: total.saturating_sub(success_count),
                            })
                            .await;
                        return;
                    }

                    let desc = format!("{} [{:?}, {}]", variant_tag, format, ratio.aspect_label());
                    let _ = tx_clone
                        .send(AppEvent::FlowExportProgress {
                            completed: idx,
                            total,
                            current_label: desc.clone(),
                            percent: if total > 0 {
                                (idx as f32 / total as f32) * 100.0
                            } else {
                                0.0
                            },
                        })
                        .await;

                    if let Ok(crate::export::ExportOutcome::Success { .. }) =
                        pipeline.execute_export(None, &job).await
                    {
                        success_count += 1;
                    }

                    // Check cancellation immediately after current job finishes
                    if cancel_token.is_cancelled() {
                        let _ = tx_clone
                            .send(AppEvent::FlowExportCancelled {
                                processed: success_count,
                                total,
                                cancelled: total.saturating_sub(success_count),
                            })
                            .await;
                        return;
                    }

                    let _ = tx_clone
                        .send(AppEvent::FlowExportProgress {
                            completed: idx + 1,
                            total,
                            current_label: desc,
                            percent: if total > 0 {
                                ((idx + 1) as f32 / total as f32) * 100.0
                            } else {
                                100.0
                            },
                        })
                        .await;
                }

                let _ = tx_clone
                    .send(AppEvent::FlowExportFinished {
                        successful: success_count,
                        total,
                        output_dir: "export/interactive".into(),
                    })
                    .await;
            });
        } else {
            // Synchronous execution fallback (for unit tests without event loop)
            let mut success_count = 0usize;
            for (job, _, _, _) in jobs {
                if cancel_token.is_cancelled() {
                    break;
                }
                if let Ok(crate::export::ExportOutcome::Success { .. }) =
                    pipeline.execute_export(None, &job).await
                {
                    success_count += 1;
                }
            }
            self.flow_cancel_token = None;
            self.flow_state.close();
            self.mode_a.clear_selection();
            self.status_message = format!(
                "Exportación completada. Guardado OK: {} archivo(s) generados en export/interactive/",
                success_count
            );
        }

        self.needs_render = true;
        Ok(())
    }

    /// Process keyboard inputs when the top interactive wizard flow is active.
    pub async fn handle_flow_key(&mut self, key: KeyEvent) -> Result<(), AppError> {
        match self.flow_state.step {
            WizardStep::Photos => {
                if key.code == KeyCode::Char('e') || key.code == KeyCode::Char('E') {
                    self.flow_state.step = WizardStep::AspectRatios;
                }
            }
            WizardStep::Variants => match key.code {
                KeyCode::Esc => {
                    self.flow_state.close();
                    self.status_message = "Asistente cancelado.".into();
                }
                KeyCode::Enter => {
                    if !self.flow_state.selected_variants.is_empty() {
                        self.flow_state.step = WizardStep::AspectRatios;
                    } else {
                        let targets = self.mode_a.get_selected_or_focused_indices();
                        self.flow_state.start_with_variants(targets);
                        self.flow_state.step = WizardStep::AspectRatios;
                    }
                }
                _ => {}
            },
            WizardStep::AspectRatios => match key.code {
                // Arrow navigation
                KeyCode::Left | KeyCode::Char('h') => {
                    self.flow_state.prev_ratio();
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    self.flow_state.next_ratio();
                }
                // Toggle focused ratio with Space
                KeyCode::Char(' ') => {
                    self.flow_state.toggle_focused_ratio();
                }
                // Direct numeric hotkeys
                KeyCode::Char('1') => self.flow_state.toggle_ratio(AspectRatio::Square1x1),
                KeyCode::Char('2') => self.flow_state.toggle_ratio(AspectRatio::Photo3x2),
                KeyCode::Char('3') => self.flow_state.toggle_ratio(AspectRatio::Classic4x3),
                KeyCode::Char('4') => self.flow_state.toggle_ratio(AspectRatio::Widescreen16x9),
                KeyCode::Char('5') => self.flow_state.toggle_ratio(AspectRatio::Social4x5),
                KeyCode::Char('6') => self.flow_state.toggle_ratio(AspectRatio::Original),
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    if self.flow_state.selected_ratios.len() == AspectRatio::ALL.len() {
                        self.flow_state.selected_ratios = vec![AspectRatio::Original];
                    } else {
                        self.flow_state.selected_ratios = AspectRatio::ALL.to_vec();
                    }
                }
                // Enter confirms and continues
                KeyCode::Enter => {
                    if self.flow_state.selected_ratios.is_empty() {
                        self.flow_state.toggle_focused_ratio();
                    }
                    if !self.flow_state.selected_ratios.is_empty() {
                        self.flow_state.step = WizardStep::FileFormats;
                    } else {
                        self.status_message =
                            "Selecciona al menos una proporción antes de continuar".into();
                    }
                }
                // Backward navigation
                KeyCode::Char('b') | KeyCode::Char('B') | KeyCode::Esc | KeyCode::Backspace => {
                    self.flow_state.close();
                    self.status_message = "Regresado al panel de variantes (Paso 2)".into();
                }
                _ => {}
            },
            WizardStep::FileFormats => match key.code {
                // Arrow navigation
                KeyCode::Left | KeyCode::Char('h') => {
                    self.flow_state.prev_format();
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    self.flow_state.next_format();
                }
                // Toggle focused format with Space
                KeyCode::Char(' ') => {
                    self.flow_state.toggle_focused_format();
                }
                // Direct numeric hotkeys
                KeyCode::Char('1') => self
                    .flow_state
                    .toggle_format(crate::export::OutputFormat::Tiff),
                KeyCode::Char('2') => self
                    .flow_state
                    .toggle_format(crate::export::OutputFormat::Jpeg),
                KeyCode::Char('3') => self
                    .flow_state
                    .toggle_format(crate::export::OutputFormat::Png),
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    if self.flow_state.selected_formats.len()
                        == crate::export::OutputFormat::ALL.len()
                    {
                        self.flow_state.selected_formats = vec![crate::export::OutputFormat::Tiff];
                    } else {
                        self.flow_state.selected_formats =
                            crate::export::OutputFormat::ALL.to_vec();
                    }
                }
                // Enter confirms and continues
                KeyCode::Enter => {
                    if self.flow_state.selected_formats.is_empty() {
                        self.flow_state.toggle_focused_format();
                    }
                    if !self.flow_state.selected_formats.is_empty() {
                        self.flow_state.step = WizardStep::Summary;
                    } else {
                        self.status_message =
                            "Selecciona al menos un formato antes de continuar".into();
                    }
                }
                // Backward navigation
                KeyCode::Char('b') | KeyCode::Char('B') | KeyCode::Backspace => {
                    self.flow_state.step = WizardStep::AspectRatios;
                }
                KeyCode::Esc => {
                    self.flow_state.close();
                    self.status_message = "Asistente cancelado.".into();
                }
                _ => {}
            },
            WizardStep::Summary => match key.code {
                // Arrow navigation
                KeyCode::Left | KeyCode::Char('h') => {
                    self.flow_state.prev_summary();
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    self.flow_state.next_summary();
                }
                // Enter executes the focused summary action
                KeyCode::Enter => match self.flow_state.focused_summary_idx {
                    0 => {
                        self.execute_flow_export_batch().await?;
                    }
                    1 => {
                        self.flow_state.close();
                        self.status_message =
                                "Modificando variantes en Panel 2. Pulsa [E] para reanudar el asistente."
                                    .into();
                    }
                    2 => {
                        self.flow_state.step = WizardStep::AspectRatios;
                    }
                    3 => {
                        self.flow_state.step = WizardStep::FileFormats;
                    }
                    4 => {
                        self.flow_state.step = WizardStep::SavePreset;
                    }
                    5 => {
                        self.flow_state.close();
                        self.status_message = "Exportación cancelada.".into();
                    }
                    _ => {}
                },
                // Direct hotkeys
                KeyCode::Char('1') => {
                    self.execute_flow_export_batch().await?;
                }
                KeyCode::Char('2') => {
                    self.flow_state.close();
                    self.status_message =
                        "Modificando variantes en Panel 2. Pulsa [E] para reanudar el asistente."
                            .into();
                }
                KeyCode::Char('3') => {
                    self.flow_state.step = WizardStep::AspectRatios;
                }
                KeyCode::Char('4') => {
                    self.flow_state.step = WizardStep::FileFormats;
                }
                KeyCode::Char('p') | KeyCode::Char('P') => {
                    self.flow_state.step = WizardStep::SavePreset;
                }
                KeyCode::Char('5') | KeyCode::Esc => {
                    self.flow_state.close();
                    self.status_message = "Exportación cancelada.".into();
                }
                _ => {}
            },
            WizardStep::Exporting => {
                if self.flow_state.is_confirming_cancel {
                    match key.code {
                        KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Enter => {
                            if let Some(ref token) = self.flow_cancel_token {
                                token.cancel();
                            }
                            self.flow_state.is_confirming_cancel = false;
                            self.status_message =
                                "Deteniendo exportación... finalizando archivo en curso.".into();
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            self.flow_state.is_confirming_cancel = false;
                            self.status_message =
                                "Cancelación descartada. Exportación en curso.".into();
                        }
                        _ => {}
                    }
                } else if key.code == KeyCode::Esc {
                    self.flow_state.is_confirming_cancel = true;
                    self.status_message =
                        "¿Desea cancelar la exportación en curso? [S]í / [N]o (o [Esc] para continuar)"
                            .into();
                }
            }
            WizardStep::SavePreset => match key.code {
                KeyCode::Enter | KeyCode::Char('b') | KeyCode::Char('B') | KeyCode::Esc => {
                    self.flow_state.step = WizardStep::Summary;
                    self.status_message =
                        "Configuración registrada como preset. Pulsa [1/Enter] para exportar."
                            .into();
                }
                _ => {}
            },
        }
        self.needs_render = true;
        Ok(())
    }

    fn is_animating(&self) -> bool {
        self.mode_b.is_running && !self.mode_b.is_paused
    }

    /// Top-level Ratatui render pass.
    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        let layout = AppLayout::compute_with_flow(area, self.flow_state.is_active);

        if layout.is_too_small {
            self.render_too_small_overlay(frame, area);
            return;
        }

        // 1. Render Header (or Top Wizard Banner if flow active)
        self.render_header(frame, layout.header_area);

        // 2. Render Library Explorer Pane
        self.render_library(frame, layout.library_area, &layout);

        // 3. Render Center Canvas (Mode A vs Mode B)
        self.render_center(frame, layout.preview_area);

        // 4. Render Status Bar
        self.render_status(frame, layout.status_area);

        // 5. Render Help Modal on top if active
        if self.show_help_modal {
            self.render_help_modal(frame);
        }
    }

    fn render_too_small_overlay(&self, frame: &mut Frame, area: Rect) {
        if area.width < 10 || area.height < 3 {
            return;
        }
        let msg = if area.width >= 40 && area.height >= 7 {
            format!(
                "VENTANA DEMASIADO PEQUEÑA\n\nTamaño actual: {}x{}\nMínimo requerido: {}x{}\nPor favor amplía la consola.\n[F5] Restablecer",
                area.width, area.height, AppLayout::MIN_WIDTH, AppLayout::MIN_HEIGHT
            )
        } else {
            format!(
                "MIN: {}x{}\nACT: {}x{}",
                AppLayout::MIN_WIDTH,
                AppLayout::MIN_HEIGHT,
                area.width,
                area.height
            )
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .title(" Alerta ");
        let p = Paragraph::new(msg)
            .block(block)
            .alignment(Alignment::Center);
        frame.render_widget(p, area);
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        if self.flow_state.is_active {
            self.render_top_wizard_banner(frame, area);
            return;
        }

        let mode_label = match self.mode {
            AppMode::SinglePhoto => "Mode A",
            AppMode::BatchQueue => "Mode B",
        };

        let step1_active = self.focused_pane == PaneId::Library;
        let step1_style = if step1_active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };
        let step2_style = if !step1_active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };

        let app_name = Span::styled(
            " LightCraft TUI ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
        let mode_badge = Span::styled(
            format!(" [{}] ", mode_label),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
        let sep1 = Span::styled(
            format!(" {} ", self.glyph_set.separator()),
            Style::default().fg(Color::DarkGray),
        );
        let step1 = Span::styled(" [1. Fotos] ", step1_style);
        let arrow = Span::styled(
            format!(" {} ", self.glyph_set.step_arrow()),
            Style::default().fg(Color::DarkGray),
        );
        let step2 = Span::styled(" [2. Variantes] ", step2_style);
        let sep2 = Span::styled(
            format!(" {} ", self.glyph_set.separator()),
            Style::default().fg(Color::DarkGray),
        );
        let mcp_badge = Span::styled(
            " [MCP: OK] ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        );

        let header_line = Line::from(vec![
            app_name, mode_badge, sep1, step1, arrow, step2, sep2, mcp_badge,
        ]);

        let p = Paragraph::new(header_line);
        frame.render_widget(p, area);
    }

    fn render_top_wizard_banner(&self, frame: &mut Frame, area: Rect) {
        let step_title = self.flow_state.step.title();

        let step1_style = Style::default().fg(Color::Cyan);
        let step2_style = Style::default().fg(Color::Yellow);

        let step3_style = if self.flow_state.step == WizardStep::AspectRatios {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };
        let step4_style = if self.flow_state.step == WizardStep::FileFormats {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };
        let step5_style = if self.flow_state.step == WizardStep::Summary
            || self.flow_state.step == WizardStep::SavePreset
        {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };
        let step6_style = if self.flow_state.step == WizardStep::Exporting {
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightCyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::LightCyan)
        };

        let arr = self.glyph_set.step_arrow();
        let breadcrumb_line = Line::from(vec![
            Span::styled(
                " [ASISTENTE] ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" [1. Fotos] ", step1_style),
            Span::raw(arr),
            Span::styled(" [2. Variantes] ", step2_style),
            Span::raw(arr),
            Span::styled(" [3. Proporciones] ", step3_style),
            Span::raw(arr),
            Span::styled(" [4. Formatos] ", step4_style),
            Span::raw(arr),
            Span::styled(" [5. Resumen] ", step5_style),
            Span::raw(arr),
            Span::styled(" [6. Renderizado] ", step6_style),
        ]);

        let (info_line, help_line) = match self.flow_state.step {
            WizardStep::Photos | WizardStep::Variants => (
                Line::from(vec![Span::styled(
                    format!(
                        " Variantes seleccionadas: {} ",
                        self.flow_state.selected_variants.len()
                    ),
                    Style::default().fg(Color::White),
                )]),
                Line::from(vec![Span::styled(
                    " [Space] Seleccionar variante | [Enter] Continuar a Proporciones | [Esc] Cancelar ",
                    Style::default().fg(Color::Yellow),
                )]),
            ),
            WizardStep::AspectRatios => {
                let ratio_items: [(AspectRatio, &str, &str); 6] = [
                    (AspectRatio::Square1x1, "1", "1:1 Cuadrado"),
                    (AspectRatio::Photo3x2, "2", "3:2 Foto"),
                    (AspectRatio::Classic4x3, "3", "4:3 Clásico"),
                    (AspectRatio::Widescreen16x9, "4", "16:9 Panorámico"),
                    (AspectRatio::Social4x5, "5", "4:5 Retrato"),
                    (AspectRatio::Original, "6", "Original"),
                ];

                let mut spans = Vec::new();
                for (idx, (ratio, key_num, label)) in ratio_items.iter().enumerate() {
                    let is_sel = self.flow_state.selected_ratios.contains(ratio);
                    let is_foc = idx == self.flow_state.focused_ratio_idx;

                    let prefix = if is_foc { format!("{} ", self.glyph_set.triangle_right()) } else { "  ".to_string() };
                    let suffix = if is_foc { format!(" {}", self.glyph_set.triangle_left()) } else { "  ".to_string() };
                    let mark = if is_sel { self.glyph_set.check_box_checked() } else { self.glyph_set.check_box_unchecked() };
                    let item_text = format!("{}{}[{}] {}{}", prefix, mark, key_num, label, suffix);

                    let style = if is_sel {
                        // Azul claro al estar seleccionado
                        let mut st = Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD);
                        if is_foc {
                            st = st.bg(Color::Rgb(20, 50, 80));
                        }
                        st
                    } else {
                        // Blanco sin selección
                        let mut st = Style::default().fg(Color::White);
                        if is_foc {
                            st = st.bg(Color::Rgb(50, 50, 50)).add_modifier(Modifier::BOLD);
                        }
                        st
                    };

                    spans.push(Span::styled(item_text, style));
                    spans.push(Span::raw(" "));
                }

                // Opción 7: Todas
                let all_sel = self.flow_state.selected_ratios.len() == AspectRatio::ALL.len();
                let all_foc = self.flow_state.focused_ratio_idx == 6;
                let all_prefix = if all_foc { format!("{} ", self.glyph_set.triangle_right()) } else { "  ".to_string() };
                let all_suffix = if all_foc { format!(" {}", self.glyph_set.triangle_left()) } else { "  ".to_string() };
                let all_mark = if all_sel { self.glyph_set.check_box_checked() } else { self.glyph_set.check_box_unchecked() };
                let all_text = format!("{}{}[A] Todas{}", all_prefix, all_mark, all_suffix);
                let all_style = if all_sel {
                    let mut st = Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD);
                    if all_foc {
                        st = st.bg(Color::Rgb(20, 50, 80));
                    }
                    st
                } else {
                    let mut st = Style::default().fg(Color::White);
                    if all_foc {
                        st = st.bg(Color::Rgb(50, 50, 50)).add_modifier(Modifier::BOLD);
                    }
                    st
                };
                spans.push(Span::styled(all_text, all_style));

                (
                    Line::from(spans),
                    Line::from(vec![Span::styled(
                        format!(
                            " [{}] Moverse | [Espacio] Seleccionar/Desmarcar {} | [Enter] Continuar a Formatos {} | [1..6/A] Atajo | [B] Volver | [Esc] Cancelar ",
                            self.glyph_set.arrow_left_right(),
                            self.glyph_set.check_box_checked(),
                            self.glyph_set.step_arrow()
                        ),
                        Style::default().fg(Color::Yellow),
                    )]),
                )
            }
            WizardStep::FileFormats => {
                let format_items: [(crate::export::OutputFormat, &str, &str); 3] = [
                    (crate::export::OutputFormat::Tiff, "1", "TIFF (16-bit)"),
                    (crate::export::OutputFormat::Jpeg, "2", "JPEG (Web)"),
                    (crate::export::OutputFormat::Png, "3", "PNG"),
                ];

                let mut spans = Vec::new();
                for (idx, (format, key_num, label)) in format_items.iter().enumerate() {
                    let is_sel = self.flow_state.selected_formats.contains(format);
                    let is_foc = idx == self.flow_state.focused_format_idx;

                    let prefix = if is_foc { format!("{} ", self.glyph_set.triangle_right()) } else { "  ".to_string() };
                    let suffix = if is_foc { format!(" {}", self.glyph_set.triangle_left()) } else { "  ".to_string() };
                    let mark = if is_sel { self.glyph_set.check_box_checked() } else { self.glyph_set.check_box_unchecked() };
                    let item_text = format!("{}{}[{}] {}{}", prefix, mark, key_num, label, suffix);

                    let style = if is_sel {
                        // Azul claro al estar seleccionado
                        let mut st = Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD);
                        if is_foc {
                            st = st.bg(Color::Rgb(20, 50, 80));
                        }
                        st
                    } else {
                        // Blanco sin selección
                        let mut st = Style::default().fg(Color::White);
                        if is_foc {
                            st = st.bg(Color::Rgb(50, 50, 50)).add_modifier(Modifier::BOLD);
                        }
                        st
                    };

                    spans.push(Span::styled(item_text, style));
                    spans.push(Span::raw(" "));
                }

                // Opción 4: Todos
                let all_sel = self.flow_state.selected_formats.len() == crate::export::OutputFormat::ALL.len();
                let all_foc = self.flow_state.focused_format_idx == 3;
                let all_prefix = if all_foc { format!("{} ", self.glyph_set.triangle_right()) } else { "  ".to_string() };
                let all_suffix = if all_foc { format!(" {}", self.glyph_set.triangle_left()) } else { "  ".to_string() };
                let all_mark = if all_sel { self.glyph_set.check_box_checked() } else { self.glyph_set.check_box_unchecked() };
                let all_text = format!("{}{}[A] Todos{}", all_prefix, all_mark, all_suffix);
                let all_style = if all_sel {
                    let mut st = Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD);
                    if all_foc {
                        st = st.bg(Color::Rgb(20, 50, 80));
                    }
                    st
                } else {
                    let mut st = Style::default().fg(Color::White);
                    if all_foc {
                        st = st.bg(Color::Rgb(50, 50, 50)).add_modifier(Modifier::BOLD);
                    }
                    st
                };
                spans.push(Span::styled(all_text, all_style));

                (
                    Line::from(spans),
                    Line::from(vec![Span::styled(
                        format!(
                            " [{}] Moverse | [Espacio] Seleccionar/Desmarcar {} | [Enter] Continuar al Resumen {} | [1..3/A] Atajo | [B] Proporciones | [Esc] Cancelar ",
                            self.glyph_set.arrow_left_right(),
                            self.glyph_set.check_box_checked(),
                            self.glyph_set.step_arrow()
                        ),
                        Style::default().fg(Color::Yellow),
                    )]),
                )
            }
            WizardStep::Summary => {
                let photos = 1;
                let variants = self.flow_state.selected_variants.len().max(1);
                let ratios = self.flow_state.selected_ratios.len().max(1);
                let formats = self.flow_state.selected_formats.len().max(1);
                let plan = JobPlan::new(photos, variants, ratios, formats);

                let summary_actions = [
                    (0, "[1/Enter] INICIAR EXPORTACIÓN"),
                    (1, "[2] Variantes"),
                    (2, "[3] Proporciones"),
                    (3, "[4] Formatos"),
                    (4, "[P] Guardar Preset"),
                    (5, "[Esc] Cancelar"),
                ];

                let mut action_spans = Vec::new();
                for (idx, label) in summary_actions {
                    let is_foc = idx == self.flow_state.focused_summary_idx;
                    let prefix = if is_foc { format!("{} ", self.glyph_set.triangle_right()) } else { " ".to_string() };
                    let suffix = if is_foc { format!(" {}", self.glyph_set.triangle_left()) } else { " ".to_string() };
                    let text = format!("{}{}{}", prefix, label, suffix);

                    let style = if idx == 0 {
                        if is_foc {
                            Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                        }
                    } else if is_foc {
                        Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    action_spans.push(Span::styled(text, style));
                    action_spans.push(Span::raw("  "));
                }

                (
                    Line::from(vec![
                        Span::styled(
                            format!(
                                " TOTAL ESTIMADO: {} foto(s) {} {} variante(s) {} {} proporción(es) {} {} formato(s) = {} ARCHIVOS ",
                                plan.photos_count, self.glyph_set.times(), plan.variants_count, self.glyph_set.times(), plan.ratios_count, self.glyph_set.times(), plan.formats_count, plan.total_output_files()
                            ),
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            "| Carpeta: export/interactive/ ",
                            Style::default().fg(Color::White),
                        ),
                    ]),
                    Line::from(action_spans),
                )
            }
            WizardStep::Exporting => {
                if self.flow_state.is_confirming_cancel {
                    (
                        Line::from(vec![
                            Span::styled(
                                " [?] ¿CANCELAR EXPORTACIÓN EN CURSO? ",
                                Style::default()
                                    .fg(Color::White)
                                    .bg(Color::Red)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!(
                                    " Archivos completados: {}/{} ({:.0}%) ",
                                    self.flow_state.export_completed,
                                    self.flow_state.export_total,
                                    self.flow_state.export_percent
                                ),
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]),
                        Line::from(vec![
                            Span::styled(
                                " [S/Enter] Sí, detener exportación | [N/Esc] No, continuar exportando ",
                                Style::default()
                                    .fg(Color::LightYellow)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]),
                    )
                } else {
                    let spinner = self.glyph_set.spinner_frame(self.flow_state.export_completed * 2);
                    let pct = self.flow_state.export_percent.clamp(0.0, 100.0);
                    let filled = ((pct / 100.0) * 20.0).round() as usize;
                    let empty = 20usize.saturating_sub(filled);
                    let (fill_c, empty_c) = self.glyph_set.progress_bar_chars();
                    let bar = format!("[{}{}] {:.1}%", fill_c.to_string().repeat(filled), empty_c.to_string().repeat(empty), pct);

                    let elapsed = self
                        .flow_state
                        .export_start_time
                        .map(|t| t.elapsed().as_secs_f32())
                        .unwrap_or(0.0);

                    let eta_str = if self.flow_state.export_completed > 0
                        && self.flow_state.export_total > self.flow_state.export_completed
                    {
                        let rate = elapsed / self.flow_state.export_completed as f32;
                        let remaining = self.flow_state.export_total - self.flow_state.export_completed;
                        format!("~{:.1}s restantes", rate * remaining as f32)
                    } else if self.flow_state.export_completed >= self.flow_state.export_total && self.flow_state.export_total > 0 {
                        "¡Completado!".into()
                    } else {
                        "calculando...".into()
                    };

                    (
                        Line::from(vec![
                            Span::styled(
                                format!(" {} PROCESANDO LOTE: ", spinner),
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!(" {} ", bar),
                                Style::default()
                                    .fg(Color::LightCyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!(
                                    " Archivo {} de {} ",
                                    self.flow_state.export_completed, self.flow_state.export_total
                                ),
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]),
                        Line::from(vec![
                            Span::styled(
                                format!(" Tarea: {} ", self.flow_state.export_current_label),
                                Style::default().fg(Color::Cyan),
                            ),
                            Span::styled(
                                format!("| Transcurrido: {:.1}s | ETA: {} | [Esc] Interrumpir ", elapsed, eta_str),
                                Style::default().fg(Color::Yellow),
                            ),
                        ]),
                    )
                }
            }
            WizardStep::SavePreset => (
                Line::from(vec![Span::styled(
                    " CONFIGURACIÓN REGISTRADA COMO PRESET DE EXPORTACIÓN ",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(vec![Span::styled(
                    " [Enter/B] Volver al Resumen ",
                    Style::default().fg(Color::Yellow),
                )]),
            ),
        };

        let border_color = if self.flow_state.step == WizardStep::Exporting {
            Color::Yellow
        } else {
            Color::Cyan
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(
                Style::default()
                    .fg(border_color)
                    .add_modifier(Modifier::BOLD),
            )
            .title(format!(
                " {} {} {} ",
                self.glyph_set.step_arrow(),
                step_title,
                self.glyph_set.step_left()
            ));

        let paragraph = Paragraph::new(vec![breadcrumb_line, info_line, help_line])
            .block(block)
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    fn render_library(&self, frame: &mut Frame, area: Rect, layout: &AppLayout) {
        let is_pane_focused = self.focused_pane == PaneId::Library;
        let block = layout.block_for_with_glyphs(
            PaneId::Library,
            self.focused_pane,
            "[1] Biblioteca",
            &self.glyph_set,
        );
        let inner = block.inner(area);
        let max_visible = inner.height as usize;

        // Viewport scroll window to keep selected_index strictly in view
        let scroll_offset = if max_visible > 0 && self.selected_index >= max_visible {
            self.selected_index + 1 - max_visible
        } else {
            0
        };

        let list_items: Vec<ListItem> = self
            .items
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(if max_visible > 0 {
                max_visible
            } else {
                self.items.len()
            })
            .map(|(idx, item)| {
                let is_focused = idx == self.selected_index;
                let is_sel = item.is_selected;
                let prefix = if is_focused {
                    format!("{} ", self.glyph_set.triangle_right())
                } else {
                    "  ".to_string()
                };
                let suffix = if is_focused {
                    format!(" {}", self.glyph_set.triangle_left())
                } else {
                    "  ".to_string()
                };
                let flag = if is_sel {
                    format!("{} ", self.glyph_set.check_box_checked())
                } else {
                    format!("{} ", self.glyph_set.check_box_unchecked())
                };
                let raw_badge = if item.is_cr3 { " [CR3]" } else { " [IMG]" };
                let star_badge = if item.rating > 0 {
                    format!(" {}{}", item.rating, self.glyph_set.favorite_star())
                } else {
                    "".into()
                };
                let pick_badge = if item.is_picked {
                    " [P]"
                } else if item.is_rejected {
                    " [X]"
                } else {
                    ""
                };

                let style = if is_sel {
                    // Azul claro al estar seleccionado
                    let mut st = Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD);
                    if is_focused {
                        st = st.bg(if is_pane_focused {
                            Color::Rgb(20, 50, 80)
                        } else {
                            Color::Rgb(15, 30, 45)
                        });
                    }
                    st
                } else {
                    // Blanco sin selección
                    let mut st = Style::default().fg(Color::White);
                    if is_focused {
                        st = st
                            .bg(if is_pane_focused {
                                Color::Rgb(50, 50, 50)
                            } else {
                                Color::Rgb(30, 30, 30)
                            })
                            .add_modifier(Modifier::BOLD);
                    }
                    st
                };
                ListItem::new(format!(
                    "{}{}{}{}{}{}{}",
                    prefix, flag, item.filename, raw_badge, star_badge, pick_badge, suffix
                ))
                .style(style)
            })
            .collect();

        let list = List::new(list_items).block(block);
        frame.render_widget(list, area);
    }

    fn render_center(&self, frame: &mut Frame, area: Rect) {
        let is_focused = self.focused_pane == PaneId::PreviewCanvas;
        match self.mode {
            AppMode::SinglePhoto => render_mode_a(
                frame,
                area,
                &self.mode_a,
                &self.graphics,
                is_focused,
                &self.glyph_set,
            ),
            AppMode::BatchQueue => {
                render_mode_b(frame, area, &self.mode_b, is_focused, &self.glyph_set)
            }
        }
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        if self.export_prompt {
            let prompt_line = Line::from(vec![
                Span::styled(
                    " [EXPORTAR FORMATO] ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Rgb(255, 165, 0))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " [1] Solo TIFF | [2] Solo JPEG | [3 / Enter] Ambos (TIFF + JPEG) | [Esc] Cancelar ",
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            frame.render_widget(Paragraph::new(prompt_line), area);
            return;
        }

        let total_w = area.width as usize;
        let total_items = self.items.len();
        let current_idx = if total_items == 0 {
            0
        } else {
            self.selected_index + 1
        };
        let photo_name = self
            .items
            .get(self.selected_index)
            .map(|i| i.filename.as_str())
            .unwrap_or("Sin fotos");

        // 1. Sección izquierda: Contador de foto activa (`Foto X/Y`) y nombre del archivo seleccionado
        let left_str = format!(
            " Foto {}/{} | {} ",
            current_idx,
            total_items.max(1),
            photo_name
        );
        let left_span = Span::styled(
            left_str,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

        // 2. Sección derecha: Indicador compacto de comandos globales: `[F1 Ayuda]  [Tab Panel]  [Q Salir]`
        let right_str = " [F1 Ayuda]  [Tab Panel]  [Q Salir] ";
        let right_span = Span::styled(
            right_str,
            Style::default()
                .fg(Color::Yellow)
                .bg(Color::Black)
                .add_modifier(Modifier::BOLD),
        );

        // 3. Sección central: Mensaje de estado dinámico o badge de selección (`Sel: N`)
        let sel_count = self.items.iter().filter(|i| i.is_selected).count();
        let sel_badge = if sel_count > 1 {
            format!("[Sel: {}] ", sel_count)
        } else {
            String::new()
        };

        let raw_status = format!(" {}{} ", sel_badge, self.status_message.trim());
        let left_len = left_span.width();
        let right_len = right_span.width();

        // Calculate space for central message and padding to keep strictly 1 line
        let avail_center = total_w.saturating_sub(left_len + right_len);
        let center_text = if raw_status.chars().count() > avail_center {
            if avail_center > 3 {
                let keep = avail_center.saturating_sub(3);
                format!("{}...", raw_status.chars().take(keep).collect::<String>())
            } else {
                String::new()
            }
        } else {
            raw_status
        };

        let center_span = Span::styled(
            center_text,
            Style::default().fg(Color::White).bg(Color::DarkGray),
        );

        let used_len = left_len + center_span.width() + right_len;
        let padding = total_w.saturating_sub(used_len);
        let pad_span = Span::raw(" ".repeat(padding));

        let status_line = Line::from(vec![left_span, center_span, pad_span, right_span]);
        let p = Paragraph::new(status_line);
        frame.render_widget(p, area);
    }

    fn render_help_modal(&self, frame: &mut Frame) {
        let area = frame.area();
        // FR-3: 75% width, 70% height
        let modal_w = ((area.width as u32 * 75) / 100)
            .max(50)
            .min(area.width as u32) as u16;
        let modal_h = ((area.height as u32 * 70) / 100)
            .max(16)
            .min(area.height as u32) as u16;
        let x = (area.width.saturating_sub(modal_w)) / 2;
        let y = (area.height.saturating_sub(modal_h)) / 2;
        let modal_area = Rect::new(x, y, modal_w, modal_h);

        // Clear background under modal
        frame.render_widget(Clear, modal_area);

        let glyph_mode_tag = if self.glyph_set.is_ascii() {
            "Modo ASCII"
        } else {
            "Modo UTF-8"
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(self.glyph_set.border_set(true))
            .border_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .title(format!(
                " {} GUÍA DE COMANDOS Y ATAJOS [{}] {} ",
                self.glyph_set.step_arrow(),
                glyph_mode_tag,
                self.glyph_set.step_left(),
            ))
            .title_alignment(Alignment::Center);

        let inner = block.inner(modal_area);
        frame.render_widget(block, modal_area);

        if inner.height < 5 || inner.width < 30 {
            return;
        }

        let vert = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // Tabs
                Constraint::Min(4),    // Tab content
                Constraint::Length(1), // Footer hint
            ])
            .split(inner);

        // 1. Tabs
        let tab_titles = vec![
            "[1] General",
            "[2] Biblioteca",
            "[3] Variantes/Visor",
            "[4] Exportar",
        ];
        let tabs = Tabs::new(tab_titles)
            .select(self.help_active_tab)
            .style(Style::default().fg(Color::DarkGray))
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .divider(Span::styled(
                format!(" {} ", self.glyph_set.separator()),
                Style::default().fg(Color::DarkGray),
            ));
        frame.render_widget(tabs, vert[0]);

        // 2. Tab Content
        let b = self.glyph_set.bullet();
        let arr = self.glyph_set.step_arrow();
        let star = self.glyph_set.favorite_star();
        let chk = self.glyph_set.check_box_checked();

        let content_lines = match self.help_active_tab {
            0 => vec![
                Line::from(vec![
                    Span::styled("NAVEGACIÓN GLOBAL Y ENTORNO", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [Tab] / [Shift+Tab]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Alternar foco entre Biblioteca (Panel 1) y Visor/Canvas (Panel 2).", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [M] / [F10]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Alternar entre Modo A (Inspección individual) y Modo B (Cola de lotes).", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [F5] / [R] / [Ctrl+L]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Forzar redibujado de la terminal y restablecer caché gráfica.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [F1] / [?]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Abrir o cerrar esta ventana de ayuda y tutorial.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [Q] / [Ctrl+C]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Salir de la aplicación de forma segura restaurando la consola.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(format!("  {} Glifos y compatibilidad: ", arr), Style::default().fg(Color::LightCyan)),
                    Span::styled("Si su terminal no soporta Unicode, ejecute con '--ascii' o 'LC_ASCII=1' para fallback seguro.", Style::default().fg(Color::DarkGray)),
                ]),
            ],
            1 => vec![
                Line::from(vec![
                    Span::styled("PANEL 1: BIBLIOTECA DE FOTOS RAW", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [{}] / [k] / [j]: ", b, self.glyph_set.arrow_up_down()), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Moverse verticalmente por las fotos RAW / imágenes encontradas.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [Enter]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Confirmar foto activa y transferir el foco al Visor de Variantes.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [Espacio]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("Marcar / desmarcar foto actual para selección múltiple ({}).", chk), Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [E]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Lanzar el Asistente de Exportación interactivo para la selección.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [F5] / [R]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Recargar directorio DRAW y metadatos EXIF / RAW.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(format!("  {} Inmutabilidad garantizada: ", arr), Style::default().fg(Color::Green)),
                    Span::styled("Los archivos RAW originales nunca son modificados en disco (SHA-256 verificado).", Style::default().fg(Color::DarkGray)),
                ]),
            ],
            2 => vec![
                Line::from(vec![
                    Span::styled("PANEL 2: VISOR, MATRIZ DE VARIANTES Y COMPARACIÓN", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [{}] / [h] / [l]: ", b, self.glyph_set.arrow_left_right()), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Navegar entre las variantes de color generadas por el motor.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [Espacio]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("Marcar o desmarcar variante para exportación ({}).", chk), Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [F]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("Marcar / alternar variante activa como favorita ({}).", star), Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [Enter]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Promover la variante seleccionada como receta activa principal.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [1] / [2] / [4]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Cambiar modo de vista: [1] Hero individual, [2] Doble, [4] Matriz de 4.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [S]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Alternar modo Split-View Antes / Después (A/B comparativo).", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [E]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Continuar al Asistente de Proporciones y Formatos de Exportación.", Style::default().fg(Color::Gray)),
                ]),
            ],
            _ => vec![
                Line::from(vec![
                    Span::styled("ASISTENTE DE FLUJO Y EXPORTACIÓN", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} Paso 1 & 2: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Seleccionar foto(s) RAW y variante(s) de revelado deseadas.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} Paso 3 (Proporciones): ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("[1..6] Atajos directos (1:1, 3:2, 16:9, etc.), [A] Todas, [Espacio] Toggle.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} Paso 4 (Formatos): ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("[1] TIFF 16-bit | [2] JPEG 100% | [3] PNG | [4] TIFF+JPEG | [5] Todos.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} Paso 5 (Resumen y Preset): ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("[Enter] Iniciar renderizado masivo sin bloqueo | [P] Guardar como preset.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [B] / [Retroceso]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Regresar al paso anterior conservando todas las opciones marcadas.", Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  {} [Esc]: ", b), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("Cancelar el asistente y volver al área de trabajo principal.", Style::default().fg(Color::Gray)),
                ]),
            ],
        };

        let content_p = Paragraph::new(content_lines).wrap(Wrap { trim: false });
        frame.render_widget(content_p, vert[1]);

        // 3. Footer navigation
        let footer_hint = Line::from(vec![
            Span::styled(
                " [Tab / 1..4 / Flechas] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Cambiar pestaña   ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}   ", self.glyph_set.separator()),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                " [Esc / F1 / Q] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Cerrar ventana de ayuda", Style::default().fg(Color::Gray)),
        ]);
        let footer_p = Paragraph::new(footer_hint).alignment(Alignment::Center);
        frame.render_widget(footer_p, vert[2]);
    }
}
