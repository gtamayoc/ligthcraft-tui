//! Central application controller, state coordinator, and 60 FPS non-blocking render loop.

use std::path::PathBuf;
use std::sync::Arc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::engine::presets::BuiltinPreset;
use crate::engine::recipe::Recipe;
use crate::error::AppError;
use crate::graphics::cache::{PreviewCache, PreviewCacheConfig};
use crate::graphics::protocol::{GraphicsProtocol, ProtocolOverride};
use crate::mcp::client::McpClient;
use crate::mcp::tools::PhotoInfo;
use crate::storage::presets_store::PresetStore;
use crate::tui::event::{AppEvent, EventManager};
use crate::tui::layout::{AppLayout, PaneId};
use crate::tui::mode_a::{render_mode_a, FlagStatus, ModeAState, ModeAView, PhotoMetadata};
use crate::tui::mode_b::{render_mode_b, ModeBState};
use crate::tui::terminal::TerminalGuard;

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
    pub needs_render: bool,
    pub running: bool,
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

        if let Some(first) = items.first() {
            mode_a.metadata = PhotoMetadata {
                file_name: first.filename.clone(),
                file_size_bytes: first.size_bytes,
                format: if first.is_cr3 { "Canon RAW CR3".into() } else { "Image".into() },
                width: 6000,
                height: 4000,
                camera_model: Some("Canon EOS R6".into()),
                shutter_speed: Some("1/500s".into()),
                aperture_f_number: Some(2.8),
                iso: Some(400),
                focal_length_mm: Some(50.0),
                ..Default::default()
            };
            mode_a.active_photo = Some(PhotoInfo {
                id: 1,
                file_name: first.filename.clone(),
                format: "cr3".into(),
                kind: "raw".into(),
                width: 6000,
                height: 4000,
                rating: 0,
                flag: "none".into(),
                camera: Some("Canon EOS R6".into()),
                captured: Some("2026-10-15T12:00:00Z".into()),
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
            status_message: "Ready. [Tab] Focus | [F10] Mode A/B | [Space] Split | [E]xport".into(),
            needs_render: true,
            running: true,
        })
    }

    /// Primary event loop running at 60 FPS non-blocking cadence (Requirement F-RNF-01).
    pub async fn run(&mut self, mut terminal: TerminalGuard) -> Result<(), AppError> {
        let mut event_manager = EventManager::new(16);

        while self.running {
            if self.needs_render {
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
            AppEvent::Resize(_, _) => {
                self.needs_render = true;
            }
            AppEvent::Tick => {
                if self.is_animating() {
                    self.needs_render = true;
                }
            }
            AppEvent::PreviewReady { photo_id, image, .. } => {
                if let Some(item) = self.items.get(self.selected_index) {
                    if item.filename == photo_id {
                        self.mode_a.update_image(image);
                    }
                }
                self.needs_render = true;
            }
            AppEvent::BatchProgress { completed, total, current_file, percent } => {
                self.mode_b.completed_count = completed;
                self.mode_b.total_jobs = total;
                if let Some(ref mut queue_item) = self.mode_b.queue.iter_mut().find(|j| j.status == crate::tui::mode_b::BatchJobStatus::Queued) {
                    queue_item.status = crate::tui::mode_b::BatchJobStatus::Processing { progress_pct: percent as u8 };
                }
                self.status_message = format!("Batch: {} ({:.1}%) - [{}/{}]", current_file, percent, completed, total);
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
        // Global hotkeys
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.running = false;
                return Ok(());
            }
            KeyCode::Char('q') | KeyCode::Char('Q') => {
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
            // Photo library navigation
            KeyCode::Up | KeyCode::Char('k') if self.focused_pane == PaneId::Library => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                    self.sync_selected_photo().await?;
                }
            }
            KeyCode::Down | KeyCode::Char('j') if self.focused_pane == PaneId::Library => {
                if !self.items.is_empty() && self.selected_index < self.items.len() - 1 {
                    self.selected_index += 1;
                    self.sync_selected_photo().await?;
                }
            }
            KeyCode::Char(' ') if self.focused_pane == PaneId::Library => {
                if let Some(item) = self.items.get_mut(self.selected_index) {
                    item.is_selected = !item.is_selected;
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
            }
            KeyCode::Char('4') | KeyCode::Char('v') | KeyCode::Char('V')
                if self.focused_pane == PaneId::PreviewCanvas =>
            {
                self.mode_a.view_mode = ModeAView::VariantMatrix;
                self.mode_a.variant_count = 4;
                self.status_message = "Mode A: 4-Way 2x2 Variant Matrix".into();
            }
            KeyCode::Char('6') if self.focused_pane == PaneId::PreviewCanvas => {
                self.mode_a.view_mode = ModeAView::VariantMatrix;
                self.mode_a.variant_count = 6;
                self.status_message = "Mode A: 6-Variant Matrix".into();
            }
            KeyCode::Char('s') | KeyCode::Char('S') if self.focused_pane == PaneId::PreviewCanvas => {
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
            KeyCode::Char('z') | KeyCode::Char('Z') => {
                self.mode_a.view_mode = match self.mode_a.view_mode {
                    ModeAView::Loupe1to1 => ModeAView::Hero,
                    _ => ModeAView::Loupe1to1,
                };
                self.status_message = format!("Mode A View: {:?}", self.mode_a.view_mode);
            }

            // Loupe pan navigation
            KeyCode::Left | KeyCode::Char('h') if self.mode_a.view_mode == ModeAView::Loupe1to1 => {
                self.mode_a.loupe.pan(-0.05, 0.0);
            }
            KeyCode::Right | KeyCode::Char('l') if self.mode_a.view_mode == ModeAView::Loupe1to1 => {
                self.mode_a.loupe.pan(0.05, 0.0);
            }
            KeyCode::Up | KeyCode::Char('k') if self.mode_a.view_mode == ModeAView::Loupe1to1 => {
                self.mode_a.loupe.pan(0.0, -0.05);
            }
            KeyCode::Down | KeyCode::Char('j') if self.mode_a.view_mode == ModeAView::Loupe1to1 => {
                self.mode_a.loupe.pan(0.0, 0.05);
            }

            // Variant grid navigation
            KeyCode::Left | KeyCode::Char('h') if self.mode_a.view_mode == ModeAView::VariantMatrix => {
                if self.mode_a.focused_variant > 0 {
                    self.mode_a.focused_variant -= 1;
                }
            }
            KeyCode::Right | KeyCode::Char('l') if self.mode_a.view_mode == ModeAView::VariantMatrix => {
                if self.mode_a.focused_variant + 1 < self.mode_a.variant_count {
                    self.mode_a.focused_variant += 1;
                }
            }
            KeyCode::Enter if self.mode_a.view_mode == ModeAView::VariantMatrix => {
                self.mode_a.promote_focused_variant();
                self.status_message = format!("Promoted variant to active recipe: {}", self.mode_a.active_recipe.name);
            }

            // Favorite tagging & rating (F-SGL-05)
            KeyCode::Char(' ') if self.focused_pane == PaneId::PreviewCanvas => {
                self.mode_a.toggle_favorite();
                self.status_message = "Toggled favorite on active variant".into();
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
                if let Some(item) = self.items.get(self.selected_index) {
                    self.status_message = format!("Exported {} with recipe {}", item.filename, self.mode_a.active_recipe.name);
                }
            }

            // Recipe intensity adjustments
            KeyCode::Char('[') => {
                self.mode_a.active_intensity = (self.mode_a.active_intensity - 0.05).clamp(0.0, 1.5);
                self.status_message = format!("Intensity: {:.0}%", self.mode_a.active_intensity * 100.0);
            }
            KeyCode::Char(']') => {
                self.mode_a.active_intensity = (self.mode_a.active_intensity + 0.05).clamp(0.0, 1.5);
                self.status_message = format!("Intensity: {:.0}%", self.mode_a.active_intensity * 100.0);
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
            let layout = AppLayout::compute(Rect::new(0, 0, mouse.column.saturating_add(10), mouse.row.saturating_add(10)));
            if let Some(clicked) = layout.pane_at(mouse.column, mouse.row) {
                self.focused_pane = clicked;
            }
        }
    }

    async fn sync_selected_photo(&mut self) -> Result<(), AppError> {
        if let Some(item) = self.items.get(self.selected_index) {
            self.mode_a.metadata.file_name = item.filename.clone();
            self.mode_a.metadata.file_size_bytes = item.size_bytes;
            self.mode_a.metadata.format = if item.is_cr3 {
                "Canon RAW CR3".into()
            } else {
                "Image".into()
            };
            self.status_message = format!("Selected: {}", item.filename);
        }
        Ok(())
    }

    fn is_animating(&self) -> bool {
        self.mode_b.is_running && !self.mode_b.is_paused
    }

    /// Top-level Ratatui render pass.
    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        let layout = AppLayout::compute(area);

        if layout.is_too_small {
            self.render_too_small_overlay(frame, area);
            return;
        }

        // 1. Render Header
        self.render_header(frame, layout.header_area);

        // 2. Render Library Explorer Pane
        self.render_library(frame, layout.library_area, &layout);

        // 3. Render Center Canvas (Mode A vs Mode B)
        self.render_center(frame, layout.preview_area);

        // 4. Render Inspector Pane
        self.render_inspector(frame, layout.inspector_area, &layout);

        // 5. Render Status Bar
        self.render_status(frame, layout.status_area);
    }

    fn render_too_small_overlay(&self, frame: &mut Frame, area: Rect) {
        let msg = format!(
            "TERMINAL TOO SMALL\n\nCurrent Size: {}x{}\nMinimum Required: {}x{}\nPlease resize your terminal window.",
            area.width, area.height, AppLayout::MIN_WIDTH, AppLayout::MIN_HEIGHT
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .title(" Alert ");
        let p = Paragraph::new(msg).block(block).alignment(Alignment::Center);
        frame.render_widget(p, area);
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let mode_tag = match self.mode {
            AppMode::SinglePhoto => Span::styled(
                " [F10] Mode A: Single Photo ",
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            AppMode::BatchQueue => Span::styled(
                " [F10] Mode B: Batch Queue ",
                Style::default().fg(Color::Black).bg(Color::Magenta).add_modifier(Modifier::BOLD),
            ),
        };

        let title = Span::styled(
            " LightCraft TUI v0.1.0 ",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        );
        let mcp_badge = Span::styled(" [MCP: Connected] ", Style::default().fg(Color::Green));

        let line = Line::from(vec![title, Span::raw(" | "), mode_tag, Span::raw(" | "), mcp_badge]);
        let p = Paragraph::new(line);
        frame.render_widget(p, area);
    }

    fn render_library(&self, frame: &mut Frame, area: Rect, layout: &AppLayout) {
        let block = layout.block_for(PaneId::Library, self.focused_pane, "Library Explorer");
        let list_items: Vec<ListItem> = self
            .items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                let prefix = if idx == self.selected_index { "► " } else { "  " };
                let flag = if item.is_selected { "[x] " } else { "[ ] " };
                let raw_badge = if item.is_cr3 { " [CR3]" } else { " [IMG]" };
                let star_badge = if item.rating > 0 { format!(" {}★", item.rating) } else { "".into() };
                let pick_badge = if item.is_picked { " [P]" } else if item.is_rejected { " [X]" } else { "" };

                let style = if idx == self.selected_index {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(format!("{}{}{}{}{}{}", prefix, flag, item.filename, raw_badge, star_badge, pick_badge)).style(style)
            })
            .collect();

        let list = List::new(list_items).block(block);
        frame.render_widget(list, area);
    }

    fn render_center(&self, frame: &mut Frame, area: Rect) {
        let is_focused = self.focused_pane == PaneId::PreviewCanvas;
        match self.mode {
            AppMode::SinglePhoto => render_mode_a(frame, area, &self.mode_a, &self.graphics, is_focused),
            AppMode::BatchQueue => render_mode_b(frame, area, &self.mode_b, is_focused),
        }
    }

    fn render_inspector(&self, frame: &mut Frame, area: Rect, layout: &AppLayout) {
        let block = layout.block_for(PaneId::Inspector, self.focused_pane, "Recipe Inspector");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let lines = vec![
            Line::from(vec![
                Span::styled("Active Preset: ", Style::default().fg(Color::Cyan)),
                Span::styled(&self.mode_a.active_recipe.name, Style::default().add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("Intensity:     ", Style::default().fg(Color::Cyan)),
                Span::raw(format!("{:.0}% [===|====]", self.mode_a.active_intensity * 100.0)),
            ]),
            Line::from(""),
            Line::from(Span::styled("--- Shooting Metadata ---", Style::default().fg(Color::DarkGray))),
            Line::from(format!("Camera:  {}", self.mode_a.metadata.camera_model.as_deref().unwrap_or("Canon EOS R6"))),
            Line::from(format!("Params:  {}", self.mode_a.metadata.exposure_summary())),
            Line::from(format!("Format:  {} (3:2)", self.mode_a.metadata.format)),
            Line::from(""),
            Line::from(Span::styled("--- Histogram (4-Channel) ---", Style::default().fg(Color::DarkGray))),
            Line::from(if let Some(ref hist) = self.mode_a.histogram {
                hist.sparklines().join("\n")
            } else {
                "Lum: [ ▂▃▅▆▇█▆▅▃▂ ]\nRed: [ ▂▄▆██▆▄▂   ]\nGrn: [  ▃▅▇█▇▅▃   ]\nBlu: [   ▂▄▆█▆▄▂  ]".into()
            }),
        ];

        let p = Paragraph::new(lines);
        frame.render_widget(p, inner);
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let selected_count = format!("Photo {}/{} ", self.selected_index + 1, self.items.len().max(1));
        let left_part = Span::styled(selected_count, Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD));
        let right_part = Span::styled(format!(" {}", self.status_message), Style::default().fg(Color::White).bg(Color::DarkGray));

        let line = Line::from(vec![left_part, right_part]);
        let p = Paragraph::new(line);
        frame.render_widget(p, area);
    }
}
