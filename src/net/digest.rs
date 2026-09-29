//! MD5 Digest state for the bundled ACS policy (RFC 7616 auth subset).
use crate::{
    domain::secret::Secret,
    error::{Error, Result},
};
use md5::{Digest as _, Md5};
use ring::rand::{SecureRandom as _, SystemRandom};
use std::collections::BTreeMap;
use std::fmt::Write as _;

pub(super) struct Digest {
    username: Secret,
    password: Secret,
    pub(super) challenge: Option<Challenge>,
    pub(super) cnonce: String,
}

pub(super) struct Challenge {
    realm: String,
    nonce: String,
    opaque: Option<String>,
    qop_auth: bool,
    pub(super) nc: u32,
}

fn reject() -> Error {
    Error::Protocol
}

fn token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

fn parameters(input: &str) -> Result<BTreeMap<String, String>> {
    if !input.is_ascii() || input.chars().any(char::is_control) {
        return Err(reject());
    }
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut values = BTreeMap::new();
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index == bytes.len() {
            break;
        }
        let start = index;
        while index < bytes.len() && token_byte(bytes[index]) {
            index += 1;
        }
        if index == start {
            return Err(reject());
        }
        let key = input[start..index].to_ascii_lowercase();
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if bytes.get(index) != Some(&b'=') {
            return Err(reject());
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let value = if bytes.get(index) == Some(&b'"') {
            index += 1;
            let mut out = String::new();
            let mut escaped = false;
            let mut closed = false;
            while index < bytes.len() {
                let ch = input[index..].chars().next().ok_or_else(reject)?;
                index += ch.len_utf8();
                if escaped {
                    out.push(ch);
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    closed = true;
                    break;
                } else {
                    out.push(ch);
                }
            }
            if !closed || escaped {
                return Err(reject());
            }
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            out
        } else {
            let start = index;
            while index < bytes.len() && token_byte(bytes[index]) {
                index += 1;
            }
            if index == start {
                return Err(reject());
            }
            let value = input[start..index].to_string();
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            value
        };
        if values.insert(key, value).is_some() {
            return Err(reject());
        }
        if index < bytes.len() {
            if bytes[index] != b',' {
                return Err(reject());
            }
            index += 1;
            if input[index..].trim().is_empty() {
                return Err(reject());
            }
        }
    }
    Ok(values)
}

impl Challenge {
    fn parse(header: &str) -> Result<(Self, bool)> {
        let Some(scheme) = header.get(..6) else {
            return Err(reject());
        };
        if !scheme.eq_ignore_ascii_case("digest") || header.as_bytes().get(6) != Some(&b' ') {
            return Err(reject());
        }
        let mut values = parameters(&header[7..])?;
        let realm = values.remove("realm").ok_or_else(reject)?;
        let nonce = values.remove("nonce").ok_or_else(reject)?;
        if nonce.is_empty() {
            return Err(reject());
        }
        if values
            .remove("algorithm")
            .is_some_and(|algorithm| !algorithm.eq_ignore_ascii_case("MD5"))
        {
            return Err(reject());
        }
        let qop_auth = match values.remove("qop") {
            Some(qop) => {
                if !qop
                    .split(',')
                    .any(|part| part.trim().eq_ignore_ascii_case("auth"))
                {
                    return Err(reject());
                }
                true
            }
            None => false,
        };
        let stale = values
            .remove("stale")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        Ok((
            Self {
                realm,
                nonce,
                opaque: values.remove("opaque"),
                qop_auth,
                nc: 0,
            },
            stale,
        ))
    }
}

fn md5_hex(value: impl AsRef<[u8]>) -> String {
    format!("{:x}", Md5::digest(value.as_ref()))
}

fn quoted(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '"' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

impl Digest {
    pub(super) fn new(username: Secret, password: Secret) -> Self {
        Self {
            username,
            password,
            challenge: None,
            cnonce: String::new(),
        }
    }

    pub(super) fn accept(&mut self, header: &str) -> Result<()> {
        let (mut challenge, stale) = Challenge::parse(header)?;
        if let Some(old) = self
            .challenge
            .as_ref()
            .filter(|old| !stale && old.nonce == challenge.nonce)
        {
            challenge.nc = old.nc;
        } else {
            let mut random = [0u8; 16];
            SystemRandom::new()
                .fill(&mut random)
                .map_err(|_| Error::Internal)?;
            self.cnonce = String::with_capacity(32);
            for byte in random {
                write!(&mut self.cnonce, "{byte:02x}").map_err(|_| Error::Internal)?;
            }
        }
        self.challenge = Some(challenge);
        Ok(())
    }

    pub(super) fn authorization(&mut self, uri: &str) -> Result<Option<String>> {
        let Some(challenge) = &mut self.challenge else {
            return Ok(None);
        };
        if challenge.nc == u32::MAX
            || self.username.expose().chars().any(char::is_control)
            || uri.chars().any(char::is_control)
        {
            return Err(reject());
        }
        challenge.nc += 1;
        let ha1 = md5_hex(format!(
            "{}:{}:{}",
            self.username.expose(),
            challenge.realm,
            self.password.expose()
        ));
        let ha2 = md5_hex(format!("POST:{uri}"));
        let response = if challenge.qop_auth {
            md5_hex(format!(
                "{ha1}:{}:{:08x}:{}:auth:{ha2}",
                challenge.nonce, challenge.nc, self.cnonce
            ))
        } else {
            md5_hex(format!("{ha1}:{}:{ha2}", challenge.nonce))
        };
        let mut header = format!(
            "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\"",
            quoted(self.username.expose()),
            quoted(&challenge.realm),
            quoted(&challenge.nonce),
            quoted(uri)
        );
        if challenge.qop_auth {
            write!(
                &mut header,
                ", qop=auth, nc={:08x}, cnonce=\"{}\"",
                challenge.nc,
                quoted(&self.cnonce)
            )
            .map_err(|_| Error::Internal)?;
        }
        write!(&mut header, ", response=\"{response}\"").map_err(|_| Error::Internal)?;
        if let Some(opaque) = &challenge.opaque {
            write!(&mut header, ", opaque=\"{}\"", quoted(opaque)).map_err(|_| Error::Internal)?;
        }
        header.push_str(", algorithm=MD5");
        Ok(Some(header))
    }
}
