//! Centralized glyph dictionary, UTF-8 console detection, and safe ASCII fallbacks.

/// Operating glyph mode for terminal rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphMode {
    /// Modern Unicode glyphs (❯, ✓, ★, etc.)
    Unicode,
    /// Universal ASCII characters (>, [x], *, etc.) safe for all legacy consoles.
    Ascii,
}

/// Unified glyph provider for all LightCraft TUI panels and dialogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphSet {
    Unicode,
    Ascii,
}

impl GlyphSet {
    /// Detect terminal capability and environment overrides to select the appropriate GlyphSet.
    ///
    /// Precedence:
    /// 1. `LC_ASCII=1` or `LC_ASCII=true` environment variable -> `GlyphSet::Ascii`
    /// 2. `LC_ASCII=0` or `LC_ASCII=false` environment variable -> `GlyphSet::Unicode`
    /// 3. Modern terminal emulator environment variables (`WT_SESSION`, `TERM_PROGRAM`, etc.) -> `GlyphSet::Unicode`
    /// 4. Windows legacy console host (conhost.exe) without modern font coverage -> `GlyphSet::Ascii`
    /// 5. Default -> `GlyphSet::Unicode`
    pub fn detect() -> Self {
        // 1. Explicit override via LC_ASCII environment variable
        if let Ok(val) = std::env::var("LC_ASCII") {
            let v = val.trim();
            if v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes") {
                return GlyphSet::Ascii;
            }
            if v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("no") {
                return GlyphSet::Unicode;
            }
        }

        // 2. Windows-specific console environment inspection
        #[cfg(windows)]
        {
            let has_modern_terminal = std::env::var("WT_SESSION").is_ok()
                || std::env::var("TERM_PROGRAM").is_ok()
                || std::env::var("ConEmuPID").is_ok()
                || std::env::var("ALACRITTY_LOG").is_ok()
                || std::env::var("WEZTERM_PANE").is_ok();

            if !has_modern_terminal {
                // In legacy Windows console host (cmd.exe / PowerShell 5.1 in conhost.exe),
                // raster and bitmap console fonts lack glyph coverage for ornamental Unicode
                // symbols (like ❯, ✓, ★), causing them to render as '?'.
                // Fall back safely to universal ASCII mode unless overridden.
                return GlyphSet::Ascii;
            }
        }

        GlyphSet::Unicode
    }

    /// Returns `true` if currently in ASCII fallback mode.
    pub fn is_ascii(&self) -> bool {
        matches!(self, GlyphSet::Ascii)
    }

    /// Returns `true` if currently in Unicode mode.
    pub fn is_unicode(&self) -> bool {
        matches!(self, GlyphSet::Unicode)
    }

    /// Primary step arrow pointing right.
    pub fn step_arrow(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "❯",
            GlyphSet::Ascii => ">",
        }
    }

    /// Primary step arrow pointing left.
    pub fn step_left(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "❮",
            GlyphSet::Ascii => "<",
        }
    }

    /// Standalone check mark.
    pub fn check_mark(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "✓",
            GlyphSet::Ascii => "x",
        }
    }

    /// Checked box indicator.
    pub fn check_box_checked(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "[✓]",
            GlyphSet::Ascii => "[x]",
        }
    }

    /// Unchecked box indicator.
    pub fn check_box_unchecked(&self) -> &'static str {
        "[ ]"
    }

    /// Filled favorite star.
    pub fn favorite_star(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "★",
            GlyphSet::Ascii => "*",
        }
    }

    /// Empty star for ratings.
    pub fn star_empty(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "☆",
            GlyphSet::Ascii => "-",
        }
    }

    /// Left-pointing triangle pointer.
    pub fn triangle_left(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "◄",
            GlyphSet::Ascii => "<",
        }
    }

    /// Right-pointing triangle pointer.
    pub fn triangle_right(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "►",
            GlyphSet::Ascii => ">",
        }
    }

    /// List item bullet.
    pub fn bullet(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "•",
            GlyphSet::Ascii => "-",
        }
    }

    /// Vertical separator bar.
    pub fn separator(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "│",
            GlyphSet::Ascii => "|",
        }
    }

    /// Horizontal line bar.
    pub fn bar_horizontal(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "─",
            GlyphSet::Ascii => "-",
        }
    }

    /// Upward arrow indicator.
    pub fn arrow_up(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "▲",
            GlyphSet::Ascii => "^",
        }
    }

    /// Downward arrow indicator.
    pub fn arrow_down(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "▼",
            GlyphSet::Ascii => "v",
        }
    }

    /// Task completed or ready status indicator.
    pub fn ready_mark(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "✔",
            GlyphSet::Ascii => "OK",
        }
    }

    /// Task error or failure mark.
    pub fn error_mark(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "✖",
            GlyphSet::Ascii => "X",
        }
    }

    /// In-progress spinner or refresh glyph.
    pub fn progress_spinner(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "⟳",
            GlyphSet::Ascii => "~",
        }
    }

    /// Pending task indicator.
    pub fn pending(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "⏳",
            GlyphSet::Ascii => "...",
        }
    }

    /// Paused status indicator.
    pub fn paused(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "⏸",
            GlyphSet::Ascii => "||",
        }
    }

    /// Histogram bar character for levels 0..7.
    pub fn histogram_bar(&self, level: usize) -> char {
        match self {
            GlyphSet::Unicode => {
                const BARS: [char; 8] = [' ', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
                BARS[level.min(7)]
            }
            GlyphSet::Ascii => {
                const BARS: [char; 8] = [' ', '.', ':', '-', '=', '+', '*', '#'];
                BARS[level.min(7)]
            }
        }
    }

    /// Horizontal bidirectional arrow indication for navigation shortcuts.
    pub fn arrow_left_right(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "←/→",
            GlyphSet::Ascii => "< / >",
        }
    }

    /// Vertical bidirectional arrow indication for navigation shortcuts.
    pub fn arrow_up_down(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "↑/↓",
            GlyphSet::Ascii => "^/v",
        }
    }

    /// Left arrow indicator.
    pub fn arrow_left(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "←",
            GlyphSet::Ascii => "<",
        }
    }

    /// Right arrow indicator.
    pub fn arrow_right(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "→",
            GlyphSet::Ascii => ">",
        }
    }

    /// Multiplication/times symbol for calculations.
    pub fn times(&self) -> &'static str {
        match self {
            GlyphSet::Unicode => "×",
            GlyphSet::Ascii => "x",
        }
    }

    /// Returns progress bar fill and empty characters.
    pub fn progress_bar_chars(&self) -> (char, char) {
        match self {
            GlyphSet::Unicode => ('█', '░'),
            GlyphSet::Ascii => ('#', '-'),
        }
    }

    /// Progress spinner frame for a given animation step.
    pub fn spinner_frame(&self, step: usize) -> &'static str {
        match self {
            GlyphSet::Unicode => {
                const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                FRAMES[step % FRAMES.len()]
            }
            GlyphSet::Ascii => {
                const FRAMES: [&str; 4] = ["-", "\\", "|", "/"];
                FRAMES[step % FRAMES.len()]
            }
        }
    }

    /// Border symbol set for Ratatui Blocks depending on glyph mode and focus.
    pub fn border_set(&self, is_focused: bool) -> ratatui::symbols::border::Set<'static> {
        match self {
            GlyphSet::Unicode => {
                if is_focused {
                    ratatui::symbols::border::DOUBLE
                } else {
                    ratatui::symbols::border::ROUNDED
                }
            }
            GlyphSet::Ascii => {
                if is_focused {
                    ratatui::symbols::border::Set {
                        top_left: "#",
                        top_right: "#",
                        bottom_left: "#",
                        bottom_right: "#",
                        vertical_left: "|",
                        vertical_right: "|",
                        horizontal_top: "=",
                        horizontal_bottom: "=",
                    }
                } else {
                    ratatui::symbols::border::Set {
                        top_left: "+",
                        top_right: "+",
                        bottom_left: "+",
                        bottom_right: "+",
                        vertical_left: "|",
                        vertical_right: "|",
                        horizontal_top: "-",
                        horizontal_bottom: "-",
                    }
                }
            }
        }
    }
}

impl Default for GlyphSet {
    fn default() -> Self {
        Self::detect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glyph_set_unicode_and_ascii_variants() {
        let uni = GlyphSet::Unicode;
        assert_eq!(uni.step_arrow(), "❯");
        assert_eq!(uni.check_mark(), "✓");
        assert_eq!(uni.favorite_star(), "★");
        assert_eq!(uni.triangle_right(), "►");
        assert_eq!(uni.triangle_left(), "◄");
        assert!(uni.is_unicode());
        assert!(!uni.is_ascii());

        let asc = GlyphSet::Ascii;
        assert_eq!(asc.step_arrow(), ">");
        assert_eq!(asc.check_mark(), "x");
        assert_eq!(asc.favorite_star(), "*");
        assert_eq!(asc.triangle_right(), ">");
        assert_eq!(asc.triangle_left(), "<");
        assert!(asc.is_ascii());
        assert!(!asc.is_unicode());
    }

    #[test]
    fn test_glyph_set_histogram_bars() {
        let uni = GlyphSet::Unicode;
        assert_eq!(uni.histogram_bar(0), ' ');
        assert_eq!(uni.histogram_bar(7), '█');

        let asc = GlyphSet::Ascii;
        assert_eq!(asc.histogram_bar(0), ' ');
        assert_eq!(asc.histogram_bar(7), '#');
    }

    #[test]
    fn test_lc_ascii_env_override() {
        std::env::set_var("LC_ASCII", "1");
        let detected = GlyphSet::detect();
        assert_eq!(detected, GlyphSet::Ascii);
        std::env::remove_var("LC_ASCII");

        std::env::set_var("LC_ASCII", "0");
        let detected_uni = GlyphSet::detect();
        assert_eq!(detected_uni, GlyphSet::Unicode);
        std::env::remove_var("LC_ASCII");
    }

    #[test]
    fn test_glyph_set_border_sets_and_arrows() {
        let uni = GlyphSet::Unicode;
        assert_eq!(uni.arrow_left_right(), "←/→");
        assert_eq!(uni.arrow_up_down(), "↑/↓");
        assert_eq!(uni.times(), "×");
        assert_eq!(uni.progress_bar_chars(), ('█', '░'));

        let asc = GlyphSet::Ascii;
        assert_eq!(asc.arrow_left_right(), "< / >");
        assert_eq!(asc.arrow_up_down(), "^/v");
        assert_eq!(asc.times(), "x");
        assert_eq!(asc.progress_bar_chars(), ('#', '-'));

        let border_focused = asc.border_set(true);
        assert_eq!(border_focused.top_left, "#");
        assert_eq!(border_focused.horizontal_top, "=");

        let border_unfocused = asc.border_set(false);
        assert_eq!(border_unfocused.top_left, "+");
        assert_eq!(border_unfocused.horizontal_top, "-");
    }
}
