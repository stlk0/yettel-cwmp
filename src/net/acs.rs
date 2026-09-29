//! Connection setup, retry policy, and authenticated ACS posts.
use super::{
    Cancel, INACTIVITY, POLL, PostKind, Transport,
    digest::Digest,
    http::{ReadError, Response, cookie_pair, read_response},
    tls::PinVerifier,
};
use crate::catalog::Acs;
use crate::{
    domain::secret::Secret,
    error::{Error, Result},
};
use rustls::pki_types::ServerName;
use std::fmt::Write as _;
use std::{
    collections::BTreeMap,
    io::{self, BufReader, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

type Connection = BufReader<rustls::StreamOwned<rustls::ClientConnection, Socket>>;

// Detect an idle close before queuing HTTP bytes. A close racing with the
// subsequent write/read remains an error: replaying a CWMP POST is unsafe when
// the peer might already have accepted it (RFC 9110 section 9.2.2).
fn idle_connection_closed(connection: &mut Connection) -> io::Result<bool> {
    if !connection.buffer().is_empty() {
        return Ok(false);
    }
    let stream = connection.get_mut();
    let pending = stream
        .conn
        .process_new_packets()
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    if pending.plaintext_bytes_to_read() != 0 {
        return Ok(false);
    }
    if pending.peer_has_closed() {
        return Ok(true);
    }
    stream.sock.stream.set_nonblocking(true)?;
    let checked = read_pending_records(stream);
    // Restore blocking mode on both success and error before any later I/O.
    stream.sock.stream.set_nonblocking(false)?;
    checked
}

// Process pending TLS records, including close_notify and session tickets,
// without waiting for an idle peer or changing socket cancellation policy.
fn read_pending_records(
    stream: &mut rustls::StreamOwned<rustls::ClientConnection, Socket>,
) -> io::Result<bool> {
    for _ in 0..8 {
        match stream.conn.read_tls(&mut stream.sock.stream) {
            Ok(0) => return Ok(true),
            Ok(_) => {
                let state = stream
                    .conn
                    .process_new_packets()
                    .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
                if state.plaintext_bytes_to_read() != 0 {
                    return Ok(false);
                }
                if state.peer_has_closed() {
                    return Ok(true);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::UnexpectedEof
                ) =>
            {
                return Ok(true);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

fn handshake_error(
    error: &io::Error,
    pin_mismatch: bool,
    cancelled: bool,
    deadline_expired: bool,
) -> Error {
    if pin_mismatch {
        Error::PinMismatch
    } else if cancelled {
        Error::Cancelled
    } else if deadline_expired {
        Error::Deadline
    } else if matches!(
        error.kind(),
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput
    ) {
        // rustls reports a malformed handshake or invalid signature as protocol data.
        Error::Tls
    } else {
        Error::network_io(error)
    }
}

struct Socket {
    stream: TcpStream,
    cancel: Cancel,
    timeout: Duration,
    deadline: Option<Instant>,
}
impl Socket {
    fn operation<T>(
        &mut self,
        mut op: impl FnMut(&mut TcpStream) -> io::Result<T>,
    ) -> io::Result<T> {
        let inactivity_deadline = Instant::now() + self.timeout;
        loop {
            if self.cancel.is_cancelled() {
                return Err(io::ErrorKind::ConnectionAborted.into());
            }
            if self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
                || Instant::now() >= inactivity_deadline
            {
                return Err(io::ErrorKind::TimedOut.into());
            }
            match op(&mut self.stream) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    if Instant::now() >= inactivity_deadline {
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                }
                result => {
                    if self
                        .deadline
                        .is_some_and(|deadline| Instant::now() >= deadline)
                    {
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                    return result;
                }
            }
        }
    }
}
impl Drop for Socket {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}
impl Read for Socket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.operation(|s| s.read(buf))
    }
}
impl Write for Socket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.operation(|s| s.write(buf))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.operation(Write::flush)
    }
}
pub(super) fn connect_addresses(
    addresses: &[SocketAddr],
    deadline: Instant,
    cancel: &Cancel,
) -> Result<TcpStream> {
    let mut error = Error::Network;
    for (index, address) in addresses.iter().enumerate() {
        cancel.check()?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(Error::Timeout)?;
        // Leave time for other DNS answers if this address silently drops packets.
        let remaining_addresses = u32::try_from(addresses.len() - index).unwrap_or(u32::MAX);
        let timeout = (remaining / remaining_addresses).max(Duration::from_nanos(1));
        match TcpStream::connect_timeout(address, timeout) {
            Ok(stream) => {
                cancel.check()?;
                return Ok(stream);
            }
            Err(e) => error = Error::network_io(&e),
        }
    }
    cancel.check()?;
    Err(error)
}
/// Resolve the endpoint and connect to its first reachable address before `deadline`.
fn resolve_and_connect(
    host: &str,
    port: u16,
    deadline: Instant,
    cancel: &Cancel,
) -> Result<TcpStream> {
    let addresses = resolved_addresses((host, port).to_socket_addrs())?;
    connect_addresses(&addresses, deadline, cancel)
}

fn resolved_addresses(
    result: io::Result<impl IntoIterator<Item = SocketAddr>>,
) -> Result<Vec<SocketAddr>> {
    // DNS lookup has a distinct recovery action even though the resolver uses io::Error.
    result
        .map(|addresses| addresses.into_iter().collect())
        .map_err(|_| Error::Dns)
}

/// Adapt the library to the device's MD5/auth-only policy and nonce reuse.
pub struct AcsTransport {
    host: String,
    port: u16,
    target: String,
    host_header: String,
    config: Arc<rustls::ClientConfig>,
    mismatch: Arc<AtomicBool>,
    connection: Option<Connection>,
    pub(super) digest: Digest,
    pub(super) cookies: BTreeMap<String, String>,
    cancel: Cancel,
    timeout: Duration,
    deadline: Option<Instant>,
}
impl AcsTransport {
    /// Create a pinned HTTPS transport for the provider endpoint and credentials.
    pub fn new(
        acs: &Acs,
        pins: &[crate::catalog::Pin],
        username: Secret,
        password: Secret,
        cancel: Cancel,
    ) -> Result<Self> {
        // The validated catalog always supplies a pin; an empty set is a build defect.
        if pins.is_empty() {
            return Err(Error::Internal);
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mismatch = Arc::new(AtomicBool::new(false));
        let verifier = Arc::new(PinVerifier {
            pins: pins.to_vec(),
            mismatch: mismatch.clone(),
            provider: provider.clone(),
        });
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|_| Error::Tls)?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        // Disable resumption: every connection must present and verify the leaf certificate.
        let mut config = config;
        config.resumption = rustls::client::Resumption::disabled();
        let host = acs.host.clone();
        let port = acs.port;
        let host_header = if port == 443 {
            host.clone()
        } else {
            format!("{host}:{port}")
        };
        let target = acs.path.clone();
        Ok(Self {
            host,
            port,
            target,
            host_header,
            config: Arc::new(config),
            mismatch,
            connection: None,
            digest: Digest::new(username, password),
            cookies: BTreeMap::new(),
            cancel,
            timeout: INACTIVITY,
            deadline: None,
        })
    }
    /// Set the inactivity timeout used by each socket operation.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    fn io_error(&self, error: &io::Error) -> Error {
        if self.cancel.is_cancelled() {
            Error::Cancelled
        } else if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            Error::Deadline
        } else {
            Error::network_io(error)
        }
    }
    fn check_deadline(&self) -> Result<()> {
        self.cancel.check()?;
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(Error::Deadline);
        }
        Ok(())
    }
    fn connect(&mut self) -> Result<()> {
        self.check_deadline()?;
        let (tx, rx) = mpsc::sync_channel(1);
        let host = self.host.clone();
        let port = self.port;
        let inactivity_deadline = Instant::now() + self.timeout;
        let deadline = self.deadline.map_or(inactivity_deadline, |absolute| {
            absolute.min(inactivity_deadline)
        });
        let cancel = self.cancel.clone();
        // DNS can block inside the OS. Only this secret-free connector may outlive cancellation.
        std::thread::spawn(move || {
            let _ = tx.send(resolve_and_connect(&host, port, deadline, &cancel));
        });
        let stream = loop {
            self.check_deadline()?;
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| self.io_error(&io::Error::from(io::ErrorKind::TimedOut)))?;
            match rx.recv_timeout(POLL.min(remaining)) {
                Ok(Ok(stream)) => break stream,
                Ok(Err(error)) => {
                    self.check_deadline()?;
                    return Err(error);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Err(Error::Network),
            }
        };
        // A CWMP exchange waits for each response; delaying a trailing TLS
        // fragment for TCP coalescing only adds latency to the whole session.
        stream
            .set_nodelay(true)
            .map_err(|e| Error::network_io(&e))?;
        stream
            .set_read_timeout(Some(POLL.min(self.timeout)))
            .map_err(|e| Error::network_io(&e))?;
        stream
            .set_write_timeout(Some(POLL.min(self.timeout)))
            .map_err(|e| Error::network_io(&e))?;
        let socket = Socket {
            stream,
            cancel: self.cancel.clone(),
            timeout: self.timeout,
            deadline: self.deadline,
        };
        let name = ServerName::try_from(self.host.clone()).map_err(|_| Error::Network)?;
        let conn =
            rustls::ClientConnection::new(self.config.clone(), name).map_err(|_| Error::Tls)?;
        self.mismatch.store(false, Ordering::SeqCst);
        let mut stream = rustls::StreamOwned::new(conn, socket);
        // Complete pin + signature validation before queuing ANY HTTP application bytes.
        while stream.conn.is_handshaking() {
            if let Err(e) = stream.conn.complete_io(&mut stream.sock) {
                return Err(handshake_error(
                    &e,
                    self.mismatch.load(Ordering::SeqCst),
                    self.cancel.is_cancelled(),
                    self.deadline
                        .is_some_and(|deadline| Instant::now() >= deadline),
                ));
            }
        }
        self.check_deadline()?;
        self.connection = Some(BufReader::new(stream));
        Ok(())
    }
    pub(super) fn request_head(&mut self, body_len: usize, opening: bool) -> Result<String> {
        let mut head = String::new();
        write!(
            &mut head,
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\n",
            self.target, self.host_header
        )
        .map_err(|_| Error::Internal)?;
        // The empty Keep-Alive value, Connection: TE, Keep-Alive, and TE: trailers
        // reproduce the ZTE H3600P wire shape for ACS compatibility. Change them
        // only together with the byte-for-byte HTTP golden fixtures.
        if opening {
            head.push_str("Keep-Alive: \r\n");
        }
        write!(
            &mut head,
            "Connection: {}\r\nTE: trailers\r\nContent-Length: {}\r\n",
            if opening { "TE, Keep-Alive" } else { "TE" },
            body_len
        )
        .map_err(|_| Error::Internal)?;
        if let Some(auth) = self.digest.authorization(&self.target)? {
            write!(&mut head, "Authorization: {auth}\r\n").map_err(|_| Error::Internal)?;
        }
        if !self.cookies.is_empty() {
            write!(
                &mut head,
                "Cookie: {}\r\n",
                self.cookies
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
            .map_err(|_| Error::Internal)?;
        }
        head.push_str("\r\n");
        Ok(head)
    }
    fn exchange(&mut self, body: &[u8], opening: bool) -> Result<Response> {
        self.check_deadline()?;
        if let Some(connection) = &mut self.connection {
            match idle_connection_closed(connection) {
                Ok(true) => self.connection = None,
                Ok(false) => {}
                Err(error) => return Err(self.io_error(&error)),
            }
        }
        if self.connection.is_none() {
            self.connect()?;
        }
        let head = self.request_head(body.len(), opening)?;
        // Send headers and body together so the peer need not wait for a second
        // small TLS write while acknowledging the first TCP segment.
        let mut request = head.into_bytes();
        request.extend_from_slice(body);
        let conn = self.connection.as_mut().ok_or(Error::Network)?;
        if let Err(e) = conn
            .get_mut()
            .write_all(&request)
            .and_then(|_| conn.get_mut().flush())
        {
            return Err(self.io_error(&e));
        }
        let result = read_response(conn);
        self.accept_response(result)
    }

    fn accept_response(
        &self,
        result: std::result::Result<Response, ReadError>,
    ) -> Result<Response> {
        // Accept complete replies even if cancellation arrives concurrently.
        // Socket waits and the next exchange still check cancellation.
        result.map_err(|e| match e {
            ReadError::Io(e) => self.io_error(&e),
            ReadError::Invalid => Error::Protocol,
        })
    }
    /// Send one request, answering a single authentication challenge.
    fn authenticated_post(&mut self, body: &[u8], opening: bool) -> Result<Vec<u8>> {
        for attempt in 0..2 {
            let response = self.exchange(body, opening)?;
            for (_, value) in response
                .headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
            {
                if let Some((name, value)) = cookie_pair(value) {
                    self.cookies.insert(name, value);
                }
            }
            if response.close {
                self.connection = None;
            }
            if response.status == 401 && attempt == 0 {
                let challenge = response
                    .headers
                    .iter()
                    .filter(|(n, _)| n.eq_ignore_ascii_case("www-authenticate"))
                    .map(|(_, v)| v)
                    .find(|v| v.to_ascii_lowercase().starts_with("digest "))
                    .ok_or(Error::Protocol)?;
                self.digest.accept(challenge)?;
                continue;
            }
            if response.status == 401 {
                return Err(Error::AuthRejected);
            }
            if !(200..300).contains(&response.status) {
                return Err(Error::HttpStatus(response.status));
            }
            return Ok(response.body);
        }
        Err(Error::Protocol)
    }
}
impl Transport for AcsTransport {
    fn post(&mut self, body: &[u8], kind: PostKind) -> Result<Vec<u8>> {
        let result = self.authenticated_post(body, matches!(kind, PostKind::Inform));
        if result.is_err() {
            self.connection = None;
        }
        result
    }

    fn set_deadline(&mut self, deadline: Option<Instant>) {
        self.deadline = deadline;
        if let Some(connection) = &mut self.connection {
            connection.get_mut().sock.deadline = deadline;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_failure_keeps_pin_cancel_and_io_causes_distinct() {
        let reset = io::Error::from(io::ErrorKind::ConnectionReset);
        assert_eq!(
            handshake_error(&reset, true, true, true),
            Error::PinMismatch
        );
        assert_eq!(handshake_error(&reset, false, true, true), Error::Cancelled);
        assert_eq!(handshake_error(&reset, false, false, true), Error::Deadline);
        for kind in [
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::UnexpectedEof,
        ] {
            assert_eq!(
                handshake_error(&io::Error::from(kind), false, false, false),
                Error::Network
            );
        }
        assert_eq!(
            handshake_error(
                &io::Error::from(io::ErrorKind::TimedOut),
                false,
                false,
                false
            ),
            Error::Timeout
        );
        assert_eq!(
            handshake_error(
                &io::Error::from(io::ErrorKind::InvalidData),
                false,
                false,
                false
            ),
            Error::Tls
        );
    }

    #[test]
    fn resolver_failure_is_dns_without_a_real_lookup() {
        let fail: io::Result<Vec<SocketAddr>> = Err(io::ErrorKind::NotFound.into());
        assert!(matches!(resolved_addresses(fail), Err(Error::Dns)));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod completion_tests {
    use super::*;
    use std::io::BufRead;

    struct CancelOnLastByte<'a> {
        bytes: &'a [u8],
        cancel: Cancel,
    }

    impl Read for CancelOnLastByte<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            let n = out.len().min(self.bytes.len());
            out[..n].copy_from_slice(&self.bytes[..n]);
            self.consume(n);
            Ok(n)
        }
    }

    impl BufRead for CancelOnLastByte<'_> {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            Ok(self.bytes)
        }

        fn consume(&mut self, n: usize) {
            self.bytes = &self.bytes[n..];
            if self.bytes.is_empty() {
                self.cancel.cancel();
            }
        }
    }

    #[test]
    fn completed_http_response_wins_over_cancellation_on_its_last_byte() {
        let cancel = Cancel::default();
        let client = AcsTransport::new(
            &Acs {
                host: "localhost".into(),
                port: 12345,
                path: "/synthetic".into(),
            },
            &[crate::catalog::Pin {
                kind: crate::catalog::PinKind::CertificateSha256,
                sha256: [0; 32],
            }],
            Secret::new("synthetic-user"),
            Secret::new("synthetic-password"),
            cancel.clone(),
        )
        .unwrap();
        let mut reader = CancelOnLastByte {
            bytes: b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
            cancel: cancel.clone(),
        };
        let parsed = read_response(&mut reader);
        assert!(cancel.is_cancelled());
        assert_eq!(client.accept_response(parsed).unwrap().body, b"ok");
        assert!(matches!(
            client.accept_response(Err(ReadError::Io(io::ErrorKind::ConnectionAborted.into()))),
            Err(Error::Cancelled)
        ));
    }
}
