//! Bundled provider policy and device parameter tree.
use crate::{
    domain::export::{InternetConfig, WanProtocol},
    error::{Error, Result},
};
use serde::{Deserialize, Deserializer, de::Error as _};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

// The asset files also carry descriptive fields (ids, names, notes, IPTV reference values)
// for maintainers; the structs below read only what the app uses.
#[derive(Clone, Deserialize)]
/// Provider endpoint, trust pins and internet presets.
pub struct Provider {
    /// Provider management endpoint.
    pub acs: Acs,
    /// Accepted leaf certificate or public-key hashes.
    pub pins: Vec<Pin>,
    /// Reference settings for the primary internet connection.
    pub internet: InternetPreset,
}
#[derive(Clone, Deserialize)]
/// Management endpoint supplied by a provider.
pub struct Acs {
    /// Hostname used for DNS, TLS SNI and the HTTP Host field.
    pub host: String,
    /// TCP port for pinned HTTPS.
    pub port: u16,
    /// Absolute request path, including any query string.
    pub path: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
/// The part of a TLS leaf certificate hashed for a pin.
pub enum PinKind {
    /// SHA-256 of the complete DER leaf certificate.
    CertificateSha256,
    /// SHA-256 of the complete DER SubjectPublicKeyInfo sequence.
    SpkiSha256,
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// One decoded SHA-256 certificate or public-key pin.
pub struct Pin {
    /// Which certificate bytes must match.
    pub kind: PinKind,
    /// The expected 32-byte SHA-256 digest.
    pub sha256: [u8; 32],
}
impl Pin {
    /// Decode a lowercase 64-character SHA-256 hex string.
    pub fn from_hex(kind: PinKind, value: &str) -> Option<Self> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return None;
        }
        let mut sha256 = [0; 32];
        for (index, byte) in sha256.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
        }
        Some(Self { kind, sha256 })
    }
}
impl<'de> Deserialize<'de> for Pin {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct RawPin {
            kind: PinKind,
            sha256: String,
        }
        let raw = RawPin::deserialize(deserializer)?;
        Self::from_hex(raw.kind, &raw.sha256).ok_or_else(|| D::Error::custom("invalid SHA-256 pin"))
    }
}
#[derive(Clone, Deserialize)]
/// Provider's reference internet connection settings.
pub struct InternetPreset {
    /// WAN protocol expected for the internet connection.
    pub protocol: WanProtocol,
    /// Internet VLAN identifier.
    pub vlan_id: u16,
    /// Reference maximum transmission unit.
    pub mtu: u16,
}
impl InternetPreset {
    /// Convert the reference values to the saved internet configuration.
    pub fn config(&self) -> InternetConfig {
        InternetConfig {
            protocol: self.protocol,
            vlan_id: self.vlan_id,
            mtu: self.mtu,
        }
    }
}
#[derive(Clone, Deserialize)]
/// Parameter tree and identity rules for one supported router model.
pub struct DeviceTemplate {
    /// Compact model name for the application header.
    pub short_name: String,
    /// Serial-number and derived-parameter rules.
    pub identity: Identity,
    /// Management and internet credential parameter paths.
    pub credentials: CredentialPaths,
    /// Parameters sent in the opening Inform.
    pub inform_parameters: Vec<String>,
    /// Known leaf paths and their default values.
    pub parameters: BTreeMap<String, Parameter>,
    /// Known object paths in the parameter tree.
    pub objects: BTreeSet<String>,
    /// Leaf or object paths accepted by SetParameterValues.
    pub writable: BTreeSet<String>,
    /// Parameters whose values must not appear in logs or normal UI text.
    pub hidden_parameters: BTreeSet<String>,
}
#[derive(Clone, Deserialize)]
/// Serial prefix and values derived from router identity fields.
pub struct Identity {
    /// Prefix prepended to the label serial for protocol identity.
    pub serial_prefix: String,
    /// Parameter templates expanded from serial and MAC values.
    pub derived_parameters: Vec<DerivedParameter>,
}
#[derive(Clone, Deserialize)]
/// One parameter whose value is expanded from router identity fields.
pub struct DerivedParameter {
    /// Full parameter path in the device tree.
    pub parameter: String,
    /// Value template containing supported placeholders.
    pub value: String,
}
#[derive(Clone, Deserialize)]
/// Parameter paths used for management and internet credentials.
pub struct CredentialPaths {
    /// Management-server username parameter.
    pub acs_username: String,
    /// Management-server password parameter.
    pub acs_password: String,
    /// Management-server parameter-key path.
    pub parameter_key: String,
    /// Internet PPP username parameter.
    pub ppp_username: String,
    /// Internet PPP password parameter.
    pub ppp_password: String,
}
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "(String, String)")]
/// Parameter kind and current value from the device tree.
pub struct Parameter {
    /// Data type name such as `string` or `unsignedInt`.
    pub kind: String,
    /// Current value, which may contain a management or internet credential.
    pub value: String,
}
impl std::fmt::Debug for Parameter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Parameter")
            .field("kind", &self.kind)
            .field("value", &"[redacted]")
            .finish()
    }
}
impl From<(String, String)> for Parameter {
    fn from((kind, value): (String, String)) -> Self {
        Self { kind, value }
    }
}

/// Load and validate the bundled provider and router once.
pub fn bundled() -> Result<&'static (Provider, DeviceTemplate)> {
    static BUNDLED: OnceLock<Result<(Provider, DeviceTemplate)>> = OnceLock::new();
    BUNDLED
        .get_or_init(|| {
            let provider = serde_json::from_str(include_str!("../assets/providers/cetin-rs.json"))
                .map_err(|_| Error::Internal)?;
            let template: DeviceTemplate =
                serde_json::from_str(include_str!("../assets/devices/zte-h3600p.json"))
                    .map_err(|_| Error::Internal)?;
            template.validate()?;
            Ok((provider, template))
        })
        .as_ref()
        .map_err(|error| *error)
}
impl DeviceTemplate {
    /// Reject invalid parameter references and identity placeholder templates.
    pub fn validate(&self) -> Result<()> {
        if self.identity.serial_prefix.is_empty() {
            return Err(Error::Internal);
        }
        let paths = &self.credentials;
        for path in [
            &paths.acs_username,
            &paths.acs_password,
            &paths.parameter_key,
            &paths.ppp_username,
            &paths.ppp_password,
        ] {
            if !self.parameters.contains_key(path) {
                return Err(Error::Internal);
            }
        }
        let username_prefix = paths.ppp_username.strip_suffix("Username");
        let password_prefix = paths.ppp_password.strip_suffix("Password");
        if username_prefix.is_none() || username_prefix != password_prefix {
            return Err(Error::Internal);
        }
        for derived in &self.identity.derived_parameters {
            if !self.parameters.contains_key(&derived.parameter) {
                return Err(Error::Internal);
            }
            expand_placeholders(&derived.value, |_| Some(String::new()))?;
        }
        if self
            .inform_parameters
            .iter()
            .chain(self.hidden_parameters.iter())
            .any(|name| !self.parameters.contains_key(name))
        {
            return Err(Error::Internal);
        }
        // The preserved tree marks both leaves and parent object paths writable.
        if self.writable.iter().any(|name| {
            !self.parameters.contains_key(name)
                && !self.objects.contains(name)
                && !(name.ends_with('.')
                    && self.parameters.keys().any(|leaf| leaf.starts_with(name)))
        }) {
            return Err(Error::Internal);
        }
        Ok(())
    }
}

/// Expand only documented identity placeholders; reject unknown or unbalanced braces.
pub fn expand_placeholders(
    input: &str,
    mut value: impl FnMut(&str) -> Option<String>,
) -> Result<String> {
    let mut out = String::new();
    let mut remaining = input;
    while let Some(open) = remaining.find('{') {
        let plain = &remaining[..open];
        if plain.contains('}') {
            return Err(Error::Internal);
        }
        out.push_str(plain);
        remaining = &remaining[open + 1..];
        let close = remaining.find('}').ok_or(Error::Internal)?;
        let token = &remaining[..close];
        let valid = matches!(token, "serial" | "prefixed_serial" | "router_mac")
            || token.strip_prefix("router_mac+").is_some_and(|n| {
                !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && n.parse::<u64>().is_ok()
            });
        if !valid {
            return Err(Error::Internal);
        }
        out.push_str(&value(token).ok_or(Error::Internal)?);
        remaining = &remaining[close + 1..];
    }
    if remaining.contains('}') {
        return Err(Error::Internal);
    }
    out.push_str(remaining);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parameter_debug_never_exposes_runtime_value() {
        let parameter = Parameter {
            kind: "string".into(),
            value: "synthetic-management-secret".into(),
        };
        let debug = format!("{parameter:?}");
        assert!(debug.contains("kind: \"string\""));
        assert!(debug.contains("value: \"[redacted]\""));
        assert!(!debug.contains("synthetic-management-secret"));
    }

    #[test]
    fn bundled_catalog_validates_complete_tree() {
        let (provider, template) = bundled().unwrap();
        assert_eq!(provider.pins.len(), 1);
        assert_eq!(template.inform_parameters.len(), 8);
    }

    #[test]
    fn malformed_derived_template_and_missing_parameter_are_rejected() {
        let original = &bundled().unwrap().1;
        let mut template = original.clone();
        template.identity.derived_parameters[0].value = "{wrong}".into();
        assert!(template.validate().is_err());
        template.identity.derived_parameters[0].value = "{serial}".into();
        template.credentials.acs_username = "Missing.Parameter".into();
        assert!(template.validate().is_err());

        let mut template = original.clone();
        template.credentials.ppp_username = "InternetGatewayDevice.DeviceInfo.Manufacturer".into();
        template.credentials.ppp_password = "InternetGatewayDevice.DeviceInfo.Description".into();
        assert!(template.validate().is_err());

        let mut template = original.clone();
        template.writable.insert("Unknown.Object.".into());
        assert!(template.validate().is_err());
    }

    #[test]
    fn pins_require_lowercase_sha256_and_a_supported_kind() {
        for kind in ["certificate_sha256", "spki_sha256"] {
            let mut value = serde_json::json!({"kind": kind, "sha256": "a".repeat(64)});
            assert!(serde_json::from_value::<Pin>(value.clone()).is_ok());
            for invalid in ["invalid".to_string(), "A".repeat(64)] {
                value["sha256"] = invalid.into();
                assert!(serde_json::from_value::<Pin>(value.clone()).is_err());
            }
        }
    }
}
