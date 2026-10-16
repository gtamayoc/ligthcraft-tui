//! Terminal graphic protocols auto-detection, fallback, and CLI configuration.

use std::env;
use std::fmt;
use ratatui_image::picker::{Picker, ProtocolType};

/// Graphic protocol selection override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProtocolOverride {
    #[default]
    Auto,
    Kitty,
    Sixel,
    Halfblocks,
}

impl ProtocolOverride {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "kitty" => Some(Self::Kitty),
            "sixel" => Some(Self::Sixel),
            "halfblocks" | "halfblock" | "unicode" => Some(Self::Halfblocks),
            _ => None,
        }
    }
}

impl fmt::Display for ProtocolOverride {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Kitty => write!(f, "kitty"),
            Self::Sixel => write!(f, "sixel"),
            Self::Halfblocks => write!(f, "halfblocks"),
        }
    }
}

/// Terminal graphic protocol coordinator.
/// Detects terminal capabilities (Kitty, Sixel, Halfblocks) and holds font metrics.
#[derive(Clone)]
pub struct GraphicsProtocol {
    picker: Picker,
    mode: ProtocolOverride,
}

impl fmt::Debug for GraphicsProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphicsProtocol")
            .field("protocol_type", &self.picker.protocol_type())
            .field("font_size", &self.picker.font_size())
            .field("mode", &self.mode)
            .finish()
    }
}

impl GraphicsProtocol {
    /// Initialize graphics protocol detection.
    ///
    /// Note: Must be called after raw mode and alternate screen setup,
    /// but before crossterm EventStream starts polling stdin.
    pub fn new(mode: ProtocolOverride) -> Self {
        // Check environment variable override if Auto
        let effective_mode = if mode == ProtocolOverride::Auto {
            if let Ok(val) = env::var("LIGHTCRAFT_GRAPHICS") {
                ProtocolOverride::from_str_loose(&val).unwrap_or(ProtocolOverride::Auto)
            } else {
                ProtocolOverride::Auto
            }
        } else {
            mode
        };

        let picker = match effective_mode {
            ProtocolOverride::Halfblocks => Picker::halfblocks(),
            ProtocolOverride::Kitty => {
                let mut p = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
                p.set_protocol_type(ProtocolType::Kitty);
                p
            }
            ProtocolOverride::Sixel => {
                let mut p = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
                p.set_protocol_type(ProtocolType::Sixel);
                p
            }
            ProtocolOverride::Auto => {
                Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks())
            }
        };

        Self {
            picker,
            mode: effective_mode,
        }
    }

    /// Construct an explicit Halfblocks fallback protocol (safe for tests & non-interactive).
    pub fn halfblocks() -> Self {
        Self {
            picker: Picker::halfblocks(),
            mode: ProtocolOverride::Halfblocks,
        }
    }

    pub fn picker(&self) -> &Picker {
        &self.picker
    }

    pub fn picker_mut(&mut self) -> &mut Picker {
        &mut self.picker
    }

    pub fn protocol_type(&self) -> ProtocolType {
        self.picker.protocol_type()
    }

    pub fn font_size(&self) -> ratatui_image::FontSize {
        self.picker.font_size()
    }

    pub fn is_halfblocks(&self) -> bool {
        self.picker.protocol_type() == ProtocolType::Halfblocks
    }

    pub fn mode(&self) -> ProtocolOverride {
        self.mode
    }

    pub fn summary(&self) -> String {
        format!(
            "{:?} (cell size: {}x{}px)",
            self.picker.protocol_type(),
            self.picker.font_size().width,
            self.picker.font_size().height
        )
    }
}
