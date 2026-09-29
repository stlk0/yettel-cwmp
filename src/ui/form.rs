//! Profile form input, field validation, and key erasure.
use super::Action;
use crate::domain::mac::format_partial;
use crate::{
    domain::{mac::MacAddress, profile::validate_label_key, serial::Serial},
    error::Error,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::str::FromStr;
use zeroize::Zeroize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// The active input field in a router profile form.
pub enum Field {
    #[default]
    /// Router serial number.
    Serial,
    /// Router MAC address.
    Mac,
    /// Wi-Fi key from the router label.
    Key,
}
impl Field {
    fn next(self) -> Self {
        match self {
            Self::Serial => Self::Mac,
            Self::Mac => Self::Key,
            Self::Key => Self::Serial,
        }
    }
    fn previous(self) -> Self {
        self.next().next()
    }
}
#[derive(Default)]
/// Draft values and validation state for creating or updating a router.
pub struct ProfileForm {
    /// Draft serial number.
    pub serial: String,
    /// Draft MAC address.
    pub mac: String,
    /// Draft label key, masked in the terminal by default.
    pub key: String,
    /// Field receiving keyboard input.
    pub field: Field,
    /// Last local validation error.
    pub error: Option<Error>,
    /// Whether the label key is temporarily shown.
    pub reveal: bool,
}
impl ProfileForm {
    fn current(&mut self) -> &mut String {
        match self.field {
            Field::Serial => &mut self.serial,
            Field::Mac => &mut self.mac,
            Field::Key => &mut self.key,
        }
    }
    pub(super) fn insert(&mut self, text: &str, paste: bool) {
        if self.field == Field::Mac {
            if !text
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'-')
            {
                self.error = Some(Error::InvalidMac);
                return;
            }
            let incoming: String = text
                .chars()
                .filter(char::is_ascii_hexdigit)
                .map(|c| c.to_ascii_uppercase())
                .collect();
            let mut digits: String = self.mac.chars().filter(char::is_ascii_hexdigit).collect();
            // A full address replaces the field; partial pastes append like typing.
            if paste && incoming.len() == 12 {
                digits.clear();
            }
            if digits.len() + incoming.len() > 12 {
                self.error = Some(Error::InvalidMac);
                return;
            }
            digits.push_str(&incoming);
            self.mac = format_partial(&digits);
        } else {
            let field = self.current();
            if text.chars().any(char::is_control)
                || field.chars().count() + text.chars().count() > 1024
            {
                self.error = Some(Error::InvalidText);
                return;
            }
            field.push_str(text);
        }
        self.error = None;
    }
    fn validate(&mut self, field: Field, serial_prefix: &str) -> bool {
        let result = match field {
            Field::Serial => Serial::parse_with_prefix(&self.serial, serial_prefix)
                .map(|serial| self.serial = serial.to_string()),
            Field::Mac => MacAddress::from_str(&self.mac).map(|mac| self.mac = mac.to_string()),
            Field::Key => validate_label_key(&self.key),
        };
        self.error = result.err();
        if self.error.is_some() {
            self.field = field;
            false
        } else {
            true
        }
    }
    /// Apply one key press; `serial_prefix` is the optional label prefix of the router model.
    pub(super) fn input(&mut self, key: KeyEvent, change_key: bool, serial_prefix: &str) -> Action {
        if matches!(key.code, KeyCode::F(2))
            || (matches!(key.code, KeyCode::Char('r' | 'R'))
                && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            self.reveal = !self.reveal;
            return Action::None;
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down if !change_key => self.field = self.field.next(),
            KeyCode::BackTab | KeyCode::Up if !change_key => self.field = self.field.previous(),
            KeyCode::Enter if change_key => {
                if self.validate(Field::Key, serial_prefix) {
                    return Action::ChangeKey;
                }
            }
            KeyCode::Enter => {
                if self.field != Field::Key {
                    if self.validate(self.field, serial_prefix) {
                        self.field = self.field.next();
                    }
                } else if [Field::Serial, Field::Mac, Field::Key]
                    .into_iter()
                    .all(|field| self.validate(field, serial_prefix))
                {
                    return Action::Create;
                }
            }
            KeyCode::Backspace => {
                if self.field == Field::Mac {
                    let mut digits: String =
                        self.mac.chars().filter(char::is_ascii_hexdigit).collect();
                    digits.pop();
                    self.mac = format_partial(&digits);
                } else {
                    self.current().pop();
                }
                self.error = None;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.current().zeroize();
                self.error = None;
            }
            KeyCode::Char(c)
                if !c.is_control() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.insert(&c.to_string(), false)
            }
            _ => (),
        }
        Action::None
    }
}
impl Drop for ProfileForm {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}
