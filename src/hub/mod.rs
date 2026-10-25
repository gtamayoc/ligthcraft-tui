//! LightCraft Modular Tool Launcher Hub.
//!
//! Provides the top-level tool selection menu, allowing users to choose between
//! LightCraft Studio (RAW photographic developer), SD Ingest & Storage (automated
//! 100% read-only SD ingest with EXIF camera fingerprinting), or Exit.

pub mod view;

use serde::{Deserialize, Serialize};
use std::fmt;

/// Available primary tool options selectable from the launcher hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ToolOption {
    /// Professional RAW photo developer, recipe editor, split-view A/B, and batch exporter.
    #[default]
    Studio,
    /// Automated, 100% read-only SD card scanner, EXIF camera fingerprinting, and streaming ingest.
    SdIngest,
    /// Terminate the launcher and exit to terminal.
    Exit,
}

impl ToolOption {
    /// Display title for the tool card.
    pub fn title(&self) -> &'static str {
        match self {
            Self::Studio => "LightCraft Studio",
            Self::SdIngest => "SD Ingest & Storage",
            Self::Exit => "Salir",
        }
    }

    /// Single character keyboard shortcut for direct selection.
    pub fn shortcut_char(&self) -> char {
        match self {
            Self::Studio => '1',
            Self::SdIngest => '2',
            Self::Exit => 'Q',
        }
    }

    /// Short descriptive subtitle / tagline.
    pub fn tagline(&self) -> &'static str {
        match self {
            Self::Studio => "Revelador RAW Profesional & Pipeline de Color",
            Self::SdIngest => "Extracción Automatizada 100% Solo Lectura & EXIF",
            Self::Exit => "Cerrar la aplicación y volver a la terminal",
        }
    }

    /// Bulleted capability description lines.
    pub fn descriptions(&self) -> &'static [&'static str] {
        match self {
            Self::Studio => &[
                "Revelado foto a foto e inspección con zoom 1:1 Loupe",
                "Ajustes no destructivos, split-view A/B y matriz de variantes",
                "Exportación por lotes inmediata (TIFF 16-bit + JPEG 100%)",
            ],
            Self::SdIngest => &[
                "Detección en caliente de unidades extraíbles (Win32 DCIM)",
                "Identificación biométrica de cámara por serial EXIF (0xa431)",
                "Streaming asíncrono 1 MB, SHA-256 en vuelo y Toast nativo",
            ],
            Self::Exit => &[
                "Restaura el estado de la consola y finaliza el proceso",
            ],
        }
    }

    /// Status badge or operating mode description.
    pub fn status_text(&self) -> &'static str {
        match self {
            Self::Studio => "Listo para revelado (MCP + Presets activos)",
            Self::SdIngest => "Modo 100% Solo Lectura (SD Write-Lock Safe)",
            Self::Exit => "Salida limpia",
        }
    }
}

impl fmt::Display for ToolOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.title())
    }
}

/// Errors originating within the launcher hub.
#[derive(Debug, thiserror::Error)]
pub enum HubError {
    #[error("Terminal I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Terminal rendering error: {0}")]
    Render(String),
}

impl From<HubError> for crate::error::AppError {
    fn from(err: HubError) -> Self {
        match err {
            HubError::Io(e) => crate::error::AppError::Io(e),
            HubError::Render(msg) => crate::error::AppError::Config(msg),
        }
    }
}

pub type Result<T, E = HubError> = std::result::Result<T, E>;

pub use view::{render_hub, run_ingest_stub, run_launcher, HubState, TOOL_OPTIONS};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_option_default_is_studio() {
        assert_eq!(ToolOption::default(), ToolOption::Studio);
    }

    #[test]
    fn test_tool_option_equality_and_variants() {
        let studio = ToolOption::Studio;
        let ingest = ToolOption::SdIngest;
        let exit = ToolOption::Exit;

        assert_eq!(studio, ToolOption::Studio);
        assert_eq!(ingest, ToolOption::SdIngest);
        assert_eq!(exit, ToolOption::Exit);

        assert_ne!(studio, ingest);
        assert_ne!(studio, exit);
        assert_ne!(ingest, exit);
    }

    #[test]
    fn test_tool_option_titles_and_shortcuts() {
        assert_eq!(ToolOption::Studio.title(), "LightCraft Studio");
        assert_eq!(ToolOption::Studio.shortcut_char(), '1');

        assert_eq!(ToolOption::SdIngest.title(), "SD Ingest & Storage");
        assert_eq!(ToolOption::SdIngest.shortcut_char(), '2');

        assert_eq!(ToolOption::Exit.title(), "Salir");
        assert_eq!(ToolOption::Exit.shortcut_char(), 'Q');
    }

    #[test]
    fn test_tool_option_taglines_and_descriptions() {
        assert!(!ToolOption::Studio.tagline().is_empty());
        assert!(!ToolOption::SdIngest.tagline().is_empty());
        assert!(!ToolOption::Exit.tagline().is_empty());

        assert!(!ToolOption::Studio.descriptions().is_empty());
        assert!(!ToolOption::SdIngest.descriptions().is_empty());
        assert!(!ToolOption::Exit.descriptions().is_empty());

        assert!(!ToolOption::Studio.status_text().is_empty());
        assert!(!ToolOption::SdIngest.status_text().is_empty());
        assert!(!ToolOption::Exit.status_text().is_empty());
    }

    #[test]
    fn test_tool_option_display_formatting() {
        assert_eq!(format!("{}", ToolOption::Studio), "LightCraft Studio");
        assert_eq!(format!("{}", ToolOption::SdIngest), "SD Ingest & Storage");
        assert_eq!(format!("{}", ToolOption::Exit), "Salir");
    }

    #[test]
    fn test_tool_option_serde_roundtrip() {
        let variants = [ToolOption::Studio, ToolOption::SdIngest, ToolOption::Exit];
        for variant in &variants {
            let serialized = serde_json::to_string(variant).expect("serialization failed");
            let deserialized: ToolOption =
                serde_json::from_str(&serialized).expect("deserialization failed");
            assert_eq!(*variant, deserialized);
        }
    }
}
