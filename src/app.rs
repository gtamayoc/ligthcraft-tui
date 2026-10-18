//! Central application controller, state coordinator, and 60 FPS non-blocking render loop.

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use image::DynamicImage;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
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
            status_message:
                "Listo. [↑/↓] Moverse por fotos | [Enter] Confirmar y continuar ❯ | [Space] Selección múltiple | [E] Asistente".into(),
            terminal_size: Rect::new(0, 0, 120, 30),
            needs_render: true,
            running: true,
            force_clear: false,
            export_prompt: false,
            flow_state: InteractiveFlowState::default(),
            event_sender: None,
            flow_cancel_token: None,
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
            // Photo library navigation (Up / Down / Left / Right / k / j / h / l) with wrap-around
            KeyCode::Up | KeyCode::Left | KeyCode::Char('k') | KeyCode::Char('h')
                if self.focused_pane == PaneId::Library =>
            {
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
            }
            KeyCode::Down | KeyCode::Right | KeyCode::Char('j') | KeyCode::Char('l')
                if self.focused_pane == PaneId::Library =>
            {
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
                            "seleccionada [✓]"
                        } else {
                            "deseleccionada [ ]"
                        }
                    );
                }
            }

            // Photo navigation from PreviewCanvas (Hero and SplitView)
            KeyCode::Up | KeyCode::Left | KeyCode::Char('k') | KeyCode::Char('h')
                if self.focused_pane == PaneId::PreviewCanvas
                    && self.mode_a.view_mode != ModeAView::VariantMatrix =>
            {
                if !self.items.is_empty() {
                    if self.selected_index > 0 {
                        self.selected_index -= 1;
                    } else {
                        self.selected_index = self.items.len() - 1;
                    }
                    self.sync_selected_photo().await?;
                }
            }
            KeyCode::Down | KeyCode::Right | KeyCode::Char('j') | KeyCode::Char('l')
                if self.focused_pane == PaneId::PreviewCanvas
                    && self.mode_a.view_mode != ModeAView::VariantMatrix =>
            {
                if !self.items.is_empty() {
                    if self.selected_index < self.items.len() - 1 {
                        self.selected_index += 1;
                    } else {
                        self.selected_index = 0;
                    }
                    self.sync_selected_photo().await?;
                }
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
                    2 => 2,
                    3 => 3,
                    4 => 2,
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
                    2 => 2,
                    3 => 3,
                    4 => 2,
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
                self.status_message = "Toggled favorite (★) on active variant".into();
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
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
            let layout = AppLayout::compute(self.terminal_size);
            if let Some(clicked) = layout.pane_at(mouse.column, mouse.row) {
                // Solo cambiar foco si se hace clic dentro de uno de los 2 paneles interactivos
                if [PaneId::Library, PaneId::PreviewCanvas].contains(&clicked) {
                    self.focused_pane = clicked;
                }
            }
        }
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
                KeyCode::Char('1') => {
                    self.flow_state.selected_formats = vec![crate::export::OutputFormat::Tiff];
                }
                KeyCode::Char('2') => {
                    self.flow_state.selected_formats = vec![crate::export::OutputFormat::Jpeg];
                }
                KeyCode::Char('3') => {
                    self.flow_state.selected_formats = vec![crate::export::OutputFormat::Png];
                }
                KeyCode::Char('4') => {
                    self.flow_state.selected_formats = vec![
                        crate::export::OutputFormat::Tiff,
                        crate::export::OutputFormat::Jpeg,
                    ];
                }
                KeyCode::Char('5') => {
                    self.flow_state.selected_formats = vec![
                        crate::export::OutputFormat::Tiff,
                        crate::export::OutputFormat::Jpeg,
                        crate::export::OutputFormat::Png,
                    ];
                }
                // Enter confirms and continues
                KeyCode::Enter => {
                    if self.flow_state.selected_formats.is_empty() {
                        self.flow_state.toggle_focused_format();
                    }
                    if !self.flow_state.selected_formats.is_empty() {
                        self.flow_state.step = WizardStep::Summary;
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

        // Permanent Assistant guidance during standard browsing
        let active_step = if self.focused_pane == PaneId::Library {
            WizardStep::Photos
        } else {
            WizardStep::Variants
        };

        let step1_style = if active_step == WizardStep::Photos {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };
        let step2_style = if active_step == WizardStep::Variants {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };

        let mode_label = match self.mode {
            AppMode::SinglePhoto => "Mode A",
            AppMode::BatchQueue => "Mode B",
        };

        let breadcrumb_line = Line::from(vec![
            Span::styled(
                " LightCraft TUI ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" [{}] ", mode_label),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" | "),
            Span::styled(
                " [ASISTENTE] ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(" [1. Fotos] ", step1_style),
            Span::raw(" ❯ "),
            Span::styled(" [2. Variantes] ", step2_style),
            Span::styled(
                " ❯ [3. Proporciones] ❯ [4. Formatos] ❯ [5. Resumen] ❯ [6. Renderizado] ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(" [MCP: OK] ", Style::default().fg(Color::Green)),
        ]);

        let (guide_line, help_line) = if active_step == WizardStep::Photos {
            let photo_name = self
                .items
                .get(self.selected_index)
                .map(|i| i.filename.as_str())
                .unwrap_or("Sin fotos");
            let sel_count = self.items.iter().filter(|i| i.is_selected).count();
            (
                Line::from(vec![
                    Span::styled(
                        " Foto activa: ",
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {} ", photo_name),
                        Style::default()
                            .fg(Color::LightCyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            "[✓ {} seleccionada(s)] ({}x{}, ISO {}, {}) ",
                            sel_count,
                            self.mode_a.metadata.width,
                            self.mode_a.metadata.height,
                            self.mode_a.metadata.iso.map(|v| v.to_string()).unwrap_or_else(|| "N/A".into()),
                            self.mode_a.metadata.camera_model.as_deref().unwrap_or("General")
                        ),
                        Style::default().fg(Color::Gray),
                    ),
                ]),
                Line::from(vec![Span::styled(
                    " [↑/↓] Moverse por fotos | [Enter] Confirmar y continuar a Variantes ❯ | [Space] Selección múltiple | [E] Asistente ",
                    Style::default().fg(Color::Yellow),
                )]),
            )
        } else {
            let sel_var = self
                .mode_a
                .variants
                .iter()
                .take(self.mode_a.variant_count)
                .filter(|v| v.is_selected)
                .count();
            (
                Line::from(vec![
                    Span::styled(
                        " Matriz de Variantes: ",
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {} variante(s) seleccionadas ", sel_var),
                        Style::default()
                            .fg(Color::LightCyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" | Receta activa: {} ", self.mode_a.active_recipe.name),
                        Style::default().fg(Color::Gray),
                    ),
                ]),
                Line::from(vec![Span::styled(
                    " [←/→/↑/↓] Moverse | [Space] Alternar selección | [Enter] Promover | [E] Siguiente paso: Proporciones | [Tab] Fotos ",
                    Style::default().fg(Color::Yellow),
                )]),
            )
        };

        if area.height <= 1 {
            let p = Paragraph::new(breadcrumb_line);
            frame.render_widget(p, area);
        } else if area.height <= 3 {
            let p = Paragraph::new(vec![breadcrumb_line, help_line]);
            frame.render_widget(p, area);
        } else {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(format!(" ❯ ASISTENTE DE FLUJO: {} ❮ ", active_step.title()));
            let p = Paragraph::new(vec![breadcrumb_line, guide_line, help_line])
                .block(block)
                .wrap(Wrap { trim: false });
            frame.render_widget(p, area);
        }
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

        let breadcrumb_line = Line::from(vec![
            Span::styled(
                " [ASISTENTE] ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" [1. Fotos] ", step1_style),
            Span::raw("❯"),
            Span::styled(" [2. Variantes] ", step2_style),
            Span::raw("❯"),
            Span::styled(" [3. Proporciones] ", step3_style),
            Span::raw("❯"),
            Span::styled(" [4. Formatos] ", step4_style),
            Span::raw("❯"),
            Span::styled(" [5. Resumen] ", step5_style),
            Span::raw("❯"),
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

                    let prefix = if is_foc { "► " } else { "  " };
                    let suffix = if is_foc { " ◄" } else { "  " };
                    let mark = if is_sel { "[✓]" } else { "[ ]" };
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
                let all_prefix = if all_foc { "► " } else { "  " };
                let all_suffix = if all_foc { " ◄" } else { "  " };
                let all_mark = if all_sel { "[✓]" } else { "[ ]" };
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
                        " [←/→] Moverse | [Espacio] Seleccionar/Desmarcar [✓] | [Enter] Continuar a Formatos ❯ | [1..6/A] Atajo | [B] Volver | [Esc] Cancelar ",
                        Style::default().fg(Color::Yellow),
                    )]),
                )
            }
            WizardStep::FileFormats => {
                let format_items = [
                    (
                        "1",
                        "Solo TIFF (16-bit)",
                        self.flow_state.selected_formats == vec![crate::export::OutputFormat::Tiff],
                    ),
                    (
                        "2",
                        "Solo JPEG (Web)",
                        self.flow_state.selected_formats == vec![crate::export::OutputFormat::Jpeg],
                    ),
                    (
                        "3",
                        "Solo PNG",
                        self.flow_state.selected_formats == vec![crate::export::OutputFormat::Png],
                    ),
                    (
                        "4",
                        "TIFF + JPEG",
                        self.flow_state.selected_formats.contains(&crate::export::OutputFormat::Tiff)
                            && self.flow_state.selected_formats.contains(&crate::export::OutputFormat::Jpeg)
                            && self.flow_state.selected_formats.len() == 2,
                    ),
                    (
                        "5",
                        "Todos (TIFF+JPEG+PNG)",
                        self.flow_state.selected_formats.len() == 3,
                    ),
                ];

                let mut spans = Vec::new();
                for (idx, (key_num, label, is_sel)) in format_items.iter().enumerate() {
                    let is_foc = idx == self.flow_state.focused_format_idx;
                    let prefix = if is_foc { "► " } else { "  " };
                    let suffix = if is_foc { " ◄" } else { "  " };
                    let mark = if *is_sel { "[✓]" } else { "[ ]" };
                    let item_text = format!("{}{}[{}] {}{}", prefix, mark, key_num, label, suffix);

                    let style = if *is_sel {
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

                (
                    Line::from(spans),
                    Line::from(vec![Span::styled(
                        " [←/→] Moverse | [Espacio] Seleccionar formato [✓] | [Enter] Continuar al Resumen ❯ | [1..5] Atajo | [B] Proporciones | [Esc] Salir ",
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
                    let prefix = if is_foc { "► " } else { " " };
                    let suffix = if is_foc { " ◄" } else { " " };
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
                                " TOTAL ESTIMADO: {} foto(s) × {} variante(s) × {} proporción(es) × {} formato(s) = {} ARCHIVOS ",
                                plan.photos_count, plan.variants_count, plan.ratios_count, plan.formats_count, plan.total_output_files()
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
                    let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                    let spinner = spinner_frames[(self.flow_state.export_completed * 2) % spinner_frames.len()];
                    let pct = self.flow_state.export_percent.clamp(0.0, 100.0);
                    let filled = ((pct / 100.0) * 20.0).round() as usize;
                    let empty = 20usize.saturating_sub(filled);
                    let bar = format!("[{}{}] {:.1}%", "█".repeat(filled), "░".repeat(empty), pct);

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
            .title(format!(" ❯ {} ❮ ", step_title));

        let paragraph = Paragraph::new(vec![breadcrumb_line, info_line, help_line])
            .block(block)
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    fn render_library(&self, frame: &mut Frame, area: Rect, layout: &AppLayout) {
        let is_pane_focused = self.focused_pane == PaneId::Library;
        let block = layout.block_for(
            PaneId::Library,
            self.focused_pane,
            "1. BIBLIOTECA (Archivos RAW/Imágenes)",
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
                let prefix = if is_focused { "► " } else { "  " };
                let suffix = if is_focused { " ◄" } else { "  " };
                let flag = if is_sel { "[✓] " } else { "[ ] " };
                let raw_badge = if item.is_cr3 { " [CR3]" } else { " [IMG]" };
                let star_badge = if item.rating > 0 {
                    format!(" {}★", item.rating)
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
                    if is_focused && is_pane_focused {
                        st = st.bg(Color::Rgb(20, 50, 80));
                    }
                    st
                } else {
                    // Blanco sin selección
                    let mut st = Style::default().fg(Color::White);
                    if is_focused && is_pane_focused {
                        st = st.bg(Color::Rgb(50, 50, 50)).add_modifier(Modifier::BOLD);
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
            AppMode::SinglePhoto => {
                render_mode_a(frame, area, &self.mode_a, &self.graphics, is_focused)
            }
            AppMode::BatchQueue => render_mode_b(frame, area, &self.mode_b, is_focused),
        }
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        if self.export_prompt {
            let targets = self.mode_a.get_selected_or_focused_indices();
            let prompt_text = format!(
                " EXPORTACIÓN: {} variante(s) seleccionada(s) | [1] Solo TIFF | [2] Solo JPEG | [3 / Enter] Ambos (TIFF + JPEG) | [Esc] Cancelar ",
                targets.len()
            );
            let p = Paragraph::new(Line::from(vec![
                Span::styled(
                    " [EXPORTAR FORMATO] ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Rgb(255, 165, 0))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    prompt_text,
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            frame.render_widget(p, area);
            return;
        }

        let active_pane_str = match self.focused_pane {
            PaneId::Library => "Panel Activo: [1. BIBLIOTECA]",
            PaneId::PreviewCanvas => "Panel Activo: [2. PREVISUALIZADOR]",
            PaneId::QueueStatus => "Panel Activo: [COLA/ESTADO]",
        };
        let selected_count = format!(
            "Foto {}/{} ",
            self.selected_index + 1,
            self.items.len().max(1)
        );
        let left_part = Span::styled(
            selected_count,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
        let pane_badge = Span::styled(
            format!(" {} ", active_pane_str),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
        let right_part = Span::styled(
            format!(" {} ", self.status_message),
            Style::default().fg(Color::White).bg(Color::DarkGray),
        );

        // Variant progress badge if in VariantMatrix view
        let variant_badge = if self.mode_a.view_mode == ModeAView::VariantMatrix {
            let ready_count = self
                .mode_a
                .variants
                .iter()
                .take(self.mode_a.variant_count)
                .filter(|v| matches!(v.status, crate::tui::mode_a::VariantRenderStatus::Ready))
                .count();
            let total = self.mode_a.variant_count;
            let sel_count = self
                .mode_a
                .variants
                .iter()
                .take(total)
                .filter(|v| v.is_selected)
                .count();
            let sel_str = if sel_count > 0 {
                format!(" [Sel: {}]", sel_count)
            } else {
                String::new()
            };
            format!(" [Var: {}/{}{}]", ready_count, total, sel_str)
        } else {
            String::new()
        };

        let help_text = match (self.focused_pane, self.mode_a.view_mode) {
            (PaneId::Library, _) => " | [↑/↓/←/→]: Fotos | [Space]: Sel [X] | [Tab]: Ir a Canvas | [F5/R]: Refrescar | [E]: Exportar Max Calidad | [Q]: Salir ",
            (PaneId::PreviewCanvas, ModeAView::VariantMatrix) => " | [←/→/↑/↓]: Variantes | [Space]: Sel [X] | [F]: Fav ★ | [Enter]: Promover | [E]: Exportar Lote | [1]: Hero | [F5/R]: Refrescar | [Q]: Salir ",
            (PaneId::PreviewCanvas, _) => " | [↑/↓/←/→]: Fotos | [1]: Hero | [2/4/6]: Matriz Variantes | [S]: Split A/B | [F5/R]: Refrescar | [E]: Exportar Max Calidad | [Q]: Salir ",
            _ => " | [Tab]: Foco 1↔2 | [F5/R]: Refrescar Pantalla | [E]: Exportar Max Calidad | [Q]: Salir ",
        };

        let help_guide = Span::styled(
            format!("{}{}", variant_badge, help_text),
            Style::default().fg(Color::Yellow).bg(Color::Black),
        );

        let line = Line::from(vec![left_part, pane_badge, right_part, help_guide]);
        let p = Paragraph::new(line).wrap(Wrap { trim: true });
        frame.render_widget(p, area);
    }
}
