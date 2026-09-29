//! Command-line parsing, state directory precedence, and startup errors.
use crate::{
    cwmp::model::Device,
    error::Error,
    i18n::{self, Language, t},
    store, ui,
};
use lexopt::{Arg, Parser};
use std::{
    ffi::OsString,
    io::{self, IsTerminal},
    path::PathBuf,
    process::ExitCode,
};

/// Startup options, retained while parsing so errors use the requested language.
#[derive(Default)]
struct Options {
    state: Option<PathBuf>,
    language: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
/// Requested startup action.
enum Invocation {
    Help,
    Version,
    Run,
}

fn value(parser: &mut Parser) -> Result<OsString, &'static str> {
    parser.value().map_err(|_| "cli.option_needs_value")
}
fn text_value(parser: &mut Parser) -> Result<String, &'static str> {
    value(parser)?
        .into_string()
        .map_err(|_| "cli.option_value_text")
}
fn once<T>(slot: &mut Option<T>, value: T) -> Result<(), &'static str> {
    if slot.replace(value).is_some() {
        return Err("cli.repeated_option");
    }
    Ok(())
}

/// Parse startup options without retaining or echoing invalid argument values.
fn parse(parser: &mut Parser, options: &mut Options) -> Result<Invocation, &'static str> {
    let mut help = false;
    let mut version = false;
    while let Some(arg) = parser.next().map_err(|_| "cli.invalid_option")? {
        match arg {
            Arg::Long("help") | Arg::Short('h') => help = true,
            Arg::Long("version") | Arg::Short('V') => version = true,
            Arg::Long("state-dir") => once(&mut options.state, PathBuf::from(value(parser)?))?,
            Arg::Long("lang") => once(&mut options.language, text_value(parser)?)?,
            _ => return Err("cli.unknown_option"),
        }
    }
    Ok(if version {
        Invocation::Version
    } else if help {
        Invocation::Help
    } else {
        Invocation::Run
    })
}

#[allow(clippy::print_stderr)] // Startup failures are shown outside the TUI.
fn fallback(message: &str) -> ExitCode {
    eprintln!("{message}");
    if should_pause_for_double_click() {
        eprint!("{}", t("cli.pause_close"));
        let _ = io::stdin().read_line(&mut String::new());
    }
    ExitCode::from(1)
}

/// Select `--lang`, otherwise Serbian for a Serbian system locale, otherwise English.
/// An unknown `--lang` value is reported in the detected language.
fn select_language(requested: Option<&str>) -> (Language, bool) {
    let detected = if sys_locale::get_locale().is_some_and(|locale| locale.starts_with("sr")) {
        Language::Sr
    } else {
        Language::En
    };
    match requested {
        None => (detected, true),
        Some("en") => (Language::En, true),
        Some("sr") => (Language::Sr, true),
        Some(_) => (detected, false),
    }
}

/// Parse the command line, select the language and run the terminal interface.
#[allow(clippy::print_stdout, clippy::print_stderr)] // Terminal CLI output is intentional.
pub fn application() -> ExitCode {
    let mut options = Options::default();
    let mut invocation = parse(&mut Parser::from_env(), &mut options);
    let (language, known) = select_language(options.language.as_deref());
    if !known && invocation.is_ok() {
        invocation = Err("cli.unknown_language");
    }
    i18n::init(language);
    match invocation {
        Ok(Invocation::Run) => (),
        Ok(Invocation::Version) => {
            #[cfg(feature = "dev-provider-override")]
            let annotation = crate::dev::version_annotation();
            #[cfg(feature = "dev-provider-override")]
            println!("yettel-cwmp {} ({annotation})", env!("CARGO_PKG_VERSION"));
            #[cfg(not(feature = "dev-provider-override"))]
            println!("yettel-cwmp {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Ok(Invocation::Help) => {
            println!("{}", t("cli.usage"));
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("{}", t(message));
            return ExitCode::FAILURE;
        }
    };
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!("{}", t("cli.need_terminal"));
        return ExitCode::from(2);
    }
    let report = |error: Error| {
        fallback(&format!(
            "{} ({})",
            i18n::error_message(error),
            error.code()
        ))
    };
    let root = match options
        .state
        .map(Ok)
        .unwrap_or_else(store::default_state_dir)
    {
        Ok(path) => path,
        Err(error) => return report(error),
    };
    let device = match selected_device() {
        Ok(device) => device,
        Err(error) => return report(error),
    };
    match ui::run(&root, device) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report(error),
    }
}

/// The provider and router for this run. Development builds may point it at a
/// loopback fake provider; release builds always use the bundled endpoint.
fn selected_device() -> crate::error::Result<Device> {
    let device = Device::bundled()?;
    #[cfg(feature = "dev-provider-override")]
    let device = crate::dev::with_provider_override(device)?;
    Ok(device)
}

#[cfg(windows)]
fn should_pause_for_double_click() -> bool {
    use windows_sys::Win32::System::Console::GetConsoleProcessList;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return false;
    }
    let mut processes = [0_u32; 2];
    // A console with one attached process is likely a double-click launch.
    // SAFETY: the fixed array is writable for its declared length; the API only fills it.
    unsafe {
        GetConsoleProcessList(
            processes.as_mut_ptr(),
            u32::try_from(processes.len()).unwrap_or(0),
        ) == 1
    }
}

#[cfg(not(windows))]
fn should_pause_for_double_click() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args<const N: usize>(args: [&str; N]) -> Result<Invocation, &'static str> {
        parse(&mut Parser::from_args(args), &mut Options::default())
    }

    #[test]
    fn syntax_errors_never_echo_supplied_values() {
        assert_eq!(
            parse_args(["--unknown-secret"]).err(),
            Some("cli.unknown_option")
        );
        assert_eq!(
            parse_args(["--state-dir"]).err(),
            Some("cli.option_needs_value")
        );
        assert_eq!(
            parse_args(["--lang", "en", "--lang", "sr"]).err(),
            Some("cli.repeated_option")
        );
    }

    #[test]
    fn startup_options_and_noninteractive_information() {
        let mut options = Options::default();
        let invocation = parse(
            &mut Parser::from_args(["--state-dir", "synthetic-state", "--lang", "sr"]),
            &mut options,
        )
        .unwrap();
        assert_eq!(invocation, Invocation::Run);
        assert_eq!(options.state, Some(PathBuf::from("synthetic-state")));
        assert_eq!(options.language.as_deref(), Some("sr"));
        assert_eq!(parse_args([]).unwrap(), Invocation::Run);
        assert_eq!(parse_args(["--help"]).unwrap(), Invocation::Help);
        assert_eq!(parse_args(["-h"]).unwrap(), Invocation::Help);
        assert_eq!(parse_args(["--version"]).unwrap(), Invocation::Version);
        assert_eq!(parse_args(["-V"]).unwrap(), Invocation::Version);
        assert_eq!(
            parse_args(["--help", "--version"]).unwrap(),
            Invocation::Version
        );
        assert!(parse_args(["unexpected-command"]).is_err());
    }
}
