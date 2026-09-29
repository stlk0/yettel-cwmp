//! Process entry point and payload-free panic reporting.
use std::process::ExitCode;
use yettel_cwmp::{cli, i18n::t, ui};

#[allow(clippy::print_stderr)] // A panic restores the terminal before reporting its location.
fn main() -> ExitCode {
    let _ = ui::LOCAL_OFFSET
        .set(time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC));
    // Only a main-thread panic ends the app. A worker panic is reported by the UI as an
    // internal error, so the terminal stays in the app's screen mode.
    let main_thread = std::thread::current().id();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() != main_thread {
            return;
        }
        ui::restore_terminal();
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_default();
        eprintln!("{} {location}", t("cli.internal"));
    }));
    cli::application()
}
