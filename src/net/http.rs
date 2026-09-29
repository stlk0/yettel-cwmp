//! Bounded HTTP/1.1 response parsing and cookie handling.
//! Header, body, and chunk-line limits are security boundaries.
use super::MAX_BODY;
use std::io::{self, BufRead, Read};

pub(super) struct Response {
    pub(super) status: u16,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) close: bool,
}
pub(super) fn cookie_pair(header: &str) -> Option<(String, String)> {
    if header.chars().any(char::is_control) {
        return None;
    }
    let mut quoted = false;
    let mut escaped = false;
    let mut end = header.len();
    for (i, c) in header.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && quoted {
            escaped = true;
            continue;
        }
        if c == '"' {
            quoted = !quoted;
        }
        if c == ';' && !quoted {
            end = i;
            break;
        }
    }
    if quoted || escaped {
        return None;
    }
    let pair = &header[..end];
    let (name, value) = pair.split_once('=')?;
    let name = name.trim();
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
    {
        return None;
    }
    Some((name.into(), value.trim().into()))
}
pub(super) enum ReadError {
    Io(io::Error),
    Invalid,
}
impl From<io::Error> for ReadError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
fn line<R: BufRead>(reader: &mut R, limit: usize) -> Result<Vec<u8>, ReadError> {
    let mut value = vec![];
    reader
        .take((limit + 1) as u64)
        .read_until(b'\n', &mut value)?;
    if value.is_empty() {
        // A closed peer is a transport failure, not malformed HTTP syntax.
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
    }
    if value.len() > limit || !value.ends_with(b"\r\n") {
        return Err(ReadError::Invalid);
    }
    Ok(value)
}
/// Maximum total response header bytes accepted before parsing.
const MAX_HEAD: usize = 64 * 1024;
/// Maximum bytes in each chunk-size or trailer line.
const MAX_LINE: usize = 8 * 1024;
/// Maximum aggregate trailer bytes after a zero-sized chunk.
const MAX_TRAILERS: usize = 64 * 1024;

struct Head {
    status: u16,
    headers: Vec<(String, String)>,
    close: bool,
}

fn read_head<R: BufRead>(reader: &mut R) -> Result<Head, ReadError> {
    let mut head = vec![];
    loop {
        let chunk = line(reader, MAX_HEAD - head.len())?;
        let done = chunk == b"\r\n";
        head.extend(chunk);
        if done {
            break;
        }
        if head.len() >= MAX_HEAD {
            return Err(ReadError::Invalid);
        }
    }
    let mut raw_headers = [httparse::EMPTY_HEADER; 128];
    let mut parsed = httparse::Response::new(&mut raw_headers);
    if !parsed
        .parse(&head)
        .map_err(|_| ReadError::Invalid)?
        .is_complete()
    {
        return Err(ReadError::Invalid);
    }
    let status = parsed.code.ok_or(ReadError::Invalid)?;
    let version = parsed.version;
    let headers: Vec<_> = parsed
        .headers
        .iter()
        // Unknown fields can contain non-UTF-8 bytes; only decode fields we use.
        .filter(|h| {
            [
                "connection",
                "content-length",
                "transfer-encoding",
                "set-cookie",
                "www-authenticate",
            ]
            .iter()
            .any(|name| h.name.eq_ignore_ascii_case(name))
        })
        .map(|h| {
            Ok((
                h.name.to_string(),
                std::str::from_utf8(h.value)
                    .map_err(|_| ReadError::Invalid)?
                    .to_string(),
            ))
        })
        .collect::<Result<_, ReadError>>()?;
    let close = version == Some(0)
        || headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("connection"))
            .any(|(_, value)| {
                value
                    .split(',')
                    .any(|part| part.trim().eq_ignore_ascii_case("close"))
            });
    Ok(Head {
        status,
        headers,
        close,
    })
}

fn read_chunked_body<R: BufRead>(reader: &mut R) -> Result<Vec<u8>, ReadError> {
    let mut body = vec![];
    loop {
        let size_line = line(reader, MAX_LINE)?;
        let size = match httparse::parse_chunk_size(&size_line).map_err(|_| ReadError::Invalid)? {
            httparse::Status::Complete((_, size)) => {
                usize::try_from(size).map_err(|_| ReadError::Invalid)?
            }
            _ => return Err(ReadError::Invalid),
        };
        if size == 0 {
            let mut trailer_bytes = 0;
            loop {
                let trailer = line(reader, MAX_LINE)?;
                trailer_bytes += trailer.len();
                if trailer_bytes > MAX_TRAILERS {
                    return Err(ReadError::Invalid);
                }
                if trailer == b"\r\n" {
                    return Ok(body);
                }
            }
        }
        if size > MAX_BODY - body.len() {
            return Err(ReadError::Invalid);
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..])?;
        if line(reader, 2)? != b"\r\n" {
            return Err(ReadError::Invalid);
        }
    }
}

fn read_fixed_body<R: BufRead>(reader: &mut R, length: usize) -> Result<Vec<u8>, ReadError> {
    if length > MAX_BODY {
        return Err(ReadError::Invalid);
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(body)
}

fn read_body_until_close<R: BufRead>(reader: &mut R) -> Result<Vec<u8>, ReadError> {
    let mut body = vec![];
    reader.take(MAX_BODY as u64 + 1).read_to_end(&mut body)?;
    if body.len() > MAX_BODY {
        return Err(ReadError::Invalid);
    }
    Ok(body)
}

pub(super) fn read_response<R: BufRead>(reader: &mut R) -> Result<Response, ReadError> {
    for _ in 0..8 {
        let head = read_head(reader)?;
        if head.status < 200 {
            if head.status == 101 {
                return Err(ReadError::Invalid);
            }
            continue;
        }
        let mut close = head.close;
        let body = if head.status == 204 || head.status == 304 {
            vec![]
        } else {
            let transfer: Vec<_> = head
                .headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("transfer-encoding"))
                .map(|(_, value)| value.as_str())
                .collect();
            let lengths: Vec<_> = head
                .headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.as_str())
                .collect();
            if !transfer.is_empty() {
                if transfer.len() != 1
                    || !transfer[0].eq_ignore_ascii_case("chunked")
                    || !lengths.is_empty()
                {
                    return Err(ReadError::Invalid);
                }
                read_chunked_body(reader)?
            } else if !lengths.is_empty() {
                if lengths.len() != 1 {
                    return Err(ReadError::Invalid);
                }
                let length = lengths[0]
                    .parse::<usize>()
                    .map_err(|_| ReadError::Invalid)?;
                read_fixed_body(reader, length)?
            } else {
                close = true;
                read_body_until_close(reader)?
            }
        };
        return Ok(Response {
            status: head.status,
            headers: head.headers,
            body,
            close,
        });
    }
    Err(ReadError::Invalid)
}
