#![allow(dead_code, unused_imports, unused_variables)]

//! Common test helpers, assertions, environment detectors, and fixtures.

pub mod cr3_integrity;
pub mod mock_mcp;

use std::path::PathBuf;
use std::sync::Once;

static INIT_TEST_LOGGING: Once = Once::new();

/// Initialize test logging if needed.
pub fn init_test_env() {
    INIT_TEST_LOGGING.call_once(|| {
        // Ensure test environment is normalized
    });
}

/// Check if live LightCraft MCP server testing is requested via environment variable.
pub fn is_live_mcp_enabled() -> bool {
    std::env::var("LIGHTCRAFT_LIVE_TEST")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Retrieve the path to the LightCraft CLI binary.
pub fn get_lightcraft_cli_path() -> PathBuf {
    if let Ok(p) = std::env::var("LIGHTCRAFT_CLI_PATH") {
        return PathBuf::from(p);
    }
    PathBuf::from(r"C:\Program Files\LightCraft\lightcraft-cli.exe")
}

/// Create an isolated, clean scratch directory for test export artifacts.
/// Strictly prohibits creating output folders inside `./DRAW/`.
pub fn create_test_output_dir(test_name: &str) -> PathBuf {
    let base = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("target")
        .join("test_artifacts")
        .join(test_name);

    if base.exists() {
        let _ = std::fs::remove_dir_all(&base);
    }
    std::fs::create_dir_all(&base).expect("Failed to create isolated test output directory");
    base
}

/// Standardized Test Harness combining Mock MCP Server and Canon CR3 Immutability Guard.
pub struct TestFixture {
    pub mock_server: mock_mcp::MockMcpServer,
    pub integrity_guard: cr3_integrity::Cr3IntegrityGuard,
    pub output_dir: PathBuf,
}

impl TestFixture {
    pub fn new(test_name: &str) -> Self {
        init_test_env();
        let guard = cr3_integrity::Cr3IntegrityGuard::new();
        let output_dir = create_test_output_dir(test_name);
        let mock_server = mock_mcp::MockMcpServer::new();

        Self {
            mock_server,
            integrity_guard: guard,
            output_dir,
        }
    }

    /// Explicitly trigger immutability verification.
    pub fn assert_integrity(&self) {
        self.integrity_guard.verify_immutability();
    }
}
