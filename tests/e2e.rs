//! Loopback end-to-end tests for the application and synthetic provider.
#![cfg(all(unix, feature = "dev-provider-override"))]
// Integration tests use assertions and unwraps to make fixture failures explicit.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[allow(dead_code)]
#[path = "../examples/fake_acs.rs"]
mod fake_acs;
#[path = "common/pty.rs"]
mod pty;
use pty::Pty;
use ring::digest::{SHA256, digest};
use rustls::pki_types::{CertificateDer, pem::PemObject};
use std::{
    fs,
    io::Write,
    net::TcpListener,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use yettel_cwmp::{domain::profile::Profile, store::Store};

struct ProcessGuard(Child);

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// The scenario keeps one loopback session lifecycle together for test clarity.
#[allow(clippy::too_many_lines)]
fn scenario(name: &str, expected: &str) {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let cert =
        CertificateDer::from_pem_slice(include_bytes!("fixtures/localhost-cert.pem")).unwrap();
    let pin = digest(&SHA256, cert.as_ref())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let provider = serde_json::json!({
        "acs": {"host": "localhost", "port": port, "path": "/synthetic"},
        "pins": [{"kind":"certificate_sha256", "sha256": pin}]
    });
    let provider_path = root.path().join("provider.json");
    fs::write(&provider_path, serde_json::to_vec(&provider).unwrap()).unwrap();
    let state = root.path().join("state");
    let profile = Profile::new(
        "SYN123456".parse().unwrap(),
        "02:11:22:33:44:55".parse().unwrap(),
        "synthetic-wlan".into(),
    )
    .unwrap();
    Store::open(&state).unwrap().save(&profile).unwrap();
    let server_name = name.to_string();
    let server = thread::spawn(move || fake_acs::run(&server_name, &listener));
    let mut pty = Pty::new(24, 80);
    let mut child = ProcessGuard(
        Command::new(env!("CARGO_BIN_EXE_yettel-cwmp"))
            .args(["--state-dir", state.to_str().unwrap()])
            .env("YETTEL_CWMP_DEV_PROVIDER", &provider_path)
            .args(["--lang", "en"])
            .env("TERM", "xterm-256color")
            .stdin(Stdio::from(pty.slave.try_clone().unwrap()))
            .stdout(Stdio::from(pty.slave.try_clone().unwrap()))
            .stderr(Stdio::from(pty.slave.try_clone().unwrap()))
            .spawn()
            .unwrap(),
    );
    let mut bytes = vec![];
    let mut screen = vt100::Parser::new(24, 80, 0);
    fn wait_for(
        pty: &mut Pty,
        bytes: &mut Vec<u8>,
        screen: &mut vt100::Parser,
        text: &str,
        child: &mut std::process::Child,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let start = bytes.len();
            pty.read_available(bytes);
            screen.process(&bytes[start..]);
            if screen.screen().contents().contains(text) {
                return;
            }
            assert!(
                Instant::now() < deadline && child.try_wait().unwrap().is_none(),
                "missing {text:?}: {}",
                screen.screen().contents()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    wait_for(&mut pty, &mut bytes, &mut screen, "DEV BUILD", &mut child.0);
    pty.master.write_all(b"\r").unwrap();
    wait_for(
        &mut pty,
        &mut bytes,
        &mut screen,
        "Router SYN123456",
        &mut child.0,
    );
    pty.master.write_all(b"R").unwrap();
    wait_for(
        &mut pty,
        &mut bytes,
        &mut screen,
        "Before you connect",
        &mut child.0,
    );
    pty.master.write_all(b"\r").unwrap();
    if name == "slow" {
        wait_for(
            &mut pty,
            &mut bytes,
            &mut screen,
            "Receiving settings",
            &mut child.0,
        );
        let before_cancel = bytes.len();
        pty.master.write_all(b"\x1b").unwrap();
        wait_for(&mut pty, &mut bytes, &mut screen, expected, &mut child.0);
        assert!(
            bytes[before_cancel..]
                .windows(b"Cancelling".len())
                .any(|part| part == b"Cancelling"),
            "the cancellation state was never emitted"
        );
    } else {
        wait_for(&mut pty, &mut bytes, &mut screen, expected, &mut child.0);
    }
    for secret in [
        "synthetic-ppp-user",
        "synthetic-ppp-password",
        "synthetic-wlan",
        "synthetic-rotated-user",
        "synthetic-rotated-password",
    ] {
        assert!(
            !screen.screen().contents().contains(secret),
            "screen leaked {secret}"
        );
        assert!(
            !bytes
                .windows(secret.len())
                .any(|part| part == secret.as_bytes()),
            "terminal bytes leaked {secret}"
        );
    }
    if name == "auth-rejected" {
        pty.master.write_all(b"K").unwrap();
        wait_for(
            &mut pty,
            &mut bytes,
            &mut screen,
            "Change Wi-Fi key",
            &mut child.0,
        );
        pty.master.write_all(b"synthetic-new-wlan\r").unwrap();
        wait_for(
            &mut pty,
            &mut bytes,
            &mut screen,
            "Router SYN123456",
            &mut child.0,
        );
        let repaired: serde_json::Value = serde_json::from_slice(
            &fs::read(state.join("devices/SYN123456/profile.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(repaired["password"], "synthetic-new-wlan");
    }
    if name == "success" {
        pty.master.write_all(b"\x1b").unwrap();
        wait_for(
            &mut pty,
            &mut bytes,
            &mut screen,
            "Router SYN123456",
            &mut child.0,
        );
        pty.master.write_all(b"R").unwrap();
        wait_for(
            &mut pty,
            &mut bytes,
            &mut screen,
            "Before you connect",
            &mut child.0,
        );
        pty.master.write_all(b"\x1b").unwrap();
        wait_for(
            &mut pty,
            &mut bytes,
            &mut screen,
            "Router SYN123456",
            &mut child.0,
        );
    }
    pty.master.write_all(b"Q").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.0.try_wait().unwrap().is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(child.0.try_wait().unwrap().unwrap().success());
    pty.read_available(&mut bytes);
    pty.assert_restored(&bytes);
    if name != "slow" {
        assert!(server.join().unwrap().is_ok());
    }
}

#[test]
fn success_hides_credentials() {
    scenario("success", "Done: internet settings received");
}

#[test]
fn rejected_credentials_show_error_and_allow_key_repair() {
    scenario("auth-rejected", "Error code: ACS-AUTH");
}

#[test]
fn provider_http_error_has_retry_action() {
    scenario("http-500", "Error code: ACS-HTTP");
}

#[test]
fn missing_password_is_reported_as_incomplete() {
    scenario("incomplete", "Error code: ACS-INCOMPLETE");
}

#[test]
fn slow_session_can_be_cancelled_during_receiving() {
    scenario("slow", "Cancelled");
}
