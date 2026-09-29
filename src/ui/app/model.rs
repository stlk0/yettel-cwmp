//! Public screen and action states for the terminal application.
use crate::{
    capture::Outcome,
    domain::{profile::ProfileSummary, serial::Serial},
    error::Error,
    progress::Progress,
};
use std::time::Instant;

/// Current terminal view and the data required to render it.
pub enum Screen {
    /// List saved routers and navigation actions.
    Profiles,
    /// Enter identity and label credentials for a new router.
    Create,
    /// Show actions for a saved router.
    Device(ProfileSummary),
    /// Edit the router label key.
    ChangeKey(ProfileSummary),
    /// Confirm replacement of server-issued management credentials.
    ReplaceSaved(ProfileSummary),
    /// Confirm deletion, including recovery from a corrupt profile.
    DeleteConfirm {
        /// Selected serial even when its profile cannot be read.
        serial: Serial,
        /// Readable profile, when available.
        profile: Option<ProfileSummary>,
    },
    /// Require acknowledgement before every provider connection.
    ConnectNotice(ProfileSummary),
    /// Show active network progress and cancellation state.
    Running {
        /// Router being queried.
        profile: ProfileSummary,
        /// Last reported session progress.
        progress: Progress,
        /// Whether cancellation was requested.
        cancelling: bool,
        /// Time when the session started.
        started: Instant,
    },
    /// Show captured or previously saved internet settings.
    Result {
        /// Router associated with these settings.
        profile: ProfileSummary,
        /// Captured settings and saved path.
        outcome: Outcome,
        /// Whether credentials are explicitly visible.
        reveal: bool,
        /// Whether the result came from the current session.
        fresh: bool,
    },
    /// Show a safe error and the last session stage and RPC.
    Error {
        /// Readable profile, when available.
        profile: Option<ProfileSummary>,
        /// Selected serial even if the profile is corrupt.
        serial: Option<Serial>,
        /// Classified failure.
        error: Error,
        /// Last reported session progress.
        progress: Progress,
    },
}
impl Screen {
    pub(super) fn profile(&self) -> Option<&ProfileSummary> {
        match self {
            Self::Device(profile)
            | Self::ChangeKey(profile)
            | Self::ReplaceSaved(profile)
            | Self::ConnectNotice(profile)
            | Self::Running { profile, .. }
            | Self::Result { profile, .. } => Some(profile),
            Self::DeleteConfirm { profile, .. } | Self::Error { profile, .. } => profile.as_ref(),
            Self::Profiles | Self::Create => None,
        }
    }
    pub(super) fn into_profile(self) -> Option<ProfileSummary> {
        match self {
            Self::Device(profile)
            | Self::ChangeKey(profile)
            | Self::ReplaceSaved(profile)
            | Self::ConnectNotice(profile)
            | Self::Running { profile, .. }
            | Self::Result { profile, .. } => Some(profile),
            Self::DeleteConfirm { profile, .. } | Self::Error { profile, .. } => profile,
            Self::Profiles | Self::Create => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// An effect requested by a terminal key event.
pub enum Action {
    /// No effect beyond local view updates.
    None,
    /// Exit the application.
    Quit,
    /// Open the selected saved router.
    Open,
    /// Load previously saved internet settings.
    ViewSaved,
    /// Save a new router profile.
    Create,
    /// Begin or continue a provider session.
    Start,
    /// Replace the router label key.
    ChangeKey,
    /// Confirm replacement of server-issued credentials.
    ConfirmReplace,
    /// Confirm deletion of the selected router.
    ConfirmDelete,
    /// Retry the previous operation.
    Retry,
    /// Copy the saved internet password explicitly.
    CopyPassword,
    /// Copy the saved internet username explicitly.
    CopyUsername,
}
