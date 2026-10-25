//! Interactive Ratatui interface for the LightCraft Launcher Hub.
//!
//! Provides visual cards for `[1] LightCraft Studio` and `[2] SD Ingest & Storage`,
//! plus `[Q] Salir`, arrow navigation (`Up`/`Down`/`j`/`k`/`Left`/`Right`/`Tab`),
//! direct execution shortcuts (`1`, `2`, `q`/`Q`/`Esc`/`Ctrl+C`), and a pure state reducer
//! for unit testability.

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use std::time::Duration;

use crate::hub::ToolOption;
use crate::tui::glyphs::GlyphSet;
use crate::tui::TerminalGuard;

/// The primary selectable tool options displayed as cards.
pub const TOOL_OPTIONS: [ToolOption; 2] = [ToolOption::Studio, ToolOption::SdIngest];

/// Encapsulates all state for the Launcher Hub view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubState {
    /// Currently highlighted tool index in `TOOL_OPTIONS` (0..TOOL_OPTIONS.len()).
    pub selected_index: usize,
    /// Active terminal glyph set (Unicode vs ASCII fallback).
    pub glyph_set: GlyphSet,
}

impl Default for HubState {
    fn default() -> Self {
        Self::new()
    }
}

impl HubState {
    /// Create a new HubState with default selection (Studio) and auto-detected glyph set.
    pub fn new() -> Self {
        Self {
            selected_index: 0,
            glyph_set: GlyphSet::detect(),
        }
    }

    /// Create a new HubState with an explicit glyph set (useful for deterministic tests).
    pub fn with_glyph_set(glyph_set: GlyphSet) -> Self {
        Self {
            selected_index: 0,
            glyph_set,
        }
    }

    /// Retrieve the currently selected ToolOption.
    pub fn selected_tool(&self) -> ToolOption {
        TOOL_OPTIONS[self.selected_index % TOOL_OPTIONS.len()]
    }

    /// Advance selection to the next tool option (with wrapping).
    pub fn select_next(&mut self) {
        self.selected_index = (self.selected_index + 1) % TOOL_OPTIONS.len();
    }

    /// Move selection to the previous tool option (with wrapping).
    pub fn select_prev(&mut self) {
        if self.selected_index == 0 {
            self.selected_index = TOOL_OPTIONS.len() - 1;
        } else {
            self.selected_index -= 1;
        }
    }

    /// Pure reducer function: processes a key code and returns:
    /// - `Some(ToolOption)` if a tool was triggered for launch or exit.
    /// - `None` if the event only modified navigation state or was unhandled.
    pub fn handle_key_code(&mut self, code: KeyCode) -> Option<ToolOption> {
        match code {
            // Navigation: Next option
            KeyCode::Down | KeyCode::Right | KeyCode::Char('j') | KeyCode::Tab => {
                self.select_next();
                None
            }
            // Navigation: Previous option
            KeyCode::Up | KeyCode::Left | KeyCode::Char('k') | KeyCode::BackTab => {
                self.select_prev();
                None
            }
            // Direct shortcut: [1] LightCraft Studio
            KeyCode::Char('1') => {
                self.selected_index = 0;
                Some(ToolOption::Studio)
            }
            // Direct shortcut: [2] SD Ingest & Storage
            KeyCode::Char('2') => {
                self.selected_index = 1;
                Some(ToolOption::SdIngest)
            }
            // Direct shortcut: [Q] / [Esc] Salir
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => Some(ToolOption::Exit),
            // Launch currently highlighted tool
            KeyCode::Enter => Some(self.selected_tool()),
            // Ignored / unmapped keys
            _ => None,
        }
    }

    /// Pure event reducer: handles crossterm `KeyEvent`, safely filtering out
    /// Windows `KeyEventKind::Release` events and handling Ctrl+C, then delegating to `handle_key_code`.
    pub fn handle_key_event(&mut self, key: KeyEvent) -> Option<ToolOption> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(ToolOption::Exit);
        }
        self.handle_key_code(key.code)
    }
}

/// Render the complete Launcher Hub interface into the provided Ratatui Frame.
pub fn render_hub(frame: &mut Frame, state: &HubState) {
    let area = frame.area();

    // Fallback for extremely cramped terminals
    if area.width < 50 || area.height < 14 {
        let fallback = Paragraph::new(vec![
            Line::from(Span::styled(
                "LightCraft Hub (Terminal demasiado pequeña)",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("[1] LightCraft Studio"),
            Line::from("[2] SD Ingest & Storage"),
            Line::from("[Q] Salir"),
        ])
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
        frame.render_widget(fallback, area);
        return;
    }

    // Compute centered dialog container dimensions
    let container_w = area.width.saturating_sub(4).clamp(52, 88);
    let container_h = area.height.saturating_sub(2).clamp(16, 26);
    let x = (area.width.saturating_sub(container_w)) / 2;
    let y = (area.height.saturating_sub(container_h)) / 2;
    let container_area = Rect::new(x, y, container_w, container_h);

    // Clear background
    frame.render_widget(Clear, container_area);

    // Split container vertically into: [Header, Cards, Footer]
    let vertical_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header title & subtitle
            Constraint::Min(8),    // Interactive cards
            Constraint::Length(3), // Navigation & shortcuts footer
        ])
        .split(container_area);

    // 1. Render Header
    let header_lines = vec![
        Line::from(vec![
            Span::styled(
                " LightCraft ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Launcher Hub ",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "v0.1.0",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![Span::styled(
            "Centro de Control Fotográfico & Herramientas Modulares",
            Style::default().fg(Color::Gray),
        )]),
    ];
    let header = Paragraph::new(header_lines).alignment(Alignment::Center);
    frame.render_widget(header, vertical_chunks[0]);

    // 2. Render Cards Area
    let cards_area = vertical_chunks[1];
    let is_stacked = container_h >= 20;

    let card_chunks = if is_stacked {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Ratio(1, 2),
                Constraint::Ratio(1, 2),
            ])
            .split(cards_area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 2),
                Constraint::Ratio(1, 2),
            ])
            .split(cards_area)
    };

    for (idx, &opt) in TOOL_OPTIONS.iter().enumerate() {
        let is_selected = state.selected_index == idx;
        render_tool_card(frame, card_chunks[idx], opt, is_selected, state.glyph_set);
    }

    // 3. Render Footer Bar
    let footer_lines = vec![
        Line::from(vec![
            Span::styled(
                " [1] ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Studio  ", Style::default().fg(Color::White)),
            Span::styled(
                " [2] ",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled("SD Ingest  ", Style::default().fg(Color::White)),
            Span::styled(
                " [↑/↓] ",
                Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Navegar  ", Style::default().fg(Color::White)),
            Span::styled(
                " [Enter] ",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Iniciar  ", Style::default().fg(Color::White)),
            Span::styled(
                " [Q/Esc] ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Salir", Style::default().fg(Color::White)),
        ]),
    ];
    let footer = Paragraph::new(footer_lines)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    frame.render_widget(footer, vertical_chunks[2]);
}

/// Helper function to render a single tool option card.
fn render_tool_card(
    frame: &mut Frame,
    area: Rect,
    option: ToolOption,
    is_selected: bool,
    glyph_set: GlyphSet,
) {
    let border_color = if is_selected {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let title_color = if is_selected {
        Color::Yellow
    } else {
        Color::Gray
    };

    let arrow = if is_selected {
        glyph_set.step_arrow()
    } else {
        " "
    };

    let title = format!(
        " {} [{}] {} ",
        arrow,
        option.shortcut_char(),
        option.title()
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(glyph_set.border_set(is_selected))
        .border_style(
            Style::default()
                .fg(border_color)
                .add_modifier(if is_selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        )
        .title(Span::styled(
            title,
            Style::default()
                .fg(title_color)
                .add_modifier(Modifier::BOLD),
        ));

    let mut lines = Vec::new();

    // Tagline
    lines.push(Line::from(vec![Span::styled(
        format!("  {}", option.tagline()),
        Style::default()
            .fg(if is_selected { Color::White } else { Color::Gray })
            .add_modifier(Modifier::ITALIC),
    )]));

    // Capabilities bullets
    for desc in option.descriptions() {
        let bullet = if glyph_set.is_ascii() { "-" } else { "•" };
        lines.push(Line::from(vec![
            Span::styled(
                format!("    {} ", bullet),
                Style::default().fg(if is_selected { Color::Cyan } else { Color::DarkGray }),
            ),
            Span::styled(
                *desc,
                Style::default().fg(if is_selected { Color::White } else { Color::DarkGray }),
            ),
        ]));
    }

    // Status badge and action hint
    let status_bullet = if glyph_set.is_ascii() { "*" } else { "●" };
    lines.push(Line::from(vec![
        Span::styled(
            format!("    {} Estado: ", status_bullet),
            Style::default().fg(if is_selected { Color::Green } else { Color::DarkGray }),
        ),
        Span::styled(
            option.status_text(),
            Style::default().fg(if is_selected { Color::LightGreen } else { Color::DarkGray }),
        ),
        Span::raw("  "),
        Span::styled(
            format!("[Pulsar '{}' o Enter]", option.shortcut_char()),
            Style::default()
                .fg(if is_selected { Color::Yellow } else { Color::DarkGray })
                .add_modifier(if is_selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
    ]));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

/// Runs the interactive Launcher Hub event loop on the provided TerminalGuard.
///
/// Returns `Ok(ToolOption)` indicating the user's choice, or `Err(...)` on terminal failure.
/// The `TerminalGuard` remains active in alternate screen / raw mode so that subsequent
/// tools (like LightCraft Studio or SD Ingest) can reuse the terminal with zero flicker.
pub async fn run_launcher(
    terminal: &mut TerminalGuard,
) -> Result<ToolOption, Box<dyn std::error::Error + Send + Sync>> {
    let mut state = HubState::new();
    let mut reader = EventStream::new();
    let mut tick_interval = tokio::time::interval(Duration::from_millis(16));

    // Render initial frame
    terminal
        .draw(|frame| render_hub(frame, &state))
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

    loop {
        tokio::select! {
            _ = tick_interval.tick() => {
                // Periodic tick (~16ms rate / 60 FPS) ensures responsive timers and background handling
            }
            maybe_event = reader.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) => {
                        if let Some(tool) = state.handle_key_event(key) {
                            return Ok(tool);
                        }
                        terminal
                            .draw(|frame| render_hub(frame, &state))
                            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                    }
                    Some(Ok(Event::Resize(_cols, _rows))) => {
                        let _ = terminal.clear();
                        terminal
                            .draw(|frame| render_hub(frame, &state))
                            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                    }
                    Some(Ok(_)) => {
                        // Ignore other events (mouse, focus) in launcher hub
                    }
                    Some(Err(e)) => {
                        return Err(Box::new(e));
                    }
                    None => {
                        break;
                    }
                }
            }
        }
    }

    Ok(ToolOption::Exit)
}

/// Milestone 1 placeholder for the SD Ingest module.
/// Restores the terminal and prints the status banner.
pub async fn run_ingest_stub(
    terminal: &mut TerminalGuard,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = terminal.restore();
    println!("============================================================");
    println!("  LightCraft SD Ingest & Storage Analyzer (M1 Preview)      ");
    println!("============================================================");
    println!("Módulo de Ingesta SD seleccionado.");
    println!("El backend de escaneo de unidades y EXIF se integra en M2-M5.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn make_release(code: KeyCode) -> KeyEvent {
        let mut event = KeyEvent::new(code, KeyModifiers::NONE);
        event.kind = KeyEventKind::Release;
        event
    }

    #[test]
    fn test_hub_initial_state_defaults_to_studio() {
        let state = HubState::new();
        assert_eq!(state.selected_index, 0);
        assert_eq!(state.selected_tool(), ToolOption::Studio);
    }

    #[test]
    fn test_hub_arrow_navigation_down_and_up() {
        let mut state = HubState::new();

        // Down moves to SD Ingest
        let action = state.handle_key_code(KeyCode::Down);
        assert_eq!(action, None);
        assert_eq!(state.selected_index, 1);
        assert_eq!(state.selected_tool(), ToolOption::SdIngest);

        // Up moves back to Studio
        let action = state.handle_key_code(KeyCode::Up);
        assert_eq!(action, None);
        assert_eq!(state.selected_index, 0);
        assert_eq!(state.selected_tool(), ToolOption::Studio);
    }

    #[test]
    fn test_hub_vim_navigation_j_and_k() {
        let mut state = HubState::new();

        // 'j' moves down to SD Ingest
        let action = state.handle_key_code(KeyCode::Char('j'));
        assert_eq!(action, None);
        assert_eq!(state.selected_index, 1);
        assert_eq!(state.selected_tool(), ToolOption::SdIngest);

        // 'k' moves up to Studio
        let action = state.handle_key_code(KeyCode::Char('k'));
        assert_eq!(action, None);
        assert_eq!(state.selected_index, 0);
        assert_eq!(state.selected_tool(), ToolOption::Studio);
    }

    #[test]
    fn test_hub_navigation_wrapping() {
        let mut state = HubState::new();
        assert_eq!(state.selected_index, 0);

        // Prev from 0 wraps to 1 (SD Ingest)
        state.select_prev();
        assert_eq!(state.selected_index, 1);
        assert_eq!(state.selected_tool(), ToolOption::SdIngest);

        // Next from 1 wraps to 0 (Studio)
        state.select_next();
        assert_eq!(state.selected_index, 0);
        assert_eq!(state.selected_tool(), ToolOption::Studio);
    }

    #[test]
    fn test_hub_horizontal_arrows_and_tabs() {
        let mut state = HubState::new();

        // Tab moves forward
        assert_eq!(state.handle_key_code(KeyCode::Tab), None);
        assert_eq!(state.selected_index, 1);

        // BackTab moves backward
        assert_eq!(state.handle_key_code(KeyCode::BackTab), None);
        assert_eq!(state.selected_index, 0);

        // Right moves forward
        assert_eq!(state.handle_key_code(KeyCode::Right), None);
        assert_eq!(state.selected_index, 1);

        // Left moves backward
        assert_eq!(state.handle_key_code(KeyCode::Left), None);
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn test_hub_direct_numeric_shortcut_1() {
        let mut state = HubState::new();
        state.selected_index = 1; // start on SD Ingest

        let action = state.handle_key_code(KeyCode::Char('1'));
        assert_eq!(action, Some(ToolOption::Studio));
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn test_hub_direct_numeric_shortcut_2() {
        let mut state = HubState::new();
        state.selected_index = 0; // start on Studio

        let action = state.handle_key_code(KeyCode::Char('2'));
        assert_eq!(action, Some(ToolOption::SdIngest));
        assert_eq!(state.selected_index, 1);
    }

    #[test]
    fn test_hub_exit_shortcuts_q_esc_ctrl_c() {
        let mut state = HubState::new();

        assert_eq!(state.handle_key_code(KeyCode::Char('q')), Some(ToolOption::Exit));
        assert_eq!(state.handle_key_code(KeyCode::Char('Q')), Some(ToolOption::Exit));
        assert_eq!(state.handle_key_code(KeyCode::Esc), Some(ToolOption::Exit));

        // Ctrl+C via handle_key_event
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(state.handle_key_event(ctrl_c), Some(ToolOption::Exit));
    }

    #[test]
    fn test_hub_enter_executes_current_selection() {
        let mut state = HubState::new();

        // Index 0 selected: Enter launches Studio
        assert_eq!(state.selected_index, 0);
        assert_eq!(state.handle_key_code(KeyCode::Enter), Some(ToolOption::Studio));

        // Switch to index 1: Enter launches SD Ingest
        state.handle_key_code(KeyCode::Down);
        assert_eq!(state.selected_index, 1);
        assert_eq!(state.handle_key_code(KeyCode::Enter), Some(ToolOption::SdIngest));
    }

    #[test]
    fn test_hub_unmapped_keys_ignored() {
        let mut state = HubState::new();

        assert_eq!(state.handle_key_code(KeyCode::Char('z')), None);
        assert_eq!(state.handle_key_code(KeyCode::F(1)), None);
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn test_hub_key_event_release_ignored() {
        let mut state = HubState::new();

        // Press '2' triggers launch
        let press_2 = make_press(KeyCode::Char('2'));
        assert_eq!(state.handle_key_event(press_2), Some(ToolOption::SdIngest));

        // Release event is completely ignored
        let release_1 = make_release(KeyCode::Char('1'));
        assert_eq!(state.handle_key_event(release_1), None);
        // Index remains 1, not modified by release of '1'
        assert_eq!(state.selected_index, 1);
    }

    #[test]
    fn test_hub_state_with_explicit_glyph_set() {
        let ascii_glyphs = GlyphSet::Ascii;
        let state = HubState::with_glyph_set(ascii_glyphs);
        assert_eq!(state.selected_index, 0);
        assert!(state.glyph_set.is_ascii());
    }

    #[test]
    fn test_hub_render_into_test_backend() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = HubState::new();

        terminal
            .draw(|frame| {
                render_hub(frame, &state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let content: String = buffer.content().iter().map(|c| c.symbol()).collect();

        assert!(content.contains("LightCraft"));
        assert!(content.contains("Launcher Hub"));
        assert!(content.contains("Studio"));
        assert!(content.contains("SD Ingest"));
    }
}
