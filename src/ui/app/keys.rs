//! Keyboard navigation for each screen.
use super::{Action, App, Screen};
use crate::ui::form::{Field, ProfileForm};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

impl App {
    /// Map a key press to navigation or an application action.
    pub fn key(&mut self, mut key: KeyEvent) -> Action {
        if key.kind != KeyEventKind::Press {
            return Action::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'C'))
        {
            return Action::Quit;
        }
        // Q remains an exit key on help and during a session; forms keep it as text.
        if matches!(key.code, KeyCode::Char('q' | 'Q'))
            && (self.help_open || !matches!(self.screen, Screen::Create | Screen::ChangeKey(_)))
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return Action::Quit;
        }
        if self.help_open {
            match key.code {
                KeyCode::Esc | KeyCode::F(1) => self.help_open = false,
                KeyCode::PageDown => {
                    self.scroll = self.scroll.saturating_add(4).min(self.max_scroll)
                }
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(4),
                KeyCode::Home => self.scroll = 0,
                KeyCode::End => self.scroll = self.max_scroll,
                _ => {}
            }
            return Action::None;
        }
        if matches!(key.code, KeyCode::F(1))
            || (key.code == KeyCode::Char('?')
                && !matches!(self.screen, Screen::Create | Screen::ChangeKey(_)))
        {
            self.help_open = true;
            self.scroll = 0;
            return Action::None;
        }
        if matches!(self.screen, Screen::Create | Screen::ChangeKey(_)) {
            return self.handle_form_key(key);
        }
        if let KeyCode::Char(value) = key.code {
            key.code = KeyCode::Char(value.to_ascii_lowercase());
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return Action::None;
        }
        if self.handle_scroll_key(key.code) {
            return Action::None;
        }
        if matches!(self.screen, Screen::Profiles) {
            return self.handle_profiles_key(key);
        }
        if let Screen::Running { cancelling, .. } = &mut self.screen {
            if key.code == KeyCode::Esc {
                *cancelling = true;
            }
            return Action::None;
        }
        let menu = self.menu();
        let selected = self.menu_list.selected().unwrap_or_default();
        match key.code {
            KeyCode::Up | KeyCode::BackTab => {
                self.menu_list.select(Some(selected.saturating_sub(1)));
                return Action::None;
            }
            KeyCode::Down | KeyCode::Tab => {
                self.menu_list
                    .select(Some((selected + 1).min(menu.len().saturating_sub(1))));
                return Action::None;
            }
            KeyCode::Enter if !menu.is_empty() => {
                key.code = menu[selected.min(menu.len() - 1)].code
            }
            _ => {}
        }
        match &mut self.screen {
            Screen::Device(_) => self.handle_device_key(key),
            Screen::ReplaceSaved(_) => self.handle_replace_key(key),
            Screen::DeleteConfirm { .. } => self.handle_delete_key(key),
            Screen::ConnectNotice(_) => self.handle_notice_key(key),
            Screen::Result { .. } => self.handle_result_key(key),
            Screen::Error { .. } => self.handle_error_key(key),
            Screen::Profiles | Screen::Create | Screen::ChangeKey(_) | Screen::Running { .. } => {
                Action::None
            }
        }
    }
    fn handle_scroll_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::PageDown => {
                if matches!(self.screen, Screen::Profiles) {
                    self.profile_list.select(Some(
                        (self.profile_list.selected().unwrap_or(0) + 4)
                            .min(self.profiles.len() + 1),
                    ));
                } else {
                    self.scroll = self.scroll.saturating_add(4).min(self.max_scroll);
                }
            }
            KeyCode::PageUp => {
                if matches!(self.screen, Screen::Profiles) {
                    self.profile_list.select(Some(
                        self.profile_list.selected().unwrap_or(0).saturating_sub(4),
                    ));
                } else {
                    self.scroll = self.scroll.saturating_sub(4);
                }
            }
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.max_scroll,
            _ => return false,
        }
        true
    }

    fn handle_profiles_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => {}
            KeyCode::Down | KeyCode::Tab => self.profile_list.select(Some(
                (self.profile_list.selected().unwrap_or(0) + 1).min(self.profiles.len() + 1),
            )),
            KeyCode::Up | KeyCode::BackTab => self.profile_list.select(Some(
                self.profile_list.selected().unwrap_or(0).saturating_sub(1),
            )),
            KeyCode::Char('n') => self.show(Screen::Create),
            KeyCode::Enter => match self.profile_list.selected().unwrap_or(0) {
                index if index < self.profiles.len() => return Action::Open,
                index if index == self.profiles.len() => self.show(Screen::Create),
                _ => return Action::Quit,
            },
            _ => {}
        }
        Action::None
    }
    fn handle_form_key(&mut self, key: KeyEvent) -> Action {
        if key.code == KeyCode::Esc {
            self.back();
            return Action::None;
        }
        let change = matches!(self.screen, Screen::ChangeKey(_));
        self.form.input(key, change, self.device.serial_prefix())
    }
    fn handle_device_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('r') => Action::Start,
            KeyCode::Char('v') if self.export_modified.is_some() => Action::ViewSaved,
            KeyCode::Char('k') => {
                self.form = ProfileForm::default();
                self.form.field = Field::Key;
                if let Ok(profile) = self.take_profile() {
                    self.show(Screen::ChangeKey(profile));
                }
                Action::None
            }
            KeyCode::Char('d') => {
                if let Ok(profile) = self.take_profile() {
                    self.show(Screen::DeleteConfirm {
                        serial: profile.serial.clone(),
                        profile: Some(profile),
                    });
                }
                Action::None
            }
            KeyCode::Esc => {
                self.show(Screen::Profiles);
                Action::None
            }
            _ => Action::None,
        }
    }
    fn handle_replace_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('y') => Action::ConfirmReplace,
            KeyCode::Esc => {
                self.back();
                Action::None
            }
            _ => Action::None,
        }
    }
    fn handle_delete_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('y') => Action::ConfirmDelete,
            KeyCode::Esc => {
                self.back();
                Action::None
            }
            _ => Action::None,
        }
    }
    fn handle_notice_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => Action::Start,
            KeyCode::Esc => {
                self.back();
                Action::None
            }
            _ => Action::None,
        }
    }
    fn handle_result_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('s') => {
                if let Screen::Result { reveal, .. } = &mut self.screen {
                    *reveal = !*reveal;
                }
                Action::None
            }
            KeyCode::Char('c') => Action::CopyPassword,
            KeyCode::Char('u') => Action::CopyUsername,
            KeyCode::Esc => {
                self.back();
                Action::None
            }
            _ => Action::None,
        }
    }
    fn handle_error_key(&mut self, key: KeyEvent) -> Action {
        let hints = self.menu();
        if !hints.iter().any(|hint| hint.code == key.code) {
            return Action::None;
        }
        match key.code {
            KeyCode::Esc => {
                self.back();
                Action::None
            }
            KeyCode::Char('r') => Action::Retry,
            KeyCode::Char('k') => {
                if let Ok(profile) = self.take_profile() {
                    self.form = ProfileForm::default();
                    self.form.field = Field::Key;
                    self.show(Screen::ChangeKey(profile));
                }
                Action::None
            }
            KeyCode::Char('x') => {
                let previous = std::mem::replace(&mut self.screen, Screen::Profiles);
                if let Screen::Error {
                    serial: Some(serial),
                    profile,
                    ..
                } = previous
                {
                    self.show(Screen::DeleteConfirm { serial, profile });
                }
                Action::None
            }
            _ => Action::None,
        }
    }
}
