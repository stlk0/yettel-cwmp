//! Private profile lifecycle, session ordering, and wire behavior regressions.
mod common;
use common::*;
use std::{
    fs,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use yettel_cwmp::{
    capture,
    cwmp::{model::Assignments, rpc::Cpe, session, soap},
    domain::secret::Secret,
    error::Error,
    net::Cancel,
    progress::{Rpc, Stage},
    store::Store,
};

#[test]
fn profile_identity_is_validated_before_transport_construction() {
    let root = tempfile::tempdir().unwrap();
    let p = profile();
    let d = device();
    let store = Store::open(root.path()).unwrap();
    store.save(&p).unwrap();
    {
        let path = root
            .path()
            .join("devices")
            .join(p.serial.as_ref())
            .join("profile.json");
        let mut corrupt: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        corrupt["serial"] = "DIFFERENT123".into();
        fs::write(&path, serde_json::to_vec(&corrupt).unwrap()).unwrap();
        assert!(matches!(store.load(&p.serial), Err(Error::ProfileInvalid)));
    }
    let attempted = AtomicBool::new(false);
    let result = capture::capture_with(
        &store,
        &p.serial,
        &d,
        &Cancel::default(),
        |_| {
            attempted.store(true, Ordering::SeqCst);
            Ok(Script::new(vec![]))
        },
        |_| {},
    );
    assert!(matches!(
        result.map_err(|failure| failure.error),
        Err(Error::ProfileInvalid)
    ));
    assert!(!attempted.load(Ordering::SeqCst));
}
#[test]
fn inform_and_metadata() {
    let d = device();
    assert_eq!(d.serial_prefix(), "38165A-");
    let model = d.model(&profile()).unwrap();
    let raw = soap::inform(&model, d.inform_parameters()).unwrap();
    let doc = soap::parse(&raw).unwrap();
    let id = doc.id();
    let rpc = doc.method().unwrap();
    assert_eq!(id, Some("1"));
    assert!(rpc.has_tag_name((soap::CWMP, "Inform")));
    let text = String::from_utf8(raw.clone()).unwrap();
    assert!(text.contains("0 BOOTSTRAP"));
    assert!(text.contains("1 BOOT"));
    assert!(text.contains("38165A-SYN123456"));
    assert!(!raw.starts_with(b"<?xml"));
    assert_eq!(d.inform_parameters().len(), 8);
    assert!(model.hidden.iter().any(|p| p.ends_with("WEPKey")));
    assert_eq!(model.params["InternetGatewayDevice.WANDevice.1.WANConnectionDevice.1.WANIPConnection.3.MACAddress"].value,"02:11:22:33:44:58");
}
#[test]
fn rpc_hidden_values_and_faults() {
    let mut cpe = Cpe::new(device().model(&profile()).unwrap());
    let raw = spv(&[
        (acs_password(), "old"),
        (acs_password(), "new<!--ignore-->part<?pi x?>end"),
    ]);
    cpe.handle(&raw).reply.unwrap();
    assert_eq!(cpe.received[acs_password()].expose(), "newpartend");
    assert_eq!(cpe.model.params[parameter_key()].value, "synthetic-key");
    let reply = cpe
        .handle(&rpc(
            "GetParameterValues",
            &format!(
                "<ParameterNames><string>{}</string></ParameterNames>",
                acs_password()
            ),
        ))
        .reply
        .unwrap();
    let text = String::from_utf8(reply).unwrap();
    assert!(!text.contains("newpartend"));
    assert!(text.contains("xsd:string"));
    let handled = cpe.handle(&rpc("Reboot", ""));
    assert_eq!(handled.rpc, Rpc::Unsupported);
    let reply = handled.reply.unwrap();
    let text = String::from_utf8(reply).unwrap();
    assert!(text.contains("9000"));
    assert!(text.contains("<faultcode>Server</faultcode>"));
    assert_eq!(cpe.handle(b"<secret-broken>").reply, Err(Error::Protocol));
    assert_eq!(cpe.handle(b"<secret-broken>").rpc, Rpc::Unknown);
    let fault = soap::envelope(None, soap::Element::new("SOAP-ENV:Fault")).unwrap();
    assert_eq!(cpe.handle(&fault).reply, Err(Error::Protocol));
}
#[test]
fn parameter_names_natural_order_and_next_level() {
    let d = device();
    let mut model = d.model(&profile()).unwrap();
    model.params.insert(
        "A.10.Value".into(),
        ("xsd:string".into(), "ten".into()).into(),
    );
    model.params.insert(
        "A.2.Value".into(),
        ("xsd:string".into(), "two".into()).into(),
    );
    model
        .objects
        .extend(["A.", "A.2.", "A.10."].map(String::from));
    let mut cpe = Cpe::new(model);
    let reply = cpe
        .handle(&rpc(
            "GetParameterNames",
            "<ParameterPath>A.</ParameterPath><NextLevel>1</NextLevel>",
        ))
        .reply
        .unwrap();
    let text = String::from_utf8(reply).unwrap();
    assert!(text.find("A.2.").unwrap() < text.find("A.10.").unwrap());
    assert!(!text.contains("A.2.Value"));
    let reply = cpe
        .handle(&rpc(
            "GetParameterNames",
            "<ParameterPath>A.2.Value</ParameterPath><NextLevel>true</NextLevel>",
        ))
        .reply
        .unwrap();
    assert!(String::from_utf8(reply).unwrap().contains("9003"));
}
#[test]
fn successful_session_latest_assignments_and_export_shape() {
    let root = tempfile::tempdir().unwrap();
    let p = profile();
    let d = device();
    let store = Store::open(root.path()).unwrap();
    store.save(&p).unwrap();
    let user = format!("{}Username", ppp_prefix());
    let pass = format!("{}Password", ppp_prefix());
    let script = Script::new(vec![
        Ok(inform_response()),
        Ok(spv(&[(&user, "old-user"), (&pass, "synthetic-pass")])),
        Ok(spv(&[(&user, "latest-user")])),
        Ok(b" \r\n".to_vec()),
    ]);
    let posts = script.posts.clone();
    let out = capture::capture_with(
        &store,
        &p.serial,
        &d,
        &Cancel::default(),
        |_| Ok(script),
        |_| {},
    )
    .unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(out.path).unwrap()).unwrap();
    assert_eq!(
        saved,
        serde_json::json!({"internet":{"protocol":"PPPoE","vlan_id":710,"mtu":1492,"username":"latest-user","password":"synthetic-pass"}})
    );
    assert!(posts.lock().unwrap()[1].is_empty());
    assert_eq!(posts.lock().unwrap().len(), 4);
}
#[test]
fn persistence_before_ack_failure_context_and_cancel() {
    let d = device();
    let token = Cancel::default();
    let mut script = Script::new(vec![
        Ok(inform_response()),
        Ok(spv(&[(acs_password(), "rotated")])),
        Ok(vec![]),
    ]);
    let posts = script.posts.clone();
    let mut last = Default::default();
    let result = session::run(session::SessionParams {
        transport: &mut script,
        device: &d,
        model: d.model(&profile()).unwrap(),
        cancel: &token,
        limit: session::SESSION_LIMIT,
        persist: |_: &Assignments| {
            assert_eq!(posts.lock().unwrap().len(), 2);
            Err(Error::Storage)
        },
        progress: |p| last = p,
    });
    assert_eq!(result, Err(Error::Storage));
    assert_eq!(last.rpc, Rpc::SetParameterValues);
    assert_eq!(last.stage, Stage::Session);
    assert_eq!(posts.lock().unwrap().len(), 2);
    let mut script = Script::new(vec![
        Ok(inform_response()),
        Ok(spv(&[(acs_password(), "rotated")])),
        Ok(vec![]),
    ]);
    let mut saved = false;
    let result = session::run(session::SessionParams {
        transport: &mut script,
        device: &d,
        model: d.model(&profile()).unwrap(),
        cancel: &token,
        limit: session::SESSION_LIMIT,
        persist: |values: &Assignments| {
            token.cancel();
            assert_eq!(values[acs_password()].expose(), "rotated");
            saved = true;
            Ok(())
        },
        progress: |_| {},
    });
    assert_eq!(result, Err(Error::Cancelled));
    assert!(saved);
    assert_eq!(script.posts.lock().unwrap().len(), 2);
}
#[test]
fn incomplete_deadline_and_bad_inform() {
    let d = device();
    let mut received = Assignments::new();
    received.insert(format!("{}Username", ppp_prefix()), Secret::new("u"));
    received.insert(
        format!("{}Password", ppp_prefix()),
        Secret::new(String::new()),
    );
    assert!(matches!(d.export(&received), Err(Error::Incomplete)));
    let mut script = Script::new(vec![]);
    let result = session::run(session::SessionParams {
        transport: &mut script,
        device: &d,
        model: d.model(&profile()).unwrap(),
        cancel: &Cancel::default(),
        limit: Duration::ZERO,
        persist: |_: &Assignments| Ok(()),
        progress: |_| {},
    });
    assert_eq!(result, Err(Error::Deadline));
    assert!(script.posts.lock().unwrap().is_empty());
    let mut script = Script::new(vec![Ok(rpc("GetRPCMethods", ""))]);
    let result = session::run(session::SessionParams {
        transport: &mut script,
        device: &d,
        model: d.model(&profile()).unwrap(),
        cancel: &Cancel::default(),
        limit: session::SESSION_LIMIT,
        persist: |_: &Assignments| Ok(()),
        progress: |_| {},
    });
    assert_eq!(result, Err(Error::Protocol));
}
#[test]
fn rotations_are_saved_before_ack_and_survive_unsuccessful_sessions() {
    use yettel_cwmp::net::Transport;
    struct InspectAck<'a> {
        script: Script,
        path: std::path::PathBuf,
        username: &'a str,
        observed: &'a mut bool,
    }
    impl Transport for InspectAck<'_> {
        fn post(
            &mut self,
            body: &[u8],
            kind: yettel_cwmp::net::PostKind,
        ) -> yettel_cwmp::error::Result<Vec<u8>> {
            if self.script.posts.lock().unwrap().len() == 2 {
                let saved: serde_json::Value =
                    serde_json::from_slice(&fs::read(&self.path).unwrap()).unwrap();
                assert_eq!(saved["username"], self.username);
                assert_eq!(saved["password"], "rotated");
                assert_eq!(saved["credentials_source"], "server");
                assert!(String::from_utf8_lossy(body).contains("SetParameterValuesResponse"));
                *self.observed = true;
            }
            self.script.post(body, kind)
        }
    }
    for (username, ending, expected_error) in [
        (Some("new-user"), Err(Error::Network), Error::Network),
        (None, Ok(vec![]), Error::Incomplete),
    ] {
        let root = tempfile::tempdir().unwrap();
        let p = profile();
        let d = device();
        let store = Store::open(root.path()).unwrap();
        store.save(&p).unwrap();
        let path = root
            .path()
            .join("devices")
            .join(p.serial.as_ref())
            .join("profile.json");
        let old = root
            .path()
            .join("devices")
            .join(p.serial.as_ref())
            .join("extracted-credentials.json");
        fs::write(&old, br#"{"previous":true}"#).unwrap();
        let previous = fs::read(&old).unwrap();
        let mut assignments = vec![(acs_password(), "rotated")];
        if let Some(username) = username {
            assignments.push((acs_username(), username));
        }
        let username = username.unwrap_or(p.username.expose());
        let mut observed = false;
        let script = Script::new(vec![Ok(inform_response()), Ok(spv(&assignments)), ending]);
        let result = capture::capture_with(
            &store,
            &p.serial,
            &d,
            &Cancel::default(),
            |_| {
                Ok(InspectAck {
                    script,
                    path,
                    username,
                    observed: &mut observed,
                })
            },
            |_| {},
        );
        assert!(matches!(result, Err(failure) if failure.error == expected_error));
        assert!(observed);
        assert_eq!(fs::read(old).unwrap(), previous);
        let retry = capture::capture_with(
            &store,
            &p.serial,
            &d,
            &Cancel::default(),
            |p| {
                assert_eq!(p.username.expose(), username);
                assert_eq!(p.password.expose(), "rotated");
                assert_eq!(
                    p.credentials_source,
                    yettel_cwmp::domain::profile::CredentialsSource::Server
                );
                Err::<Script, _>(Error::Network)
            },
            |_| {},
        );
        assert!(matches!(retry, Err(failure) if failure.error == Error::Network));
    }
}

#[test]
fn cancellation_after_accepted_rotation_keeps_failure_facts_and_saved_credentials() {
    let root = tempfile::tempdir().unwrap();
    let p = profile();
    let d = device();
    let store = Store::open(root.path()).unwrap();
    store.save(&p).unwrap();
    let cancel = Cancel::default();
    let script = Script::new(vec![
        Ok(inform_response()),
        Ok(spv(&[(acs_password(), "synthetic-rotated-before-cancel")])),
        Ok(vec![]),
    ]);
    let posts = script.posts.clone();
    let result = capture::capture_with(
        &store,
        &p.serial,
        &d,
        &cancel,
        |_| Ok(script),
        |progress| {
            if progress.rpc == Rpc::SetParameterValues {
                cancel.cancel();
            }
        },
    );
    let failure = result.err().unwrap();
    assert_eq!(failure.error, Error::Cancelled);
    assert_eq!(failure.progress.rpc, Rpc::SetParameterValues);
    assert_eq!(posts.lock().unwrap().len(), 2);
    let saved = store.load(&p.serial).unwrap();
    assert_eq!(saved.password.expose(), "synthetic-rotated-before-cancel");
}

#[test]
fn invalid_device_template_returns_error_instead_of_panicking() {
    let mut device = device();
    device.template.parameters.remove(acs_username());
    assert!(matches!(device.model(&profile()), Err(Error::Internal)));
}

#[test]
fn accepted_assignment_debug_redacts_every_credential() {
    let device = device();
    let paths = &device.template.credentials;
    let secrets = [
        (&paths.acs_username, "synthetic-management-user"),
        (&paths.acs_password, "synthetic-management-pass"),
        (&paths.ppp_username, "synthetic-internet-user"),
        (&paths.ppp_password, "synthetic-internet-pass"),
    ];
    let request = secrets
        .iter()
        .map(|(name, value)| (name.as_str(), *value))
        .collect::<Vec<_>>();
    let mut cpe = Cpe::new(device.model(&profile()).unwrap());
    cpe.handle(&spv(&request)).reply.unwrap();
    let debug = format!("{:?}", cpe.received);
    assert_eq!(cpe.received.len(), 4);
    assert_eq!(debug.matches("[redacted]").count(), 4);
    for (name, value) in secrets {
        assert_eq!(cpe.received[name].expose(), value);
        assert!(!debug.contains(value));
    }
}
