//! Bundled device data and instantiated parameter model.
//! Keep parameter ordering and identity values stable for the golden wire snapshots.
use crate::{
    catalog::{self, CredentialPaths, DeviceTemplate, Parameter, Provider, expand_placeholders},
    domain::{
        export::{Export, InternetConfig, InternetSettings},
        profile::Profile,
        secret::Secret,
    },
    error::{Error, Result},
};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

/// Accepted credential values, redacted in Debug and cleared on drop.
pub type Assignments = BTreeMap<String, Secret>;
/// The selected provider policy and router model for one session.
#[derive(Clone)]
pub struct Device {
    /// Provider policy selected for this session.
    pub provider: Provider,
    /// Device parameter template validated by the catalog.
    pub template: DeviceTemplate,
}
impl Device {
    /// The bundled default provider and router.
    pub fn bundled() -> Result<Self> {
        let (provider, template) = catalog::bundled()?;
        Ok(Self {
            provider: provider.clone(),
            template: template.clone(),
        })
    }
    /// Bundled internet defaults combined with received credentials.
    pub fn internet(&self) -> InternetConfig {
        self.provider.internet.config()
    }
    /// Ordered parameter names included in the opening Inform.
    pub fn inform_parameters(&self) -> &[String] {
        &self.template.inform_parameters
    }
    /// Optional prefix printed before the serial number on this router's label.
    pub fn serial_prefix(&self) -> &str {
        &self.template.identity.serial_prefix
    }
    /// Instantiate a device model, rejecting invalid template references.
    pub fn model(&self, profile: &Profile) -> Result<DataModel> {
        DataModel::instantiate(&self.template, profile)
    }
    /// Combine complete received internet credentials with bundled defaults.
    pub fn export(&self, received: &Assignments) -> Result<Export> {
        let paths = &self.template.credentials;
        let credential = |path: &str| {
            received
                .get(path)
                .filter(|value| !value.expose().is_empty())
                .cloned()
                .ok_or(Error::Incomplete)
        };
        Ok(Export {
            internet: InternetSettings {
                config: self.internet(),
                username: credential(&paths.ppp_username)?,
                password: credential(&paths.ppp_password)?,
            },
        })
    }
}
/// Per-session parameter tree, including private management credentials.
pub struct DataModel {
    /// Parameter types and current values, which may contain secrets.
    pub params: BTreeMap<String, Parameter>,
    /// Known object paths, including derived parent paths.
    pub objects: BTreeSet<String>,
    /// Paths that accept assignments from the provider.
    pub writable: BTreeSet<String>,
    /// Parameters whose values are hidden in read responses.
    pub hidden: BTreeSet<String>,
    /// Paths identifying the management and internet credential pairs.
    pub credentials: CredentialPaths,
}
impl DataModel {
    /// Expand device placeholders and insert this profile’s management credentials.
    pub fn instantiate(template: &DeviceTemplate, profile: &Profile) -> Result<Self> {
        let mut params = template.parameters.clone();
        let serial = profile.serial.to_string();
        let prefixed = format!("{}{}", template.identity.serial_prefix, serial);
        let mac = profile.router_mac.to_string();
        for derived in &template.identity.derived_parameters {
            let value = expand_placeholders(&derived.value, |token| match token {
                "serial" => Some(serial.clone()),
                "prefixed_serial" => Some(prefixed.clone()),
                "router_mac" => Some(mac.clone()),
                token => token
                    .strip_prefix("router_mac+")
                    .and_then(|offset| offset.parse::<u64>().ok())
                    .map(|offset| profile.router_mac.wrapping_offset(offset).to_string()),
            })?;
            params
                .get_mut(&derived.parameter)
                .ok_or(Error::Internal)?
                .value = value;
        }
        let paths = &template.credentials;
        for (name, value) in [
            (&paths.acs_username, profile.username.expose()),
            (&paths.acs_password, profile.password.expose()),
        ] {
            params.get_mut(name).ok_or(Error::Internal)?.value = value.to_owned();
        }
        let mut objects = template.objects.clone();
        for name in params.keys().chain(template.objects.iter()) {
            for (index, _) in name.match_indices('.') {
                objects.insert(name[..=index].into());
            }
        }
        Ok(Self {
            params,
            objects,
            writable: template.writable.clone(),
            hidden: template.hidden_parameters.clone(),
            credentials: paths.clone(),
        })
    }
}

/// Numeric path segments sort numerically, without machine-integer overflow.
pub fn parameter_order(a: &str, b: &str) -> Ordering {
    for (a, b) in a.split('.').zip(b.split('.')) {
        let an = !a.is_empty() && a.bytes().all(|v| v.is_ascii_digit());
        let bn = !b.is_empty() && b.bytes().all(|v| v.is_ascii_digit());
        let order = match (an, bn) {
            (true, true) => {
                let a = a.trim_start_matches('0');
                let b = b.trim_start_matches('0');
                a.len().cmp(&b.len()).then(a.cmp(b))
            }
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => a.cmp(b),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.split('.').count().cmp(&b.split('.').count())
}
