//! Interactive TUI View for SD Card Ingest Setup & Automated Transfer (Milestone 5).
//!
//! Authoritative requirements:
//! - US-005 & FR-6: First-time camera detection modal prompting for target directory and saving to `~/.lightcraft/camera_registry.toml`.
//! - US-005 & FR-7: Repeat insertion recognition banner loading remembered target directory and auto-starting transfer after 2s countdown.
//! - US-007 & FR-10: 1-key `[Enter]` prompt for newly formatted empty cards matching registered capacity.
//! - R3 & FR-3: 100% Read-Only SD Protocol (zero writes to SD card).
//! - R5 & FR-11: Streaming transfer with SHA-256 verification and native Windows Toast notification upon completion.

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap};
use ratatui::Frame;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::ingest::empty_handler::{check_empty_card, format_card_capacity};
use crate::ingest::exif_fingerprint::{extract_fingerprint_from_volume, CameraFingerprint};
use crate::ingest::pipeline::{
    IngestCancellationToken, IngestPipeline, IngestProgress, IngestStatus, IngestSummary,
};
use crate::ingest::registry::{CameraProfile, CameraRegistry};
use crate::ingest::scanner::{
    crawl_media_summary, MediaSummary, SdCardVolume, VolumeScanner, WindowsVolumeScanner,
};
use crate::ingest::toast::{ToastNotifier, WindowsToastNotifier};
use crate::ingest::IngestError;
use crate::tui::glyphs::GlyphSet;
use crate::tui::TerminalGuard;

/// Active input field during first-time camera setup modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupInputField {
    /// Editable target destination directory path.
    TargetDirectory,
    /// Editable friendly display name for the camera.
    DisplayName,
}

/// Phase of the interactive SD Ingest workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestPhase {
    /// No removable drives with DCIM or matching cards were found.
    NoDrivesFound,
    /// Newly formatted empty card detected matching a known camera's capacity (US-007).
    EmptyCardPrompt {
        volume: SdCardVolume,
        matched_profile: CameraProfile,
        capacity_label: String,
    },
    /// Empty card was bound by user via `[Enter]`; waiting for photos or exit.
    EmptyCardBound {
        volume: SdCardVolume,
        bound_profile: CameraProfile,
    },
    /// Empty card with unknown capacity (no registered camera matches yet).
    EmptyCardUnknown {
        volume: SdCardVolume,
        capacity_label: String,
    },
    /// First-time detection of a camera with photos: prompts user for destination path (US-005, FR-6).
    FirstTimeSetup {
        volume: SdCardVolume,
        media_summary: MediaSummary,
        fingerprint: CameraFingerprint,
        display_name_input: String,
        target_dir_input: String,
        active_field: SetupInputField,
        validation_error: Option<String>,
    },
    /// Repeat insertion of a registered camera: shows banner and 2-second auto-start countdown (US-005, FR-7).
    RepeatAutoCountdown {
        volume: SdCardVolume,
        media_summary: MediaSummary,
        profile: CameraProfile,
        remaining_ms: u64,
    },
    /// Active streaming transfer in progress (US-006).
    Transferring {
        volume: SdCardVolume,
        profile: CameraProfile,
        progress: IngestProgress,
    },
    /// Ingest session completed successfully (FR-8, FR-11).
    Completed {
        volume: SdCardVolume,
        profile: CameraProfile,
        summary: IngestSummary,
        toast_dispatched: bool,
    },
    /// Error or cancellation state.
    Failed {
        message: String,
    },
}

/// Action returned by the pure state reducer upon key or tick events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestUiAction {
    /// Continue running UI loop without side effects.
    None,
    /// Rescan removable volumes.
    RescanVolumes,
    /// Save new profile to registry and immediately start ingest transfer.
    SaveProfileAndStartIngest {
        volume: SdCardVolume,
        profile: CameraProfile,
    },
    /// Start ingest transfer for an already registered camera profile.
    StartIngest {
        volume: SdCardVolume,
        profile: CameraProfile,
    },
    /// Cancel an in-flight transfer.
    CancelTransfer,
    /// Exit the SD Ingest view.
    Exit,
}

/// Complete state container for the SD Ingest TUI screen.
#[derive(Debug, Clone)]
pub struct IngestSetupState {
    pub phase: IngestPhase,
    pub glyph_set: GlyphSet,
}

impl IngestSetupState {
    /// Construct initial state by evaluating scanned volumes against the loaded camera registry.
    pub fn from_volumes_and_registry(
        volumes: Vec<SdCardVolume>,
        registry: &mut CameraRegistry,
        glyph_set: GlyphSet,
    ) -> Self {
        let Some(volume) = volumes.into_iter().next() else {
            return Self {
                phase: IngestPhase::NoDrivesFound,
                glyph_set,
            };
        };

        Self::evaluate_volume(volume, registry, glyph_set)
    }

    /// Evaluate a single detected volume and transition to the appropriate workflow phase.
    pub fn evaluate_volume(
        volume: SdCardVolume,
        registry: &mut CameraRegistry,
        glyph_set: GlyphSet,
    ) -> Self {
        // 1. Check if this is a newly formatted empty card (US-007, FR-10)
        if check_empty_card(&volume) {
            let capacity_label = format_card_capacity(volume.total_bytes);
            if let Some(matched) = registry.find_by_capacity(volume.total_bytes).cloned() {
                return Self {
                    phase: IngestPhase::EmptyCardPrompt {
                        volume,
                        matched_profile: matched,
                        capacity_label,
                    },
                    glyph_set,
                };
            } else {
                return Self {
                    phase: IngestPhase::EmptyCardUnknown {
                        volume,
                        capacity_label,
                    },
                    glyph_set,
                };
            }
        }

        let media_summary = crawl_media_summary(&volume.camera_folders);

        // 2. Extract EXIF camera fingerprint in <50ms without writing to SD card (US-003, R3)
        let fingerprint = match extract_fingerprint_from_volume(&volume) {
            Ok(fp) => fp,
            Err(_) => {
                // Fallback fingerprint using volume label if no readable EXIF header exists
                let fallback_model = if volume.volume_label.trim().is_empty() {
                    format!("SD_Drive_{}", volume.drive_letter)
                } else {
                    volume.volume_label.trim().to_string()
                };
                CameraFingerprint::new("Generic", fallback_model, None, None)
            }
        };

        let camera_id = fingerprint.camera_id();

        // 3. Lookup camera in local registry (US-004, US-005)
        if let Some(existing) = registry.get_profile(&camera_id).cloned() {
            let mut updated_profile = existing;
            if volume.total_bytes > 0
                && !updated_profile
                    .known_card_capacities
                    .contains(&volume.total_bytes)
            {
                updated_profile.add_card_capacity(volume.total_bytes);
                let _ = registry.register_or_update(updated_profile.clone());
            }

            Self {
                phase: IngestPhase::RepeatAutoCountdown {
                    volume,
                    media_summary,
                    profile: updated_profile,
                    remaining_ms: 2000,
                },
                glyph_set,
            }
        } else {
            // First time seeing this camera! Prompt user for destination folder (FR-6)
            let default_display = if fingerprint.model != "Unknown Model" {
                fingerprint.model.clone()
            } else {
                format!("Camera {}", camera_id)
            };

            let sanitized_folder = camera_id
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '_' || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect::<String>();

            let default_target = dirs::picture_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| PathBuf::from("."))
                .join("LightCraft_Ingest")
                .join(sanitized_folder)
                .to_string_lossy()
                .to_string();

            Self {
                phase: IngestPhase::FirstTimeSetup {
                    volume,
                    media_summary,
                    fingerprint,
                    display_name_input: default_display,
                    target_dir_input: default_target,
                    active_field: SetupInputField::TargetDirectory,
                    validation_error: None,
                },
                glyph_set,
            }
        }
    }

    /// Process a keyboard event (pure state reducer).
    pub fn handle_key_event(&mut self, key: KeyEvent) -> IngestUiAction {
        if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
            return IngestUiAction::None;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if matches!(self.phase, IngestPhase::Transferring { .. }) {
                return IngestUiAction::CancelTransfer;
            }
            return IngestUiAction::Exit;
        }

        self.handle_key_code(key.code)
    }

    /// Process a key code and return the resulting action.
    pub fn handle_key_code(&mut self, code: KeyCode) -> IngestUiAction {
        match &mut self.phase {
            IngestPhase::NoDrivesFound | IngestPhase::EmptyCardUnknown { .. } => match code {
                KeyCode::Char('r') | KeyCode::Char('R') | KeyCode::F(5) => {
                    IngestUiAction::RescanVolumes
                }
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => IngestUiAction::Exit,
                _ => IngestUiAction::None,
            },

            IngestPhase::EmptyCardPrompt {
                volume,
                matched_profile,
                ..
            } => match code {
                KeyCode::Enter => {
                    let vol = volume.clone();
                    let prof = matched_profile.clone();
                    self.phase = IngestPhase::EmptyCardBound {
                        volume: vol,
                        bound_profile: prof,
                    };
                    IngestUiAction::None
                }
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                    self.phase = IngestPhase::NoDrivesFound;
                    IngestUiAction::None
                }
                _ => IngestUiAction::None,
            },

            IngestPhase::EmptyCardBound { .. } => match code {
                KeyCode::Char('r') | KeyCode::Char('R') => IngestUiAction::RescanVolumes,
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Enter => {
                    IngestUiAction::Exit
                }
                _ => IngestUiAction::None,
            },

            IngestPhase::FirstTimeSetup {
                volume,
                fingerprint,
                display_name_input,
                target_dir_input,
                active_field,
                validation_error,
                ..
            } => match code {
                KeyCode::Tab | KeyCode::Up | KeyCode::Down => {
                    *active_field = match *active_field {
                        SetupInputField::TargetDirectory => SetupInputField::DisplayName,
                        SetupInputField::DisplayName => SetupInputField::TargetDirectory,
                    };
                    IngestUiAction::None
                }
                KeyCode::Backspace => {
                    *validation_error = None;
                    match *active_field {
                        SetupInputField::TargetDirectory => {
                            target_dir_input.pop();
                        }
                        SetupInputField::DisplayName => {
                            display_name_input.pop();
                        }
                    }
                    IngestUiAction::None
                }
                KeyCode::Char(c) => {
                    *validation_error = None;
                    match *active_field {
                        SetupInputField::TargetDirectory => target_dir_input.push(c),
                        SetupInputField::DisplayName => display_name_input.push(c),
                    }
                    IngestUiAction::None
                }
                KeyCode::Enter => {
                    let trimmed_dir = target_dir_input.trim();
                    if trimmed_dir.is_empty() {
                        *validation_error = Some(
                            "La ruta de destino no puede estar vacía. Ingresa una carpeta válida."
                                .to_string(),
                        );
                        return IngestUiAction::None;
                    }

                    // Ensure user didn't point destination onto the read-only SD card itself
                    let sd_root = volume.root_path();
                    let target_path = PathBuf::from(trimmed_dir);
                    if target_path.starts_with(&sd_root) {
                        *validation_error = Some(
                            "Prohibido guardar dentro de la misma tarjeta SD (Modo 100% Solo Lectura)."
                                .to_string(),
                        );
                        return IngestUiAction::None;
                    }

                    let final_name = if display_name_input.trim().is_empty() {
                        fingerprint.model.clone()
                    } else {
                        display_name_input.trim().to_string()
                    };

                    let profile = CameraProfile::new(
                        fingerprint.camera_id(),
                        final_name,
                        target_path,
                    )
                    .with_capacity(volume.total_bytes);

                    IngestUiAction::SaveProfileAndStartIngest {
                        volume: volume.clone(),
                        profile,
                    }
                }
                KeyCode::Esc => IngestUiAction::Exit,
                _ => IngestUiAction::None,
            },

            IngestPhase::RepeatAutoCountdown {
                volume, profile, ..
            } => match code {
                KeyCode::Enter => IngestUiAction::StartIngest {
                    volume: volume.clone(),
                    profile: profile.clone(),
                },
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => IngestUiAction::Exit,
                _ => IngestUiAction::None,
            },

            IngestPhase::Transferring { .. } => match code {
                KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('C') => {
                    IngestUiAction::CancelTransfer
                }
                _ => IngestUiAction::None,
            },

            IngestPhase::Completed { .. } | IngestPhase::Failed { .. } => match code {
                KeyCode::Char('r') | KeyCode::Char('R') => IngestUiAction::RescanVolumes,
                KeyCode::Esc
                | KeyCode::Enter
                | KeyCode::Char('q')
                | KeyCode::Char('Q') => IngestUiAction::Exit,
                _ => IngestUiAction::None,
            },
        }
    }

    /// Advance countdown timer by `elapsed_ms`. Returns `StartIngest` when countdown reaches 0.
    pub fn tick_countdown(&mut self, elapsed_ms: u64) -> IngestUiAction {
        if let IngestPhase::RepeatAutoCountdown {
            volume,
            profile,
            remaining_ms,
            ..
        } = &mut self.phase
        {
            *remaining_ms = remaining_ms.saturating_sub(elapsed_ms);
            if *remaining_ms == 0 {
                return IngestUiAction::StartIngest {
                    volume: volume.clone(),
                    profile: profile.clone(),
                };
            }
        }
        IngestUiAction::None
    }
}

/// Render the SD Ingest Setup & Progress interface into a Ratatui frame.
pub fn render_ingest_setup(frame: &mut Frame, state: &IngestSetupState) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_set(state.glyph_set.border_set(true))
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " LightCraft SD Ingest & Storage Hub (100% Read-Only SD) ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    match &state.phase {
        IngestPhase::NoDrivesFound => {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No se detectaron tarjetas SD ni unidades extraíbles con estructura DCIM.",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from("Inserta una memoria SD en el lector de tu PC y presiona [R] para escanear."),
                Line::from("El sistema opera en modo 100% Solo Lectura sobre la tarjeta física."),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[R] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw("Re-escanear unidades    "),
                    Span::styled("[Esc / Q] ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::raw("Salir"),
                ]),
            ];
            let p = Paragraph::new(lines)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            frame.render_widget(p, inner);
        }

        IngestPhase::EmptyCardPrompt {
            volume,
            matched_profile,
            capacity_label,
        } => {
            let prompt_banner = format!(
                "[TARJETA VACÍA DETECTADA: {}] ¿Pertenece a {}? [Enter = Sí / Esc = Ignorar]",
                capacity_label, matched_profile.display_name
            );
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    prompt_banner,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(format!(
                    "Unidad: {}:\\ ({}) | Formato: {}",
                    volume.drive_letter, volume.volume_label, volume.file_system
                )),
                Line::from(format!(
                    "Cámara registrada coincidente: {} (ID: {})",
                    matched_profile.display_name, matched_profile.camera_id
                )),
                Line::from(format!(
                    "Carpeta de destino vinculada: {}",
                    matched_profile.target_directory.display()
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "[Enter] ",
                        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("Confirmar asociación    "),
                    Span::styled(
                        "[Esc] ",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("Ignorar"),
                ]),
            ];
            let p = Paragraph::new(lines)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            frame.render_widget(p, inner);
        }

        IngestPhase::EmptyCardBound {
            volume,
            bound_profile,
        } => {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!(
                        "Tarjeta {}: vinculada a {}",
                        volume.drive_letter, bound_profile.display_name
                    ),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(format!(
                    "Destino configurado listo para cuando captures fotos: {}",
                    bound_profile.target_directory.display()
                )),
                Line::from("Modo 100% Solo Lectura mantenido (0 bytes escritos en la tarjeta SD)."),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "[Enter / Esc] ",
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("Finalizar"),
                ]),
            ];
            let p = Paragraph::new(lines)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            frame.render_widget(p, inner);
        }

        IngestPhase::EmptyCardUnknown {
            volume,
            capacity_label,
        } => {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!(
                        "Tarjeta recién formateada vacía detectada ({}:\\ - {})",
                        volume.drive_letter, capacity_label
                    ),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from("Aún no hay una cámara registrada con esta capacidad."),
                Line::from("Toma al menos 1 foto con tu cámara e inserta la tarjeta para registrarla automáticamente por EXIF."),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[R] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw("Re-escanear    "),
                    Span::styled("[Esc / Q] ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::raw("Salir"),
                ]),
            ];
            let p = Paragraph::new(lines)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            frame.render_widget(p, inner);
        }

        IngestPhase::FirstTimeSetup {
            volume,
            media_summary,
            fingerprint,
            display_name_input,
            target_dir_input,
            active_field,
            validation_error,
        } => {
            let serial_display = fingerprint
                .body_serial
                .as_deref()
                .unwrap_or("Fallback Modelo");

            let dir_style = if *active_field == SetupInputField::TargetDirectory {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let name_style = if *active_field == SetupInputField::DisplayName {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let mut lines = vec![
                Line::from(Span::styled(
                    format!(
                        "Nueva cámara detectada: {} (Serial: {})",
                        fingerprint.model, serial_display
                    ),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(format!(
                    "Unidad {}:\\ ({}) | {} RAWs, {} JPEGs, {} Videos ({:.1} MB)",
                    volume.drive_letter,
                    format_card_capacity(volume.total_bytes),
                    media_summary.raw_count,
                    media_summary.jpeg_count,
                    media_summary.video_count,
                    media_summary.total_media_bytes as f64 / 1_048_576.0
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Configuración de Primera Inserción (se recordará automáticamente en el futuro):",
                    Style::default().fg(Color::Yellow),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("1. Carpeta de Destino: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(format!(" {} ", target_dir_input), dir_style),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("2. Nombre de Cámara:   ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(format!(" {} ", display_name_input), name_style),
                ]),
                Line::from(""),
            ];

            if let Some(err) = validation_error {
                lines.push(Line::from(Span::styled(
                    format!("⚠ {}", err),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
            }

            lines.push(Line::from(vec![
                Span::styled("[Tab/↑/↓] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw("Cambiar campo   "),
                Span::styled("[Enter] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::raw("Guardar ruta e iniciar ingesta   "),
                Span::styled("[Esc] ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::raw("Cancelar"),
            ]));

            let p = Paragraph::new(lines).wrap(Wrap { trim: false });
            frame.render_widget(p, inner);
        }

        IngestPhase::RepeatAutoCountdown {
            volume,
            media_summary,
            profile,
            remaining_ms,
        } => {
            let secs = (*remaining_ms as f64 / 1000.0).ceil() as u64;
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!(
                        "Cámara reconocida: {}. Destino: {}",
                        profile.display_name,
                        profile.target_directory.display()
                    ),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(format!(
                    "Unidad {}:\\ | Serial ID: {} | {} archivos detectados ({:.1} MB)",
                    volume.drive_letter,
                    profile.camera_id,
                    media_summary.total_count(),
                    media_summary.total_media_bytes as f64 / 1_048_576.0
                )),
                Line::from(format!(
                    "Ruta guardada cargada automáticamente desde camera_registry.toml"
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!(
                        "Iniciando transferencia automática en {}s...",
                        secs
                    ),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "[Enter] ",
                        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("Iniciar ahora mismo    "),
                    Span::styled(
                        "[Esc] ",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("Cancelar"),
                ]),
            ];
            let p = Paragraph::new(lines)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            frame.render_widget(p, inner);
        }

        IngestPhase::Transferring {
            profile, progress, ..
        } => {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(6),
                    Constraint::Length(3),
                    Constraint::Min(2),
                ])
                .split(inner);

            let info_lines = vec![
                Line::from(Span::styled(
                    format!("Transfiriendo fotos de {} (100% Read-Only SD)", profile.display_name),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )),
                Line::from(format!(
                    "Destino: {}",
                    profile.target_directory.display()
                )),
                Line::from(format!(
                    "Archivo actual ({}/{}): {}",
                    progress.current_file_index, progress.total_files, progress.current_file
                )),
                Line::from(format!(
                    "Duplicados omitidos: {} | Verificación SHA-256 en vuelo activa",
                    progress.skipped_duplicates
                )),
            ];
            frame.render_widget(Paragraph::new(info_lines), chunks[0]);

            let ratio = if progress.total_expected_bytes > 0 {
                (progress.total_bytes_transferred as f64 / progress.total_expected_bytes as f64)
                    .clamp(0.0, 1.0)
            } else if progress.total_files > 0 {
                (progress.current_file_index as f64 / progress.total_files as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };

            let gauge = Gauge::default()
                .block(Block::default().borders(Borders::ALL).title(" Progreso "))
                .gauge_style(Style::default().fg(Color::Green))
                .ratio(ratio);
            frame.render_widget(gauge, chunks[1]);

            let footer = Paragraph::new(Line::from(vec![
                Span::styled("[Esc] ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::raw("Cancelar transferencia de forma segura (limpia archivos .tmp)"),
            ]));
            frame.render_widget(footer, chunks[2]);
        }

        IngestPhase::Completed {
            profile,
            summary,
            toast_dispatched,
            ..
        } => {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "INGESTA COMPLETADA CON ÉXITO (SHA-256 VERIFICADO)",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(format!("Cámara:                {}", profile.display_name)),
                Line::from(format!(
                    "Carpeta de destino:    {}",
                    profile.target_directory.display()
                )),
                Line::from(format!(
                    "Fotos transferidas:    {}",
                    summary.transferred_files
                )),
                Line::from(format!(
                    "Duplicados omitidos:   {}",
                    summary.skipped_duplicates
                )),
                Line::from(format!(
                    "Volumen transferido:   {:.2} MB",
                    summary.total_bytes as f64 / 1_048_576.0
                )),
                Line::from(format!(
                    "Manifiesto de sesión:  {}",
                    summary.manifest_path.display()
                )),
                Line::from(format!(
                    "Notificación Toast OS: {}",
                    if *toast_dispatched {
                        "Enviada"
                    } else {
                        "Omitida"
                    }
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[R] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::raw("Escanear otra tarjeta    "),
                    Span::styled("[Enter / Esc / Q] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw("Finalizar"),
                ]),
            ];
            let p = Paragraph::new(lines).wrap(Wrap { trim: false });
            frame.render_widget(p, inner);
        }

        IngestPhase::Failed { message } => {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Operación detenida o con error:",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(message.as_str()),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[R] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::raw("Reintentar escaneo    "),
                    Span::styled("[Esc / Q] ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::raw("Salir"),
                ]),
            ];
            let p = Paragraph::new(lines)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            frame.render_widget(p, inner);
        }
    }
}

/// Launch the interactive SD Ingest TUI workflow.
///
/// If invoked in a non-interactive / piped headless environment (such as CLI smoke tests),
/// prints the status header and exits cleanly without locking the terminal.
pub async fn run_ingest_tui(terminal: &mut TerminalGuard) -> Result<(), IngestError> {
    if !std::io::stdout().is_terminal() {
        let _ = terminal.restore();
        println!("============================================================");
        println!("  LightCraft SD Ingest & Storage Analyzer                   ");
        println!("============================================================");
        println!("Módulo de Ingesta SD seleccionado.");
        return Ok(());
    }

    let scanner = WindowsVolumeScanner::new();
    let mut registry = CameraRegistry::load().unwrap_or_else(|_| CameraRegistry::new());
    let volumes = scanner.scan_volumes().unwrap_or_default();
    let glyph_set = GlyphSet::detect();

    let mut state = IngestSetupState::from_volumes_and_registry(volumes, &mut registry, glyph_set);
    let mut reader = EventStream::new();
    let mut tick_interval = tokio::time::interval(Duration::from_millis(100));
    let mut last_tick = Instant::now();

    loop {
        terminal
            .draw(|frame| render_ingest_setup(frame, &state))
            .map_err(|e| IngestError::Io(e.to_string()))?;

        let action = tokio::select! {
            maybe_event = reader.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) => state.handle_key_event(key),
                    Some(Ok(Event::Resize(_, _))) => IngestUiAction::None,
                    Some(Err(e)) => return Err(IngestError::Io(e.to_string())),
                    None => IngestUiAction::Exit,
                    _ => IngestUiAction::None,
                }
            }
            _ = tick_interval.tick() => {
                let elapsed = last_tick.elapsed().as_millis() as u64;
                last_tick = Instant::now();
                state.tick_countdown(elapsed)
            }
        };

        match action {
            IngestUiAction::None => {}
            IngestUiAction::Exit => break,
            IngestUiAction::RescanVolumes => {
                let vols = scanner.scan_volumes().unwrap_or_default();
                state = IngestSetupState::from_volumes_and_registry(
                    vols,
                    &mut registry,
                    state.glyph_set,
                );
                last_tick = Instant::now();
            }
            IngestUiAction::SaveProfileAndStartIngest { volume, profile } => {
                if let Err(e) = registry.register_or_update(profile.clone()) {
                    state.phase = IngestPhase::Failed {
                        message: format!("Error guardando perfil en registro: {}", e),
                    };
                    continue;
                }
                execute_transfer_with_ui(terminal, &mut reader, &mut state, &mut registry, volume, profile).await?;
                last_tick = Instant::now();
            }
            IngestUiAction::StartIngest { volume, profile } => {
                execute_transfer_with_ui(terminal, &mut reader, &mut state, &mut registry, volume, profile).await?;
                last_tick = Instant::now();
            }
            IngestUiAction::CancelTransfer => {}
        }
    }

    Ok(())
}

/// Drive an active asynchronous ingest pipeline while rendering live progress and listening for `[Esc]` cancellation.
async fn execute_transfer_with_ui(
    terminal: &mut TerminalGuard,
    reader: &mut EventStream,
    state: &mut IngestSetupState,
    registry: &mut CameraRegistry,
    volume: SdCardVolume,
    profile: CameraProfile,
) -> Result<(), IngestError> {
    let cancel_token = IngestCancellationToken::new();
    let toast_notifier: Arc<dyn ToastNotifier> = Arc::new(WindowsToastNotifier::new());
    let pipeline = IngestPipeline::new(Some(toast_notifier))
        .with_cancellation_token(cancel_token.clone());

    let (tx, mut rx) = mpsc::channel::<IngestProgress>(64);
    let vol_clone = volume.clone();
    let prof_clone = profile.clone();

    state.phase = IngestPhase::Transferring {
        volume: volume.clone(),
        profile: profile.clone(),
        progress: IngestProgress {
            current_file: "Preparando archivos...".to_string(),
            current_file_index: 0,
            total_files: 0,
            current_file_bytes: 0,
            current_file_total_bytes: 0,
            total_bytes_transferred: 0,
            total_expected_bytes: 0,
            skipped_duplicates: 0,
            status: IngestStatus::Starting,
        },
    };

    let mut transfer_handle = tokio::spawn(async move {
        pipeline
            .run_ingest(&vol_clone, &prof_clone, Some(tx))
            .await
    });

    loop {
        terminal
            .draw(|frame| render_ingest_setup(frame, state))
            .map_err(|e| IngestError::Io(e.to_string()))?;

        tokio::select! {
            res = &mut transfer_handle => {
                match res {
                    Ok(Ok(summary)) => {
                        let _ = registry.record_ingest(&profile.camera_id, summary.transferred_files as u64);
                        state.phase = IngestPhase::Completed {
                            volume,
                            profile,
                            summary,
                            toast_dispatched: true,
                        };
                    }
                    Ok(Err(err)) => {
                        state.phase = IngestPhase::Failed {
                            message: err.to_string(),
                        };
                    }
                    Err(join_err) => {
                        state.phase = IngestPhase::Failed {
                            message: format!("Error en tarea de transferencia: {}", join_err),
                        };
                    }
                }
                break;
            }
            Some(prog) = rx.recv() => {
                if let IngestPhase::Transferring { progress, .. } = &mut state.phase {
                    *progress = prog;
                }
            }
            maybe_ev = reader.next() => {
                if let Some(Ok(Event::Key(key))) = maybe_ev {
                    if state.handle_key_event(key) == IngestUiAction::CancelTransfer {
                        cancel_token.cancel();
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use tempfile::tempdir;

    fn sample_volume(drive_letter: char, total_bytes: u64, free_bytes: u64) -> SdCardVolume {
        SdCardVolume {
            drive_letter,
            volume_label: "EOS_DIGITAL".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes,
            free_bytes,
            has_dcim: true,
            camera_folders: Vec::new(),
        }
    }

    #[test]
    fn test_no_drives_found_state_and_navigation() {
        let mut reg = CameraRegistry::new();
        let mut state =
            IngestSetupState::from_volumes_and_registry(vec![], &mut reg, GlyphSet::Unicode);
        assert!(matches!(state.phase, IngestPhase::NoDrivesFound));

        assert_eq!(
            state.handle_key_code(KeyCode::Char('r')),
            IngestUiAction::RescanVolumes
        );
        assert_eq!(
            state.handle_key_code(KeyCode::Esc),
            IngestUiAction::Exit
        );
    }

    #[test]
    fn test_empty_card_matching_profile_1_key_binding() {
        let dir = tempdir().unwrap();
        let reg_path = dir.path().join("camera_registry.toml");
        let mut reg = CameraRegistry::with_path(&reg_path);

        let cap_128gb = 128 * 1024 * 1024 * 1024;
        let profile = CameraProfile::new("042021000123", "Canon EOS R6", r"D:\Fotos\Canon_R6")
            .with_capacity(cap_128gb);
        reg.register_or_update(profile).unwrap();

        // Empty formatted card (free_bytes == total_bytes, 0 photos)
        let empty_vol = sample_volume('E', cap_128gb, cap_128gb);

        let mut state = IngestSetupState::from_volumes_and_registry(
            vec![empty_vol],
            &mut reg,
            GlyphSet::Unicode,
        );

        assert!(matches!(
            state.phase,
            IngestPhase::EmptyCardPrompt { .. }
        ));

        // Render and verify prompt text
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_ingest_setup(f, &state))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(content.contains("TARJETA VACÍA DETECTADA: 128 GB"));
        assert!(content.contains("Canon EOS R6"));

        // Press Enter -> binds in 1 touch
        let action = state.handle_key_code(KeyCode::Enter);
        assert_eq!(action, IngestUiAction::None);
        assert!(matches!(state.phase, IngestPhase::EmptyCardBound { .. }));
    }

    #[test]
    fn test_first_time_setup_modal_and_save_action() {
        let mut state = IngestSetupState {
            phase: IngestPhase::FirstTimeSetup {
                volume: sample_volume('E', 64 * 1024 * 1024 * 1024, 32 * 1024 * 1024 * 1024),
                media_summary: MediaSummary {
                    raw_count: 10,
                    jpeg_count: 5,
                    video_count: 0,
                    total_media_bytes: 150_000_000,
                },
                fingerprint: CameraFingerprint::new(
                    "Canon",
                    "Canon EOS R6 Mark II",
                    Some("SN998877".to_string()),
                    None,
                ),
                display_name_input: "Canon EOS R6 Mark II".to_string(),
                target_dir_input: String::new(),
                active_field: SetupInputField::TargetDirectory,
                validation_error: None,
            },
            glyph_set: GlyphSet::Unicode,
        };

        // Empty path should trigger validation error on Enter
        assert_eq!(state.handle_key_code(KeyCode::Enter), IngestUiAction::None);
        if let IngestPhase::FirstTimeSetup {
            validation_error, ..
        } = &state.phase
        {
            assert!(validation_error.is_some());
        }

        // Type destination path: "D:\Photos\R6"
        for ch in r"D:\Photos\R6".chars() {
            state.handle_key_code(KeyCode::Char(ch));
        }

        // Press Enter -> returns SaveProfileAndStartIngest with exact typed path
        let action = state.handle_key_code(KeyCode::Enter);
        match action {
            IngestUiAction::SaveProfileAndStartIngest { profile, .. } => {
                assert_eq!(profile.camera_id, "SN998877");
                assert_eq!(profile.target_directory, PathBuf::from(r"D:\Photos\R6"));
                assert_eq!(profile.display_name, "Canon EOS R6 Mark II");
            }
            other => panic!("Expected SaveProfileAndStartIngest, got {:?}", other),
        }
    }

    #[test]
    fn test_repeat_insertion_countdown_auto_triggers_ingest_with_remembered_folder() {
        let profile = CameraProfile::new(
            "SN998877",
            "Canon EOS R6 Mark II",
            r"D:\Remembered_Folder\Canon_R6",
        );
        let mut state = IngestSetupState {
            phase: IngestPhase::RepeatAutoCountdown {
                volume: sample_volume('F', 64 * 1024 * 1024 * 1024, 32 * 1024 * 1024 * 1024),
                media_summary: MediaSummary {
                    raw_count: 25,
                    jpeg_count: 0,
                    video_count: 1,
                    total_media_bytes: 500_000_000,
                },
                profile: profile.clone(),
                remaining_ms: 2000,
            },
            glyph_set: GlyphSet::Unicode,
        };

        // Tick 1000ms -> still counting down
        assert_eq!(state.tick_countdown(1000), IngestUiAction::None);

        // Tick another 1000ms -> reaches 0 and auto-triggers StartIngest to remembered folder!
        let action = state.tick_countdown(1000);
        match action {
            IngestUiAction::StartIngest {
                profile: triggered_prof,
                ..
            } => {
                assert_eq!(
                    triggered_prof.target_directory,
                    PathBuf::from(r"D:\Remembered_Folder\Canon_R6")
                );
            }
            other => panic!("Expected StartIngest on countdown completion, got {:?}", other),
        }
    }
}
