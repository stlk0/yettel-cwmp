//! Loopback-only fake ACS for development and terminal checks.
use md5::{Digest as _, Md5};
use ring::digest::{SHA256, digest};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
    time::Duration,
};

fn soap(method: &str) -> Vec<u8> {
    format!("<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:c=\"urn:dslforum-org:cwmp-1-0\"><s:Body><c:{method}/></s:Body></s:Envelope>").into_bytes()
}

fn assignments(values: &[(&str, &str)]) -> Vec<u8> {
    let fields = values.iter().map(|(name, value)| format!("<ParameterValueStruct><Name>{name}</Name><Value>{value}</Value></ParameterValueStruct>")).collect::<String>();
    format!("<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:c=\"urn:dslforum-org:cwmp-1-0\"><s:Body><c:SetParameterValues><ParameterList>{fields}</ParameterList><ParameterKey>synthetic-key</ParameterKey></c:SetParameterValues></s:Body></s:Envelope>").into_bytes()
}

fn reply(
    stream: &mut rustls::StreamOwned<rustls::ServerConnection, TcpStream>,
    status: u16,
    headers: &str,
    body: &[u8],
) -> std::io::Result<()> {
    stream.write_all(
        format!(
            "HTTP/1.1 {status} Synthetic\r\nContent-Length: {}\r\n{headers}\r\n",
            body.len()
        )
        .as_bytes(),
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn request(
    reader: &mut BufReader<rustls::StreamOwned<rustls::ServerConnection, TcpStream>>,
) -> std::io::Result<String> {
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
        if head.len() > 65536 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    }
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    if length > 8 * 1024 * 1024 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(head)
}

fn authorization_fields(header: &str) -> Option<BTreeMap<String, String>> {
    if !header.get(..7)?.eq_ignore_ascii_case("Digest ") || header.chars().any(char::is_control) {
        return None;
    }
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for ch in header[7..].chars() {
        if escaped {
            part.push(ch);
            escaped = false;
        } else if ch == '\\' && quoted {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
            part.push(ch);
        } else if ch == ',' && !quoted {
            parts.push(std::mem::take(&mut part));
        } else {
            part.push(ch);
        }
    }
    if quoted || escaped {
        return None;
    }
    parts.push(part);
    let mut fields = BTreeMap::new();
    for part in parts {
        let (key, value) = part.trim().split_once('=')?;
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        let value = if value.starts_with('"') {
            value.strip_prefix('"')?.strip_suffix('"')?
        } else {
            value
        };
        if key.is_empty() || fields.insert(key, value.to_string()).is_some() {
            return None;
        }
    }
    Some(fields)
}

fn authorized(head: &str, authorization: &str, last_nc: u32) -> Option<u32> {
    if !head.starts_with("POST /synthetic HTTP/1.1\r\n") {
        return None;
    }
    let fields = authorization_fields(authorization)?;
    let value = |name| fields.get(name).map(String::as_str);
    if value("realm") != Some("synthetic")
        || value("nonce") != Some("nonce-1")
        || value("uri") != Some("/synthetic")
        || value("qop") != Some("auth")
        || value("algorithm") != Some("MD5")
    {
        return None;
    }
    let nc_text = value("nc")?;
    if nc_text.len() != 8
        || !nc_text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let nc = u32::from_str_radix(nc_text, 16).ok()?;
    if last_nc.checked_add(1) != Some(nc) {
        return None;
    }
    let cnonce = value("cnonce")?;
    if cnonce.is_empty() {
        return None;
    }
    let (username, password) = match value("username")? {
        "SYN123456" => ("SYN123456", "synthetic-wlan"),
        "synthetic-rotated-user" => ("synthetic-rotated-user", "synthetic-rotated-password"),
        _ => return None,
    };
    let ha1 = format!(
        "{:x}",
        Md5::digest(format!("{username}:synthetic:{password}"))
    );
    let ha2 = format!("{:x}", Md5::digest(b"POST:/synthetic"));
    let expected = format!(
        "{:x}",
        Md5::digest(format!("{ha1}:nonce-1:{nc_text}:{cnonce}:auth:{ha2}"))
    );
    (value("response") == Some(expected.as_str())).then_some(nc)
}

fn server_config(scenario: &str) -> std::io::Result<Arc<rustls::ServerConfig>> {
    let mut certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(include_bytes!("../tests/fixtures/localhost-cert.pem"))
            .collect::<Result<_, _>>()
            .map_err(std::io::Error::other)?;
    if scenario == "pin-mismatch" {
        let mut bytes = certs[0].as_ref().to_vec();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        certs[0] = CertificateDer::from(bytes);
    }
    let key = PrivateKeyDer::from_pem_slice(include_bytes!("../tests/fixtures/localhost-key.pem"))
        .map_err(std::io::Error::other)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Ok(Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(std::io::Error::other)?
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(std::io::Error::other)?,
    ))
}

#[allow(clippy::print_stderr)] // Manual QA reports why the loopback peer closed early.
pub(crate) fn run(scenario: &str, listener: &TcpListener) -> std::io::Result<()> {
    let config = server_config(scenario)?;
    let (socket, _) = listener.accept()?;
    socket.set_read_timeout(Some(Duration::from_secs(40)))?;
    let stream = rustls::StreamOwned::new(
        rustls::ServerConnection::new(config).map_err(std::io::Error::other)?,
        socket,
    );
    let mut reader = BufReader::new(stream);
    let challenge =
        "WWW-Authenticate: Digest realm=\"synthetic\",nonce=\"nonce-1\",qop=\"auth\"\r\n";
    let mut last_nc = 0;
    for step in 0..4 {
        let head = loop {
            let head = match request(&mut reader) {
                Ok(h) => h,
                Err(e) if step > 0 => {
                    eprintln!("Fake ACS ended: {e}");
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            let authorization = head
                .lines()
                .find_map(|line| line.strip_prefix("Authorization: "));
            if scenario == "auth-rejected" {
                reply(reader.get_mut(), 401, challenge, b"")?;
                if authorization.is_some() {
                    return Ok(());
                }
                continue;
            }
            let Some(authorization) = authorization else {
                reply(reader.get_mut(), 401, challenge, b"")?;
                continue;
            };
            let Some(nc) = authorized(&head, authorization, last_nc) else {
                reply(reader.get_mut(), 401, challenge, b"")?;
                return Ok(());
            };
            last_nc = nc;
            break head;
        };
        if (scenario == "slow" && step == 1) || (scenario == "rotate-then-slow" && step == 2) {
            thread::sleep(Duration::from_secs(20));
        }
        if scenario == "http-500" {
            reply(reader.get_mut(), 500, "", b"")?;
            break;
        }
        if scenario == "malformed" {
            reply(reader.get_mut(), 200, "", b"invalid SOAP")?;
            break;
        }
        let body = match step {
            0 => soap("InformResponse"),
            1 => assignments(&[
                (
                    "InternetGatewayDevice.ManagementServer.Username",
                    "synthetic-rotated-user",
                ),
                (
                    "InternetGatewayDevice.ManagementServer.Password",
                    "synthetic-rotated-password",
                ),
            ]),
            2 => {
                if scenario == "incomplete" {
                    assignments(&[(
                        "InternetGatewayDevice.WANDevice.1.WANConnectionDevice.1.WANPPPConnection.1.Username",
                        "synthetic-ppp-user",
                    )])
                } else {
                    assignments(&[
                        (
                            "InternetGatewayDevice.WANDevice.1.WANConnectionDevice.1.WANPPPConnection.1.Username",
                            "synthetic-ppp-user",
                        ),
                        (
                            "InternetGatewayDevice.WANDevice.1.WANConnectionDevice.1.WANPPPConnection.1.Password",
                            "synthetic-ppp-password",
                        ),
                    ])
                }
            }
            _ => vec![],
        };
        reply(reader.get_mut(), 200, "", &body)?;
        if step >= 3 {
            break;
        }
        let _ = head;
    }
    Ok(())
}

#[allow(clippy::print_stdout)] // Manual QA prints the local URL and pin for the override file.
fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut scenario = "success".to_string();
    let mut output = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scenario" => scenario = args.next().ok_or(std::io::ErrorKind::InvalidInput)?,
            "--write-provider" => {
                output = Some(args.next().ok_or(std::io::ErrorKind::InvalidInput)?)
            }
            _ => return Err(std::io::ErrorKind::InvalidInput.into()),
        }
    }
    if ![
        "success",
        "auth-rejected",
        "slow",
        "rotate-then-slow",
        "http-500",
        "incomplete",
        "malformed",
        "pin-mismatch",
    ]
    .contains(&scenario.as_str())
    {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let cert =
        CertificateDer::from_pem_slice(include_bytes!("../tests/fixtures/localhost-cert.pem"))
            .map_err(std::io::Error::other)?;
    let pin = digest(&SHA256, cert.as_ref())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let provider = serde_json::json!({"acs":{"host":"localhost","port":port,"path":"/synthetic"}, "pins":[{"kind":"certificate_sha256","sha256":pin}]});
    let json = serde_json::to_string_pretty(&provider)?;
    if let Some(path) = output {
        fs::write(path, &json)?;
    }
    println!("Fake ACS listening on 127.0.0.1:{port}\n{json}");
    std::io::stdout().flush()?;
    run(&scenario, &listener)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(username: &str, password: &str) -> String {
        let ha1 = format!(
            "{:x}",
            Md5::digest(format!("{username}:synthetic:{password}"))
        );
        let ha2 = format!("{:x}", Md5::digest(b"POST:/synthetic"));
        let response = format!(
            "{:x}",
            Md5::digest(format!("{ha1}:nonce-1:00000001:abcdef:auth:{ha2}"))
        );
        format!(
            "Digest username=\"{username}\", realm=\"synthetic\", nonce=\"nonce-1\", uri=\"/synthetic\", qop=auth, nc=00000001, cnonce=\"abcdef\", response=\"{response}\", algorithm=MD5"
        )
    }

    #[test]
    fn fake_acs_checks_context_count_and_rotated_credentials() {
        let head = "POST /synthetic HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let initial = header("SYN123456", "synthetic-wlan");
        assert_eq!(authorized(head, &initial, 0), Some(1));
        assert_eq!(authorized(head, &initial, 1), None);
        for (old, new) in [
            ("nonce=\"nonce-1\"", "nonce=\"wrong\""),
            ("realm=\"synthetic\"", "realm=\"wrong\""),
            ("uri=\"/synthetic\"", "uri=\"/wrong\""),
            ("username=\"SYN123456\"", "username=\"other\""),
        ] {
            assert_eq!(authorized(head, &initial.replace(old, new), 0), None);
        }
        let rotated = header("synthetic-rotated-user", "synthetic-rotated-password");
        assert_eq!(authorized(head, &rotated, 0), Some(1));
        assert_eq!(
            authorized(head, &header("synthetic-rotated-user", "wrong"), 0),
            None
        );
    }
}
