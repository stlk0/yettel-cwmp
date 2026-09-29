//! Profile identity, normalization, and redacted secrets.
//! Profile validation precedes transport construction.
use crate::{
    domain::{mac::MacAddress, secret::Secret, serial::Serial},
    error::{Error, Result},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Origin of the credentials currently stored in the profile.
pub enum CredentialsSource {
    /// Derived from the router label.
    Label,
    /// Rotated or supplied by the provider, or migrated from version 1.
    #[default]
    #[serde(alias = "unknown")]
    Server,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Persistent router identity and management credentials.
pub struct Profile {
    /// Normalized router serial number.
    pub serial: Serial,
    /// Router MAC address printed on its label.
    pub router_mac: MacAddress,
    /// Management username, which can be rotated by the server.
    pub username: Secret,
    /// Management password.
    pub password: Secret,
    /// Where the current management credentials came from.
    pub credentials_source: CredentialsSource,
}
#[derive(Deserialize)]
pub(crate) struct ProfileV1 {
    pub serial: Serial,
    pub router_mac: MacAddress,
    pub username: Secret,
    pub password: Secret,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct StoredProfile<T> {
    pub(crate) version: u32,
    #[serde(flatten)]
    pub(crate) profile: T,
}
/// Validate one router-label Wi-Fi key across CLI, UI, and persistence.
pub fn validate_label_key(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(Error::EmptyKey);
    }
    if value.chars().count() > 1024 || value.chars().any(char::is_control) {
        return Err(Error::InvalidText);
    }
    Ok(())
}

/// What an interface shows about a saved router. It holds no management credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileSummary {
    /// Normalized router serial number.
    pub serial: Serial,
    /// Router MAC address printed on its label.
    pub router_mac: MacAddress,
}

// A label-derived management login is the serial number with the label's Wi-Fi key.
fn label_username(serial: &Serial) -> Secret {
    Secret::new(serial.to_string())
}

impl Profile {
    /// Create a profile from the router label.
    pub fn new(serial: Serial, mac: MacAddress, key: Secret) -> Result<Self> {
        validate_label_key(key.expose())?;
        Ok(Self {
            username: label_username(&serial),
            serial,
            router_mac: mac,
            password: key,
            credentials_source: CredentialsSource::Label,
        })
    }
    /// The parts of this profile an interface may keep and show.
    pub fn summary(&self) -> ProfileSummary {
        ProfileSummary {
            serial: self.serial.clone(),
            router_mac: self.router_mac,
        }
    }
    /// Replace the management credentials with the label pair: serial and Wi-Fi key.
    pub fn use_label_key(&mut self, key: Secret) -> Result<()> {
        validate_label_key(key.expose())?;
        self.username = label_username(&self.serial);
        self.password = key;
        self.credentials_source = CredentialsSource::Label;
        Ok(())
    }
    pub(crate) fn from_v1(value: ProfileV1) -> Self {
        Self {
            serial: value.serial,
            router_mac: value.router_mac,
            username: value.username,
            password: value.password,
            credentials_source: CredentialsSource::Server,
        }
    }
    /// Validate that the saved identity matches its directory.
    pub fn validate(&self, serial: &Serial) -> Result<()> {
        if &self.serial != serial {
            return Err(Error::ProfileInvalid);
        }
        Ok(())
    }
}
