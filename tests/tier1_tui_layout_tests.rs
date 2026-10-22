//! Tier 1 Feature Integration Tests: Responsive 4-Pane TUI Layout, Focus Ring, and Dual Operating Modes.
//!
//! Covers Milestone M3 features:
//! - F-TUI-01: Multi-Pane Ratatui Layout (Header, Library, Canvas, Inspector, Status Bar)
//! - F-TUI-02: Dual Operating Modes (A: Single Photo vs B: Batch Queue) with State Retention
//! - F-RNF-01: 60 FPS Async Event Loop & Responsive Geometry

use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use std::path::PathBuf;
use std::sync::Arc;

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

    assert!(
        !layout.is_too_small,
        "120x40 must satisfy minimum dimensions"
    );
    assert_eq!(layout.total_area, area);

    // Verify vertical division: Header, Middle, Status
    assert_eq!(layout.header_area.y, 0);
    assert_eq!(layout.header_area.height, 3);
    assert_eq!(layout.status_area.height, 3);
    assert_eq!(layout.status_area.y, 37);

    // Verify 2-column middle section: Library, Canvas
    assert!(
        layout.library_area.width > 20,
        "Library must have adequate width"
    );
    assert!(
        layout.preview_area.width > 50,
        "Preview canvas must be the primary central region"
    );

    // All horizontal widths must sum exactly to terminal width
    let total_width = layout.library_area.width + layout.preview_area.width;
    assert_eq!(total_width, 120);

    // Heights of both middle columns must match
    assert_eq!(layout.library_area.height, 34);
    assert_eq!(layout.preview_area.height, 34);
}

#[test]
fn test_t1_tui01_02_ultrawide_layout_200x50() {
    let area = Rect::new(0, 0, 200, 50);
    let layout = AppLayout::compute(area);

    assert!(!layout.is_too_small);
    // On wide (width >= 120), preview canvas occupies 80% = 160 columns, library 20% = 40
    assert_eq!(layout.preview_area.width, 160);
    assert_eq!(layout.library_area.width, 40);

    let sum = layout.library_area.width + layout.preview_area.width;
    assert_eq!(sum, 200);
}

#[test]
fn test_t1_tui01_03_subminimum_terminal_alert_overlay() {
    // Test terminal smaller than MIN_WIDTH x MIN_HEIGHT requirement
    let small_areas = [
        Rect::new(0, 0, AppLayout::MIN_WIDTH - 1, 40), // Too narrow
        Rect::new(0, 0, 120, AppLayout::MIN_HEIGHT - 1), // Too short
        Rect::new(0, 0, 40, 10),                       // Extremely tiny
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

    // Boundary check at exactly minimum threshold
    let exact_min = Rect::new(0, 0, AppLayout::MIN_WIDTH, AppLayout::MIN_HEIGHT);
    let layout_exact = AppLayout::compute(exact_min);
    assert!(
        !layout_exact.is_too_small,
        "Minimum dimensions must be accepted"
    );
    assert!(layout_exact.preview_area.width > 0);
}

#[test]
fn test_t1_tui01_04_tab_focus_ring_traversal() {
    let mut focus = PaneId::Library;

    // Forward cycle: Library -> PreviewCanvas -> Library
    focus = focus.next();
    assert_eq!(focus, PaneId::PreviewCanvas);
    focus = focus.next();
    assert_eq!(focus, PaneId::Library);

    // Reverse cycle: Library -> PreviewCanvas -> Library
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
    assert_eq!(
        layout.pane_at(lib_point.0, lib_point.1),
        Some(PaneId::Library)
    );

    // Hit test inside Preview Canvas pane
    let canvas_point = (layout.preview_area.x + 10, layout.preview_area.y + 10);
    assert_eq!(
        layout.pane_at(canvas_point.0, canvas_point.1),
        Some(PaneId::PreviewCanvas)
    );

    // Hit test inside Status Bar
    let status_point = (layout.status_area.x + 10, layout.status_area.y + 1);
    assert_eq!(
        layout.pane_at(status_point.0, status_point.1),
        Some(PaneId::QueueStatus)
    );

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
    let app = AppController::new(
        mcp,
        store,
        PathBuf::from("DRAW"),
        ProtocolOverride::Halfblocks,
    )
    .await
    .unwrap();

    assert_eq!(app.mode, AppMode::SinglePhoto);
    assert_eq!(app.focused_pane, PaneId::Library);
    assert_eq!(app.mode_a.view_mode, ModeAView::Hero);
    assert_eq!(app.selected_index, 0);
    if PathBuf::from("DRAW").exists()
        && std::fs::read_dir("DRAW")
            .map(|mut it| it.next().is_some())
            .unwrap_or(false)
    {
        assert!(
            !app.items.is_empty(),
            "Items should be discovered from DRAW directory"
        );
    }
}

#[tokio::test]
async fn test_t1_tui02_02_mode_toggle_preserves_state_and_edits() {
    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(
        mcp,
        store,
        PathBuf::from("DRAW"),
        ProtocolOverride::Halfblocks,
    )
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
    let mut app = AppController::new(
        mcp,
        store,
        PathBuf::from("DRAW"),
        ProtocolOverride::Halfblocks,
    )
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
    let app = AppController::new(
        mcp,
        store,
        PathBuf::from("DRAW"),
        ProtocolOverride::Halfblocks,
    )
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
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("LightCraft TUI"));
    assert!(content.contains("Mode A"));
    assert!(content.contains("[1] Biblioteca"));
    assert!(!content.contains("[3] Inspector"));
    assert!(content.contains("[F1 Ayuda]"));
    assert!(content.contains("[Tab Panel]"));
}

#[tokio::test]
async fn test_key_release_events_ignored_preventing_double_trigger() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(
        mcp,
        store,
        PathBuf::from("DRAW"),
        ProtocolOverride::Halfblocks,
    )
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
    assert_eq!(
        app.focused_pane, initial_pane,
        "Release event must be ignored"
    );

    // Simulate Press event of Tab -> should advance exactly 1 pane
    let press_tab = KeyEvent {
        code: KeyCode::Tab,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    };
    app.handle_key(press_tab).await.unwrap();
    assert_eq!(
        app.focused_pane,
        initial_pane.next(),
        "Press event advances 1 pane"
    );

    // Simulate Release of F10 -> should be ignored
    let release_f10 = KeyEvent {
        code: KeyCode::F(10),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Release,
        state: KeyEventState::empty(),
    };
    app.handle_key(release_f10).await.unwrap();
    assert_eq!(
        app.mode, initial_mode,
        "Release of F10 must not switch mode"
    );

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
    let mut app = AppController::new(
        mcp,
        store,
        PathBuf::from("DRAW"),
        ProtocolOverride::Halfblocks,
    )
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

    // Click inside PreviewCanvas panel
    let canvas_x = layout.preview_area.x + 2;
    let canvas_y = layout.preview_area.y + 2;
    app.handle_event(lightcraft_tui::tui::event::AppEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: canvas_x,
        row: canvas_y,
        modifiers: crossterm::event::KeyModifiers::empty(),
    }))
    .await
    .unwrap();
    assert_eq!(app.focused_pane, PaneId::PreviewCanvas);

    // Click outside in header area (y = 0) -> focus must NOT change
    app.handle_event(lightcraft_tui::tui::event::AppEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row: 0,
        modifiers: crossterm::event::KeyModifiers::empty(),
    }))
    .await
    .unwrap();
    assert_eq!(
        app.focused_pane,
        PaneId::PreviewCanvas,
        "Header click must not alter focused pane"
    );
}

#[tokio::test]
async fn test_key_e_immediate_sequential_dual_export() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(
        mcp,
        store,
        PathBuf::from("DRAW"),
        ProtocolOverride::Halfblocks,
    )
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
            app.status_message.contains("Guardado OK")
                || app.status_message.contains("Guardado inmediato OK")
                || app.status_message.contains("Exportación"),
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
            assert!(
                !entries.is_empty(),
                "Interactive export must create output files immediately"
            );
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

#[tokio::test]
async fn test_resize_event_unconditionally_triggers_force_clear_and_needs_render() {
    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(
        mcp,
        store,
        PathBuf::from("DRAW"),
        ProtocolOverride::Halfblocks,
    )
    .await
    .unwrap();

    app.terminal_size = Rect::new(0, 0, 120, 30);
    app.force_clear = false;
    app.needs_render = false;

    // Simulate resizing between two normal/valid sizes (e.g. 120x30 -> 200x50, window maximized)
    app.handle_event(lightcraft_tui::tui::event::AppEvent::Resize(200, 50))
        .await
        .unwrap();

    assert!(
        app.force_clear,
        "Resizing between valid sizes MUST unconditionally set force_clear"
    );
    assert!(app.needs_render, "Resizing must set needs_render");
    assert_eq!(app.terminal_size, Rect::new(0, 0, 200, 50));
}

#[tokio::test]
async fn test_screen_refresh_shortcuts_f5_r_ctrl_l() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(
        mcp,
        store,
        PathBuf::from("DRAW"),
        ProtocolOverride::Halfblocks,
    )
    .await
    .unwrap();

    // 1. Test F5
    app.force_clear = false;
    app.handle_key(KeyEvent {
        code: KeyCode::F(5),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert!(app.force_clear, "F5 must set force_clear to true");
    assert!(app.status_message.contains("F5"));

    // 2. Test 'r' / 'R'
    app.force_clear = false;
    app.handle_key(KeyEvent {
        code: KeyCode::Char('r'),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert!(app.force_clear, "Key 'r' must set force_clear to true");
    assert!(app.status_message.to_lowercase().contains("restablecid"));

    // 3. Test Ctrl+L
    app.force_clear = false;
    app.handle_key(KeyEvent {
        code: KeyCode::Char('l'),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert!(app.force_clear, "Ctrl+L must set force_clear to true");
}

#[tokio::test]
async fn test_arrow_keys_navigation_and_wrapping_in_library_and_canvas() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(
        mcp,
        store,
        PathBuf::from("DRAW"),
        ProtocolOverride::Halfblocks,
    )
    .await
    .unwrap();

    if app.items.len() >= 2 {
        app.selected_index = 0;
        app.focused_pane = PaneId::Library;

        // Up on index 0 should wrap to last item
        app.handle_key(KeyEvent {
            code: KeyCode::Up,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        })
        .await
        .unwrap();
        assert_eq!(
            app.selected_index,
            app.items.len() - 1,
            "Up arrow at index 0 should wrap to last item"
        );

        // Down on last item should wrap to first item
        app.handle_key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        })
        .await
        .unwrap();
        assert_eq!(
            app.selected_index, 0,
            "Down arrow at end should wrap to first item"
        );

        // Left arrow should navigate to previous item (wrapping to end)
        app.handle_key(KeyEvent {
            code: KeyCode::Left,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        })
        .await
        .unwrap();
        assert_eq!(
            app.selected_index,
            app.items.len() - 1,
            "Left arrow should navigate backward"
        );

        // Right arrow should navigate forward (wrapping to 0)
        app.handle_key(KeyEvent {
            code: KeyCode::Right,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        })
        .await
        .unwrap();
        assert_eq!(app.selected_index, 0, "Right arrow should navigate forward");

        // Focus on PreviewCanvas in Hero view -> arrows should also navigate photos
        app.focused_pane = PaneId::PreviewCanvas;
        app.mode_a.view_mode = ModeAView::Hero;

        app.handle_key(KeyEvent {
            code: KeyCode::Right,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        })
        .await
        .unwrap();
        assert_eq!(
            app.selected_index, 1,
            "Right arrow in PreviewCanvas should advance photo"
        );
    }
}

#[tokio::test]
async fn test_help_modal_toggle_and_tab_navigation() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(
        mcp,
        store,
        PathBuf::from("DRAW"),
        ProtocolOverride::Halfblocks,
    )
    .await
    .unwrap();

    assert!(!app.show_help_modal);
    assert_eq!(app.help_active_tab, 0);

    // 1. Open with F1
    app.handle_key(KeyEvent {
        code: KeyCode::F(1),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert!(app.show_help_modal);

    // 2. Tab switching with numbers 1..4 (0-indexed tabs 0..3)
    app.handle_key(KeyEvent {
        code: KeyCode::Char('2'),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert_eq!(app.help_active_tab, 1);

    app.handle_key(KeyEvent {
        code: KeyCode::Char('4'),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert_eq!(app.help_active_tab, 3);

    // 3. Tab cycling with Tab and Right
    app.handle_key(KeyEvent {
        code: KeyCode::Tab,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert_eq!(app.help_active_tab, 0, "Tab should wrap around to 0");

    app.handle_key(KeyEvent {
        code: KeyCode::Right,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert_eq!(app.help_active_tab, 1);

    // 4. Tab cycling backward with Left and BackTab
    app.handle_key(KeyEvent {
        code: KeyCode::Left,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert_eq!(app.help_active_tab, 0);

    app.handle_key(KeyEvent {
        code: KeyCode::BackTab,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert_eq!(app.help_active_tab, 3, "BackTab should wrap around to 3");

    // 5. Normal keys (like 'e' for export, or 'm' for mode) must be suppressed when modal is open
    let old_mode = app.mode;
    app.handle_key(KeyEvent {
        code: KeyCode::Char('m'),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert_eq!(
        app.mode, old_mode,
        "Mode should not change while help modal is open"
    );

    // 6. Close with Esc
    app.handle_key(KeyEvent {
        code: KeyCode::Esc,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert!(!app.show_help_modal);

    // 7. Open with '?' and close with 'q'
    app.handle_key(KeyEvent {
        code: KeyCode::Char('?'),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert!(app.show_help_modal);

    app.handle_key(KeyEvent {
        code: KeyCode::Char('q'),
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
    .await
    .unwrap();
    assert!(!app.show_help_modal);
}

#[tokio::test]
async fn test_help_modal_rendering_with_test_backend() {
    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(
        mcp,
        store,
        PathBuf::from("DRAW"),
        ProtocolOverride::Halfblocks,
    )
    .await
    .unwrap();

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();

    // Open help modal
    app.show_help_modal = true;
    app.help_active_tab = 0;

    terminal.draw(|frame| app.render(frame)).unwrap();

    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("GUÍA DE COMANDOS Y ATAJOS"));
    assert!(content.contains("[1] General"));
    assert!(content.contains("[2] Biblioteca"));
    assert!(content.contains("[3] Variantes"));
    assert!(content.contains("[4] Exportar"));
    assert!(content.contains("Cerrar"));

    // Render tab 1 (Biblioteca)
    app.help_active_tab = 1;
    terminal.draw(|frame| app.render(frame)).unwrap();
    let buffer2 = terminal.backend().buffer();
    let content2: String = buffer2.content().iter().map(|cell| cell.symbol()).collect();
    assert!(content2.contains("BIBLIOTECA DE FOTOS RAW"));

    // Render tab 3 (Exportar)
    app.help_active_tab = 3;
    terminal.draw(|frame| app.render(frame)).unwrap();
    let buffer3 = terminal.backend().buffer();
    let content3: String = buffer3.content().iter().map(|cell| cell.symbol()).collect();
    assert!(content3.contains("Exportar"));
}

#[test]
fn test_glyph_set_ascii_mode_purity() {
    use lightcraft_tui::tui::GlyphSet;

    let ascii = GlyphSet::Ascii;
    assert!(ascii.is_ascii());

    assert!(ascii.step_arrow().is_ascii());
    assert!(ascii.step_left().is_ascii());
    assert!(ascii.check_mark().is_ascii());
    assert!(ascii.check_box_checked().is_ascii());
    assert!(ascii.check_box_unchecked().is_ascii());
    assert!(ascii.favorite_star().is_ascii());
    assert!(ascii.star_empty().is_ascii());
    assert!(ascii.triangle_left().is_ascii());
    assert!(ascii.triangle_right().is_ascii());
    assert!(ascii.bullet().is_ascii());
    assert!(ascii.separator().is_ascii());
    assert!(ascii.bar_horizontal().is_ascii());
    assert!(ascii.arrow_up().is_ascii());
    assert!(ascii.arrow_down().is_ascii());
    assert!(ascii.ready_mark().is_ascii());
    assert!(ascii.error_mark().is_ascii());
    assert!(ascii.progress_spinner().is_ascii());
    assert!(ascii.pending().is_ascii());
    assert!(ascii.paused().is_ascii());

    assert!(ascii.arrow_left_right().is_ascii());
    assert!(ascii.arrow_up_down().is_ascii());
    assert!(ascii.arrow_left().is_ascii());
    assert!(ascii.arrow_right().is_ascii());
    assert!(ascii.times().is_ascii());

    for i in 0..8 {
        assert!(ascii.histogram_bar(i).is_ascii());
    }
}

#[tokio::test]
async fn test_full_render_ascii_mode_buffer_purity() {
    let mcp = Arc::new(McpClient::spawn(McpClientConfig::mock()).await.unwrap());
    let store = PresetStore::default_store(PresetStore::default_builtins()).unwrap();
    let mut app = AppController::new(
        mcp,
        store,
        PathBuf::from("DRAW"),
        ProtocolOverride::Halfblocks,
    )
    .await
    .unwrap();

    app.glyph_set = lightcraft_tui::tui::GlyphSet::Ascii;

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Draw standard app in ASCII mode
    terminal.draw(|frame| app.render(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    // Must contain ASCII panel indicators
    assert!(content.contains("> [1] Biblioteca <") || content.contains("[1] Biblioteca"));
    assert!(content.contains("[F1 Ayuda]"));
    assert!(content.contains("[Tab Panel]"));
    assert!(content.contains("[Q Salir]"));

    // Must NOT contain Unicode ornamental arrows, stars, or checks
    assert!(!content.contains('❯'));
    assert!(!content.contains('❮'));
    assert!(!content.contains('★'));
    assert!(!content.contains('✓'));

    // 2. Open help modal in ASCII mode
    app.show_help_modal = true;
    for tab in 0..4 {
        app.help_active_tab = tab;
        terminal.draw(|frame| app.render(frame)).unwrap();
        let modal_buf = terminal.backend().buffer();
        let modal_content: String = modal_buf
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(modal_content.contains("GUÍA DE COMANDOS Y ATAJOS [Modo ASCII]"));
        assert!(!modal_content.contains('❯'));
        assert!(!modal_content.contains('❮'));
        assert!(!modal_content.contains('★'));
        assert!(!modal_content.contains('✓'));
    }
}
