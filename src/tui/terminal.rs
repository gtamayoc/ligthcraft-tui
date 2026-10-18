//! Terminal lifecycle manager, RAII raw mode guard, and safety panic hook installer.

use std::io::{stdout, IsTerminal, Stdout, Write};
use std::ops::{Deref, DerefMut};
use std::panic::{self, PanicHookInfo};
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::error::AppError;

static TERMINAL_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// RAII Guard that manages crossterm raw mode, alternate screen, and mouse capture.
/// Ensures the terminal is 100% restored upon normal exit, error return, or panic.
pub struct TerminalGuard {
    terminal: Option<Terminal<CrosstermBackend<Stdout>>>,
    is_raw: bool,
}

impl TerminalGuard {
    /// Initialize the terminal: enable raw mode, enter alternate screen, enable mouse capture,
    /// hide cursor, and install the safety panic hook.
    pub fn init() -> Result<Self, AppError> {
        let mut out = stdout();

        // 1. Install safety panic hook before altering terminal state
        Self::install_panic_hook();

        // 2. Check if running in a real interactive terminal
        let is_tty = out.is_terminal();
        if is_tty {
            Self::maximize_window();
            enable_raw_mode().map_err(AppError::Io)?;
            execute!(out, EnterAlternateScreen, EnableMouseCapture, Hide)
                .map_err(AppError::Io)?;
        }

        let backend = CrosstermBackend::new(out);
        let terminal = Terminal::new(backend).map_err(AppError::Io)?;

        TERMINAL_INITIALIZED.store(true, Ordering::SeqCst);

        Ok(Self {
            terminal: Some(terminal),
            is_raw: is_tty,
        })
    }

    /// Install a panic hook that resets the terminal back to a normal interactive state
    /// before emitting the panic message and backtrace.
    fn install_panic_hook() {
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info: &PanicHookInfo| {
            // Restore terminal immediately
            Self::restore_terminal_raw();
            // Call original hook to print stack trace / panic info to clean stderr
            default_hook(info);
        }));
    }

    /// Low-level unconditional terminal restoration (safe for signal / panic contexts).
    pub fn restore_terminal_raw() {
        if TERMINAL_INITIALIZED.swap(false, Ordering::SeqCst) {
            let mut out = stdout();
            let _ = disable_raw_mode();
            let _ = execute!(out, LeaveAlternateScreen, DisableMouseCapture, Show);
            let _ = out.flush();
        }
    }

    /// Graceful explicit terminal restoration.
    pub fn restore(&mut self) -> Result<(), AppError> {
        if self.is_raw && self.terminal.is_some() {
            Self::restore_terminal_raw();
            self.is_raw = false;
        }
        Ok(())
    }

    /// Check whether this terminal was initialized in raw TTY mode.
    pub fn is_raw(&self) -> bool {
        self.is_raw
    }

    /// Automatically maximize console window on Windows without entering borderless fullscreen.
    #[cfg(windows)]
    pub fn maximize_window() {
        unsafe {
            #[link(name = "kernel32")]
            extern "system" {
                fn GetConsoleWindow() -> *mut std::ffi::c_void;
            }
            #[link(name = "user32")]
            extern "system" {
                fn ShowWindow(hWnd: *mut std::ffi::c_void, nCmdShow: i32) -> i32;
            }

            let hwnd = GetConsoleWindow();
            if !hwnd.is_null() {
                // SW_MAXIMIZE = 3
                ShowWindow(hwnd, 3);
            }
        }
    }

    /// No-op on non-Windows platforms.
    #[cfg(not(windows))]
    pub fn maximize_window() {}
}

impl Deref for TerminalGuard {
    type Target = Terminal<CrosstermBackend<Stdout>>;

    fn deref(&self) -> &Self::Target {
        self.terminal.as_ref().expect("Terminal already restored")
    }
}

impl DerefMut for TerminalGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.terminal.as_mut().expect("Terminal already restored")
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
