//! Credentials that stay out of Debug output and are cleared from memory on drop.
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(transparent)]
/// A credential that redacts its value in debug output and clears memory on drop.
pub struct Secret(String);
impl Secret {
    /// Wrap a credential for redacted handling.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the credential for a protocol or explicit user operation.
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}
