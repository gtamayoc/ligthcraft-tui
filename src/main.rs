//! LightCraft MCP Rust TUI Entry Point (Milestones M1, M2 & M3)

pub mod app;
pub mod engine;
pub mod error;
pub mod export;
pub mod graphics;
pub mod mcp;
pub mod queue;
pub mod storage;
pub mod tui;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use clap::Parser;

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
    #[arg(long)]
    pub tui: bool,

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
                    println!("  LightCraft Presets Catalog ({:>2} presets available)       ", presets.len());
                    println!("============================================================");
                    println!("{:<20} {:<20} {:<12} {:<25}", "ID", "NAME", "TYPE", "CATEGORY");
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
        println!("Pre-flight check passed for {} master files.", pre_results.len());

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
                        Ok(export::ExportOutcome::Success { output_path: _, width, height, bytes_written, elapsed }) => {
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
                println!("SUCCESS: 100% byte-for-byte immutability confirmed across all 5 master files.");
            }
            Err(e) => {
                eprintln!("CRITICAL ERROR: Post-flight immutability failed: {}", e);
                return ExitCode::FAILURE;
            }
        }
    }

    // Launch TUI mode if explicitly requested, or if interactive terminal and no diagnostic flags
    if cli.tui || (!had_action && std::io::stdout().is_terminal()) {
        had_action = true;
        let draw_dir = cli.draw_dir.clone().unwrap_or_else(find_draw_dir);
        let store = match cli.presets_dir {
            Some(ref dir) => PresetStore::with_default_builtins(dir.clone()).unwrap_or_else(|_| PresetStore::default_store(PresetStore::default_builtins()).unwrap()),
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

        let graphics_override = ProtocolOverride::from_str_loose(&cli.graphics).unwrap_or(ProtocolOverride::Auto);

        let terminal = match TerminalGuard::init() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Failed to initialize terminal: {}", e);
                return ExitCode::FAILURE;
            }
        };

        match AppController::new(mcp_client, store, draw_dir, graphics_override).await {
            Ok(mut app) => {
                if let Err(e) = app.run(terminal).await {
                    eprintln!("TUI Application Error: {}", e);
                    return ExitCode::FAILURE;
                }
            }
            Err(e) => {
                eprintln!("Failed to initialize AppController: {}", e);
                return ExitCode::FAILURE;
            }
        }
    }

    if !had_action {
        println!("LightCraft TUI v0.1.0 (Milestones M1, M2 & M3)");
        println!("Use --help for command line options.");
        println!("Available commands:");
        println!("  lightcraft-tui --tui [--graphics <auto|kitty|sixel|halfblocks>] [--mock]");
        println!("  lightcraft-tui --verify-integrity");
        println!("  lightcraft-tui --production-workload [--output-dir <dir>]");
        println!("  lightcraft-tui --check-mcp");
        println!("  lightcraft-tui --list-presets");
        println!("  lightcraft-tui --show-preset <id> [--format <toml|json>]");
    }

    ExitCode::SUCCESS
}
