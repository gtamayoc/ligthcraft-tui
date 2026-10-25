//! LightCraft MCP Rust TUI Entry Point (Milestones M1, M2 & M3)

pub mod app;
pub mod engine;
pub mod error;
pub mod export;
pub mod graphics;
pub mod hub;
pub mod ingest;
pub mod mcp;
pub mod queue;
pub mod storage;
pub mod tui;

use clap::Parser;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use app::AppController;
use graphics::protocol::ProtocolOverride;
use mcp::client::{ClientMode, McpClient, McpClientConfig};
use mcp::integrity::{find_draw_dir, verify_cr3_baseline};
use storage::presets_store::{PresetFormat, PresetStore};
use tui::TerminalGuard;

/// Command-line arguments for LightCraft TUI
#[derive(Parser, Debug)]
#[command(
    name = "lightcraft-tui",
    version = "0.1.0",
    about = "High-performance photographic automation TUI with LightCraft MCP integration"
)]
pub struct Cli {
    /// Launch interactive Terminal User Interface (TUI)
    #[arg(long, conflicts_with = "sd_ingest")]
    pub tui: bool,

    /// Launch SD Card Ingest & Storage Analyzer directly (bypasses launcher hub)
    #[arg(long, conflicts_with = "tui")]
    pub sd_ingest: bool,

    /// Force ASCII glyphs instead of Unicode icons/symbols (env: LC_ASCII=1)
    #[arg(long, env = "LC_ASCII")]
    pub ascii: bool,

    /// Override graphics protocol (auto, kitty, sixel, halfblocks)
    #[arg(long, default_value = "auto")]
    pub graphics: String,

    /// Connect to LightCraft MCP, perform handshake and verify discovered tools
    #[arg(long)]
    pub check_mcp: bool,

    /// Verify cryptographic SHA-256 integrity baseline on DRAW/*.CR3 files
    #[arg(long)]
    pub verify_integrity: bool,

    /// Force offline in-memory mock MCP mode
    #[arg(long)]
    pub mock: bool,

    /// Path to Canon CR3 RAW files directory (defaults to auto-detection of ./DRAW)
    #[arg(long)]
    pub draw_dir: Option<PathBuf>,

    /// List all available built-in and user presets
    #[arg(long)]
    pub list_presets: bool,

    /// Show or output details of a specific preset by ID or name
    #[arg(long, visible_alias = "preset")]
    pub show_preset: Option<String>,

    /// Output format for preset operations (toml or json, default: toml)
    #[arg(long, default_value = "toml")]
    pub format: String,

    /// Custom directory for persistent preset storage
    #[arg(long)]
    pub presets_dir: Option<PathBuf>,

    /// Run full Canon CR3 production workload across presets and export formats
    #[arg(long)]
    pub production_workload: bool,

    /// Run sequential photo-by-photo processing with immediate max-quality disk saving (TIFF 16-bit + JPEG 100%)
    #[arg(long)]
    pub sequential_export: bool,

    /// Output destination folder for production exports (default: dist/client_gallery)
    #[arg(long, default_value = "dist/client_gallery")]
    pub output_dir: PathBuf,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let mut had_action = false;

    if cli.verify_integrity {
        had_action = true;
        println!("============================================================");
        println!("  LightCraft RAW Non-Destructive Integrity Verification     ");
        println!("============================================================");
        let draw_dir = cli.draw_dir.clone().unwrap_or_else(find_draw_dir);
        println!("Source directory: {}", draw_dir.display());

        match verify_cr3_baseline(&draw_dir) {
            Ok(results) => {
                for res in &results {
                    println!(
                        "  [PASS] {:<15} | {} bytes | sha256: {}...",
                        res.filename,
                        res.actual_size,
                        &res.actual_sha256[..16]
                    );
                }
                println!("------------------------------------------------------------");
                println!(
                    "SUCCESS: All {} Canon CR3 master files verified byte-for-byte against SHA-256 baseline.",
                    results.len()
                );
            }
            Err(err) => {
                eprintln!("FAILURE: CR3 integrity violation detected!\n{}", err);
                return ExitCode::FAILURE;
            }
        }
    }

    if cli.check_mcp {
        had_action = true;
        println!("============================================================");
        println!("  LightCraft MCP Diagnostics & Handshake Verification        ");
        println!("============================================================");

        let config = if cli.mock {
            McpClientConfig::mock()
        } else {
            let mut cfg = McpClientConfig::default();
            if cfg.resolve_executable().is_err() {
                println!("Note: Live lightcraft-cli.exe not found on system; using in-memory mock transport.");
                cfg.mode = ClientMode::Mock;
            }
            cfg
        };

        match McpClient::spawn(config).await {
            Ok(client) => {
                if let Some(info) = client.server_info().await {
                    println!("Connected Server: {} v{}", info.name, info.version);
                    if let Some(title) = info.title {
                        println!("Server Title:    {}", title);
                    }
                }

                match client.list_tools().await {
                    Ok(tools) => {
                        println!("Discovered {} MCP photographic tools:", tools.len());
                        for tool in &tools {
                            println!("  - {}", tool);
                        }
                        println!("------------------------------------------------------------");
                        println!("SUCCESS: MCP connection and tool discovery verified.");
                    }
                    Err(e) => {
                        eprintln!("Failed to enumerate tools: {}", e);
                        return ExitCode::FAILURE;
                    }
                }

                let _ = client.shutdown().await;
            }
            Err(e) => {
                eprintln!("Failed to initialize MCP client: {}", e);
                return ExitCode::FAILURE;
            }
        }
    }

    if cli.list_presets {
        had_action = true;
        let store = match cli.presets_dir {
            Some(ref dir) => PresetStore::with_default_builtins(dir.clone()),
            None => PresetStore::default_store(PresetStore::default_builtins()),
        };

        match store {
            Ok(store) => match store.list_presets() {
                Ok(presets) => {
                    println!("============================================================");
                    println!(
                        "  LightCraft Presets Catalog ({:>2} presets available)       ",
                        presets.len()
                    );
                    println!("============================================================");
                    println!(
                        "{:<20} {:<20} {:<12} {:<25}",
                        "ID", "NAME", "TYPE", "CATEGORY"
                    );
                    println!("------------------------------------------------------------");
                    for p in presets {
                        let kind = if p.is_builtin { "Built-in" } else { "User" };
                        let cat = p.category.as_deref().unwrap_or("Custom");
                        println!("{:<20} {:<20} {:<12} {:<25}", p.id, p.name, kind, cat);
                    }
                    println!("------------------------------------------------------------");
                }
                Err(e) => {
                    eprintln!("Failed to list presets: {}", e);
                    return ExitCode::FAILURE;
                }
            },
            Err(e) => {
                eprintln!("Failed to initialize preset store: {}", e);
                return ExitCode::FAILURE;
            }
        }
    }

    if let Some(ref id) = cli.show_preset {
        had_action = true;
        let store = match cli.presets_dir {
            Some(ref dir) => PresetStore::with_default_builtins(dir.clone()),
            None => PresetStore::default_store(PresetStore::default_builtins()),
        };

        match store {
            Ok(store) => match store.load_preset_file(id) {
                Ok(preset_file) => {
                    let fmt = match cli.format.to_lowercase().as_str() {
                        "json" => PresetFormat::Json,
                        _ => PresetFormat::Toml,
                    };

                    match fmt.serialize(&preset_file) {
                        Ok(serialized) => println!("{}", serialized),
                        Err(e) => {
                            eprintln!("Failed to serialize preset '{}': {}", id, e);
                            return ExitCode::FAILURE;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Preset '{}' not found: {}", id, e);
                    return ExitCode::FAILURE;
                }
            },
            Err(e) => {
                eprintln!("Failed to initialize preset store: {}", e);
                return ExitCode::FAILURE;
            }
        }
    }

    if cli.production_workload {
        had_action = true;
        println!("============================================================");
        println!("  LightCraft Canon CR3 Production Workload Execution        ");
        println!("============================================================");
        let draw_dir = cli.draw_dir.clone().unwrap_or_else(find_draw_dir);
        println!("Source directory: {}", draw_dir.display());
        println!("Output directory: {}", cli.output_dir.display());

        // Pre-flight check
        let pre_results = match verify_cr3_baseline(&draw_dir) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Pre-flight verification failed: {}", e);
                return ExitCode::FAILURE;
            }
        };
        println!(
            "Pre-flight check passed for {} master files.",
            pre_results.len()
        );

        let pipeline = export::ExportPipeline::new();
        let cr3_files = [
            "IMG_0588.CR3",
            "IMG_0589.CR3",
            "IMG_0590.CR3",
            "IMG_0969.CR3",
            "IMG_0981.CR3",
        ];

        let presets = [
            engine::presets::BuiltinPreset::Natural,
            engine::presets::BuiltinPreset::Cinema,
            engine::presets::BuiltinPreset::BwHighContrast,
            engine::presets::BuiltinPreset::TealOrange,
        ];

        let formats = [
            (export::OutputFormat::Jpeg, export::BitDepth::Eight),
            (export::OutputFormat::Png, export::BitDepth::Eight),
            (export::OutputFormat::Tiff, export::BitDepth::Sixteen),
        ];

        let mut success_count = 0;
        let mut total_bytes = 0u64;

        // Process all 5 Canon CR3s across 4 presets and 3 formats
        for &cr3_name in &cr3_files {
            let cr3_path = draw_dir.join(cr3_name);
            println!("\nProcessing {}...", cr3_name);

            for preset in &presets {
                for &(format, bit_depth) in &formats {
                    let job = export::ExportJob {
                        job_id: (success_count + 1) as u64,
                        source_path: cr3_path.clone(),
                        photo_id: Some((success_count + 1) as i64),
                        recipe_name: preset.display_name().to_string(),
                        recipe: preset.recipe(),
                        profile: export::ExportProfile {
                            name: format!("{}_{:?}", preset.display_name(), format),
                            format,
                            quality: 90,
                            bit_depth,
                            scaling: export::ScalingProfile::LongEdge {
                                max_long_edge: 2048,
                                dont_enlarge: false,
                            },
                            ..Default::default()
                        },
                        output_dir: cli.output_dir.clone(),
                        template: export::NamingTemplate::parse("{name}__{preset}.{ext}"),
                        seq_number: success_count + 1,
                        collision_policy: export::CollisionPolicy::Overwrite,
                    };

                    match pipeline.execute_export(None, &job).await {
                        Ok(export::ExportOutcome::Success {
                            output_path: _,
                            width,
                            height,
                            bytes_written,
                            elapsed,
                        }) => {
                            success_count += 1;
                            total_bytes += bytes_written;
                            println!(
                                "  [OK] {:<18} | {:<4} | {:>4}x{:<4} | {:>8} bytes | {:>6.2?}",
                                preset.display_name(),
                                format.default_extension(),
                                width,
                                height,
                                bytes_written,
                                elapsed
                            );
                        }
                        Ok(other) => {
                            println!("  [SKIP] {:?}", other);
                        }
                        Err(e) => {
                            eprintln!("  [ERR] Failed export: {}", e);
                            return ExitCode::FAILURE;
                        }
                    }
                }
            }
        }

        println!("------------------------------------------------------------");
        println!(
            "SUCCESS: Generated {} production images ({:.2} MB) into {}",
            success_count,
            total_bytes as f64 / 1_048_576.0,
            cli.output_dir.display()
        );

        // Post-flight immutability verification
        println!("\nExecuting post-flight source immutability verification...");
        match verify_cr3_baseline(&draw_dir) {
            Ok(post_results) => {
                for res in &post_results {
                    println!("  [PASS] {:<15} | sha256 verified", res.filename);
                }
                println!(
                    "SUCCESS: 100% byte-for-byte immutability confirmed across all 5 master files."
                );
            }
            Err(e) => {
                eprintln!("CRITICAL ERROR: Post-flight immutability failed: {}", e);
                return ExitCode::FAILURE;
            }
        }
    }

    if cli.sequential_export {
        had_action = true;
        println!("============================================================");
        println!("  LightCraft Secuencial Inmediato (Foto a Foto - Max Calidad)");
        println!("============================================================");
        let draw_dir = cli.draw_dir.clone().unwrap_or_else(find_draw_dir);
        println!("Directorio origen: {}", draw_dir.display());
        println!("Directorio destino: {}", cli.output_dir.display());

        // Pre-flight check
        let pre_results = match verify_cr3_baseline(&draw_dir) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Pre-flight verification failed: {}", e);
                return ExitCode::FAILURE;
            }
        };
        println!(
            "Pre-flight check OK: {} archivos maestros RAW verificados.",
            pre_results.len()
        );

        let pipeline = export::ExportPipeline::new();
        let cr3_files = [
            "IMG_0588.CR3",
            "IMG_0589.CR3",
            "IMG_0590.CR3",
            "IMG_0969.CR3",
            "IMG_0981.CR3",
        ];

        let presets = [
            engine::presets::BuiltinPreset::Natural,
            engine::presets::BuiltinPreset::Cinema,
            engine::presets::BuiltinPreset::BwHighContrast,
            engine::presets::BuiltinPreset::TealOrange,
        ];

        let mut processed_photos = 0;
        let mut total_files_generated = 0;
        let mut total_bytes_saved = 0u64;

        for &cr3_name in &cr3_files {
            let cr3_path = draw_dir.join(cr3_name);
            processed_photos += 1;
            println!(
                "\n[Foto {}/{}] Cargando y procesando: {}",
                processed_photos,
                cr3_files.len(),
                cr3_name
            );

            for preset in &presets {
                // 1. Guardar de inmediato Master TIFF 16-bit
                let tiff_job = export::ExportJob {
                    job_id: (total_files_generated + 1) as u64,
                    source_path: cr3_path.clone(),
                    photo_id: Some((total_files_generated + 1) as i64),
                    recipe_name: preset.display_name().to_string(),
                    recipe: preset.recipe(),
                    profile: export::ExportProfile {
                        name: format!("{}_TIFF16", preset.display_name()),
                        format: export::OutputFormat::Tiff,
                        quality: 100,
                        bit_depth: export::BitDepth::Sixteen,
                        scaling: export::ScalingProfile::Native,
                        ..Default::default()
                    },
                    output_dir: cli.output_dir.clone(),
                    template: export::NamingTemplate::parse("{name}__{preset}.{ext}"),
                    seq_number: total_files_generated + 1,
                    collision_policy: export::CollisionPolicy::Overwrite,
                };

                match pipeline.execute_export(None, &tiff_job).await {
                    Ok(export::ExportOutcome::Success {
                        bytes_written,
                        output_path,
                        elapsed,
                        ..
                    }) => {
                        total_files_generated += 1;
                        total_bytes_saved += bytes_written;
                        println!(
                            "  [OK] Guardado inmediato: {:<30} | TIFF 16-bit | {:.1} MB | {:?}",
                            output_path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                            bytes_written as f64 / 1_048_576.0,
                            elapsed
                        );
                    }
                    Ok(other) => {
                        println!("  [AVISO] {:?}", other);
                    }
                    Err(e) => {
                        eprintln!("  [ERROR] Falló exportación TIFF: {}", e);
                        return ExitCode::FAILURE;
                    }
                }

                // 2. Guardar de inmediato Versión JPEG 100%
                let jpeg_job = export::ExportJob {
                    job_id: (total_files_generated + 1) as u64,
                    source_path: cr3_path.clone(),
                    photo_id: Some((total_files_generated + 1) as i64),
                    recipe_name: preset.display_name().to_string(),
                    recipe: preset.recipe(),
                    profile: export::ExportProfile {
                        name: format!("{}_JPEG100", preset.display_name()),
                        format: export::OutputFormat::Jpeg,
                        quality: 100,
                        bit_depth: export::BitDepth::Eight,
                        scaling: export::ScalingProfile::Native,
                        ..Default::default()
                    },
                    output_dir: cli.output_dir.clone(),
                    template: export::NamingTemplate::parse("{name}__{preset}.{ext}"),
                    seq_number: total_files_generated + 1,
                    collision_policy: export::CollisionPolicy::Overwrite,
                };

                match pipeline.execute_export(None, &jpeg_job).await {
                    Ok(export::ExportOutcome::Success {
                        bytes_written,
                        output_path,
                        elapsed,
                        ..
                    }) => {
                        total_files_generated += 1;
                        total_bytes_saved += bytes_written;
                        println!(
                            "  [OK] Guardado inmediato: {:<30} | JPEG 100%  | {:.1} MB | {:?}",
                            output_path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                            bytes_written as f64 / 1_048_576.0,
                            elapsed
                        );
                    }
                    Ok(other) => {
                        println!("  [AVISO] {:?}", other);
                    }
                    Err(e) => {
                        eprintln!("  [ERROR] Falló exportación JPEG: {}", e);
                        return ExitCode::FAILURE;
                    }
                }
            }
        }

        println!("------------------------------------------------------------");
        println!(
            "COMPLETADO CON ÉXITO: {} fotos procesadas, {} archivos guardados inmediatamente ({:.2} MB) en {}",
            processed_photos,
            total_files_generated,
            total_bytes_saved as f64 / 1_048_576.0,
            cli.output_dir.display()
        );

        // Post-flight immutability verification
        println!("\nVerificando inmutabilidad post-exportación...");
        match verify_cr3_baseline(&draw_dir) {
            Ok(post_results) => {
                for res in &post_results {
                    println!(
                        "  [PASS] {:<15} | 100% idéntico a SHA-256 baseline",
                        res.filename
                    );
                }
                println!("INMUTABILIDAD GARANTIZADA: Ningún archivo RAW original fue modificado.");
            }
            Err(e) => {
                eprintln!("ERROR CRÍTICO: Falló verificación de inmutabilidad: {}", e);
                return ExitCode::FAILURE;
            }
        }
    }

    // Interactive TUI / Launcher Hub Dispatch
    let run_interactive =
        cli.tui || cli.sd_ingest || (!had_action && std::io::stdout().is_terminal());

    if run_interactive {
        let mut terminal = match TerminalGuard::init() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Failed to initialize terminal: {}", e);
                return ExitCode::FAILURE;
            }
        };

        // Resolve target tool either via direct CLI flag bypass or interactive Launcher Hub
        let selected_tool = if cli.tui {
            hub::ToolOption::Studio
        } else if cli.sd_ingest {
            hub::ToolOption::SdIngest
        } else {
            match hub::run_launcher(&mut terminal).await {
                Ok(hub::ToolOption::Exit) => return ExitCode::SUCCESS,
                Ok(tool) => tool,
                Err(e) => {
                    let _ = terminal.restore();
                    eprintln!("Launcher error: {}", e);
                    return ExitCode::FAILURE;
                }
            }
        };

        return match selected_tool {
            hub::ToolOption::Studio => run_studio(&cli, terminal).await,
            hub::ToolOption::SdIngest => run_sd_ingest(&cli, terminal).await,
            hub::ToolOption::Exit => ExitCode::SUCCESS,
        };
    }

    if !had_action {
        println!("LightCraft TUI v0.1.0 (Milestones M1, M2 & M3)");
        println!("Use --help for command line options.");
        println!("Available commands:");
        println!("  lightcraft-tui --tui [--graphics <auto|kitty|sixel|halfblocks>] [--mock]");
        println!("  lightcraft-tui --sd-ingest");
        println!("  lightcraft-tui --sequential-export [--output-dir <dir>]");
        println!("  lightcraft-tui --verify-integrity");
        println!("  lightcraft-tui --production-workload [--output-dir <dir>]");
        println!("  lightcraft-tui --check-mcp");
        println!("  lightcraft-tui --list-presets");
        println!("  lightcraft-tui --show-preset <id> [--format <toml|json>]");
    }

    ExitCode::SUCCESS
}

/// Launches the existing LightCraft Studio RAW developer
async fn run_studio(cli: &Cli, terminal: TerminalGuard) -> ExitCode {
    let draw_dir = cli.draw_dir.clone().unwrap_or_else(find_draw_dir);
    let store = match cli.presets_dir {
        Some(ref dir) => PresetStore::with_default_builtins(dir.clone()).unwrap_or_else(|_| {
            PresetStore::default_store(PresetStore::default_builtins()).unwrap()
        }),
        None => PresetStore::default_store(PresetStore::default_builtins()).unwrap(),
    };

    let config = if cli.mock {
        McpClientConfig::mock()
    } else {
        let mut cfg = McpClientConfig::default();
        if cfg.resolve_executable().is_err() {
            cfg.mode = ClientMode::Mock;
        }
        cfg
    };

    let mcp_client = match McpClient::spawn(config).await {
        Ok(client) => Arc::new(client),
        Err(e) => {
            eprintln!("Failed to launch MCP client: {}", e);
            return ExitCode::FAILURE;
        }
    };

    let graphics_override =
        ProtocolOverride::from_str_loose(&cli.graphics).unwrap_or(ProtocolOverride::Auto);

    match AppController::new(mcp_client, store, draw_dir, graphics_override).await {
        Ok(mut app) => {
            if cli.ascii {
                app.glyph_set = tui::GlyphSet::Ascii;
            }
            if let Err(e) = app.run(terminal).await {
                eprintln!("TUI Application Error: {}", e);
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Failed to initialize AppController: {}", e);
            ExitCode::FAILURE
        }
    }
}

/// Launches the SD Ingest & Storage Analyzer module (M5 full TUI & automated pipeline)
async fn run_sd_ingest(_cli: &Cli, mut terminal: TerminalGuard) -> ExitCode {
    match tui::run_ingest_tui(&mut terminal).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = terminal.restore();
            eprintln!("SD Ingest Error: {}", e);
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_cli_default_no_args() {
        let cli = Cli::try_parse_from(["lightcraft-tui"]).unwrap();
        assert!(!cli.tui);
        assert!(!cli.sd_ingest);
        assert!(!cli.verify_integrity);
        assert!(!cli.check_mcp);
        assert!(!cli.list_presets);
        assert!(cli.show_preset.is_none());
        assert!(!cli.production_workload);
        assert!(!cli.sequential_export);
        assert_eq!(cli.graphics, "auto");
        assert_eq!(cli.format, "toml");
    }

    #[test]
    fn test_cli_sd_ingest_flag() {
        let cli = Cli::try_parse_from(["lightcraft-tui", "--sd-ingest"]).unwrap();
        assert!(cli.sd_ingest);
        assert!(!cli.tui);
    }

    #[test]
    fn test_cli_tui_flag() {
        let cli = Cli::try_parse_from(["lightcraft-tui", "--tui"]).unwrap();
        assert!(cli.tui);
        assert!(!cli.sd_ingest);
    }

    #[test]
    fn test_cli_conflicting_tui_and_sd_ingest() {
        let res = Cli::try_parse_from(["lightcraft-tui", "--tui", "--sd-ingest"]);
        assert!(
            res.is_err(),
            "Passing both --tui and --sd-ingest must be rejected by clap"
        );
    }

    #[test]
    fn test_cli_conflicting_sd_ingest_and_tui_reverse_order() {
        let res = Cli::try_parse_from(["lightcraft-tui", "--sd-ingest", "--tui"]);
        assert!(
            res.is_err(),
            "Passing both --sd-ingest and --tui must be rejected by clap"
        );
    }

    #[test]
    fn test_cli_existing_flags_preserved() {
        let cli = Cli::try_parse_from([
            "lightcraft-tui",
            "--verify-integrity",
            "--sequential-export",
            "--mock",
            "--ascii",
            "--graphics",
            "kitty",
            "--format",
            "json",
            "--output-dir",
            "custom/output",
        ])
        .unwrap();
        assert!(cli.verify_integrity);
        assert!(cli.sequential_export);
        assert!(cli.mock);
        assert!(cli.ascii);
        assert_eq!(cli.graphics, "kitty");
        assert_eq!(cli.format, "json");
        assert_eq!(cli.output_dir, PathBuf::from("custom/output"));
    }
}
