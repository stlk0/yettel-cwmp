//! Canonical router serial numbers.
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{fmt, ops::Deref, str::FromStr};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
/// Canonical router serial without the catalog display prefix.
pub struct Serial(String);

impl Serial {
    /// Parse and normalize a serial with an optional display prefix.
    pub fn parse_with_prefix(input: &str, prefix: &str) -> Result<Self> {
        let value = input.trim().to_ascii_uppercase();
        let prefix = prefix.to_ascii_uppercase();
        let value = value.strip_prefix(&prefix).unwrap_or(&value);
        if !(6..=64).contains(&value.len())
            || !value
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return Err(Error::InvalidSerial);
        }
        Ok(Self(value.to_owned()))
    }
}

impl FromStr for Serial {
    type Err = Error;

    /// Parse a serial without a label prefix, as stored and used in paths.
    fn from_str(input: &str) -> Result<Self> {
        Self::parse_with_prefix(input, "")
    }
}

impl TryFrom<String> for Serial {
    type Error = Error;

    fn try_from(input: String) -> Result<Self> {
        input.parse()
    }
}

impl From<Serial> for String {
    fn from(value: Serial) -> Self {
        value.0
    }
}

impl AsRef<str> for Serial {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for Serial {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Serial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_cases() {
        for (input, expected) in [
            ("syn123456", "SYN123456"),
            (" syn123456 ", "SYN123456"),
            ("ABCDEF", "ABCDEF"),
        ] {
            assert_eq!(Serial::from_str(input).unwrap().as_ref(), expected);
        }
        for bad in [
            "",
            "A1234",
            "a b c d e f",
            "ABCDEF-",
            "ABCDEF/",
            "AAAAAß",
            "AAAAAﬀ",
            "../../SYN123456",
            "A".repeat(65).as_str(),
        ] {
            assert!(Serial::from_str(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            Serial::parse_with_prefix("X-SYN123456", "X-")
                .unwrap()
                .to_string(),
            "SYN123456"
        );
        // Label input is case-insensitive and the prefix is optional.
        for input in [" 38165a-syn123456 ", "38165A-SYN123456", "syn123456"] {
            assert_eq!(
                Serial::parse_with_prefix(input, "38165A-")
                    .unwrap()
                    .as_ref(),
                "SYN123456"
            );
        }
        // The canonical form never carries a label prefix.
        assert!(Serial::from_str("38165A-SYN123456").is_err());
        assert_eq!(
            serde_json::to_string(&Serial::from_str("syn123456").unwrap()).unwrap(),
            "\"SYN123456\""
        );
        assert!(Serial::from_str(&"A".repeat(64)).is_ok());
    }
}
