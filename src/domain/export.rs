//! Saved internet settings and provider defaults.
use super::secret::Secret;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
/// Saved internet settings captured from the router.
pub struct Export {
    /// Internet connection settings.
    pub internet: InternetSettings,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
/// Non-secret internet connection parameters.
pub struct InternetConfig {
    /// WAN connection protocol.
    pub protocol: WanProtocol,
    /// VLAN identifier used for the connection.
    pub vlan_id: u16,
    /// Maximum transmission unit in bytes.
    pub mtu: u16,
}
#[derive(Serialize, Deserialize)]
/// Connection parameters and credentials.
pub struct InternetSettings {
    #[serde(flatten)]
    /// Non-secret connection parameters.
    pub config: InternetConfig,
    /// Internet username.
    pub username: Secret,
    /// Internet password.
    pub password: Secret,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Supported WAN connection protocols.
pub enum WanProtocol {
    #[serde(rename = "PPPoE")]
    /// Point-to-Point Protocol over Ethernet.
    Pppoe,
}

impl WanProtocol {
    /// Return the protocol name used in exported settings.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pppoe => "PPPoE",
        }
    }
}

impl std::fmt::Display for WanProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::fmt::Debug for Export {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Export")
            .field("internet", &self.internet)
            .finish()
    }
}

impl std::fmt::Debug for InternetSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InternetSettings")
            .field("config", &self.config)
            .field("username", &self.username)
            .field("password", &self.password)
            .finish()
    }
}
