//! Screen state and navigation. Key handling changes only this state; actions that touch saved
//! routers or start a capture are handled separately in `actions`.
use super::{
    form::{Field, ProfileForm},
    theme::usable,
};
use crate::i18n::t;
use crate::{
    cwmp::model::Device,
    domain::{profile::ProfileSummary, serial::Serial},
    error::{Error, Result},
    progress::Progress,
    store::Store,
};
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{layout::Size, widgets::ListState};
use std::{
    sync::Arc,
    time::{Instant, SystemTime},
};
use zeroize::Zeroize;
mod actions;
mod keys;
mod model;

pub use model::{Action, Screen};

#[derive(Clone, Copy)]
pub struct KeyHint {
    pub key: &'static str,
    pub label: &'static str,
    pub code: KeyCode,
}
impl KeyHint {
    fn new(key: &'static str, label: &'static str, code: KeyCode) -> Self {
        Self { key, label, code }
    }
    pub fn display(self) -> String {
        format!("{} ({})", self.label, self.key)
    }
    pub fn footer(self) -> String {
        let short = self.label.split_whitespace().next().unwrap_or(self.label);
        format!("{} {}", self.key, short)
    }
}

/// Terminal application state and navigation context.
pub struct App {
    /// Current screen.
    pub screen: Screen,
    /// Saved router serial numbers.
    pub profiles: Vec<Serial>,
    /// Draft profile form.
    pub form: ProfileForm,
    /// Provider and router this UI creates profiles for and connects to.
    pub(super) device: Arc<Device>,
    pub(super) store: Option<Arc<Store>>,
    pub(super) profile_list: ListState,
    pub(super) menu_list: ListState,
    pub(super) scroll: u16,
    pub(super) max_scroll: u16,
    pub(super) export_modified: Option<SystemTime>,
    pub(super) help_open: bool,
    pub(super) copied_at: Option<Instant>,
    pub(super) spinner: usize,
}
impl App {
    /// Initialize the app for `device` with saved router serials.
    pub fn new(device: Device, profiles: Vec<Serial>) -> Self {
        Self {
            screen: Screen::Profiles,
            profiles,
            form: ProfileForm::default(),
            device: Arc::new(device),
            store: None,
            profile_list: ListState::default().with_selected(Some(0)),
            menu_list: ListState::default().with_selected(Some(0)),
            scroll: 0,
            max_scroll: 0,
            export_modified: None,
            help_open: false,
            copied_at: None,
            spinner: 0,
        }
    }
    /// Switch screens and reset view-specific navigation state.
    pub fn show(&mut self, screen: Screen) {
        self.screen = screen;
        let selected = if matches!(self.screen, Screen::Device(_)) && self.export_modified.is_some()
        {
            1
        } else {
            0
        };
        self.menu_list = ListState::default().with_selected(Some(selected));
        self.scroll = 0;
        self.max_scroll = 0;
        self.help_open = false;
        self.form.reveal = false;
    }
    pub(super) fn menu(&self) -> Vec<KeyHint> {
        let back = KeyHint::new("Esc", t("common.back"), KeyCode::Esc);
        let quit = KeyHint::new("Q", t("common.quit"), KeyCode::Char('q'));
        let retry = KeyHint::new("R", t("error.retry"), KeyCode::Char('r'));
        let change = KeyHint::new("K", t("device.change_key"), KeyCode::Char('k'));
        match &self.screen {
            Screen::Profiles => vec![
                KeyHint::new("N", t("profiles.add"), KeyCode::Char('n')),
                quit,
            ],
            Screen::Device(_) => {
                let mut hints = vec![KeyHint::new("R", t("device.receive"), KeyCode::Char('r'))];
                if self.export_modified.is_some() {
                    hints.push(KeyHint::new("V", t("device.view"), KeyCode::Char('v')));
                }
                hints.extend([
                    change,
                    KeyHint::new("D", t("device.delete"), KeyCode::Char('d')),
                    back,
                ]);
                hints
            }
            Screen::ReplaceSaved(_) => vec![
                KeyHint::new("Esc", t("device.replace.keep"), KeyCode::Esc),
                KeyHint::new("Y", t("device.replace.confirm"), KeyCode::Char('y')),
            ],
            Screen::DeleteConfirm { .. } => vec![
                KeyHint::new("Esc", t("device.delete.keep"), KeyCode::Esc),
                KeyHint::new("Y", t("device.delete.confirm"), KeyCode::Char('y')),
            ],
            Screen::ConnectNotice(_) => vec![
                KeyHint::new("Enter", t("connect.continue"), KeyCode::Enter),
                back,
            ],
            Screen::Running { .. } => vec![KeyHint::new("Esc", t("running.cancel"), KeyCode::Esc)],
            Screen::Result { .. } => vec![
                KeyHint::new("S", t("result.show"), KeyCode::Char('s')),
                KeyHint::new("C", t("result.copy_password"), KeyCode::Char('c')),
                KeyHint::new("U", t("result.copy_username"), KeyCode::Char('u')),
                back,
            ],
            Screen::Error {
                profile: None,
                serial: None,
                ..
            } => vec![retry, quit],
            Screen::Error {
                error: Error::ProfileInvalid,
                serial: Some(_),
                ..
            } => vec![
                back,
                KeyHint::new("X", t("device.delete"), KeyCode::Char('x')),
            ],
            Screen::Error {
                error: Error::AuthRejected,
                profile: Some(_),
                ..
            } => vec![back, change, retry],
            Screen::Error { error, .. } if Self::retryable(*error) => vec![back, retry],
            Screen::Error { .. } => vec![back],
            Screen::Create | Screen::ChangeKey(_) => vec![],
        }
    }
    fn retryable(error: Error) -> bool {
        matches!(
            error,
            Error::Dns
                | Error::Connect
                | Error::Timeout
                | Error::Network
                | Error::Tls
                | Error::HttpStatus(_)
                | Error::Protocol
                | Error::Incomplete
                | Error::Cancelled
                | Error::Deadline
        )
    }
    /// Build a compact footer from the actions available on this screen.
    pub fn footer(&self) -> String {
        if self.help_open {
            return format!("Esc {}", t("common.back"));
        }
        let hints = if matches!(self.screen, Screen::Create | Screen::ChangeKey(_)) {
            vec![
                KeyHint::new("Tab/Up/Down", t("form.field"), KeyCode::Tab),
                KeyHint::new("Enter", t("form.save"), KeyCode::Enter),
                KeyHint::new("Ctrl+U", t("form.clear"), KeyCode::Char('u')),
                KeyHint::new("Esc", t("common.back"), KeyCode::Esc),
            ]
        } else {
            self.menu()
        };
        let visible = if matches!(self.screen, Screen::Create | Screen::ChangeKey(_)) {
            hints.len()
        } else {
            2
        };
        let parts = hints
            .iter()
            .take(visible)
            .map(|hint| {
                if matches!(self.screen, Screen::Result { reveal: true, .. })
                    && hint.code == KeyCode::Char('s')
                {
                    format!("S {}", t("result.hide"))
                } else {
                    hint.footer()
                }
            })
            .collect::<Vec<_>>();
        let mut footer = if matches!(self.screen, Screen::Create | Screen::ChangeKey(_)) {
            format!("{}\n{}", parts[..2].join("  "), parts[2..].join("  "))
        } else {
            parts.join("  ")
        };
        if matches!(self.screen, Screen::Result { .. } | Screen::Error { .. })
            && self.max_scroll > 0
        {
            footer.push_str("  PgUp/PgDn ");
            footer.push_str(t("common.scroll"));
        }
        if !matches!(
            self.screen,
            Screen::Running { .. } | Screen::Create | Screen::ChangeKey(_)
        ) {
            footer.push_str("  ? ");
            footer.push_str(t("help.title"));
        }
        // L works on every screen but forms; only the start screen has room to show it.
        if matches!(self.screen, Screen::Profiles) {
            footer.push_str("  L ");
            footer.push_str(t("common.language"));
        }
        footer
    }
    /// Display a classified error while preserving router context.
    pub fn fail(&mut self, error: Error) {
        if matches!(self.screen, Screen::Create | Screen::ChangeKey(_)) {
            self.form.error = Some(error);
            return;
        }
        let progress = match self.screen {
            Screen::Running { progress, .. } | Screen::Error { progress, .. } => progress,
            _ => Progress::default(),
        };
        let previous = std::mem::replace(&mut self.screen, Screen::Profiles);
        let serial = match &previous {
            Screen::Error { serial, .. } => serial.clone(),
            Screen::DeleteConfirm { serial, .. } => Some(serial.clone()),
            _ => None,
        };
        let profile = previous.into_profile();
        self.show(Screen::Error {
            profile,
            serial,
            error,
            progress,
        });
    }
    // A router action on a screen without a router is an app bug, not damaged saved data.
    fn selected_profile(&self) -> Result<&ProfileSummary> {
        self.screen.profile().ok_or(Error::Internal)
    }
    fn take_profile(&mut self) -> Result<ProfileSummary> {
        std::mem::replace(&mut self.screen, Screen::Profiles)
            .into_profile()
            .ok_or(Error::Internal)
    }
    pub(super) fn back(&mut self) {
        let previous = std::mem::replace(&mut self.screen, Screen::Profiles);
        let profile = previous.into_profile();
        self.show(profile.map_or(Screen::Profiles, Screen::Device));
    }
    /// Paste is data, never shortcuts or submissions.
    pub fn paste(&mut self, value: &str) {
        if matches!(self.screen, Screen::Create | Screen::ChangeKey(_)) {
            let value = if self.form.field == Field::Key {
                value
            } else {
                value.trim()
            };
            self.form.insert(value, true);
        }
    }
    /// Process a terminal event within the given window size.
    pub fn event(&mut self, event: Event, size: Size) -> Action {
        match event {
            Event::Paste(mut value) => {
                if usable(size) {
                    self.paste(&value);
                }
                value.zeroize();
                Action::None
            }
            Event::Key(key) => {
                let ctrl_c = matches!(key.code, KeyCode::Char('c' | 'C'))
                    && key.modifiers.contains(KeyModifiers::CONTROL);
                if usable(size)
                    || ctrl_c
                    || (matches!(self.screen, Screen::Running { .. }) && key.code == KeyCode::Esc)
                {
                    self.key(key)
                } else {
                    Action::None
                }
            }
            _ => Action::None,
        }
    }
}

#[cfg(test)]
mod tests;
