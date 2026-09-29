//! Terminal UI event loop, rendering, and worker lifecycle.
//! Terminal restoration and secret masking apply on every exit path.
use crate::{
    cwmp::model::Device,
    error::{Error, Result},
};
use crossterm::event;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io::{self, IsTerminal},
    path::Path,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::TryRecvError,
    },
    time::{Duration, Instant},
};

mod app;
mod clipboard;
mod form;
pub mod terminal;
mod theme;
mod views;
mod worker;

pub use app::{Action, App, Screen};
pub use form::{Field, ProfileForm};
use terminal::TerminalGuard;
pub use terminal::restore_terminal;
use worker::Worker;

/// Startup's local UTC offset, captured before any application thread starts.
pub static LOCAL_OFFSET: OnceLock<time::UtcOffset> = OnceLock::new();

/// Minimum viewport width for all screens.
pub const MIN_WIDTH: u16 = 52;
/// Minimum viewport height for all screens.
pub const MIN_HEIGHT: u16 = 16;

// Terminal errors intentionally share one recovery message; raw terminal I/O
// details can contain escape bytes and cannot change the user action.
/// Run the interactive terminal for `device` against a dedicated state directory.
#[allow(clippy::panic)] // Development-only trigger verifies terminal restoration after unwind.
pub fn run(root: &Path, device: Device) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(Error::Terminal);
    }
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal = interrupted.clone();
    ctrlc::set_handler(move || {
        signal.store(true, Ordering::SeqCst);
    })
    .map_err(|_| Error::Terminal)?;
    let _guard = TerminalGuard::enter()?;
    // Crossterm installs its SIGWINCH reader on first poll. Register before
    // the first frame so an early resize cannot be missed. Poll keeps input queued.
    let _ = event::poll(Duration::ZERO).map_err(|_| Error::Terminal)?;
    #[cfg(feature = "dev-provider-override")]
    if std::env::var_os("YETTEL_CWMP_DEV_PANIC").is_some() {
        panic!("synthetic-panic-payload-must-never-appear");
    }
    let mut terminal =
        Terminal::new(CrosstermBackend::new(io::stdout())).map_err(|_| Error::Terminal)?;
    let (mut app, mut startup_error) = initial_app(root, device);
    let mut worker: Option<Worker> = None;
    let mut quit_after_cancel = false;
    let mut dirty = true;
    let mut last_spinner = Instant::now();
    loop {
        if interrupted.swap(false, Ordering::SeqCst) {
            if !app.cancel(worker.as_ref()) {
                break;
            }
            quit_after_cancel = true;
        }
        match poll_worker(&mut app, &mut worker, quit_after_cancel) {
            WorkerPoll::QuitReady => break,
            WorkerPoll::Changed => dirty = true,
            WorkerPoll::Idle => {}
        }
        dirty |= update_timers(&mut app, &mut last_spinner);
        if dirty {
            draw_app(&mut terminal, &mut app, root)?;
            dirty = false;
        }
        if !event::poll(Duration::from_millis(50)).map_err(|_| Error::Terminal)? {
            continue;
        }
        let event = event::read().map_err(|_| Error::Terminal)?;
        let size = terminal.size().map_err(|_| Error::Terminal)?;
        let action = app.event(event, size);
        dirty = true;
        if matches!(action, Action::Quit) {
            if !app.cancel(worker.as_ref()) {
                break;
            }
            quit_after_cancel = true;
        } else {
            if matches!(
                app.screen,
                Screen::Running {
                    cancelling: true,
                    ..
                }
            ) {
                app.cancel(worker.as_ref());
                // Draw cancellation before polling the worker again, so a fast
                // completion cannot skip the visible intermediate state.
                draw_app(&mut terminal, &mut app, root)?;
                dirty = false;
            }
            match app.execute(action, root) {
                Ok(Some(started)) => worker = Some(started),
                Ok(None) => {
                    if action == Action::Retry && matches!(app.screen, Screen::Profiles) {
                        startup_error = None;
                    }
                }
                Err(error) => {
                    if action == Action::Retry
                        && matches!(
                            app.screen,
                            Screen::Error {
                                profile: None,
                                serial: None,
                                ..
                            }
                        )
                    {
                        startup_error = Some(error);
                    }
                    app.fail(error);
                }
            }
        }
    }
    drop(worker);
    match startup_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn initial_app(root: &Path, device: Device) -> (App, Option<Error>) {
    let mut app = App::new(device, vec![]);
    match app.load_profiles(root) {
        Ok(()) => (app, None),
        Err(error) => {
            app.fail(error);
            (app, Some(error))
        }
    }
}

fn draw_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    root: &Path,
) -> Result<()> {
    terminal
        .draw(|frame| app.render(frame, root))
        .map_err(|_| Error::Terminal)?;
    Ok(())
}

fn update_timers(app: &mut App, last_spinner: &mut Instant) -> bool {
    let mut changed = false;
    if matches!(app.screen, Screen::Running { .. })
        && last_spinner.elapsed() >= Duration::from_millis(200)
    {
        app.spinner = app.spinner.wrapping_add(1);
        *last_spinner = Instant::now();
        changed = true;
    }
    if app
        .copied_at
        .is_some_and(|when| when.elapsed() >= Duration::from_secs(3))
    {
        app.copied_at = None;
        changed = true;
    }
    changed
}

/// What one poll of the session worker changed.
enum WorkerPoll {
    /// No progress or result arrived.
    Idle,
    /// Progress or a result changed the screen.
    Changed,
    /// The worker finished after the user asked to quit.
    QuitReady,
}

fn poll_worker(app: &mut App, worker: &mut Option<Worker>, quit_after_cancel: bool) -> WorkerPoll {
    let Some(active) = worker.as_ref() else {
        return WorkerPoll::Idle;
    };
    let mut changed = false;
    let finished = loop {
        match active.rx.try_recv() {
            Ok(value) => {
                if let Screen::Running { progress, .. } = &mut app.screen {
                    *progress = value;
                    changed = true;
                }
            }
            Err(TryRecvError::Empty) => break false,
            Err(TryRecvError::Disconnected) => break true,
        }
    };
    if finished && let Some(active) = worker.take() {
        let progress = match app.screen {
            Screen::Running { progress, .. } => progress,
            _ => Default::default(),
        };
        let result = active.finish(progress);
        if quit_after_cancel {
            return WorkerPoll::QuitReady;
        }
        app.complete(result);
        return WorkerPoll::Changed;
    }
    if changed {
        WorkerPoll::Changed
    } else {
        WorkerPoll::Idle
    }
}
