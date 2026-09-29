//! Canonical 48-bit router MAC addresses and form grouping.
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
/// Canonical unicast, nonzero 48-bit router MAC address.
pub struct MacAddress([u8; 6]);

impl MacAddress {
    /// Add a 48-bit offset with wraparound.
    pub fn wrapping_offset(self, offset: u64) -> Self {
        let base = self
            .0
            .iter()
            .fold(0u64, |value, byte| (value << 8) | u64::from(*byte));
        let shifted = base.wrapping_add(offset) & 0x0000_FFFF_FFFF_FFFF;
        let mut bytes = [0u8; 6];
        for (index, byte) in bytes.iter_mut().enumerate() {
            // The mask bounds the value to one octet before conversion.
            *byte = u8::try_from((shifted >> (8 * (5 - index))) & 0xff).unwrap_or_default();
        }
        Self(bytes)
    }
}

impl FromStr for MacAddress {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        let value = input.trim();
        let compact = if value.contains(':') || value.contains('-') {
            if value.contains(':') && value.contains('-') {
                return Err(Error::InvalidMac);
            }
            let delimiter = if value.contains(':') { ':' } else { '-' };
            let parts: Vec<_> = value.split(delimiter).collect();
            if parts.len() != 6 || parts.iter().any(|part| part.len() != 2) {
                return Err(Error::InvalidMac);
            }
            parts.concat()
        } else {
            value.to_owned()
        };
        if compact.len() != 12 || !compact.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::InvalidMac);
        }
        let mut bytes = [0u8; 6];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&compact[index * 2..index * 2 + 2], 16)
                .map_err(|_| Error::InvalidMac)?;
        }
        if bytes[0] & 1 != 0 || bytes == [0; 6] {
            return Err(Error::InvalidMac);
        }
        Ok(Self(bytes))
    }
}

impl TryFrom<String> for MacAddress {
    type Error = Error;

    fn try_from(input: String) -> Result<Self> {
        input.parse()
    }
}

impl From<MacAddress> for String {
    fn from(value: MacAddress) -> Self {
        value.to_string()
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

/// Group partially entered hexadecimal MAC digits for display.
pub fn format_partial(hex_digits: &str) -> String {
    let mut result = String::with_capacity(17);
    for (index, character) in hex_digits.chars().enumerate() {
        result.push(character);
        if index % 2 == 1 && index < 10 {
            result.push(':');
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_cases() {
        for input in [
            "02aabb001122",
            "02-aa-bb-00-11-22",
            "02:AA:BB:00:11:22",
            " 02:aa:BB:00:11:22 ",
        ] {
            assert_eq!(
                MacAddress::from_str(input).unwrap().to_string(),
                "02:AA:BB:00:11:22"
            );
        }
        for bad in [
            "00:00:00:00:00:00",
            "01:11:22:33:44:55",
            "+2:11:22:33:44:55",
            "02-11:22:33:44:55",
            "02:1:22:33:44:55",
            "02112233445",
            "0211223344556",
        ] {
            assert_eq!(MacAddress::from_str(bad), Err(Error::InvalidMac), "{bad:?}");
        }
        assert_eq!(
            MacAddress([0xff, 0xff, 0xff, 0xff, 0xff, 0xfe])
                .wrapping_offset(3)
                .to_string(),
            "00:00:00:00:00:01"
        );
        assert_eq!(
            MacAddress::from_str("FE:FF:FF:FF:FF:FE")
                .unwrap()
                .wrapping_offset(3)
                .to_string(),
            "FF:00:00:00:00:01"
        );
        assert_eq!(
            MacAddress::from_str("FE:FF:FF:FF:FF:FE")
                .unwrap()
                .wrapping_offset(0x010000000003)
                .to_string(),
            "00:00:00:00:00:01"
        );
        assert_eq!(
            serde_json::to_string(&MacAddress::from_str("02aabb001122").unwrap()).unwrap(),
            "\"02:AA:BB:00:11:22\""
        );
        assert_eq!(format_partial("02AABB"), "02:AA:BB:");
    }
}
