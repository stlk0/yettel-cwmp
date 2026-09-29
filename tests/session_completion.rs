//! Regression tests for preserving completed settings at cancellation boundaries.
mod common;
use common::*;
use yettel_cwmp::{
    capture,
    cwmp::{
        model::Assignments,
        session::{self, SessionParams},
    },
    domain::{profile::CredentialsSource, secret::Secret},
    error::{Error, Result},
    net::{Cancel, PostKind, Transport},
    store::Store,
};

struct CancelOnReply {
    script: Script,
    cancel: Cancel,
    cancel_on: usize,
    exchanged: usize,
}

impl Transport for CancelOnReply {
    fn post(&mut self, body: &[u8], kind: PostKind) -> Result<Vec<u8>> {
        self.exchanged += 1;
        let reply = self.script.post(body, kind);
        if self.exchanged == self.cancel_on {
            self.cancel.cancel();
        }
        reply
    }
}

#[test]
fn complete_credentials_survive_cancellation_after_rpc_or_final_reply() {
    for cancel_on in [2, 3] {
        let root = tempfile::tempdir().unwrap();
        let profile = profile();
        let device = device();
        let store = Store::open(root.path()).unwrap();
        store.save(&profile).unwrap();
        let cancel = Cancel::default();
        let user = format!("{}Username", ppp_prefix());
        let pass = format!("{}Password", ppp_prefix());
        let script = Script::new(vec![
            Ok(inform_response()),
            Ok(spv(&[
                (acs_password(), "synthetic-rotation"),
                (&user, "synthetic-user"),
                (&pass, "synthetic-pass"),
            ])),
            Ok(vec![]),
        ]);
        let posts = script.posts.clone();
        let result = capture::capture_with(
            &store,
            &profile.serial,
            &device,
            &cancel,
            |_| {
                Ok(CancelOnReply {
                    script,
                    cancel: cancel.clone(),
                    cancel_on,
                    exchanged: 0,
                })
            },
            |_| {},
        )
        .unwrap();
        assert!(cancel.is_cancelled());
        assert!(result.path.exists());
        assert_eq!(result.export.internet.username.expose(), "synthetic-user");
        assert_eq!(result.export.internet.password.expose(), "synthetic-pass");
        assert_eq!(posts.lock().unwrap().len(), cancel_on);
        assert_eq!(
            store.load(&profile.serial).unwrap().password.expose(),
            "synthetic-rotation"
        );
        assert_eq!(
            store
                .load_export(&profile.serial)
                .unwrap()
                .1
                .internet
                .password
                .expose(),
            "synthetic-pass"
        );
    }
}

// A broken synthetic fixture must fail the calling test immediately.
#[allow(clippy::unwrap_used)]
fn capture_ending_with(complete: bool, ending: Error) -> bool {
    let root = tempfile::tempdir().unwrap();
    let profile = profile();
    let store = Store::open(root.path()).unwrap();
    store.save(&profile).unwrap();
    let user = format!("{}Username", ppp_prefix());
    let pass = format!("{}Password", ppp_prefix());
    let mut values = vec![(&*user, "synthetic-user")];
    if complete {
        values.push((&*pass, "synthetic-pass"));
    }
    let script = Script::new(vec![Ok(inform_response()), Ok(spv(&values)), Err(ending)]);
    let posts = script.posts.clone();
    let result = capture::capture_with(
        &store,
        &profile.serial,
        &device(),
        &Cancel::default(),
        |_| Ok(script),
        |_| {},
    );
    assert_eq!(posts.lock().unwrap().len(), 3);
    let succeeded = match result {
        Ok(outcome) => {
            assert_eq!(outcome.export.internet.password.expose(), "synthetic-pass");
            true
        }
        Err(failure) => {
            assert_eq!(failure.error, ending);
            false
        }
    };
    let saved = store.load_export(&profile.serial).is_ok();
    assert_eq!(saved, succeeded);
    succeeded
}

#[test]
fn complete_settings_survive_cancellation_deadline_or_network_failure() {
    for ending in [
        Error::Cancelled,
        Error::Deadline,
        Error::Dns,
        Error::Connect,
        Error::Network,
        Error::Timeout,
    ] {
        assert!(!capture_ending_with(false, ending), "{ending}");
        assert!(capture_ending_with(true, ending), "{ending}");
    }
}

#[test]
fn provider_or_identity_failures_after_complete_settings_still_fail() {
    for ending in [
        Error::HttpStatus(500),
        Error::AuthRejected,
        Error::PinMismatch,
        Error::Tls,
        Error::Protocol,
    ] {
        assert!(!capture_ending_with(true, ending), "{ending}");
    }
}

#[test]
fn rotations_replace_label_or_server_credentials_and_remain_saved_after_failure() {
    for source in [CredentialsSource::Label, CredentialsSource::Server] {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let mut profile = profile();
        if source == CredentialsSource::Server {
            profile.password = Secret::new("synthetic-earlier-rotation");
        }
        profile.credentials_source = source;
        store.save(&profile).unwrap();
        let script = Script::new(vec![
            Ok(inform_response()),
            Ok(spv(&[(acs_password(), "synthetic-rotation")])),
            Err(Error::Network),
        ]);
        let failure = capture::capture_with(
            &store,
            &profile.serial,
            &device(),
            &Cancel::default(),
            |_| Ok(script),
            |_| {},
        )
        .err()
        .unwrap();
        assert_eq!(failure.error, Error::Network);
        let saved = store.load(&profile.serial).unwrap();
        assert_eq!(saved.password.expose(), "synthetic-rotation");
        assert_eq!(saved.credentials_source, CredentialsSource::Server);
    }
}

#[test]
fn rotation_write_failure_stops_before_acknowledgement() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let profile = profile();
    store.save(&profile).unwrap();
    let path = root
        .path()
        .join("devices")
        .join(profile.serial.as_ref())
        .join("profile.json");
    let script = Script::new(vec![
        Ok(inform_response()),
        Ok(spv(&[(acs_password(), "synthetic-rotation")])),
        Ok(vec![]),
    ]);
    let posts = script.posts.clone();
    let failure = capture::capture_with(
        &store,
        &profile.serial,
        &device(),
        &Cancel::default(),
        |_| {
            // Validation has succeeded; an occupied destination now prevents replacement.
            std::fs::remove_file(&path).unwrap();
            std::fs::create_dir(&path).unwrap();
            Ok(script)
        },
        |_| {},
    )
    .err()
    .unwrap();
    assert!(matches!(
        failure.error,
        Error::StorageAccess | Error::StorageFull | Error::Storage
    ));
    assert_eq!(posts.lock().unwrap().len(), 2);
    assert!(path.is_dir());
}

#[test]
fn rotation_write_failure_wins_over_cancellation_and_complete_internet_settings() {
    let device = device();
    let user = format!("{}Username", ppp_prefix());
    let pass = format!("{}Password", ppp_prefix());
    let mut script = Script::new(vec![
        Ok(inform_response()),
        Ok(spv(&[
            (acs_password(), "synthetic-rotation"),
            (&user, "synthetic-user"),
            (&pass, "synthetic-pass"),
        ])),
        Ok(vec![]),
    ]);
    let cancel = Cancel::default();
    let result = session::run(SessionParams {
        transport: &mut script,
        device: &device,
        model: device.model(&profile()).unwrap(),
        cancel: &cancel,
        limit: session::SESSION_LIMIT,
        persist: |_: &Assignments| {
            cancel.cancel();
            Err(Error::Storage)
        },
        progress: |_| {},
    });
    assert_eq!(result, Err(Error::Storage));
    assert_eq!(script.posts.lock().unwrap().len(), 2);
}
