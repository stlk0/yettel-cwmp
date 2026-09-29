//! PTY tests for terminal navigation, recovery, and restoration.
#![cfg(unix)]
// Integration tests use assertions and unwraps to make fixture failures explicit.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "common/pty.rs"]
mod pty;
use pty::Pty;
use std::{
    fs,
    io::Write,
    os::fd::AsRawFd,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(5);

struct Terminal {
    temporary: tempfile::TempDir,
    child: Child,
    pty: Pty,
    output: Vec<u8>,
    parser: vt100::Parser,
    screens: Vec<String>,
}

impl Terminal {
    fn start(bad_profile: bool) -> Self {
        Self::start_with_prefix(bad_profile, ".tmp")
    }

    fn start_with_prefix(bad_profile: bool, prefix: &str) -> Self {
        Self::start_with_options(bad_profile, prefix, false)
    }

    fn start_with_options(bad_profile: bool, prefix: &str, worker_panic: bool) -> Self {
        let temporary = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
        if bad_profile {
            let directory = temporary.path().join("state/devices/SYN123456");
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("profile.json"), "{broken synthetic data").unwrap();
        }
        let pty = Pty::new(28, 100);
        let mut command = Command::new(env!("CARGO_BIN_EXE_yettel-cwmp"));
        command
            .args(["--state-dir", "state"])
            .current_dir(temporary.path())
            .env("TERM", "xterm-256color")
            .args(["--lang", "en"])
            .stdin(Stdio::from(pty.slave.try_clone().unwrap()))
            .stdout(Stdio::from(pty.slave.try_clone().unwrap()))
            .stderr(Stdio::from(pty.slave.try_clone().unwrap()));
        if worker_panic {
            command.env("YETTEL_CWMP_DEV_WORKER_PANIC", "1");
        }
        let child = command.spawn().unwrap();
        let mut terminal = Self {
            temporary,
            child,
            pty,
            output: vec![],
            parser: vt100::Parser::new(28, 100, 0),
            screens: vec![],
        };
        terminal.expect_since(0, "Yettel internet settings");
        terminal
    }

    fn start_with_unusable_root() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("unusable");
        fs::write(&state, b"synthetic file instead of a directory").unwrap();
        let pty = Pty::new(24, 80);
        let child = Command::new(env!("CARGO_BIN_EXE_yettel-cwmp"))
            .args(["--state-dir", state.to_str().unwrap()])
            .env("TERM", "xterm-256color")
            .args(["--lang", "en"])
            .stdin(Stdio::from(pty.slave.try_clone().unwrap()))
            .stdout(Stdio::from(pty.slave.try_clone().unwrap()))
            .stderr(Stdio::from(pty.slave.try_clone().unwrap()))
            .spawn()
            .unwrap();
        let mut terminal = Self {
            temporary,
            child,
            pty,
            output: vec![],
            parser: vt100::Parser::new(24, 80, 0),
            screens: vec![],
        };
        terminal.expect_since(0, "ST-OTHER");
        terminal
    }

    fn directory(&self, serial: &str) -> PathBuf {
        self.temporary.path().join("state/devices").join(serial)
    }

    fn profile(&self, serial: &str) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.directory(serial).join("profile.json")).unwrap())
            .unwrap()
    }

    fn seed_export(&self) {
        let path = self
            .directory("SYN123456")
            .join("extracted-credentials.json");
        fs::write(&path, r#"{"internet":{"protocol":"PPPoE","vlan_id":710,"mtu":1492,"username":"synthetic-ppp","password":"7890123456789012"}}"#).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    fn send(&mut self, keys: &[u8]) {
        self.pty.master.write_all(keys).unwrap();
    }

    fn press_until(&mut self, keys: &[u8], text: &str) {
        let start = self.output.len();
        self.send(keys);
        self.expect_since(start, text);
    }

    fn read_available(&mut self) {
        let start = self.output.len();
        self.pty.read_available(&mut self.output);
        if self.output.len() != start {
            self.parser.process(&self.output[start..]);
            self.screens.push(self.parser.screen().contents());
        }
    }

    fn expect_since(&mut self, start: usize, text: &str) {
        let deadline = Instant::now() + WAIT;
        loop {
            self.read_available();
            if self.output.len() > start && self.parser.screen().contents().contains(text) {
                return;
            }
            assert!(
                self.child.try_wait().unwrap().is_none() && Instant::now() < deadline,
                "missing {text:?}; output: {:?}",
                self.parser.screen().contents()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn expect_screen(&mut self, predicate: impl Fn(&str) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            self.read_available();
            let contents = self.parser.screen().contents();
            if predicate(&contents) {
                return;
            }
            assert!(
                self.child.try_wait().unwrap().is_none() && Instant::now() < deadline,
                "screen did not settle: {contents:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn assert_hidden(&self, secret: &str) {
        assert!(self.screens.iter().all(|screen| !screen.contains(secret)));
        assert!(
            !self
                .output
                .windows(secret.len())
                .any(|part| part == secret.as_bytes())
        );
    }

    fn resize(&mut self, rows: u16, columns: u16) {
        self.parser.screen_mut().set_size(rows, columns);
        let size = libc::winsize {
            ws_row: rows,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        assert_eq!(
            // SAFETY: the slave fd is valid and size points to a live winsize value.
            unsafe { libc::ioctl(self.pty.slave.as_raw_fd(), libc::TIOCSWINSZ, &size) },
            0
        );
        self.signal(libc::SIGWINCH);
    }

    fn signal(&self, signal: libc::c_int) {
        assert_eq!(
            // SAFETY: child.id is a live child PID and signal is a valid test signal.
            unsafe { libc::kill(self.child.id() as libc::pid_t, signal) },
            0
        );
    }

    fn finish(self) {
        self.finish_code(0);
    }

    fn finish_code(mut self, expected: i32) {
        let deadline = Instant::now() + WAIT;
        loop {
            self.read_available();
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(
                    status.code(),
                    Some(expected),
                    "application exited with {status}"
                );
                break;
            }
            assert!(Instant::now() < deadline, "application did not exit");
            thread::sleep(Duration::from_millis(10));
        }
        self.read_available();
        self.pty.assert_restored(&self.output);
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn saved_result_masks_both_credentials_and_scrolls_in_minimum_window() {
    let mut terminal = Terminal::start_with_prefix(false, &"long-state-directory-".repeat(8));
    terminal.press_until(b"N", "Wi-Fi key");
    terminal.press_until(
        b"SYN123456\t02:11:22:33:44:55\tsynthetic-hidden-key\r",
        "Router SYN123456",
    );
    assert_eq!(
        terminal.profile("SYN123456")["password"],
        "synthetic-hidden-key"
    );
    terminal.assert_hidden("synthetic-hidden-key");
    terminal.seed_export();
    terminal.press_until(b"\x1b", "Your routers");
    terminal.press_until(b"\r", "Router SYN123456");
    terminal.resize(16, 52);
    terminal.expect_screen(|screen| {
        screen.contains("View saved settings")
            && screen
                .lines()
                .nth(15)
                .is_some_and(|footer| footer.contains("R Receive"))
    });
    terminal.press_until(b"\r", "Connection type: PPPoE");
    terminal.assert_hidden("synthetic-ppp");
    terminal.assert_hidden("7890123456789012");
    assert!(!terminal.output.windows(b"]52;".len()).any(|w| w == b"]52;"));
    terminal.press_until(b"S", "synthetic-ppp");
    terminal.press_until(b"C", "Copied.");
    assert!(
        terminal
            .output
            .windows(b"]52;c;Nzg5MDEyMzQ1Njc4OTAxMg==\x1b\\".len())
            .any(|w| w == b"]52;c;Nzg5MDEyMzQ1Njc4OTAxMg==\x1b\\")
    );
    terminal.press_until(b"S", "press S to show");
    for _ in 0..20 {
        terminal.send(b"\x1b[6~");
    }
    terminal.expect_screen(|screen| {
        screen.contains("Saved to:") || screen.contains("extracted-credentials")
    });
    terminal.send(b"Q");
    terminal.finish();
}

#[test]
fn corrupt_profile_can_be_deleted_from_error_screen() {
    let mut terminal = Terminal::start(true);
    terminal.press_until(b"\r", "Error code: PR-INVALID");
    let before_resize = terminal.output.len();
    terminal.resize(16, 52);
    terminal.expect_since(before_resize, "PR-INVALID");
    terminal.expect_screen(|screen| {
        screen.contains("PR-INVALID")
            && screen.contains("Delete this router")
            && screen
                .lines()
                .nth(15)
                .is_some_and(|footer| footer.contains("X Delete"))
    });
    terminal.press_until(b"X", "Delete this router?");
    terminal.press_until(b"Y", "Welcome");
    assert!(
        !terminal
            .directory("SYN123456")
            .join("profile.json")
            .exists()
    );
    terminal.send(b"Q");
    terminal.finish();
}

#[test]
fn validation_and_paste_preserve_draft() {
    let mut terminal = Terminal::start(false);
    terminal.press_until(b"N", "Wi-Fi key");
    terminal.press_until(
        b"SYN123456\t01:11:22:33:44:55\tsynthetic-draft-key\r",
        "Check the MAC address",
    );
    terminal.assert_hidden("synthetic-draft-key");
    terminal.press_until(b"\x1b", "Welcome");
    terminal.press_until(b"N", "Wi-Fi key");
    terminal.press_until(b"\x1b[200~02-11-22-33-44-55\x1b[201~", "02:11:22:33:44:55");
    terminal.press_until(b"\t\r", "Router SYN123456");
    assert_eq!(
        terminal.profile("SYN123456")["password"],
        "synthetic-draft-key"
    );
    terminal.send(b"Q");
    terminal.finish();
}

#[test]
fn unrecovered_startup_failure_exits_one_but_retry_can_recover() {
    let mut failed = Terminal::start_with_unusable_root();
    failed.send(b"Q");
    failed.finish_code(1);

    let mut recovered = Terminal::start_with_unusable_root();
    let unusable = recovered.temporary.path().join("unusable");
    fs::remove_file(unusable).unwrap();
    recovered.press_until(b"R", "Welcome");
    recovered.send(b"Q");
    recovered.finish();
}

#[test]
fn resize_below_minimum_and_control_c_restore_terminal() {
    let mut terminal = Terminal::start(false);
    let start = terminal.output.len();
    terminal.resize(8, 24);
    terminal.expect_since(start, "Window too small");
    terminal.send(b"\x03");
    terminal.finish();
}

#[test]
fn sigterm_restores_terminal() {
    let terminal = Terminal::start(false);
    terminal.signal(libc::SIGTERM);
    terminal.finish();
}

#[cfg(feature = "dev-provider-override")]
#[test]
fn worker_panic_shows_internal_error_without_payload() {
    let mut terminal = Terminal::start_with_options(false, ".tmp", true);
    terminal.press_until(b"N", "Wi-Fi key");
    terminal.press_until(
        b"SYN123456\t02:11:22:33:44:55\tsynthetic-hidden-key\r",
        "Router SYN123456",
    );
    terminal.press_until(b"R", "Before you connect");
    terminal.press_until(b"\r", "Error code: INTERNAL");
    assert!(
        !String::from_utf8_lossy(&terminal.output)
            .contains("synthetic-worker-payload-must-never-appear")
    );
    terminal.send(b"Q");
    terminal.finish();
}

#[cfg(feature = "dev-provider-override")]
#[test]
fn ui_panic_restores_terminal_and_reports_only_location() {
    let pty = Pty::new(24, 80);
    let mut child = Command::new(env!("CARGO_BIN_EXE_yettel-cwmp"))
        .env("YETTEL_CWMP_DEV_PANIC", "1")
        .args(["--lang", "en"])
        .stdin(Stdio::from(pty.slave.try_clone().unwrap()))
        .stdout(Stdio::from(pty.slave.try_clone().unwrap()))
        .stderr(Stdio::from(pty.slave.try_clone().unwrap()))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + WAIT;
    let mut output = Vec::new();
    let mut pty = pty;
    loop {
        pty.read_available(&mut output);
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(101));
            break;
        }
        assert!(Instant::now() < deadline, "panic process did not exit");
        thread::sleep(Duration::from_millis(10));
    }
    pty.read_available(&mut output);
    pty.assert_restored(&output);
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("Internal error (INTERNAL). src/ui/mod.rs:"));
    assert!(!text.contains("synthetic-panic-payload-must-never-appear"));
}
