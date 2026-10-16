//! Tier 1 Feature Integration Tests: Responsive 4-Pane TUI Layout, Focus Ring, and Dual Operating Modes.
//!
//! Covers Milestone M3 features:
//! - F-TUI-01: Multi-Pane Ratatui Layout (Header, Library, Canvas, Inspector, Status Bar)
//! - F-TUI-02: Dual Operating Modes (A: Single Photo vs B: Batch Queue) with State Retention
//! - F-RNF-01: 60 FPS Async Event Loop & Responsive Geometry

use std::path::PathBuf;
use std::sync::Arc;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

use lightcraft_tui::app::{AppController, AppMode};
use lightcraft_tui::graphics::protocol::ProtocolOverride;
use lightcraft_tui::mcp::client::{McpClient, McpClientConfig};
use lightcraft_tui::storage::presets_store::PresetStore;
use lightcraft_tui::tui::layout::{AppLayout, PaneId};
use lightcraft_tui::tui::mode_a::ModeAView;
use lightcraft_tui::tui::mode_b::BatchJobStatus;

// ============================================================================
// Feature F-TUI-01: Multi-Pane Ratatui Layout
// ============================================================================

#[test]
fn test_t1_tui01_01_standard_desktop_layout_120x40() {
    let area = Rect::new(0, 0, 120, 40);
    let layout = AppLayout::compute(area);

    assert!(!layout.is_too_small, "120x40 must satisfy minimum dimensions");
    assert_eq!(layout.total_area, area);

    // Verify vertical division: Header, Middle, Status
    assert_eq!(layout.header_area.y, 0);
    assert_eq!(layout.header_area.height, 3);
    assert_eq!(layout.status_area.height, 3);
    assert_eq!(layout.status_area.y, 37);

    // Verify 3-column middle section: Library, Canvas, Inspector
    assert!(layout.library_area.width > 20, "Library must have adequate width");
    assert!(layout.preview_area.width > 50, "Preview canvas must be the primary central region");
    assert!(layout.inspector_area.width > 25, "Inspector must fit develop controls");

    // All horizontal widths must sum exactly to terminal width
    let total_width = layout.library_area.width + layout.preview_area.width + layout.inspector_area.width;
    assert_eq!(total_width, 120);

    // Heights of all three middle columns must match
    assert_eq!(layout.library_area.height, 34);
    assert_eq!(layout.preview_area.height, 34);
    assert_eq!(layout.inspector_area.height, 34);
}

#[test]
fn test_t1_tui01_02_ultrawide_layout_200x50() {
    let area = Rect::new(0, 0, 200, 50);
    let layout = AppLayout::compute(area);

    assert!(!layout.is_too_small);
    // On ultra-wide (width >= 160), preview canvas occupies 55% = 110 columns
    assert_eq!(layout.preview_area.width, 110);
    assert_eq!(layout.library_area.width, 40); // 20% of 200
    assert_eq!(layout.inspector_area.width, 50); // 25% of 200

    let sum = layout.library_area.width + layout.preview_area.width + layout.inspector_area.width;
    assert_eq!(sum, 200);
}

#[test]
fn test_t1_tui01_03_subminimum_terminal_alert_overlay() {
    // Test terminal smaller than 80x24 minimum requirement
    let small_areas = [
        Rect::new(0, 0, 79, 40),  // Too narrow
        Rect::new(0, 0, 120, 23), // Too short
        Rect::new(0, 0, 40, 10),  // Extremely tiny
        Rect::new(0, 0, 10, 5),
    ];

    for area in small_areas {
        let layout = AppLayout::compute(area);
        assert!(
            layout.is_too_small,
            "Area {}x{} must trigger sub-minimum alert",
            area.width, area.height
        );
        assert_eq!(layout.preview_area.width, 0);
        assert_eq!(layout.library_area.width, 0);
    }

    // Boundary check at exactly 80x24 threshold
    let exact_min = Rect::new(0, 0, 80, 24);
    let layout_exact = AppLayout::compute(exact_min);
    assert!(!layout_exact.is_too_small, "80x24 must be accepted");
    assert!(layout_exact.preview_area.width > 0);
}

#[test]
fn test_t1_tui01_04_tab_focus_ring_traversal() {
    let mut focus = PaneId::Library;

    // Forward cycle: Library -> PreviewCanvas -> Inspector -> Library
    focus = focus.next();
    assert_eq!(focus, PaneId::PreviewCanvas);
    focus = focus.next();
    assert_eq!(focus, PaneId::Inspector);
    focus = focus.next();
    assert_eq!(focus, PaneId::Library);

    // Reverse cycle: Library -> Inspector -> PreviewCanvas -> Library
    focus = focus.prev();
    assert_eq!(focus, PaneId::Inspector);
    focus = focus.prev();
    assert_eq!(focus, PaneId::PreviewCanvas);
    focus = focus.prev();
    assert_eq!(focus, PaneId::Library);
}

#[test]
fn test_t1_tui01_05_mouse_hit_testing_panes() {
    let area = Rect::new(0, 0, 120, 40);
    let layout = AppLayout::compute(area);

    // Hit test inside Library pane
    let lib_point = (layout.library_area.x + 5, layout.library_area.y + 5);
    assert_eq!(layout.pane_at(lib_point.0, lib_point.1), Some(PaneId::Library));

    // Hit test inside Preview Canvas pane
    let canvas_point = (layout.preview_area.x + 10, layout.preview_area.y + 10);
    assert_eq!(layout.pane_at(canvas_point.0, canvas_point.1), Some(PaneId::PreviewCanvas));

    // Hit test inside Inspector pane
    let insp_point = (layout.inspector_area.x + 5, layout.inspector_area.y + 5);
    assert_eq!(layout.pane_at(insp_point.0, insp_point.1), Some(PaneId::Inspector));

    // Hit test inside Status Bar
    let status_point = (layout.status_area.x + 10, layout.status_area.y + 1);
    assert_eq!(layout.pane_at(status_point.0, status_point.1), Some(PaneId::QueueStatus));

    // Hit test out of bounds
    assert_eq!(layout.pane_at(250, 100), None);
}

// ============================================================================
// Feature F-TUI-02: Dual Operating Modes (A & B)
// ============================================================================

#[tokio::test]
async fn test_t1_tui02_01_default_launch_state_is_mode_a() {
    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let app = AppController::new(mcp, store, PathBuf::from("DRAW"), ProtocolOverride::Halfblocks)
        .await
        .unwrap();

    assert_eq!(app.mode, AppMode::SinglePhoto);
    assert_eq!(app.focused_pane, PaneId::Library);
    assert_eq!(app.mode_a.view_mode, ModeAView::Hero);
    assert_eq!(app.selected_index, 0);
    if PathBuf::from("DRAW").exists() && std::fs::read_dir("DRAW").map(|mut it| it.next().is_some()).unwrap_or(false) {
        assert!(!app.items.is_empty(), "Items should be discovered from DRAW directory");
    }
}

#[tokio::test]
async fn test_t1_tui02_02_mode_toggle_preserves_state_and_edits() {
    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(mcp, store, PathBuf::from("DRAW"), ProtocolOverride::Halfblocks)
        .await
        .unwrap();

    // 1. In Mode A, select photo index 2 and adjust intensity to 125%
    if app.items.len() >= 3 {
        app.selected_index = 2;
    }
    app.mode_a.active_intensity = 1.25;
    app.mode_a.view_mode = ModeAView::VariantMatrix;
    app.mode_a.focused_variant = 3;

    // 2. Toggle to Mode B (Batch Queue) via F10 key simulation
    let f10_event = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::F(10),
        crossterm::event::KeyModifiers::empty(),
    );
    app.handle_key(f10_event).await.unwrap();

    assert_eq!(app.mode, AppMode::BatchQueue);

    // 3. In Mode B, navigate the batch queue table
    let down_key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyModifiers::empty(),
    );
    app.handle_key(down_key).await.unwrap();

    // 4. Toggle back to Mode A
    app.handle_key(f10_event).await.unwrap();
    assert_eq!(app.mode, AppMode::SinglePhoto);

    // 5. CRITICAL STATE RETENTION ASSERTION:
    // Verify that photo selection, intensity, view mode, and focused variant survived untouched
    if app.items.len() >= 3 {
        assert_eq!(app.selected_index, 2);
    }
    assert_eq!(app.mode_a.active_intensity, 1.25);
    assert_eq!(app.mode_a.view_mode, ModeAView::VariantMatrix);
    assert_eq!(app.mode_a.focused_variant, 3);
}

#[tokio::test]
async fn test_t1_tui02_03_mode_b_queue_pause_resume_cancel() {
    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(mcp, store, PathBuf::from("DRAW"), ProtocolOverride::Halfblocks)
        .await
        .unwrap();

    app.mode = AppMode::BatchQueue;
    app.mode_b.is_running = true;
    app.mode_b.is_paused = false;

    // Initially running
    assert!(app.mode_b.is_running);
    assert!(!app.mode_b.is_paused);

    // Toggle pause with Space
    let space_key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char(' '),
        crossterm::event::KeyModifiers::empty(),
    );
    app.handle_key(space_key).await.unwrap();
    assert!(app.mode_b.is_paused);

    // Resume with Space
    app.handle_key(space_key).await.unwrap();
    assert!(!app.mode_b.is_paused);

    // Cancel batch with C
    let c_key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('c'),
        crossterm::event::KeyModifiers::empty(),
    );
    app.handle_key(c_key).await.unwrap();
    assert!(!app.mode_b.is_running);

    // Verify all pending jobs marked cancelled
    for job in &app.mode_b.queue {
        if !matches!(job.status, BatchJobStatus::Done { .. }) {
            assert_eq!(job.status, BatchJobStatus::Cancelled);
        }
    }
}

#[tokio::test]
async fn test_t1_tui02_04_ratatui_test_backend_full_render() {
    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let app = AppController::new(mcp, store, PathBuf::from("DRAW"), ProtocolOverride::Halfblocks)
        .await
        .unwrap();

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();

    // Verify full render pass produces no panics
    terminal.draw(|frame| app.render(frame)).unwrap();

    let buffer = terminal.backend().buffer();
    assert_eq!(buffer.area.width, 120);
    assert_eq!(buffer.area.height, 40);

    // Check for title content in rendered buffer
    let content: String = buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();

    assert!(content.contains("LightCraft TUI"));
    assert!(content.contains("Mode A"));
    assert!(content.contains("1. BIBLIOTECA"));
    assert!(content.contains("3. INSPECTOR"));
    assert!(content.contains("Panel Activo"));
}

#[tokio::test]
async fn test_key_release_events_ignored_preventing_double_trigger() {
    use crossterm::event::{KeyEvent, KeyEventKind, KeyEventState, KeyCode, KeyModifiers};

    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(mcp, store, PathBuf::from("DRAW"), ProtocolOverride::Halfblocks)
        .await
        .unwrap();

    let initial_pane = app.focused_pane;
    let initial_mode = app.mode;

    // Simulate Release event of Tab -> should do nothing
    let release_tab = KeyEvent {
        code: KeyCode::Tab,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Release,
        state: KeyEventState::empty(),
    };
    app.handle_key(release_tab).await.unwrap();
    assert_eq!(app.focused_pane, initial_pane, "Release event must be ignored");

    // Simulate Press event of Tab -> should advance exactly 1 pane
    let press_tab = KeyEvent {
        code: KeyCode::Tab,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    };
    app.handle_key(press_tab).await.unwrap();
    assert_eq!(app.focused_pane, initial_pane.next(), "Press event advances 1 pane");

    // Simulate Release of F10 -> should be ignored
    let release_f10 = KeyEvent {
        code: KeyCode::F(10),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Release,
        state: KeyEventState::empty(),
    };
    app.handle_key(release_f10).await.unwrap();
    assert_eq!(app.mode, initial_mode, "Release of F10 must not switch mode");

    // Simulate Press of F10 -> switches mode
    let press_f10 = KeyEvent {
        code: KeyCode::F(10),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    };
    app.handle_key(press_f10).await.unwrap();
    assert_ne!(app.mode, initial_mode, "Press of F10 switches mode");
}

#[tokio::test]
async fn test_mouse_focus_bounds_stabilized() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(mcp, store, PathBuf::from("DRAW"), ProtocolOverride::Halfblocks)
        .await
        .unwrap();

    app.terminal_size = Rect::new(0, 0, 120, 30);
    let layout = AppLayout::compute(app.terminal_size);

    // Click inside Library panel
    let lib_x = layout.library_area.x + 2;
    let lib_y = layout.library_area.y + 2;
    app.handle_event(lightcraft_tui::tui::event::AppEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: lib_x,
        row: lib_y,
        modifiers: crossterm::event::KeyModifiers::empty(),
    }))
    .await
    .unwrap();
    assert_eq!(app.focused_pane, PaneId::Library);

    // Click inside Inspector panel
    let insp_x = layout.inspector_area.x + 2;
    let insp_y = layout.inspector_area.y + 2;
    app.handle_event(lightcraft_tui::tui::event::AppEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: insp_x,
        row: insp_y,
        modifiers: crossterm::event::KeyModifiers::empty(),
    }))
    .await
    .unwrap();
    assert_eq!(app.focused_pane, PaneId::Inspector);

    // Click outside in header area (y = 0) -> focus must NOT change
    app.handle_event(lightcraft_tui::tui::event::AppEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row: 0,
        modifiers: crossterm::event::KeyModifiers::empty(),
    }))
    .await
    .unwrap();
    assert_eq!(app.focused_pane, PaneId::Inspector, "Header click must not alter focused pane");
}

#[tokio::test]
async fn test_key_e_immediate_sequential_dual_export() {
    use crossterm::event::{KeyEvent, KeyEventKind, KeyEventState, KeyCode, KeyModifiers};

    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(mcp, store, PathBuf::from("DRAW"), ProtocolOverride::Halfblocks)
        .await
        .unwrap();

    // Trigger key 'e' on selected photo
    let key_e = KeyEvent {
        code: KeyCode::Char('e'),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    };
    app.handle_key(key_e).await.unwrap();

    // Verify status message indicates immediate save when photo exists, or clean prompt when DRAW is empty
    if !app.items.is_empty() {
        assert!(
            app.status_message.contains("Guardado OK") || app.status_message.contains("Guardado inmediato OK") || app.status_message.contains("Exportación"),
            "Status message should report immediate export status: {}",
            app.status_message
        );

        // Verify output directory has files generated
        let interactive_dir = PathBuf::from("export").join("interactive");
        if interactive_dir.exists() {
            let entries: Vec<_> = std::fs::read_dir(&interactive_dir)
                .unwrap()
                .flatten()
                .collect();
            assert!(!entries.is_empty(), "Interactive export must create output files immediately");
        }
    } else {
        // In clean CI environment without RAW assets
        assert!(
            app.status_message.contains("Listo") || app.status_message.contains("Seleccionar"),
            "Status message should remain clean when no photo is loaded: {}",
            app.status_message
        );
    }
}


