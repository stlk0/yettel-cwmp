//! Loopback TLS, pinning, HTTP, Digest, and session-deadline regressions.
// Integration-test assertions intentionally fail immediately when a fixture or socket setup fails.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
use common::*;
use md5::{Digest as _, Md5};
use ring::digest::{SHA256, digest};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use yettel_cwmp::{
    capture,
    catalog::{Acs, Pin, PinKind},
    domain::secret::Secret,
    error::Error,
    net::{AcsTransport, Cancel, PostKind, Transport},
    store::Store,
};
#[derive(Clone, Copy, Default)]
enum CertChoice {
    #[default]
    Original,
    Reissued,
    Other,
    Damaged,
}
fn certificates(choice: CertChoice) -> Vec<CertificateDer<'static>> {
    let pem: &[u8] = match choice {
        CertChoice::Original | CertChoice::Damaged => include_bytes!("fixtures/localhost-cert.pem"),
        CertChoice::Reissued => include_bytes!("fixtures/localhost-cert-reissued.pem"),
        CertChoice::Other => include_bytes!("fixtures/other-cert.pem"),
    };
    let mut certs = CertificateDer::pem_slice_iter(pem)
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    if matches!(choice, CertChoice::Damaged) {
        let mut cert = certs[0].as_ref().to_vec();
        let last = cert.len() - 1;
        cert[last] ^= 1;
        certs[0] = CertificateDer::from(cert);
    }
    certs
}
fn pin() -> Pin {
    Pin {
        kind: PinKind::CertificateSha256,
        sha256: digest(&SHA256, certificates(CertChoice::Original)[0].as_ref())
            .as_ref()
            .try_into()
            .unwrap(),
    }
}
fn spki_pin() -> Pin {
    let cert = certificates(CertChoice::Original).remove(0);
    let parsed = webpki::EndEntityCert::try_from(&cert).unwrap();
    Pin {
        kind: PinKind::SpkiSha256,
        sha256: digest(&SHA256, parsed.subject_public_key_info().as_ref())
            .as_ref()
            .try_into()
            .unwrap(),
    }
}
fn config(choice: CertChoice) -> Arc<rustls::ServerConfig> {
    let key_pem: &[u8] = if matches!(choice, CertChoice::Other) {
        include_bytes!("fixtures/other-key.pem")
    } else {
        include_bytes!("fixtures/localhost-key.pem")
    };
    let key = PrivateKeyDer::from_pem_slice(key_pem).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certificates(choice), key)
            .unwrap(),
    )
}
struct Request {
    head: String,
    body: Vec<u8>,
    sni: Option<String>,
}
fn read_request(
    reader: &mut BufReader<rustls::StreamOwned<rustls::ServerConnection, TcpStream>>,
) -> std::io::Result<Request> {
    let mut head = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        head.push_str(&line);
        if line == "\r\n" {
            break;
        }
    }
    let length = head
        .lines()
        .find_map(|l| l.strip_prefix("Content-Length: "))
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Request {
        head,
        body,
        sni: reader.get_ref().conn.server_name().map(String::from),
    })
}
fn response(status: u16, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut raw = format!(
        "HTTP/1.1 {status} Synthetic\r\nContent-Length: {}\r\n{headers}\r\n",
        body.len()
    )
    .into_bytes();
    raw.extend(body);
    raw
}
#[derive(Default)]
struct Plan {
    cert: CertChoice,
    responses: Vec<Vec<u8>>,
    stall: bool,
    drip: bool,
    clean_close: bool,
    closed: Option<std::sync::mpsc::Sender<()>>,
}
impl Plan {
    fn responses(responses: Vec<Vec<u8>>) -> Self {
        Self {
            responses,
            ..Self::default()
        }
    }
}
struct Server {
    acs: Acs,
    requests: Arc<Mutex<Vec<Request>>>,
    handle: thread::JoinHandle<()>,
}
impl Server {
    fn finish(self) -> Vec<Request> {
        self.handle.join().unwrap();
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}
fn serve(plans: Vec<Plan>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(vec![]));
    let captured = requests.clone();
    let handle = thread::spawn(move || {
        for plan in plans {
            let deadline = Instant::now() + Duration::from_secs(5);
            let stream = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "synthetic connection deadline");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("synthetic listener: {e:?}"),
                }
            };
            // Darwin can inherit O_NONBLOCK from the polling listener. Restore
            // blocking I/O for the synchronous rustls request reader.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let conn = rustls::ServerConnection::new(config(plan.cert)).unwrap();
            let mut reader = BufReader::new(rustls::StreamOwned::new(conn, stream));
            if plan.stall {
                let _ = read_request(&mut reader).map(|r| captured.lock().unwrap().push(r));
                thread::sleep(Duration::from_millis(500));
                continue;
            }
            for response in plan.responses {
                let request = match read_request(&mut reader) {
                    Ok(r) => r,
                    Err(_) => break,
                };
                captured.lock().unwrap().push(request);
                let sent = if plan.drip {
                    response.iter().try_for_each(|byte| {
                        reader.get_mut().write_all(&[*byte])?;
                        reader.get_mut().flush()?;
                        thread::sleep(Duration::from_millis(35));
                        Ok::<(), std::io::Error>(())
                    })
                } else {
                    reader
                        .get_mut()
                        .write_all(&response)
                        .and_then(|_| reader.get_mut().flush())
                };
                if sent.is_err() {
                    break;
                }
            }
            if plan.clean_close {
                reader.get_mut().conn.send_close_notify();
                let _ = reader.get_mut().flush();
            }
            drop(reader);
            if let Some(closed) = plan.closed {
                closed.send(()).unwrap();
            }
        }
    });
    Server {
        acs: Acs {
            host: "localhost".into(),
            port,
            path: "/synthetic?test=1".into(),
        },
        requests,
        handle,
    }
}
fn transport(server: &Server, cancel: Cancel) -> AcsTransport {
    AcsTransport::new(
        &server.acs,
        &[pin()],
        Secret::new("synthetic-user"),
        Secret::new("synthetic-password"),
        cancel,
    )
    .unwrap()
}
#[test]
fn digest_cookies_headers_reuse_and_chunk_trailers() {
    let challenge = "WWW-Authenticate: Digest realm=\"synthetic\", nonce=\"nonce-1\", qop=\"auth,auth-int\", opaque=\"a,b\"\r\nSet-Cookie: SID=\"a;b\"; Path=/\r\n";
    let chunked=b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2;ext=x\r\nok\r\n0\r\nX-Test: synthetic\r\n\r\n".to_vec();
    let server = serve(vec![Plan::responses(vec![
        response(401, challenge, b"challenge body"),
        chunked,
        response(204, "", b""),
    ])]);
    let mut client = transport(&server, Cancel::default());
    assert_eq!(
        client.post(b"synthetic-inform", PostKind::Inform).unwrap(),
        b"ok"
    );
    assert!(client.post(b"", PostKind::Session).unwrap().is_empty());
    drop(client);
    let requests = server.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].sni.as_deref(), Some("localhost"));
    assert_eq!(requests[0].body, b"synthetic-inform");
    let first = &requests[0].head;
    assert!(first.starts_with("POST /synthetic?test=1 HTTP/1.1\r\nHost: localhost:"));
    assert!(first.contains("Content-Type: text/xml; charset=\"utf-8\"\r\nKeep-Alive: \r\nConnection: TE, Keep-Alive\r\nTE: trailers\r\n"));
    assert!(!first.contains("Authorization:"));
    assert!(!first.contains("Accept-Encoding"));
    assert!(requests[1].head.contains("Cookie: SID=\"a;b\""));
    assert!(requests[1].head.contains("nc=00000001"));
    assert!(requests[1].head.contains("opaque=\"a,b\""));
    assert!(requests[2].head.contains("nc=00000002"));
    assert!(requests[2].head.contains("Connection: TE\r\n"));
    assert!(!requests[2].head.contains("Keep-Alive:"));
    // Independently recompute the response with a known context using the parsed cnonce.
    let auth = requests[1]
        .head
        .lines()
        .find_map(|l| l.strip_prefix("Authorization: "))
        .unwrap();
    let field = |name: &str| {
        auth.split(&format!("{name}=\""))
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string()
    };
    let cnonce = field("cnonce");
    let ha1 = format!(
        "{:x}",
        Md5::digest(b"synthetic-user:synthetic:synthetic-password")
    );
    let ha2 = format!("{:x}", Md5::digest(b"POST:/synthetic?test=1"));
    let expected = format!(
        "{:x}",
        Md5::digest(format!("{ha1}:nonce-1:00000001:{cnonce}:auth:{ha2}"))
    );
    assert_eq!(field("response"), expected);
}
#[test]
fn wrong_pin_sends_no_http_and_reconnect_checks_again() {
    let server = serve(vec![Plan::responses(vec![response(200, "", b"unused")])]);
    let mut client = AcsTransport::new(
        &server.acs,
        &[Pin::from_hex(PinKind::CertificateSha256, &"0".repeat(64)).unwrap()],
        Secret::new("u"),
        Secret::new("p"),
        Cancel::default(),
    )
    .unwrap();
    assert_eq!(
        client.post(b"must-not-leak", PostKind::Inform),
        Err(Error::PinMismatch)
    );
    drop(client);
    assert!(server.finish().is_empty());
    let server = serve(vec![
        Plan::responses(vec![response(200, "Connection: close\r\n", b"ok")]),
        Plan {
            cert: CertChoice::Damaged,
            ..Plan::responses(vec![response(200, "", b"unused")])
        },
    ]);
    let mut client = transport(&server, Cancel::default());
    assert_eq!(client.post(b"first", PostKind::Inform).unwrap(), b"ok");
    assert_eq!(
        client.post(b"never-sent", PostKind::Session),
        Err(Error::PinMismatch)
    );
    drop(client);
    assert_eq!(server.finish().len(), 1);
}
#[test]
fn spki_pin_accepts_reissued_leaf_and_rejects_other_key_before_http() {
    let server = serve(vec![Plan {
        cert: CertChoice::Reissued,
        ..Plan::responses(vec![response(200, "", b"same key")])
    }]);
    let mut client = AcsTransport::new(
        &server.acs,
        &[spki_pin()],
        Secret::new("u"),
        Secret::new("p"),
        Cancel::default(),
    )
    .unwrap();
    assert_eq!(
        client.post(b"hello", PostKind::Inform).unwrap(),
        b"same key"
    );
    drop(client);
    assert_eq!(server.finish().len(), 1);

    let server = serve(vec![Plan {
        cert: CertChoice::Reissued,
        ..Plan::responses(vec![response(200, "", b"any pin")])
    }]);
    let mut client = AcsTransport::new(
        &server.acs,
        &[pin(), spki_pin()],
        Secret::new("u"),
        Secret::new("p"),
        Cancel::default(),
    )
    .unwrap();
    assert_eq!(client.post(b"hello", PostKind::Inform).unwrap(), b"any pin");
    drop(client);
    assert_eq!(server.finish().len(), 1);

    let server = serve(vec![Plan {
        cert: CertChoice::Other,
        ..Plan::responses(vec![response(200, "", b"must not arrive")])
    }]);
    let mut client = AcsTransport::new(
        &server.acs,
        &[spki_pin()],
        Secret::new("u"),
        Secret::new("p"),
        Cancel::default(),
    )
    .unwrap();
    assert_eq!(
        client.post(b"never sent", PostKind::Inform),
        Err(Error::PinMismatch)
    );
    drop(client);
    assert!(server.finish().is_empty());
}
#[test]
fn no_redirect_or_extra_digest_retry() {
    let challenge = "WWW-Authenticate: Digest realm=\"r\",nonce=\"n\",qop=\"auth\"\r\n";
    let server = serve(vec![Plan::responses(vec![
        response(401, challenge, b""),
        response(401, challenge, b""),
    ])]);
    let mut client = transport(&server, Cancel::default());
    assert_eq!(
        client.post(b"bootstrap", PostKind::Inform),
        Err(Error::AuthRejected)
    );
    assert_eq!(server.finish().len(), 2);
    let server = serve(vec![Plan::responses(vec![response(
        302,
        "Location: https://must-not-contact.invalid/\r\n",
        b"",
    )])]);
    let mut client = transport(&server, Cancel::default());
    assert_eq!(
        client.post(b"", PostKind::Inform),
        Err(Error::HttpStatus(302))
    );
    assert_eq!(server.finish().len(), 1);
}
#[test]
fn timeout_and_prompt_cancellation() {
    let server = serve(vec![Plan {
        stall: true,
        ..Plan::default()
    }]);
    let mut client = transport(&server, Cancel::default()).with_timeout(Duration::from_millis(150));
    assert_eq!(client.post(b"x", PostKind::Inform), Err(Error::Timeout));
    server.finish();
    let server = serve(vec![Plan {
        stall: true,
        ..Plan::default()
    }]);
    let cancel = Cancel::default();
    let token = cancel.clone();
    let requests = server.requests.clone();
    let trigger = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        while requests.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        let requested_at = Instant::now();
        token.cancel();
        requested_at
    });
    let mut client = transport(&server, cancel);
    let result = client.post(b"x", PostKind::Inform);
    let completed_at = Instant::now();
    assert_eq!(result, Err(Error::Cancelled));
    let requested_at = trigger.join().unwrap();
    assert!(completed_at.duration_since(requested_at) < Duration::from_millis(450));
    server.finish();
}
#[test]
fn absolute_session_deadline_stops_a_dripping_response() {
    let server = serve(vec![Plan {
        drip: true,
        ..Plan::responses(vec![response(200, "", b"slow synthetic response")])
    }]);
    let mut client = transport(&server, Cancel::default());
    client.set_deadline(Some(Instant::now() + Duration::from_millis(450)));
    let start = Instant::now();
    assert_eq!(client.post(b"x", PostKind::Inform), Err(Error::Deadline));
    assert!(start.elapsed() < Duration::from_secs(2));
    drop(client);
    assert_eq!(server.finish().len(), 1);
}
#[test]
fn full_cwmp_session_over_loopback_tls() {
    let username = format!("{}Username", ppp_prefix());
    let password = format!("{}Password", ppp_prefix());
    let server = serve(vec![Plan::responses(vec![
        response(200, "", &inform_response()),
        response(
            200,
            "",
            &spv(&[
                (acs_password(), "rotated"),
                (&username, "synthetic-ppp"),
                (&password, "synthetic-ppp-secret"),
            ]),
        ),
        response(204, "", b""),
    ])]);
    let root = tempfile::tempdir().unwrap();
    let mut d = device();
    d.provider.acs = server.acs.clone();
    d.provider.pins = vec![pin()];
    let p = profile();
    let store = Store::open(root.path()).unwrap();
    store.save(&p).unwrap();
    let result = capture::capture(&store, &p.serial, &d, &Cancel::default(), |_| {}).unwrap();
    assert_eq!(result.export.internet.username.expose(), "synthetic-ppp");
    assert!(result.path.exists());
    assert_eq!(store.load(&p.serial).unwrap().password.expose(), "rotated");
    let requests = server.finish();
    assert!(String::from_utf8_lossy(&requests[2].body).contains("SetParameterValuesResponse"));
}
#[test]
fn malformed_http_returns_http_error() {
    let server = serve(vec![Plan::responses(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec(),
    ])]);
    let mut client = transport(&server, Cancel::default());
    assert_eq!(client.post(b"", PostKind::Inform), Err(Error::Protocol));
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn matching_pin_does_not_bypass_handshake_signature_validation() {
    use rustls::sign::{CertifiedKey, Signer, SigningKey, SingleCertAndKey};
    #[derive(Debug)]
    struct BadKey(Arc<dyn SigningKey>);
    #[derive(Debug)]
    struct BadSignature(Box<dyn Signer>);
    impl SigningKey for BadKey {
        fn choose_scheme(&self, offered: &[rustls::SignatureScheme]) -> Option<Box<dyn Signer>> {
            self.0
                .choose_scheme(offered)
                .map(|s| Box::new(BadSignature(s)) as Box<dyn Signer>)
        }
        fn algorithm(&self) -> rustls::SignatureAlgorithm {
            self.0.algorithm()
        }
    }
    impl Signer for BadSignature {
        fn sign(&self, message: &[u8]) -> std::result::Result<Vec<u8>, rustls::Error> {
            let mut signature = self.0.sign(message)?;
            signature[0] ^= 1;
            Ok(signature)
        }
        fn scheme(&self) -> rustls::SignatureScheme {
            self.0.scheme()
        }
    }
    let private =
        PrivateKeyDer::from_pem_slice(include_bytes!("fixtures/localhost-key.pem")).unwrap();
    let signing = rustls::crypto::ring::default_provider()
        .key_provider
        .load_private_key(private)
        .unwrap();
    let certified = CertifiedKey::new(
        certificates(CertChoice::Original),
        Arc::new(BadKey(signing)),
    );
    let mut config = (*config(CertChoice::Original)).clone();
    config.cert_resolver = Arc::new(SingleCertAndKey::from(certified));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let connection = rustls::ServerConnection::new(Arc::new(config)).unwrap();
        let mut reader = BufReader::new(rustls::StreamOwned::new(connection, stream));
        assert!(read_request(&mut reader).is_err());
    });
    let mut client = AcsTransport::new(
        &Acs {
            host: "localhost".into(),
            port,
            path: "/".into(),
        },
        &[pin()],
        Secret::new("u"),
        Secret::new("p"),
        Cancel::default(),
    )
    .unwrap();
    assert_eq!(
        client.post(b"must-not-send", PostKind::Inform),
        Err(Error::Tls)
    );
    worker.join().unwrap();
}

#[test]
fn idle_peer_close_reconnects_before_next_request_without_replaying() {
    for clean_close in [false, true] {
        let (closed, wait) = std::sync::mpsc::channel();
        let server = serve(vec![
            Plan {
                clean_close,
                closed: Some(closed),
                ..Plan::responses(vec![response(200, "", b"first-response")])
            },
            Plan::responses(vec![response(200, "", b"second-response")]),
        ]);
        let mut client = transport(&server, Cancel::default());
        assert_eq!(
            client.post(b"first", PostKind::Inform).unwrap(),
            b"first-response"
        );
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(
            client.post(b"second", PostKind::Session).unwrap(),
            b"second-response"
        );
        drop(client);
        let requests = server.finish();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].body, b"first");
        assert_eq!(requests[1].body, b"second");
    }
}

#[test]
fn eof_after_request_is_reset_and_never_replayed() {
    let server = serve(vec![Plan {
        clean_close: true,
        ..Plan::responses(vec![vec![]])
    }]);
    let mut client = transport(&server, Cancel::default());
    assert_eq!(
        client.post(b"accepted-request", PostKind::Session),
        Err(Error::Network)
    );
    drop(client);
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body, b"accepted-request");
}

#[test]
fn idle_close_reconnect_revalidates_pin_before_sending() {
    let (closed, wait) = std::sync::mpsc::channel();
    let server = serve(vec![
        Plan {
            closed: Some(closed),
            ..Plan::responses(vec![response(200, "", b"ok")])
        },
        Plan {
            cert: CertChoice::Other,
            ..Plan::responses(vec![response(200, "", b"unused")])
        },
    ]);
    let mut client = transport(&server, Cancel::default());
    assert_eq!(client.post(b"first", PostKind::Inform).unwrap(), b"ok");
    wait.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(
        client.post(b"must-not-send", PostKind::Session),
        Err(Error::PinMismatch)
    );
    drop(client);
    assert_eq!(server.finish().len(), 1);
}
