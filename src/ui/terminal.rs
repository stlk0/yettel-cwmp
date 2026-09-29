//! Terminal setup and idempotent restoration on every exit path.
use crate::error::{Error, Result};
use crossterm::{
    cursor::Show,
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};

/// Restoration is idempotent and also called by the process panic hook.
pub fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        DisableBracketedPaste,
        Show,
        LeaveAlternateScreen
    );
    let _ = io::stdout().flush();
}
#[doc(hidden)]
pub struct TerminalGuard;
// Crossterm failures remain a single terminal error because their OS text may
// include control bytes and the only safe recovery is to restore the terminal.
impl TerminalGuard {
    #[doc(hidden)]
    pub fn enter() -> Result<Self> {
        let guard = Self;
        enable_raw_mode().map_err(|_| Error::Terminal)?;
        execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)
            .map_err(|_| Error::Terminal)?;
        Ok(guard)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}
