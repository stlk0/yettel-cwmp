//! Explicit OSC 52 clipboard copy for terminal emulators that support it.
use crate::error::{Error, Result};
use crossterm::{clipboard::CopyToClipboard, execute};
use std::io;

pub fn copy(value: &str) -> Result<()> {
    execute!(io::stdout(), CopyToClipboard::to_clipboard_from(value)).map_err(|_| Error::Terminal)
}
