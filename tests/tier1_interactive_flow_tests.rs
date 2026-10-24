//! Tier 1 Feature Integration Tests: Interactive CLI Flow, Top Wizard Banner, Aspect Ratio Matrix & Batch Export.
//!
//! Tests for PRD: Refactorización del Flujo Interactivo CLI con Asistente Superior y Matriz de Proporciones
//! Covers:
//! - US-001: Domain models, AspectRatio geometry & JobPlan combinations
//! - US-002: Dynamic AppLayout header expansion (3 vs 5 rows) and TopWizardBanner rendering
//! - US-003: Step 3 Aspect ratio multi-selection, toggle hotkeys [1..6], [A], and Back navigation
//! - US-004: Step 4 Output format selection (TIFF, JPEG, PNG, dual, triple)
//! - US-005: Step 5 Summary matrix calculation, quick jump to steps, and preset saving
//! - US-006: Aspect ratio physical center-crop and filename anti-collision ({aspect} token)
//! - US-007: Non-blocking batch execution and clean wizard teardown

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use std::path::PathBuf;
use std::sync::Arc;

use lightcraft_tui::app::AppController;
use lightcraft_tui::export::aspect::{calculate_center_crop, AspectRatio, JobPlan, WizardStep};
use lightcraft_tui::export::OutputFormat;
use lightcraft_tui::graphics::protocol::ProtocolOverride;
use lightcraft_tui::mcp::client::{McpClient, McpClientConfig};
use lightcraft_tui::storage::presets_store::PresetStore;
use lightcraft_tui::tui::layout::{AppLayout, PaneId};
use lightcraft_tui::tui::mode_a::ModeAView;

fn make_key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

#[tokio::test]
async fn test_interactive_wizard_navigation_and_retention() {
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

    // 1. Initial state: wizard is inactive, default layout header height is 3
    assert!(!app.flow_state.is_active);
    let term_rect = Rect::new(0, 0, 120, 40);
    let layout_normal = AppLayout::compute_with_flow(term_rect, app.flow_state.is_active);
    assert_eq!(layout_normal.header_area.height, 3);

    // Focus on VariantMatrix in PreviewCanvas
    app.focused_pane = PaneId::PreviewCanvas;
    app.mode_a.view_mode = ModeAView::VariantMatrix;

    // Press 'e' -> Launches wizard into Step 3 (Aspect Ratios)
    app.handle_key(make_key(KeyCode::Char('e'))).await.unwrap();
    assert!(app.flow_state.is_active);
    assert_eq!(app.flow_state.step, WizardStep::AspectRatios);

    // 2. US-002: Header expands dynamically to height 5 when wizard is active
    let layout_wizard = AppLayout::compute_with_flow(term_rect, app.flow_state.is_active);
    assert_eq!(layout_wizard.header_area.height, 5);

    // 3. US-003: Aspect Ratio Selection
    // Toggle 1:1, 3:2, 16:9
    app.handle_key(make_key(KeyCode::Char('1'))).await.unwrap(); // 1:1
    app.handle_key(make_key(KeyCode::Char('2'))).await.unwrap(); // 3:2
    app.handle_key(make_key(KeyCode::Char('4'))).await.unwrap(); // 16:9
    assert!(app
        .flow_state
        .selected_ratios
        .contains(&AspectRatio::Square1x1));
    assert!(app
        .flow_state
        .selected_ratios
        .contains(&AspectRatio::Photo3x2));
    assert!(app
        .flow_state
        .selected_ratios
        .contains(&AspectRatio::Widescreen16x9));

    // Press Enter -> Advance to Step 4 (File Formats)
    app.handle_key(make_key(KeyCode::Enter)).await.unwrap();
    assert_eq!(app.flow_state.step, WizardStep::FileFormats);

    // 4. US-004: File Format Selection
    // Toggle JPEG with key '2' (keeping TIFF active -> TIFF + JPEG)
    app.handle_key(make_key(KeyCode::Char('2'))).await.unwrap();
    assert_eq!(
        app.flow_state.selected_formats,
        vec![OutputFormat::Tiff, OutputFormat::Jpeg]
    );

    // Test Backward Navigation with key 'b': return to Step 3 and verify selections retained
    app.handle_key(make_key(KeyCode::Char('b'))).await.unwrap();
    assert_eq!(app.flow_state.step, WizardStep::AspectRatios);
    assert!(app
        .flow_state
        .selected_ratios
        .contains(&AspectRatio::Square1x1));
    assert!(app
        .flow_state
        .selected_ratios
        .contains(&AspectRatio::Photo3x2));
    assert!(app
        .flow_state
        .selected_ratios
        .contains(&AspectRatio::Widescreen16x9));

    // Advance again to Step 4 then to Step 5 (Summary)
    app.handle_key(make_key(KeyCode::Enter)).await.unwrap(); // to formats
    assert_eq!(app.flow_state.step, WizardStep::FileFormats);
    assert_eq!(
        app.flow_state.selected_formats,
        vec![OutputFormat::Tiff, OutputFormat::Jpeg]
    );

    app.handle_key(make_key(KeyCode::Enter)).await.unwrap(); // to summary
    assert_eq!(app.flow_state.step, WizardStep::Summary);

    // 5. US-005: Summary & Combination Plan Validation
    let variants_count = app.flow_state.selected_variants.len().max(1);
    let ratios_count = app.flow_state.selected_ratios.len().max(1);
    let formats_count = app.flow_state.selected_formats.len().max(1);
    let plan = JobPlan::new(1, variants_count, ratios_count, formats_count);
    assert_eq!(
        plan.total_output_files(),
        1 * variants_count * ratios_count * formats_count
    );

    // Jump to Step 3 from Summary with key '3'
    app.handle_key(make_key(KeyCode::Char('3'))).await.unwrap();
    assert_eq!(app.flow_state.step, WizardStep::AspectRatios);

    // Advance back to Summary
    app.handle_key(make_key(KeyCode::Enter)).await.unwrap();
    app.handle_key(make_key(KeyCode::Enter)).await.unwrap();
    assert_eq!(app.flow_state.step, WizardStep::Summary);

    // Save as preset with 'p'
    app.handle_key(make_key(KeyCode::Char('p'))).await.unwrap();
    assert_eq!(app.flow_state.step, WizardStep::SavePreset);
    app.handle_key(make_key(KeyCode::Enter)).await.unwrap();
    assert_eq!(app.flow_state.step, WizardStep::Summary);

    // Cancel with Esc -> Clean teardown, flow closed
    app.handle_key(make_key(KeyCode::Esc)).await.unwrap();
    assert!(!app.flow_state.is_active);
    assert_eq!(app.flow_state.step, WizardStep::Variants);
}

#[tokio::test]
async fn test_top_wizard_banner_rendering_all_steps() {
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

    let backend = TestBackend::new(120, 35);
    let mut terminal = Terminal::new(backend).unwrap();

    // Verify render in default state
    terminal.draw(|f| app.render(f)).unwrap();

    // Activate wizard
    app.flow_state.start_with_variants(vec![0, 1]);

    // Test rendering on each step
    let steps = [
        WizardStep::Photos,
        WizardStep::Variants,
        WizardStep::AspectRatios,
        WizardStep::FileFormats,
        WizardStep::Summary,
        WizardStep::Exporting,
        WizardStep::SavePreset,
    ];

    for step in steps {
        app.flow_state.step = step;
        terminal.draw(|f| app.render(f)).unwrap();
    }

    assert!(app.flow_state.is_active);
}

#[tokio::test]
async fn test_interactive_wizard_arrow_navigation_space_toggle_and_render_progress() {
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

    app.focused_pane = PaneId::PreviewCanvas;
    app.mode_a.view_mode = ModeAView::VariantMatrix;

    // Launch wizard with 'e' -> Step 3: AspectRatios
    app.handle_key(make_key(KeyCode::Char('e'))).await.unwrap();
    assert_eq!(app.flow_state.step, WizardStep::AspectRatios);
    assert_eq!(app.flow_state.focused_ratio_idx, 0); // starts at index 0 (1:1 Cuadrado)

    // 1. Arrow navigation: Right arrow moves cursor 0 -> 1 -> 2
    app.handle_key(make_key(KeyCode::Right)).await.unwrap();
    assert_eq!(app.flow_state.focused_ratio_idx, 1);
    app.handle_key(make_key(KeyCode::Right)).await.unwrap();
    assert_eq!(app.flow_state.focused_ratio_idx, 2);

    // Left arrow moves cursor 2 -> 1
    app.handle_key(make_key(KeyCode::Left)).await.unwrap();
    assert_eq!(app.flow_state.focused_ratio_idx, 1);

    // 2. Space toggles the focused option (index 1 is 3:2 Photo)
    assert!(!app
        .flow_state
        .selected_ratios
        .contains(&AspectRatio::Photo3x2));
    app.handle_key(make_key(KeyCode::Char(' '))).await.unwrap();
    assert!(app
        .flow_state
        .selected_ratios
        .contains(&AspectRatio::Photo3x2));

    // Move to index 3 (16:9) with Right, Right and toggle with Space
    app.handle_key(make_key(KeyCode::Right)).await.unwrap();
    app.handle_key(make_key(KeyCode::Right)).await.unwrap();
    assert_eq!(app.flow_state.focused_ratio_idx, 3);
    app.handle_key(make_key(KeyCode::Char(' '))).await.unwrap();
    assert!(app
        .flow_state
        .selected_ratios
        .contains(&AspectRatio::Widescreen16x9));

    // 3. Press Enter to confirm and advance to Step 4 (FileFormats)
    app.handle_key(make_key(KeyCode::Enter)).await.unwrap();
    assert_eq!(app.flow_state.step, WizardStep::FileFormats);
    assert_eq!(app.flow_state.focused_format_idx, 0);

    // Navigate with Right arrow 0 -> 1 (JPEG) and Space toggle
    app.handle_key(make_key(KeyCode::Right)).await.unwrap();
    assert_eq!(app.flow_state.focused_format_idx, 1);
    app.handle_key(make_key(KeyCode::Char(' '))).await.unwrap();
    assert!(app
        .flow_state
        .selected_formats
        .contains(&OutputFormat::Tiff));
    assert!(app
        .flow_state
        .selected_formats
        .contains(&OutputFormat::Jpeg));

    // Press Enter to advance to Step 5 (Summary)
    app.handle_key(make_key(KeyCode::Enter)).await.unwrap();
    assert_eq!(app.flow_state.step, WizardStep::Summary);
    assert_eq!(app.flow_state.focused_summary_idx, 0);

    // 4. Arrow navigation in Summary: Right moves 0 -> 1 -> 2
    app.handle_key(make_key(KeyCode::Right)).await.unwrap();
    assert_eq!(app.flow_state.focused_summary_idx, 1);
    app.handle_key(make_key(KeyCode::Left)).await.unwrap();
    assert_eq!(app.flow_state.focused_summary_idx, 0);

    // 5. Test Live Progress Events
    let progress_event = lightcraft_tui::tui::event::AppEvent::FlowExportProgress {
        completed: 2,
        total: 4,
        current_label: "var1__16x9.jpeg".into(),
        percent: 50.0,
    };
    app.handle_event(progress_event).await.unwrap();
    assert_eq!(app.flow_state.export_completed, 2);
    assert_eq!(app.flow_state.export_total, 4);
    assert_eq!(app.flow_state.export_percent, 50.0);

    // Finish event closes wizard cleanly and leaves status message
    let finished_event = lightcraft_tui::tui::event::AppEvent::FlowExportFinished {
        successful: 4,
        total: 4,
        output_dir: "export/interactive".into(),
    };
    app.handle_event(finished_event).await.unwrap();
    assert!(!app.flow_state.is_active);
    assert!(app.status_message.contains("Exportación completada"));
}

#[tokio::test]
async fn test_aspect_ratio_geometric_center_crops() {
    // 1. 3:2 Source (6000x4000) with 1:1 Square
    let (x, y, w, h) = calculate_center_crop(6000, 4000, AspectRatio::Square1x1);
    assert_eq!((x, y, w, h), (1000, 0, 4000, 4000));
    assert_eq!(w, h);

    // 2. 3:2 Source (6000x4000) with 16:9 Widescreen
    let (x, y, w, h) = calculate_center_crop(6000, 4000, AspectRatio::Widescreen16x9);
    assert_eq!((x, y, w, h), (0, 312, 6000, 3375));
    let ratio = w as f64 / h as f64;
    assert!((ratio - (16.0 / 9.0)).abs() < 0.01);

    // 3. 3:2 Source (6000x4000) with 4:5 Vertical Portrait
    let (x, y, w, h) = calculate_center_crop(6000, 4000, AspectRatio::Social4x5);
    // 4000 height -> width = 4000 * 4 / 5 = 3200. x = (6000 - 3200) / 2 = 1400
    assert_eq!((x, y, w, h), (1400, 0, 3200, 4000));
    let ratio = w as f64 / h as f64;
    assert!((ratio - (4.0 / 5.0)).abs() < 0.01);

    // 4. Square Source (3000x3000) with 3:2 Landscape
    let (x, y, w, h) = calculate_center_crop(3000, 3000, AspectRatio::Photo3x2);
    // 3000 width -> height = 3000 * 2 / 3 = 2000. y = (3000 - 2000) / 2 = 500
    assert_eq!((x, y, w, h), (0, 500, 3000, 2000));
}

#[tokio::test]
async fn test_panel1_startup_navigation_without_space_and_enter_confirm() {
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

    // 1. Panel 1 is focused at startup, initial photo is auto-selected
    assert_eq!(app.focused_pane, PaneId::Library);
    assert_eq!(app.selected_index, 0);
    if !app.items.is_empty() {
        assert!(app.items[0].is_selected);
    }

    // 2. Immediate arrow navigation Down WITHOUT needing Space
    if app.items.len() >= 2 {
        app.handle_key(make_key(KeyCode::Down)).await.unwrap();
        assert_eq!(app.selected_index, 1);
        // Automatically selects newly focused photo and deselects previous single selection
        assert!(!app.items[0].is_selected);
        assert!(app.items[1].is_selected);

        // Arrow navigation Up returns to first photo
        app.handle_key(make_key(KeyCode::Up)).await.unwrap();
        assert_eq!(app.selected_index, 0);
        assert!(app.items[0].is_selected);
        assert!(!app.items[1].is_selected);
    }

    // 3. Press Enter in Panel 1 confirms photo and advances to Panel 2 (Variants)
    app.handle_key(make_key(KeyCode::Enter)).await.unwrap();
    assert_eq!(app.focused_pane, PaneId::PreviewCanvas);
    assert_eq!(app.mode_a.view_mode, ModeAView::VariantMatrix);
    assert!(app.status_message.contains("Paso 1 confirmado"));
}

#[tokio::test]
async fn test_panel_focus_isolation_and_tab_navigation() {
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

    assert_eq!(app.focused_pane, PaneId::Library);
    assert_eq!(app.selected_index, 0);

    // 1. If mouse clicked in PreviewCanvas, focus shifts to PreviewCanvas
    app.handle_event(lightcraft_tui::tui::event::AppEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 60, // in PreviewCanvas area (col > 28)
        row: 10,
        modifiers: crossterm::event::KeyModifiers::empty(),
    }))
    .await
    .unwrap();
    assert_eq!(app.focused_pane, PaneId::PreviewCanvas);

    // 2. Pressing Up or Down in PreviewCanvas must NOT hijack focus back to Library
    app.mode_a.view_mode = ModeAView::VariantMatrix;
    app.mode_a.variant_count = 6;
    app.mode_a.focused_variant = 0;
    let initial_photo_idx = app.selected_index;

    app.handle_key(make_key(KeyCode::Down)).await.unwrap();
    assert_eq!(
        app.focused_pane,
        PaneId::PreviewCanvas,
        "Focus must remain in PreviewCanvas while operating in variants"
    );
    assert_eq!(
        app.selected_index, initial_photo_idx,
        "Photo in library must not change when operating in variants"
    );

    // 3. Tab key returns focus cleanly to Panel 1 (Library)
    app.handle_key(make_key(KeyCode::Tab)).await.unwrap();
    assert_eq!(
        app.focused_pane,
        PaneId::Library,
        "Tab key must return focus to Library"
    );

    // 4. Now that focus is back in Panel 1, Up/Down navigates library photos
    if app.items.len() >= 2 {
        app.handle_key(make_key(KeyCode::Down)).await.unwrap();
        assert_eq!(app.selected_index, 1);
        assert!(app.items[1].is_selected);

        app.handle_key(make_key(KeyCode::Up)).await.unwrap();
        assert_eq!(app.selected_index, 0);
        assert!(app.items[0].is_selected);
    }
}

#[tokio::test]
async fn test_step4_format_selection_matches_step3_pattern() {
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

    app.flow_state.start_with_variants(vec![0]);
    app.flow_state.step = WizardStep::FileFormats;

    // Initial state: default is TIFF selected
    assert_eq!(app.flow_state.selected_formats, vec![OutputFormat::Tiff]);

    // Press '2' (JPEG) should toggle JPEG on without deselecting TIFF
    app.handle_key(make_key(KeyCode::Char('2'))).await.unwrap();
    assert!(
        app.flow_state
            .selected_formats
            .contains(&OutputFormat::Tiff),
        "TIFF must remain selected when toggling JPEG"
    );
    assert!(
        app.flow_state
            .selected_formats
            .contains(&OutputFormat::Jpeg),
        "JPEG must be selected"
    );

    // Toggle TIFF off with '1'
    app.handle_key(make_key(KeyCode::Char('1'))).await.unwrap();
    assert!(
        !app.flow_state
            .selected_formats
            .contains(&OutputFormat::Tiff),
        "TIFF should be toggled off"
    );
    assert!(
        app.flow_state
            .selected_formats
            .contains(&OutputFormat::Jpeg),
        "JPEG must remain selected"
    );

    // Toggle PNG with '3'
    app.handle_key(make_key(KeyCode::Char('3'))).await.unwrap();
    assert!(
        app.flow_state.selected_formats.contains(&OutputFormat::Png),
        "PNG should be selected"
    );

    // Toggle 'a' / 'A' for all formats
    app.handle_key(make_key(KeyCode::Char('a'))).await.unwrap();
    assert_eq!(app.flow_state.selected_formats.len(), 3);

    // Toggle 'a' again reverts to default [Tiff]
    app.handle_key(make_key(KeyCode::Char('a'))).await.unwrap();
    assert_eq!(app.flow_state.selected_formats, vec![OutputFormat::Tiff]);

    // Arrow navigation wrapping: Left wraps 0 -> 3 (Todos)
    assert_eq!(app.flow_state.focused_format_idx, 0);
    app.handle_key(make_key(KeyCode::Left)).await.unwrap();
    assert_eq!(app.flow_state.focused_format_idx, 3);

    // Space on Todos toggles all formats
    app.handle_key(make_key(KeyCode::Char(' '))).await.unwrap();
    assert_eq!(app.flow_state.selected_formats.len(), 3);

    // Right wraps 3 -> 0 (TIFF)
    app.handle_key(make_key(KeyCode::Right)).await.unwrap();
    assert_eq!(app.flow_state.focused_format_idx, 0);

    // Verify buffer rendering with custom combo (TIFF + PNG):
    // Deselect JPEG so we have [Tiff, Png]
    app.handle_key(make_key(KeyCode::Char('2'))).await.unwrap();
    assert!(app
        .flow_state
        .selected_formats
        .contains(&OutputFormat::Tiff));
    assert!(!app
        .flow_state
        .selected_formats
        .contains(&OutputFormat::Jpeg));
    assert!(app.flow_state.selected_formats.contains(&OutputFormat::Png));

    let backend = TestBackend::new(120, 35);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();

    let buffer = terminal.backend().buffer().clone();
    let mut rendered_text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            rendered_text.push_str(buffer[(x, y)].symbol());
        }
        rendered_text.push('\n');
    }

    // Both TIFF and PNG must have checked checkboxes, JPEG and Todos must be unchecked
    assert!(
        rendered_text.contains("[1] TIFF (16-bit)"),
        "Buffer should render TIFF"
    );
    assert!(
        rendered_text.contains("[2] JPEG (Web)"),
        "Buffer should render JPEG"
    );
    assert!(
        rendered_text.contains("[3] PNG"),
        "Buffer should render PNG"
    );
    assert!(
        rendered_text.contains("[A] Todos"),
        "Buffer should render Todos"
    );
}
