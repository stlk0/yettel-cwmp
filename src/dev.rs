//! Development-only provider override for loopback testing.
//! This module is omitted entirely from production builds.
use crate::{
    catalog::{Acs, Pin},
    cwmp::model::Device,
    error::{Error, Result},
    i18n::{self, Language},
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Override {
    acs: Acs,
    pins: Vec<Pin>,
}

/// Point `device` at a loopback-only ACS from `YETTEL_CWMP_DEV_PROVIDER`, if set.
pub fn with_provider_override(mut device: Device) -> Result<Device> {
    let Some(path) = std::env::var_os("YETTEL_CWMP_DEV_PROVIDER") else {
        return Ok(device);
    };
    let bytes = std::fs::read(path).map_err(|error| Error::storage_io(&error))?;
    let override_: Override = serde_json::from_slice(&bytes).map_err(|_| Error::Network)?;
    // Override input must remain loopback-only and safe to place in an HTTP request line.
    if !matches!(override_.acs.host.as_str(), "localhost" | "127.0.0.1")
        || override_.acs.port == 0
        || !override_.acs.path.starts_with('/')
        || !override_.acs.path.is_ascii()
        || override_.acs.path.bytes().any(|byte| {
            byte.is_ascii_whitespace() || byte.is_ascii_control() || matches!(byte, b'#' | b'\\')
        })
        || override_.pins.is_empty()
    {
        return Err(Error::Network);
    }
    device.provider.acs = override_.acs;
    device.provider.pins = override_.pins;
    Ok(device)
}

/// Visible marker compiled only into the development feature.
pub fn banner() -> &'static str {
    match i18n::language() {
        Language::En => "DEV BUILD",
        Language::Sr => "RAZVOJNA VERZIJA",
    }
}

/// Version annotation compiled only into the development feature.
pub fn version_annotation() -> &'static str {
    match i18n::language() {
        Language::En => "dev build: provider override enabled",
        Language::Sr => "RAZVOJNA VERZIJA: uključen razvojni izbor provajdera",
    }
}
