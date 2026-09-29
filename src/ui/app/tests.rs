//! Application state and navigation tests.
use super::*;
use crate::{
    cwmp::model::Device,
    domain::profile::{CredentialsSource, Profile},
    store::Store,
};
use crossterm::event::KeyEvent;
use ratatui::{Terminal, backend::TestBackend};

fn profile() -> Profile {
    Profile::new(
        "SYN123456".parse().unwrap(),
        "02:11:22:33:44:55".parse().unwrap(),
        "synthetic-key".into(),
    )
    .unwrap()
}
fn app(profiles: Vec<Serial>) -> App {
    App::new(Device::bundled().unwrap(), profiles)
}
fn render(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(52, 16)).unwrap();
    let state = tempfile::tempdir().unwrap();
    terminal
        .draw(|frame| app.render(frame, state.path()))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
#[test]
fn q_quits_help_and_running_but_remains_form_text() {
    let mut app = app(vec![]);
    app.help_open = true;
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE)),
        Action::Quit
    );
    app.help_open = false;
    app.show(Screen::Running {
        profile: profile().summary(),
        progress: Progress::default(),
        cancelling: false,
        started: Instant::now(),
    });
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        Action::Quit
    );
    app.show(Screen::Create);
    app.form.field = Field::Key;
    app.help_open = true;
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE)),
        Action::Quit
    );
    app.help_open = false;
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE)),
        Action::None
    );
    assert_eq!(app.form.key, "Q");
}

#[test]
fn receiving_and_retrying_always_require_the_notice() {
    let state = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(state.path()).unwrap());
    let profile = profile();
    store.create(&profile).unwrap();
    let mut app = app(vec![profile.serial.clone()]);
    app.store = Some(store.clone());
    app.show(Screen::Device(profile.summary()));
    app.execute(Action::Start, state.path()).unwrap();
    assert!(matches!(app.screen, Screen::ConnectNotice(_)));
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.execute(Action::Start, state.path()).unwrap();
    assert!(matches!(app.screen, Screen::ConnectNotice(_)));
    app.fail(Error::Dns);
    app.execute(Action::Retry, state.path()).unwrap();
    assert!(matches!(app.screen, Screen::ConnectNotice(_)));
    assert!(matches!(
        Store::open(state.path()),
        Err(Error::AlreadyRunning)
    ));
}

#[test]
fn rotated_auth_error_key_change_requires_confirmation() {
    use crate::domain::secret::Secret;
    let state = tempfile::tempdir().unwrap();
    let root = state.path().join("state");
    let mut rotated = profile();
    rotated.username = Secret::new("rotated-user");
    rotated.password = Secret::new("rotated-password");
    rotated.credentials_source = CredentialsSource::Server;
    let store = Arc::new(Store::open(&root).unwrap());
    store.create(&rotated).unwrap();
    let mut app = app(vec![rotated.serial.clone()]);
    app.store = Some(store.clone());
    app.show(Screen::Error {
        profile: Some(rotated.summary()),
        serial: None,
        error: Error::AuthRejected,
        progress: Progress::default(),
    });
    assert!(render(&mut app).contains("Change Wi-Fi key"));
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    assert!(matches!(app.screen, Screen::ChangeKey(_)));
    app.form.key = "new-label-key".into();
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::ChangeKey
    );
    app.execute(Action::ChangeKey, &root).unwrap();
    assert!(matches!(app.screen, Screen::ReplaceSaved(_)));
    assert_eq!(
        store.load(&rotated.serial).unwrap().password.expose(),
        "rotated-password"
    );
    app.execute(Action::ConfirmReplace, &root).unwrap();
    let changed = store.load(&rotated.serial).unwrap();
    assert_eq!(changed.password.expose(), "new-label-key");
    assert_eq!(changed.credentials_source, CredentialsSource::Label);
}
