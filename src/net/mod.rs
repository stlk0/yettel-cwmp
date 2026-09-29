//! Synchronous pinned TLS transport and HTTP exchange.
//! Every connection verifies its pin before application data is sent.
use crate::error::Result;
use std::time::{Duration, Instant};

const INACTIVITY: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);
/// Maximum accepted HTTP response body bytes across all framing modes.
const MAX_BODY: usize = 8 * 1024 * 1024;
mod acs;
mod cancel;
mod digest;
mod http;
mod tls;

pub use acs::AcsTransport;
pub use cancel::Cancel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Distinguishes the opening Inform POST from later session exchanges.
pub enum PostKind {
    /// The first POST carrying a CWMP Inform.
    Inform,
    /// A POST after the opening Inform.
    Session,
}

/// HTTP exchange interface used by the CWMP session and loopback tests.
pub trait Transport {
    /// Send one request body and return its response body.
    fn post(&mut self, body: &[u8], kind: PostKind) -> Result<Vec<u8>>;

    /// Apply an absolute session deadline to future I/O; test transports may ignore it.
    fn set_deadline(&mut self, _deadline: Option<Instant>) {}
}
#[cfg(test)]
mod tests {
    use super::{
        acs::connect_addresses,
        digest::Digest,
        http::{ReadError, read_response},
        *,
    };
    use crate::{
        catalog::{Acs, Pin, PinKind},
        domain::secret::Secret,
        error::Error,
    };
    use std::{
        io,
        net::{SocketAddr, TcpListener},
    };

    fn golden(name: &str, bytes: &[u8]) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/golden")
            .join(name);
        if std::env::var_os("GOLDEN_UPDATE").is_some() {
            std::fs::write(&path, bytes).unwrap();
        }
        assert_eq!(
            bytes,
            std::fs::read(path).unwrap(),
            "snapshot {name} changed"
        );
    }

    #[test]
    fn fixed_digest_and_http_head_snapshots() {
        let mut transport = AcsTransport::new(
            &Acs {
                host: "localhost".into(),
                port: 12345,
                path: "/synthetic?test=1".into(),
            },
            &[Pin::from_hex(PinKind::CertificateSha256, &"0".repeat(64)).unwrap()],
            Secret::new("synthetic-user"),
            Secret::new("synthetic-password"),
            Cancel::default(),
        )
        .unwrap();
        transport
            .digest
            .accept("Digest realm=\"synthetic\",nonce=\"nonce-1\",qop=\"auth\",opaque=\"opaque\"")
            .unwrap();
        transport.digest.cnonce = "00112233445566778899aabbccddeeff".into();
        transport
            .cookies
            .insert("SID".into(), "synthetic-cookie".into());
        golden(
            "digest_qop_nc1.txt",
            transport
                .digest
                .authorization("/synthetic?test=1")
                .unwrap()
                .unwrap()
                .as_bytes(),
        );
        // Reset the challenge to keep the first HTTP request at nc=1.
        transport.digest.challenge.as_mut().unwrap().nc = 0;
        golden(
            "http_opening.txt",
            transport.request_head(17, true).unwrap().as_bytes(),
        );
        let session_head = transport.request_head(0, false).unwrap();
        golden("http_session.txt", session_head.as_bytes());
        let authorization = session_head
            .lines()
            .find_map(|line| line.strip_prefix("Authorization: "))
            .unwrap();
        golden("digest_qop_nc2.txt", authorization.as_bytes());
        transport
            .digest
            .accept("Digest realm=\"synthetic\",nonce=\"legacy\",opaque=\"opaque\"")
            .unwrap();
        transport.digest.cnonce = "00112233445566778899aabbccddeeff".into();
        golden(
            "digest_legacy.txt",
            transport
                .digest
                .authorization("/synthetic?test=1")
                .unwrap()
                .unwrap()
                .as_bytes(),
        );
    }

    #[test]
    fn connect_tries_another_address_after_connection_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let refused = SocketAddr::from(([127, 0, 0, 1], 0));
        let stream = connect_addresses(
            &[refused, address],
            Instant::now() + Duration::from_secs(1),
            &Cancel::default(),
        )
        .unwrap();
        assert_eq!(stream.peer_addr().unwrap(), address);
    }

    #[test]
    fn expired_dns_budget_and_cancellation_prevent_connecting() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        assert!(matches!(
            connect_addresses(&[address], Instant::now(), &Cancel::default()),
            Err(Error::Timeout)
        ));
        let cancel = Cancel::default();
        cancel.cancel();
        assert!(matches!(
            connect_addresses(&[address], Instant::now() + INACTIVITY, &cancel),
            Err(Error::Cancelled)
        ));
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn digest_nonce_reuse_legacy_and_rejections() {
        let mut digest = Digest::new(Secret::new("u"), Secret::new("p"));
        assert!(digest.authorization("/").unwrap().is_none());
        let challenge = "Digest realm=\"r\",nonce=\"n\",qop=\"auth\"";
        digest.accept(challenge).unwrap();
        let first = digest.authorization("/").unwrap().unwrap();
        assert!(first.contains("nc=00000001"));
        digest.accept(challenge).unwrap();
        let second = digest.authorization("/").unwrap().unwrap();
        assert!(second.contains("nc=00000002"));
        let cnonce = |header: &str| {
            header
                .split("cnonce=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap()
                .to_string()
        };
        assert_eq!(cnonce(&first), cnonce(&second));
        digest
            .accept("Digest realm=\"r\",nonce=\"new\",qop=\"auth\"")
            .unwrap();
        let answer = digest.authorization("/").unwrap().unwrap();
        assert!(answer.contains("nc=00000001"));
        assert_ne!(cnonce(&answer), cnonce(&first));
        digest
            .accept("Digest realm=\"r\",nonce=\"legacy\"")
            .unwrap();
        let legacy = digest.authorization("/").unwrap().unwrap();
        assert!(!legacy.contains("qop="));
        assert!(legacy.contains("algorithm=MD5"));
        for bad in [
            "Basic realm=x",
            "Digest nonce=\"n\"",
            "Digest realm=\"r\",nonce=\"\"",
            "Digest realm=\"r\",nonce=\"n\",algorithm=SHA-256",
            "Digest realm=\"r\",nonce=\"n",
            "Digest realm=\"r\",nonce=\"n\",qop=\"auth-int\"",
            "Digest realm=\"é\",nonce=\"n\"",
            "Digest realm=\"r\",nonce=\"n\",   ",
            "Digest realm=\"r\",nonce=\"n\",realm=\"again\"",
            "Digest realm=\"r\",nonce=\"n\\",
            "Digest realm=r,nonce=n x",
            "Digest realm=r,nonce=n\"",
            "Digest realm=r,nonce=n\\",
            "Digest realm=r,nonce=",
        ] {
            assert_eq!(digest.accept(bad), Err(Error::Protocol));
        }
        digest
            .accept("dIgEsT ReAlM=\"r,a\",NONCE=\"n\\\"x\",QOP=\"auth-int,auth\",opaque=\"a\\\\b\",unknown=value")
            .unwrap();
        let escaped = digest.authorization("/").unwrap().unwrap();
        assert!(escaped.contains("realm=\"r,a\""));
        assert!(escaped.contains("nonce=\"n\\\"x\""));
        assert!(escaped.contains("opaque=\"a\\\\b\""));
        assert!(escaped.contains("qop=auth, nc=00000001"));
        digest
            .accept("Digest realm=\"r,a\",nonce=\"n\\\"x\",qop=auth,stale=true")
            .unwrap();
        assert!(
            digest
                .authorization("/")
                .unwrap()
                .unwrap()
                .contains("nc=00000001")
        );
        digest.challenge.as_mut().unwrap().nc = u32::MAX;
        assert_eq!(digest.authorization("/"), Err(Error::Protocol));
    }

    #[test]
    fn malformed_http_and_size_limit() {
        for raw in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 99999999\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: \xff\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nTransfer-Encoding: chunked\r\n\r\n"
                .as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nnothex\r\n".as_slice(),
        ] {
            let mut input = raw;
            assert!(
                matches!(read_response(&mut input), Err(ReadError::Invalid)),
                "{raw:?}"
            );
        }
        let oversized_head = format!(
            "HTTP/1.1 200 OK\r\nX-Long: {}\r\n\r\n",
            "x".repeat(64 * 1024)
        );
        assert!(matches!(
            read_response(&mut oversized_head.as_bytes()),
            Err(ReadError::Invalid)
        ));
        let oversized_chunk_line = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1;{}\r\nx\r\n0\r\n\r\n",
            "x".repeat(8192)
        );
        assert!(matches!(
            read_response(&mut oversized_chunk_line.as_bytes()),
            Err(ReadError::Invalid)
        ));
    }

    #[test]
    fn unknown_non_utf8_header_preserves_next_response() {
        let mut input = &b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nX-Note: \xff\r\n\r\nokHTTP/1.1 204 No Content\r\n\r\n"[..];
        let first = read_response(&mut input).ok().expect("valid response");
        assert_eq!(first.body, b"ok");
        assert!(!first.close);
        let second = read_response(&mut input).ok().expect("next response");
        assert_eq!(second.status, 204);
        assert!(second.body.is_empty());
        assert!(input.is_empty());
    }
}
