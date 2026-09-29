//! One capture of internet settings: locked profile, pinned session, rotation persistence and
//! export. Rotated management credentials are saved before the provider receives their
//! acknowledgement, and complete settings are saved even after a late cancellation.
use crate::{
    cwmp::{
        model::{Assignments, Device},
        session::{self, SESSION_LIMIT, SessionParams},
    },
    domain::{
        export::Export,
        profile::{CredentialsSource, Profile},
        serial::Serial,
    },
    error::{Error, Result},
    net::{AcsTransport, Cancel, Transport},
    progress::{Progress, Stage},
    store::Store,
};
use std::path::PathBuf;

/// A fully received and persisted set of internet settings.
pub struct Outcome {
    /// Private destination of the exported settings.
    pub path: PathBuf,
    /// Received credentials and bundled internet defaults.
    pub export: Export,
}

/// Secret-free facts retained when a capture does not complete.
#[derive(Clone, Copy, Debug)]
pub struct Failure {
    /// Stable error category; never an underlying provider message.
    pub error: Error,
    /// Last observed session stage and RPC.
    pub progress: Progress,
}

/// Capture settings through the pinned provider connection.
pub fn capture(
    store: &Store,
    serial: &Serial,
    device: &Device,
    cancel: &Cancel,
    progress: impl FnMut(Progress),
) -> std::result::Result<Outcome, Failure> {
    capture_with(
        store,
        serial,
        device,
        cancel,
        |profile| {
            AcsTransport::new(
                &device.provider.acs,
                &device.provider.pins,
                profile.username.clone(),
                profile.password.clone(),
                cancel.clone(),
            )
        },
        progress,
    )
}

/// Build the transport only after validating the saved profile; tests inject scripted peers.
pub fn capture_with<T: Transport>(
    store: &Store,
    serial: &Serial,
    device: &Device,
    cancel: &Cancel,
    transport: impl FnOnce(&Profile) -> Result<T>,
    mut progress: impl FnMut(Progress),
) -> std::result::Result<Outcome, Failure> {
    let mut last = Progress::default();
    progress(last);
    let result = (|| {
        cancel.check()?;
        let mut profile = store.load(serial)?;
        let model = device.model(&profile)?;
        let mut transport = transport(&profile)?;
        let received = session::run(SessionParams {
            transport: &mut transport,
            device,
            model,
            cancel,
            limit: SESSION_LIMIT,
            persist: |received: &Assignments| {
                persist_rotation(store, &mut profile, device, received)
            },
            progress: |p: Progress| {
                last = p;
                progress(p);
            },
        })?;
        drop(transport);
        // Complete settings must finish their local save after a late cancellation,
        // deadline or connection failure.
        last.stage = Stage::Export;
        progress(last);
        let export = device.export(&received)?;
        let path = store.save_export(serial, &export)?;
        Ok(Outcome { path, export })
    })();
    result.map_err(|error| Failure {
        error,
        progress: last,
    })
}

fn persist_rotation(
    store: &Store,
    profile: &mut Profile,
    device: &Device,
    received: &Assignments,
) -> Result<()> {
    let credentials = &device.template.credentials;
    let changed = [
        (&credentials.acs_username, &profile.username),
        (&credentials.acs_password, &profile.password),
    ]
    .into_iter()
    .any(|(name, current)| {
        received
            .get(name)
            .is_some_and(|value| value.expose() != current.expose())
    });
    if changed {
        for (name, target) in [
            (&credentials.acs_username, &mut profile.username),
            (&credentials.acs_password, &mut profile.password),
        ] {
            if let Some(value) = received.get(name) {
                *target = value.clone();
            }
        }
        profile.credentials_source = CredentialsSource::Server;
        store.save(profile)?;
    }
    Ok(())
}
