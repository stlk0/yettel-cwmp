//! Terminal UI process tests for user-visible flows.
// Integration tests use assertions and unwraps to make fixture failures explicit.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Terminal,
    backend::{Backend, TestBackend},
    buffer::Buffer,
    layout::Size,
    style::Color,
};
use std::{process::Command, time::Instant};
use yettel_cwmp::{
    capture::Outcome,
    cwmp::model::{Assignments, Device},
    domain::{profile::ProfileSummary, secret::Secret, serial::Serial},
    error::Error,
    i18n::{self, Language},
    progress::Progress,
    ui::{Action, App, Field, Screen},
};
fn summary() -> ProfileSummary {
    common::profile().summary()
}
fn app(profiles: Vec<Serial>) -> App {
    App::new(Device::bundled().unwrap(), profiles)
}
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn render(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| app.render(frame, std::path::Path::new("/tmp/yettel-test")))
        .unwrap();
    terminal.backend().buffer().clone()
}
fn text(buffer: &Buffer) -> String {
    buffer
        .content
        .chunks(buffer.area.width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
fn rendered(app: &mut App) -> String {
    text(&render(app, 100, 28))
}
fn result_screen() -> Screen {
    Screen::Result {
        profile: summary(),
        outcome: Outcome {
            path: "synthetic.json".into(),
            export: common::device()
                .export(&Assignments::from([
                    (
                        format!("{}Username", common::ppp_prefix()),
                        Secret::new("ppp-user"),
                    ),
                    (
                        format!("{}Password", common::ppp_prefix()),
                        Secret::new("ppp-secret\u{1b}[31m"),
                    ),
                ]))
                .unwrap(),
        },
        reveal: false,
        fresh: true,
    }
}
fn running_screen() -> Screen {
    Screen::Running {
        profile: summary(),
        progress: Progress::default(),
        cancelling: false,
        started: Instant::now(),
    }
}
fn error_screen(error: Error) -> Screen {
    Screen::Error {
        profile: Some(summary()),
        serial: None,
        error,
        progress: Progress::default(),
    }
}
#[test]
fn escape_never_quits_and_q_quits_outside_forms() {
    let screens = [
        Screen::Profiles,
        Screen::Create,
        Screen::Device(summary()),
        Screen::ChangeKey(summary()),
        Screen::ReplaceSaved(summary()),
        Screen::DeleteConfirm {
            serial: "SYN123456".parse().unwrap(),
            profile: Some(summary()),
        },
        Screen::ConnectNotice(summary()),
        running_screen(),
        result_screen(),
        error_screen(Error::Dns),
    ];
    for screen in screens {
        let mut app = app(vec![]);
        app.show(screen);
        assert_ne!(app.key(key(KeyCode::Esc)), Action::Quit);
    }
    let mut app = app(vec![]);
    assert_eq!(app.key(key(KeyCode::Char('q'))), Action::Quit);
    app.show(Screen::Create);
    app.key(key(KeyCode::Char('q')));
    app.key(key(KeyCode::Char('l')));
    assert_eq!(app.form.serial, "ql");
}
#[test]
fn defaults_and_shortcuts_follow_available_actions() {
    let mut app = app(vec![]);
    app.key(key(KeyCode::Enter));
    assert!(matches!(app.screen, Screen::Create));
    app.key(key(KeyCode::Esc));
    app.show(Screen::Device(summary()));
    assert_eq!(app.key(key(KeyCode::Enter)), Action::Start);
    app.key(key(KeyCode::Char('K')));
    assert!(matches!(app.screen, Screen::ChangeKey(_)));
    app.key(key(KeyCode::Esc));
    app.key(key(KeyCode::Char('D')));
    assert!(matches!(app.screen, Screen::DeleteConfirm { .. }));
    app.key(key(KeyCode::Esc));
    app.show(result_screen());
    app.key(key(KeyCode::Enter));
    assert!(matches!(app.screen, Screen::Result { reveal: true, .. }));
}
#[test]
fn form_mask_reveal_reset_and_cursor() {
    let mut app = app(vec![]);
    app.show(Screen::Create);
    app.form.field = Field::Key;
    app.paste("synthetic-wifi-key");
    let hidden = rendered(&mut app);
    assert!(!hidden.contains("synthetic-wifi-key"));
    assert!(hidden.contains("Wi-Fi key (WLAN Security):"));
    app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    assert!(rendered(&mut app).contains("synthetic-wifi-key"));
    app.key(key(KeyCode::Esc));
    app.show(Screen::Create);
    app.form.field = Field::Key;
    assert!(!rendered(&mut app).contains("synthetic-wifi-key"));
    app.key(key(KeyCode::F(2)));
    assert!(rendered(&mut app).contains("synthetic-wifi-key"));
    let mut terminal = Terminal::new(TestBackend::new(52, 16)).unwrap();
    terminal
        .draw(|frame| app.render(frame, std::path::Path::new("/tmp/yettel-test")))
        .unwrap();
    assert!(terminal.backend_mut().get_cursor_position().unwrap().y > 0);
}
#[test]
fn mac_typing_paste_backspace_and_invalid_input() {
    let mut app = app(vec![]);
    app.show(Screen::Create);
    app.form.field = Field::Mac;
    for c in "02".chars() {
        app.key(key(KeyCode::Char(c)));
    }
    assert_eq!(app.form.mac, "02:");
    app.key(key(KeyCode::Backspace));
    assert_eq!(app.form.mac, "0");
    for pasted in ["02-aa-bb-cc-dd-ee", "02aabbccddee", "02:aa:bb:cc:dd:ee"] {
        app.paste(pasted);
        assert_eq!(app.form.mac, "02:AA:BB:CC:DD:EE");
    }
    for invalid in ["02:AA:BB:CC:DD:GG", "02aabbccddee00", "02aa\nbbccddee"] {
        app.paste(invalid);
        assert_eq!(app.form.error, Some(Error::InvalidMac));
        assert_eq!(app.form.mac, "02:AA:BB:CC:DD:EE");
    }
}
#[test]
fn validation_preserves_draft_and_key_character_limit() {
    let mut app = app(vec![]);
    app.show(Screen::Create);
    app.form.serial = "SYN123456".into();
    app.form.mac = "01:11:22:33:44:55".into();
    app.form.key = "synthetic-secret".into();
    app.form.field = Field::Key;
    assert_eq!(app.key(key(KeyCode::Enter)), Action::None);
    assert_eq!(app.form.field, Field::Mac);
    assert_eq!(app.form.error, Some(Error::InvalidMac));
    assert!(!rendered(&mut app).contains("synthetic-secret"));
    app.form.field = Field::Key;
    app.form.key.clear();
    app.paste(&"é".repeat(1024));
    assert_eq!(app.form.key.chars().count(), 1024);
    app.paste("é");
    assert_eq!(app.form.error, Some(Error::InvalidText));
}
#[test]
fn result_masks_both_credentials_and_scroll_stops_at_end() {
    let mut app = app(vec![]);
    let mut result = result_screen();
    if let Screen::Result { outcome, .. } = &mut result {
        outcome.path = format!("/{}export-marker.json", "long-directory/".repeat(20)).into();
    }
    app.show(result);
    let initial = text(&render(&mut app, 52, 16));
    assert!(initial.contains("Connection type: PPPoE"));
    assert!(!initial.contains("ppp-user"));
    assert!(!initial.contains("ppp-secret"));
    for _ in 0..100 {
        app.key(key(KeyCode::PageDown));
    }
    let bottom = text(&render(&mut app, 52, 16));
    assert!(bottom.contains("export-marker.json"), "{bottom}");
    app.key(key(KeyCode::Char('s')));
    app.key(key(KeyCode::Home));
    let revealed = text(&render(&mut app, 52, 16));
    assert!(revealed.contains("ppp-user"));
    assert!(revealed.contains("ppp-secret"));
    assert!(!revealed.contains('\u{1b}'));
}
#[test]
fn help_f1_works_in_form_and_window_minimum_blocks_paste() {
    let mut app = app(vec![]);
    app.show(Screen::Create);
    app.key(key(KeyCode::F(1)));
    assert!(rendered(&mut app).contains("Help"));
    app.key(key(KeyCode::Esc));
    assert!(matches!(app.screen, Screen::Create));
    let small = Size::new(24, 8);
    app.event(Event::Paste("hidden".into()), small);
    assert!(app.form.serial.is_empty());
    assert!(text(&render(&mut app, 24, 8)).contains("Window too small"));
    assert_eq!(
        app.event(
            Event::Key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::CONTROL)),
            small
        ),
        Action::Quit
    );
}
#[test]
fn narrow_form_keeps_end_of_long_serial_visible() {
    let mut app = app(vec![]);
    app.show(Screen::Create);
    app.paste(&format!("38165A-{}TAIL1234", "A".repeat(56)));
    let screen = text(&render(&mut app, 52, 16));
    assert!(screen.contains("TAIL1234"), "{screen}");
    app.form.field = Field::Key;
    app.form.reveal = true;
    app.paste(&format!("{}KEYTAIL9", "k".repeat(56)));
    let screen = text(&render(&mut app, 52, 16));
    assert!(screen.contains("KEYTAIL9"), "{screen}");
}
#[test]
fn no_color_uses_terminal_defaults() {
    const PROBE: &str = "YETTEL_SYNTHETIC_NO_COLOR_PROBE";
    if std::env::var_os(PROBE).is_some() {
        let mut app = app(vec![]);
        app.show(error_screen(Error::Storage));
        let buffer = render(&mut app, 52, 16);
        assert!(
            buffer
                .content
                .iter()
                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
        );
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "no_color_uses_terminal_defaults"])
        .env(PROBE, "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn errors_keep_guidance_code_and_details_visible_at_minimum_size() {
    for error in Error::ALL {
        let mut app = app(vec![]);
        app.show(error_screen(error));
        let page = text(&render(&mut app, 52, 16));
        let guidance = i18n::lookup(Language::En, &format!("error.{}", error.code()))
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            page.contains(&guidance),
            "{error:?}: missing guidance: {page}"
        );
        assert!(
            page.contains(error.code()),
            "{error:?}: missing code: {page}"
        );
        assert!(page.contains("Stage:"), "{error:?}: missing stage: {page}");
        assert!(page.contains("RPC:"), "{error:?}: missing RPC: {page}");
    }
}

#[test]
fn non_tty_help_version_unknown_option_and_no_state() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("must-not-be-created");
    let binary = env!("CARGO_BIN_EXE_yettel-cwmp");
    let result = Command::new(binary)
        .args(["--lang", "en"])
        .args(["--state-dir", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("interactive terminal"));
    assert!(!path.exists());
    let explicit_english = Command::new(binary)
        .args(["--lang", "en", "--help"])
        .output()
        .unwrap();
    assert!(explicit_english.status.success());
    assert!(String::from_utf8_lossy(&explicit_english.stdout).contains("Usage:"));
    assert!(!String::from_utf8_lossy(&explicit_english.stdout).contains("Upotreba:"));
    for (args, code) in [
        ("--help", 0),
        ("--version", 0),
        ("--unknown", 1),
        ("--state-dir", 1),
    ] {
        assert_eq!(
            Command::new(binary)
                .args(["--lang", "en"])
                .arg(args)
                .output()
                .unwrap()
                .status
                .code(),
            Some(code),
            "{args}"
        );
    }
}
/// Rerun `test` alone in a child process, where it may change the process-wide language.
/// Returns true in the child, which then runs the test body.
fn in_own_process(test: &str) -> bool {
    const PROBE: &str = "YETTEL_TEST_OWN_PROCESS";
    if std::env::var_os(PROBE).is_some() {
        return true;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test])
        .env(PROBE, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("1 passed"), "{stdout}");
    false
}
fn serbian_screen_expectations() -> Vec<(Screen, &'static [&'static str])> {
    let mut error = error_screen(Error::Dns);
    if let Screen::Error { progress, .. } = &mut error {
        progress.stage = yettel_cwmp::progress::Stage::Session;
    }
    vec![
        (
            Screen::Profiles,
            &["Dobro došli", "Dodaj ruter (N)", "Izlaz (Q)", "L English"],
        ),
        (
            Screen::Create,
            &[
                "Dodaj ruter",
                "Serijski broj:",
                "Wi-Fi ključ (WLAN Security):",
                "Tab/Up/Down Polje",
                "Enter Dalje/Sačuvaj",
                "Ctrl+U Obriši",
                "Esc Nazad",
            ],
        ),
        (
            Screen::Device(summary()),
            &[
                "Ruter SYN123456",
                "Preuzmi podešavanja za internet (R)",
                "Promeni Wi-Fi ključ (K)",
                "Obriši ovaj ruter (D)",
                "Nazad (Esc)",
            ],
        ),
        (
            Screen::ChangeKey(summary()),
            &[
                "Promeni Wi-Fi ključ",
                "Wi-Fi ključ (WLAN Security):",
                "Ctrl+U Obriši",
                "Esc Nazad",
            ],
        ),
        (
            Screen::ReplaceSaved(summary()),
            &[
                "Zameniti podatke za prijavu?",
                "Zadrži trenutne podatke (Esc)",
                "Zameni (Y)",
            ],
        ),
        (
            Screen::DeleteConfirm {
                serial: "SYN123456".parse().unwrap(),
                profile: Some(summary()),
            },
            &["Obrisati ovaj ruter?", "Zadrži (Esc)", "Obriši (Y)"],
        ),
        (
            Screen::ConnectNotice(summary()),
            &["Pre povezivanja", "Nastavi (Enter)", "Nazad (Esc)"],
        ),
        (
            running_screen(),
            &[
                "Preuzimanje podešavanja za internet",
                "Povezivanje sa provajderom",
                "Otkaži (Esc)",
            ],
        ),
        (
            result_screen(),
            &[
                "Gotovo: podešavanja za internet su preuzeta",
                "Prikaži podatke za prijavu (S)",
                "Kopiraj lozinku (C)",
                "Kopiraj korisničko ime (U)",
                "Nazad (Esc)",
            ],
        ),
        (
            error,
            &[
                "Server provajdera ne može da se pronađe",
                "Pokušaj ponovo (R)",
                "Nazad (Esc)",
                "Faza: Sesija",
                "RPC:",
                "NET-DNS",
            ],
        ),
    ]
}

#[test]
fn serbian_titles_and_available_menus_fit_minimum_window() {
    if !in_own_process("serbian_titles_and_available_menus_fit_minimum_window") {
        return;
    }
    i18n::init(Language::Sr);

    let screens = serbian_screen_expectations();
    for (screen, expected) in screens {
        let mut app = App::new(Device::bundled().unwrap(), vec![]);
        app.show(screen);
        let contents = text(&render(&mut app, 52, 16));
        assert!(!contents.contains("network:"));
        assert!(!contents.contains("synthetic-wlan"));
        for label in expected {
            assert!(
                contents.contains(label),
                "missing {label:?} in {contents:?}"
            );
        }
    }
    for error in Error::ALL {
        let mut app = App::new(Device::bundled().unwrap(), vec![]);
        app.show(error_screen(error));
        let contents = text(&render(&mut app, 52, 16));
        let guidance = i18n::lookup(Language::Sr, &format!("error.{}", error.code()))
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join(" ");
        for label in [guidance.as_str(), error.code(), "Faza:", "RPC:"] {
            assert!(
                contents.contains(label),
                "{error:?}: missing {label:?} in {contents:?}"
            );
        }
    }
    let mut app = App::new(Device::bundled().unwrap(), vec![]);
    app.show(Screen::Device(summary()));
    app.key(key(KeyCode::F(1)));
    let help = text(&render(&mut app, 52, 16));
    assert!(help.contains("Pomoć"));
    assert!(help.contains("Vaši podaci se čuvaju u:"));
}
#[test]
fn l_switches_language_outside_forms() {
    if !in_own_process("l_switches_language_outside_forms") {
        return;
    }
    let mut app = app(vec![]);
    assert!(text(&render(&mut app, 52, 16)).contains("L Srpski"));
    app.key(key(KeyCode::Char('L')));
    assert_eq!(i18n::language(), Language::Sr);
    let serbian = text(&render(&mut app, 52, 16));
    assert!(serbian.contains("Dobro došli"), "{serbian}");
    assert!(serbian.contains("L English"), "{serbian}");
    app.key(key(KeyCode::Char('?')));
    app.key(key(KeyCode::Char('l')));
    assert_eq!(i18n::language(), Language::En);
    assert!(rendered(&mut app).contains("L: Srpski"));
    app.key(key(KeyCode::Esc));
    app.show(result_screen());
    app.key(key(KeyCode::Char('l')));
    assert_eq!(i18n::language(), Language::Sr);
    assert!(matches!(app.screen, Screen::Result { .. }));
    app.show(Screen::Create);
    app.key(key(KeyCode::Char('l')));
    assert_eq!(app.form.serial, "l");
    assert_eq!(i18n::language(), Language::Sr);
}
