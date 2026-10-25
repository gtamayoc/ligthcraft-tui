//! Tier 2 Adversarial Stress Test Suite for Milestone M1
//!
//! Empirically stress-tests the `HubState` key event reducer and Launcher Hub TUI:
//! - Rapid alternating key sequences (Up/Down, Left/Right, j/k, Tab/BackTab).
//! - Boundary wrap-arounds (Up from 0, Down from 1, out-of-bounds start indices).
//! - Unknown & unhandled keys (F1-F24, PageUp/Down, Home/End, Emojis, Unicode, symbols).
//! - Modifier combinations (Ctrl+C, Ctrl+Z, Ctrl+D, Ctrl+Q, Alt, Shift, Release events).
//! - High-volume pseudo-random burst inputs (25,000 continuous mixed key events).
//! - Reference oracle state machine property verification.
//! - Extreme terminal geometry rendering stress tests (0x0 up to 300x150, fallback triggers).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use lightcraft_tui::hub::view::{render_hub, HubState, TOOL_OPTIONS};
use lightcraft_tui::hub::ToolOption;
use lightcraft_tui::tui::glyphs::GlyphSet;

/// Helper to construct a standard `KeyEvent` with press kind and no modifiers.
fn press(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// Helper to construct a `KeyEvent` with custom modifiers and press kind.
fn press_mod(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// Helper to construct a `KeyEvent` with release kind.
fn release(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Release,
        state: KeyEventState::NONE,
    }
}

/// Helper to construct a `KeyEvent` with repeat kind.
fn repeat(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Repeat,
        state: KeyEventState::NONE,
    }
}

// ============================================================================
// 1. RAPID ALTERNATING NAVIGATION SEQUENCES
// ============================================================================

#[test]
fn test_adv_m1_01_rapid_alternating_up_down_arrows() {
    let mut state = HubState::new();
    assert_eq!(state.selected_index, 0);

    const ITERATIONS: usize = 10_000;
    for i in 0..ITERATIONS {
        if i % 2 == 0 {
            // Down: from 0 -> 1
            let action = state.handle_key_event(press(KeyCode::Down));
            assert_eq!(action, None, "Navigation down must return None");
            assert_eq!(state.selected_index, 1, "Down from 0 must reach 1");
            assert_eq!(state.selected_tool(), ToolOption::SdIngest);
        } else {
            // Up: from 1 -> 0
            let action = state.handle_key_event(press(KeyCode::Up));
            assert_eq!(action, None, "Navigation up must return None");
            assert_eq!(state.selected_index, 0, "Up from 1 must reach 0");
            assert_eq!(state.selected_tool(), ToolOption::Studio);
        }
        assert!(
            state.selected_index < TOOL_OPTIONS.len(),
            "Selected index must always remain strictly < TOOL_OPTIONS.len()"
        );
    }
}

#[test]
fn test_adv_m1_02_rapid_alternating_left_right_arrows() {
    let mut state = HubState::new();

    const ITERATIONS: usize = 10_000;
    for i in 0..ITERATIONS {
        if i % 2 == 0 {
            // Right: forward 0 -> 1
            let action = state.handle_key_event(press(KeyCode::Right));
            assert_eq!(action, None);
            assert_eq!(state.selected_index, 1);
        } else {
            // Left: backward 1 -> 0
            let action = state.handle_key_event(press(KeyCode::Left));
            assert_eq!(action, None);
            assert_eq!(state.selected_index, 0);
        }
    }
}

#[test]
fn test_adv_m1_03_rapid_alternating_vim_navigation_j_k() {
    let mut state = HubState::new();

    const ITERATIONS: usize = 10_000;
    for i in 0..ITERATIONS {
        if i % 2 == 0 {
            let action = state.handle_key_event(press(KeyCode::Char('j')));
            assert_eq!(action, None);
            assert_eq!(state.selected_index, 1);
        } else {
            let action = state.handle_key_event(press(KeyCode::Char('k')));
            assert_eq!(action, None);
            assert_eq!(state.selected_index, 0);
        }
    }
}

#[test]
fn test_adv_m1_04_rapid_alternating_tabs_and_backtabs() {
    let mut state = HubState::new();

    const ITERATIONS: usize = 10_000;
    for i in 0..ITERATIONS {
        if i % 2 == 0 {
            let action = state.handle_key_event(press(KeyCode::Tab));
            assert_eq!(action, None);
            assert_eq!(state.selected_index, 1);
        } else {
            let action = state.handle_key_event(press(KeyCode::BackTab));
            assert_eq!(action, None);
            assert_eq!(state.selected_index, 0);
        }
    }
}

// ============================================================================
// 2. BOUNDARY WRAP-AROUND BEHAVIOR
// ============================================================================

#[test]
fn test_adv_m1_05_boundary_wrap_up_from_zero() {
    let mut state = HubState::new();
    assert_eq!(state.selected_index, 0);

    // Up from index 0 wraps to index 1 (SdIngest)
    let action = state.handle_key_event(press(KeyCode::Up));
    assert_eq!(action, None);
    assert_eq!(state.selected_index, 1);
    assert_eq!(state.selected_tool(), ToolOption::SdIngest);

    // Up again wraps back to index 0 (Studio)
    let action = state.handle_key_event(press(KeyCode::Up));
    assert_eq!(action, None);
    assert_eq!(state.selected_index, 0);
    assert_eq!(state.selected_tool(), ToolOption::Studio);
}

#[test]
fn test_adv_m1_06_boundary_wrap_down_from_one() {
    let mut state = HubState::new();
    state.selected_index = 1;

    // Down from index 1 wraps to index 0 (Studio)
    let action = state.handle_key_event(press(KeyCode::Down));
    assert_eq!(action, None);
    assert_eq!(state.selected_index, 0);
    assert_eq!(state.selected_tool(), ToolOption::Studio);

    // Down again wraps back to index 1 (SdIngest)
    let action = state.handle_key_event(press(KeyCode::Down));
    assert_eq!(action, None);
    assert_eq!(state.selected_index, 1);
    assert_eq!(state.selected_tool(), ToolOption::SdIngest);
}

#[test]
fn test_adv_m1_07_continuous_multi_wrap_cycles() {
    let mut state = HubState::new();

    // 500 consecutive Up arrows
    for i in 1..=500 {
        state.handle_key_event(press(KeyCode::Up));
        let expected = i % 2;
        assert_eq!(state.selected_index, expected);
    }

    // 500 consecutive Down arrows
    let current = state.selected_index;
    for i in 1..=500 {
        state.handle_key_event(press(KeyCode::Down));
        let expected = (current + i) % 2;
        assert_eq!(state.selected_index, expected);
    }
}

#[test]
fn test_adv_m1_08_out_of_bounds_recovery_and_clamping() {
    // If selected_index was set to an out-of-bounds value (e.g. 2):
    let mut state = HubState::new();
    state.selected_index = 2;

    // selected_tool uses `% TOOL_OPTIONS.len()`, so 2 % 2 = 0 (Studio)
    assert_eq!(state.selected_tool(), ToolOption::Studio);

    // Next from 2: (2 + 1) % 2 = 1 -> self-heals back into [0, 1] range
    state.select_next();
    assert_eq!(state.selected_index, 1);
    assert!(state.selected_index < TOOL_OPTIONS.len());

    // From 3: Prev from 3: 3 - 1 = 2
    state.selected_index = 3;
    state.select_prev();
    assert_eq!(state.selected_index, 2);

    // Enter while at index 2 triggers selected_tool() without panic
    state.selected_index = 2;
    let action = state.handle_key_event(press(KeyCode::Enter));
    assert_eq!(action, Some(ToolOption::Studio));

    // Direct shortcut '1' forces back to 0
    state.selected_index = 999;
    let action = state.handle_key_event(press(KeyCode::Char('1')));
    assert_eq!(action, Some(ToolOption::Studio));
    assert_eq!(state.selected_index, 0);

    // Direct shortcut '2' forces back to 1
    state.selected_index = 999;
    let action = state.handle_key_event(press(KeyCode::Char('2')));
    assert_eq!(action, Some(ToolOption::SdIngest));
    assert_eq!(state.selected_index, 1);
}

// ============================================================================
// 3. UNKNOWN AND UNHANDLED KEYS RESILIENCE
// ============================================================================

#[test]
fn test_adv_m1_09_function_keys_f1_to_f24_ignored() {
    let mut state = HubState::new();

    for f_num in 1..=24 {
        let action = state.handle_key_event(press(KeyCode::F(f_num)));
        assert_eq!(action, None, "F{} must return None", f_num);
        assert_eq!(state.selected_index, 0, "F{} must not alter selection", f_num);
    }
}

#[test]
fn test_adv_m1_10_navigation_and_paging_keys_ignored() {
    let mut state = HubState::new();
    let paging_keys = [
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Insert,
        KeyCode::Delete,
        KeyCode::Backspace,
    ];

    for &key in &paging_keys {
        let action = state.handle_key_event(press(key));
        assert_eq!(action, None, "{:?} must return None", key);
        assert_eq!(state.selected_index, 0, "{:?} must not alter selection", key);
    }
}

#[test]
fn test_adv_m1_11_unicode_and_special_characters_ignored() {
    let mut state = HubState::new();
    let unicode_chars = [
        // Accents & Latin extensions
        'á', 'é', 'í', 'ó', 'ú', 'ñ', 'ü', 'ç', 'ø', 'å', 'ß',
        // Non-Latin scripts
        'д', 'ж', 'я', 'Ω', 'α', 'ش', '中', '文', 'あ', '日',
        // Emojis & Symbols
        '🚀', '📸', '🔥', '🎉', '💾', '★', '✓', '§', '¶',
        // Whitespace and control
        ' ', '\t', '\n', '\r', '\0',
        // Non-triggering digits and punctuation
        '0', '3', '4', '5', '6', '7', '8', '9',
        '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '_', '+',
        '-', '=', '{', '}', '[', ']', '|', '\\', ':', ';', '"', '\'',
        '<', '>', ',', '.', '?', '/', '`', '~',
    ];

    for &ch in &unicode_chars {
        let action = state.handle_key_event(press(KeyCode::Char(ch)));
        assert_eq!(action, None, "Char '{}' must return None", ch);
        assert_eq!(
            state.selected_index, 0,
            "Char '{}' must not change selection",
            ch
        );
    }
}

// ============================================================================
// 4. MODIFIER COMBINATIONS & SPECIAL KEYS
// ============================================================================

#[test]
fn test_adv_m1_12_ctrl_c_terminates() {
    let mut state = HubState::new();
    let ctrl_c = press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(
        state.handle_key_event(ctrl_c),
        Some(ToolOption::Exit),
        "Ctrl+C must trigger Exit"
    );
}

#[test]
fn test_adv_m1_13_unhandled_ctrl_combinations_ignored() {
    let mut state = HubState::new();
    let ctrl_keys = ['a', 'b', 'd', 'e', 'f', 'g', 'h', 'i', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z'];

    for &ch in &ctrl_keys {
        let event = press_mod(KeyCode::Char(ch), KeyModifiers::CONTROL);
        let action = state.handle_key_event(event);
        assert_eq!(action, None, "Ctrl+{} must return None", ch);
        assert_eq!(state.selected_index, 0, "Ctrl+{} must not alter selection", ch);
    }
}

#[test]
fn test_adv_m1_14_exit_shortcut_variants() {
    let shortcuts = [
        press(KeyCode::Char('q')),
        press(KeyCode::Char('Q')),
        press(KeyCode::Esc),
        press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
        press_mod(KeyCode::Char('q'), KeyModifiers::CONTROL),
        press_mod(KeyCode::Char('Q'), KeyModifiers::SHIFT),
    ];

    for event in shortcuts {
        let mut state = HubState::new();
        assert_eq!(
            state.handle_key_event(event),
            Some(ToolOption::Exit),
            "{:?} must trigger ToolOption::Exit",
            event
        );
    }
}

#[test]
fn test_adv_m1_15_key_release_events_always_ignored() {
    let mut state = HubState::new();
    let release_events = [
        release(KeyCode::Char('1')),
        release(KeyCode::Char('2')),
        release(KeyCode::Char('q')),
        release(KeyCode::Esc),
        release(KeyCode::Enter),
        release(KeyCode::Down),
        release(KeyCode::Up),
        release(KeyCode::Tab),
    ];

    for event in release_events {
        let action = state.handle_key_event(event);
        assert_eq!(action, None, "Release event {:?} must be ignored", event);
        assert_eq!(state.selected_index, 0, "Release event must not alter state");
    }
}

#[test]
fn test_adv_m1_16_key_repeat_events_processed() {
    let mut state = HubState::new();

    // Repeat Down navigates
    let action = state.handle_key_event(repeat(KeyCode::Down));
    assert_eq!(action, None);
    assert_eq!(state.selected_index, 1);

    // Repeat Enter triggers selection
    let action = state.handle_key_event(repeat(KeyCode::Enter));
    assert_eq!(action, Some(ToolOption::SdIngest));
}

// ============================================================================
// 5. HIGH-VOLUME BURST TESTING & INVARIANT PRESERVATION
// ============================================================================

/// Fast deterministic LCG PRNG for repeatable test runs.
struct FastRng(u64);
impl FastRng {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.0 >> 32) as u32
    }
}

#[test]
fn test_adv_m1_17_high_volume_burst_stream_25000_events() {
    let mut state = HubState::new();
    let mut rng = FastRng(0xDEAD_BEEF_CAFE_BABE);

    const BURST_COUNT: usize = 25_000;
    let mut exit_triggers = 0;
    let mut studio_triggers = 0;
    let mut ingest_triggers = 0;
    let mut nav_events = 0;

    for _ in 0..BURST_COUNT {
        let choice = rng.next_u32() % 16;
        let event = match choice {
            0 => press(KeyCode::Down),
            1 => press(KeyCode::Up),
            2 => press(KeyCode::Right),
            3 => press(KeyCode::Left),
            4 => press(KeyCode::Char('j')),
            5 => press(KeyCode::Char('k')),
            6 => press(KeyCode::Tab),
            7 => press(KeyCode::BackTab),
            8 => press(KeyCode::Char('1')),
            9 => press(KeyCode::Char('2')),
            10 => press(KeyCode::Enter),
            11 => press(KeyCode::Char('q')),
            12 => press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
            13 => release(KeyCode::Down),
            14 => press(KeyCode::F((rng.next_u32() % 12 + 1) as u8)),
            _ => press(KeyCode::Char((b'a' + (rng.next_u32() % 26) as u8) as char)),
        };

        let result = state.handle_key_event(event);

        // Core Invariant 1: selected_index MUST ALWAYS be strictly < TOOL_OPTIONS.len()
        assert!(
            state.selected_index < TOOL_OPTIONS.len(),
            "Selected index out of bounds: {}",
            state.selected_index
        );

        // Core Invariant 2: selected_tool must match TOOL_OPTIONS[selected_index]
        assert_eq!(state.selected_tool(), TOOL_OPTIONS[state.selected_index]);

        // Accounting & Result Validation
        match result {
            Some(ToolOption::Studio) => studio_triggers += 1,
            Some(ToolOption::SdIngest) => ingest_triggers += 1,
            Some(ToolOption::Exit) => exit_triggers += 1,
            None => nav_events += 1,
        }
    }

    assert!(studio_triggers > 0, "Studio should have been triggered during burst");
    assert!(ingest_triggers > 0, "SdIngest should have been triggered during burst");
    assert!(exit_triggers > 0, "Exit should have been triggered during burst");
    assert!(nav_events > 0, "Navigation events should have processed");
}

// ============================================================================
// 6. REFERENCE ORACLE STATE MACHINE PROPERTY VERIFICATION
// ============================================================================

/// Reference oracle simulating the intended Hub state machine specification.
struct HubOracle {
    current_index: usize,
}

impl HubOracle {
    fn new() -> Self {
        Self { current_index: 0 }
    }

    fn step(&mut self, key: &KeyEvent) -> Option<ToolOption> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(ToolOption::Exit);
        }
        match key.code {
            KeyCode::Down | KeyCode::Right | KeyCode::Char('j') | KeyCode::Tab => {
                self.current_index = (self.current_index + 1) % 2;
                None
            }
            KeyCode::Up | KeyCode::Left | KeyCode::Char('k') | KeyCode::BackTab => {
                self.current_index = if self.current_index == 0 { 1 } else { 0 };
                None
            }
            KeyCode::Char('1') => {
                self.current_index = 0;
                Some(ToolOption::Studio)
            }
            KeyCode::Char('2') => {
                self.current_index = 1;
                Some(ToolOption::SdIngest)
            }
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => Some(ToolOption::Exit),
            KeyCode::Enter => {
                if self.current_index == 0 {
                    Some(ToolOption::Studio)
                } else {
                    Some(ToolOption::SdIngest)
                }
            }
            _ => None,
        }
    }
}

#[test]
fn test_adv_m1_18_oracle_differential_equivalence_5000_steps() {
    let mut actual_state = HubState::new();
    let mut oracle = HubOracle::new();
    let mut rng = FastRng(0x0123_4567_89AB_CDEF);

    let test_keys = [
        KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right,
        KeyCode::Char('j'), KeyCode::Char('k'), KeyCode::Tab, KeyCode::BackTab,
        KeyCode::Char('1'), KeyCode::Char('2'), KeyCode::Enter,
        KeyCode::Char('q'), KeyCode::Char('Q'), KeyCode::Esc,
        KeyCode::F(5), KeyCode::PageDown, KeyCode::Char('z'), KeyCode::Char(' '),
    ];

    for step in 0..5_000 {
        let key_idx = (rng.next_u32() as usize) % test_keys.len();
        let code = test_keys[key_idx];
        let is_release = (rng.next_u32() % 10) == 0;
        let is_ctrl = (rng.next_u32() % 15) == 0;

        let event = KeyEvent {
            code,
            modifiers: if is_ctrl { KeyModifiers::CONTROL } else { KeyModifiers::NONE },
            kind: if is_release { KeyEventKind::Release } else { KeyEventKind::Press },
            state: KeyEventState::NONE,
        };

        let actual_action = actual_state.handle_key_event(event);
        let oracle_action = oracle.step(&event);

        assert_eq!(
            actual_action, oracle_action,
            "Action mismatch at step {}: key = {:?}",
            step, event
        );
        assert_eq!(
            actual_state.selected_index, oracle.current_index,
            "Index mismatch at step {}: key = {:?}",
            step, event
        );
    }
}

// ============================================================================
// 7. TERMINAL RENDERING STRESS TESTS ACROSS EXTREME GEOMETRIES
// ============================================================================

#[test]
fn test_adv_m1_19_render_stress_boundary_terminal_dimensions() {
    let geometries = [
        // Sub-fallback geometries (< 50x14)
        (0, 0),
        (1, 1),
        (10, 5),
        (49, 13),
        (49, 20),
        (100, 13),
        // Exact fallback boundary
        (50, 14),
        (51, 15),
        // Minimum full dialog container
        (52, 16),
        // Typical terminal sizes
        (80, 24),
        (100, 19), // horizontal card layout (height < 20)
        (100, 20), // stacked card layout (height >= 20)
        (88, 26),  // max container clamp boundary
        // Ultra-wide and tall geometries
        (160, 50),
        (250, 100),
        (300, 150),
    ];

    let glyph_sets = [GlyphSet::Unicode, GlyphSet::Ascii];
    let selected_indices = [0, 1, 2, 99]; // including out-of-bounds indices

    for &(w, h) in &geometries {
        for glyph_set in glyph_sets {
            for &idx in &selected_indices {
                let backend = TestBackend::new(w, h);
                let mut terminal = Terminal::new(backend).unwrap();

                let mut state = HubState::with_glyph_set(glyph_set);
                state.selected_index = idx;

                // Rendering must never panic regardless of geometry or selection state
                let result = terminal.draw(|frame| {
                    render_hub(frame, &state);
                });
                assert!(
                    result.is_ok(),
                    "Rendering panicked or errored at geometry ({}x{}) with glyph={:?} and idx={}",
                    w, h, glyph_set, idx
                );

                // Verify buffer output contains expected text if terminal >= 50x14
                if w >= 50 && h >= 14 {
                    let content: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
                    assert!(
                        content.contains("LightCraft"),
                        "Rendered content must contain 'LightCraft' at ({}x{})",
                        w, h
                    );
                }
            }
        }
    }
}

// ============================================================================
// 8. CLI FLAG BYPASS & CONFLICT ADVERSARIAL INTEGRATION
// ============================================================================

#[test]
fn test_adv_m1_20_cli_flags_presence_and_help() {
    let mut cmd = assert_cmd::Command::cargo_bin("lightcraft-tui").expect("binary found");
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicates::str::contains("--tui"))
        .stdout(predicates::str::contains("--sd-ingest"))
        .stdout(predicates::str::contains("--verify-integrity"))
        .stdout(predicates::str::contains("--sequential-export"));
}

#[test]
fn test_adv_m1_21_cli_flag_conflict_tui_and_sd_ingest() {
    let mut cmd = assert_cmd::Command::cargo_bin("lightcraft-tui").expect("binary found");
    cmd.arg("--tui").arg("--sd-ingest");
    cmd.assert()
        .failure()
        .stderr(predicates::str::contains("cannot be used with"));
}

#[test]
fn test_adv_m1_22_cli_flag_conflict_sd_ingest_and_tui_reversed() {
    let mut cmd = assert_cmd::Command::cargo_bin("lightcraft-tui").expect("binary found");
    cmd.arg("--sd-ingest").arg("--tui");
    cmd.assert()
        .failure()
        .stderr(predicates::str::contains("cannot be used with"));
}
