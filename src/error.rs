//! Compact failures with stable codes and no retained private data.
use std::{fmt, io};

/// Classified failures safe to retain and format without exposing input or OS messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidSerial,
    InvalidMac,
    EmptyKey,
    InvalidText,
    ProfileExists,
    ProfileConflict,
    ProfileMissing,
    ProfileInvalid,
    NoSavedSettings,
    AlreadyRunning,
    StorageAccess,
    StorageFull,
    Storage,
    Dns,
    Connect,
    Timeout,
    Network,
    PinMismatch,
    Tls,
    AuthRejected,
    HttpStatus(u16),
    Protocol,
    Incomplete,
    Cancelled,
    Deadline,
    Terminal,
    Internal,
}

impl Error {
    /// Every error, for exhaustive message and layout checks. The i18n tests verify that it
    /// matches the codes declared below.
    pub const ALL: [Self; 27] = [
        Self::InvalidSerial,
        Self::InvalidMac,
        Self::EmptyKey,
        Self::InvalidText,
        Self::ProfileExists,
        Self::ProfileConflict,
        Self::ProfileMissing,
        Self::ProfileInvalid,
        Self::NoSavedSettings,
        Self::AlreadyRunning,
        Self::StorageAccess,
        Self::StorageFull,
        Self::Storage,
        Self::Dns,
        Self::Connect,
        Self::Timeout,
        Self::Network,
        Self::PinMismatch,
        Self::Tls,
        Self::AuthRejected,
        Self::HttpStatus(500),
        Self::Protocol,
        Self::Incomplete,
        Self::Cancelled,
        Self::Deadline,
        Self::Terminal,
        Self::Internal,
    ];

    /// Stable code used to select localized guidance.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidSerial => "IN-SERIAL",
            Self::InvalidMac => "IN-MAC",
            Self::EmptyKey => "IN-KEY",
            Self::InvalidText => "IN-TEXT",
            Self::ProfileExists => "PR-EXISTS",
            Self::ProfileConflict => "PR-CONFLICT",
            Self::ProfileMissing => "PR-MISSING",
            Self::ProfileInvalid => "PR-INVALID",
            Self::NoSavedSettings => "NO-EXPORT",
            Self::AlreadyRunning => "PR-LOCKED",
            Self::StorageAccess => "ST-ACCESS",
            Self::StorageFull => "ST-SPACE",
            Self::Storage => "ST-OTHER",
            Self::Dns => "NET-DNS",
            Self::Connect => "NET-CONNECT",
            Self::Timeout => "NET-TIMEOUT",
            Self::Network => "NET-OTHER",
            Self::PinMismatch => "TLS-PIN",
            Self::Tls => "TLS-HANDSHAKE",
            Self::AuthRejected => "ACS-AUTH",
            Self::HttpStatus(_) => "ACS-HTTP",
            Self::Protocol => "ACS-PROTOCOL",
            Self::Incomplete => "ACS-INCOMPLETE",
            Self::Cancelled => "CANCELLED",
            Self::Deadline => "SESSION-TIMEOUT",
            Self::Terminal => "TERM",
            Self::Internal => "INTERNAL",
        }
    }

    /// Classify a filesystem error without retaining its message or path.
    pub fn storage_io(error: &io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
                Self::StorageAccess
            }
            io::ErrorKind::StorageFull => Self::StorageFull,
            _ => Self::Storage,
        }
    }

    /// Classify socket I/O without retaining endpoint details or OS messages.
    pub fn network_io(error: &io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::ConnectionRefused
            | io::ErrorKind::NetworkUnreachable
            | io::ErrorKind::HostUnreachable => Self::Connect,
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Self::Timeout,
            _ => Self::Network,
        }
    }
}

// Serde's try_from-based Serial and MacAddress deserialization requires Display.
impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// Result shared by application operations.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_classification_discards_private_error_text() {
        let cases = [
            (
                io::ErrorKind::PermissionDenied,
                Error::StorageAccess,
                Error::Network,
            ),
            (
                io::ErrorKind::ReadOnlyFilesystem,
                Error::StorageAccess,
                Error::Network,
            ),
            (
                io::ErrorKind::StorageFull,
                Error::StorageFull,
                Error::Network,
            ),
            (io::ErrorKind::NotFound, Error::Storage, Error::Network),
            (
                io::ErrorKind::ConnectionRefused,
                Error::Storage,
                Error::Connect,
            ),
            (
                io::ErrorKind::NetworkUnreachable,
                Error::Storage,
                Error::Connect,
            ),
            (
                io::ErrorKind::HostUnreachable,
                Error::Storage,
                Error::Connect,
            ),
            (io::ErrorKind::TimedOut, Error::Storage, Error::Timeout),
            (io::ErrorKind::WouldBlock, Error::Storage, Error::Timeout),
            (
                io::ErrorKind::ConnectionReset,
                Error::Storage,
                Error::Network,
            ),
            (
                io::ErrorKind::ConnectionAborted,
                Error::Storage,
                Error::Network,
            ),
            (io::ErrorKind::UnexpectedEof, Error::Storage, Error::Network),
        ];
        for (kind, storage, network) in cases {
            let original = io::Error::new(kind, "synthetic-private-error-text");
            assert_eq!(Error::storage_io(&original), storage);
            assert_eq!(Error::network_io(&original), network);
            for classified in [storage, network] {
                assert!(
                    !format!("{classified:?} {classified}")
                        .contains("synthetic-private-error-text")
                );
            }
        }
        assert!(std::mem::size_of::<Error>() <= 8);
    }
}
