//! A bounded HTTP/1.1 parser, response writer, and optional TLS acceptor.
//!
//! The server implements only what its JSON API needs: one request per
//! connection, `Content-Length` bodies, and no chunked transfer encoding. Every
//! length and path is validated before it can reach the filesystem or allocator.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{ServerError, ServerResult};

const MAX_HEADER: usize = 64 * 1024;
const MAX_REQUEST_LINE: usize = 8 * 1024;

static NONCE: AtomicU64 = AtomicU64::new(0);

/// Returns a process-local nonce used only where no better source exists.
pub fn random_nonce() -> u64 {
    NONCE.fetch_add(1, Ordering::Relaxed)
        ^ std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0)
}

/// One parsed request.
#[derive(Clone, Debug)]
pub struct Request {
    /// The uppercased method.
    pub method: String,
    /// The decoded path without a query string.
    pub path: String,
    /// Lowercased header names and their values.
    pub headers: BTreeMap<String, String>,
    /// The request body.
    pub body: Vec<u8>,
}

impl Request {
    /// Returns one header value.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    /// Returns the bearer token, when present.
    pub fn bearer(&self) -> Option<&str> {
        self.header("authorization")
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

/// One response.
#[derive(Clone, Debug)]
pub struct Response {
    /// The HTTP status.
    pub status: u16,
    /// The content type.
    pub content_type: String,
    /// Extra response headers.
    pub headers: Vec<(String, String)>,
    /// The response body.
    pub body: Vec<u8>,
}

impl Response {
    /// A JSON response.
    pub fn json<T: Serialize>(status: u16, value: &T) -> ServerResult<Self> {
        let body = serde_json::to_vec(value).map_err(|error| {
            ServerError::internal(format!("cannot encode the response: {error}"))
        })?;
        Ok(Self {
            status,
            content_type: "application/json".to_owned(),
            headers: Vec::new(),
            body,
        })
    }

    /// An empty success response.
    pub fn ok() -> Self {
        Self {
            status: 200,
            content_type: "application/json".to_owned(),
            headers: Vec::new(),
            body: b"{\"ok\":true}".to_vec(),
        }
    }

    /// A response carrying raw bytes.
    pub fn bytes(status: u16, content_type: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type: content_type.into(),
            headers: Vec::new(),
            body,
        }
    }

    /// The standard error body.
    pub fn error(error: &ServerError) -> ServerResult<Self> {
        #[derive(Serialize)]
        struct Body<'a> {
            error: Detail<'a>,
        }
        #[derive(Serialize)]
        struct Detail<'a> {
            code: &'a str,
            message: &'a str,
        }
        let mut response = Self::json(
            200,
            &Body {
                error: Detail {
                    code: error.code(),
                    message: error.message(),
                },
            },
        )?;
        response.status = error.status();
        Ok(response)
    }
}

impl From<ServerError> for Response {
    fn from(error: ServerError) -> Self {
        Self::error(&error).unwrap_or_else(|_| Self {
            status: 500,
            content_type: "application/json".to_owned(),
            headers: Vec::new(),
            body: b"{\"error\":{\"code\":\"E8099\",\"message\":\"response encoding failed\"}}"
                .to_vec(),
        })
    }
}

/// Reads one request, or returns `None` when the peer closed the connection.
pub fn read_request(stream: &mut impl Read, max_body: usize) -> ServerResult<Option<Request>> {
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(position) = find(&buffer, b"\r\n\r\n") {
            break position + 4;
        }
        if buffer.len() > MAX_HEADER {
            return Err(ServerError::bad_request(
                "the request headers exceed the 64 KiB limit",
            ));
        }
        let read = stream.read(&mut chunk).map_err(|error| {
            ServerError::bad_request(format!("cannot read the request: {error}"))
        })?;
        if read == 0 {
            if buffer.is_empty() {
                return Ok(None);
            }
            return Err(ServerError::bad_request("the request was truncated"));
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let mut lines = split_lines(&buffer[..header_end - 4]);
    let request_line = lines
        .next()
        .ok_or_else(|| ServerError::bad_request("the request line is missing"))?;
    if request_line.len() > MAX_REQUEST_LINE {
        return Err(ServerError::bad_request("the request line is too long"));
    }
    let mut parts = request_line.split(' ');
    let method = parts
        .next()
        .filter(|method| matches!(*method, "GET" | "POST" | "HEAD"))
        .ok_or_else(|| ServerError::bad_request("unsupported HTTP method"))?;
    let target = parts
        .next()
        .ok_or_else(|| ServerError::bad_request("the request target is missing"))?;
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/1.") || parts.next().is_some() {
        return Err(ServerError::bad_request("invalid HTTP request line"));
    }
    let path = target.split_once('?').map_or(target, |(path, _query)| path);
    if !path.starts_with('/')
        || path.contains(['\\', '\0'])
        || path.split('/').any(|segment| segment == "..")
        || path.len() > 4096
    {
        return Err(ServerError::bad_request("invalid request path"));
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| ServerError::bad_request("malformed request header"))?;
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(ServerError::bad_request("invalid request header name"));
        }
        if headers
            .insert(name.clone(), value.trim().to_owned())
            .is_some()
        {
            return Err(ServerError::bad_request(format!(
                "the header `{name}` appears twice"
            )));
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err(ServerError::bad_request(
            "chunked transfer encoding is not supported",
        ));
    }
    let length = match headers.get("content-length") {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| ServerError::bad_request("invalid Content-Length"))?,
        None => 0,
    };
    if length > max_body {
        return Err(ServerError::new(
            413,
            "E8093",
            format!("the request body is {length} bytes, above the {max_body} byte limit"),
        ));
    }
    let mut body = buffer[header_end..].to_vec();
    if body.len() > length {
        body.truncate(length);
    }
    while body.len() < length {
        let read = stream
            .read(&mut chunk)
            .map_err(|error| ServerError::bad_request(format!("cannot read the body: {error}")))?;
        if read == 0 {
            return Err(ServerError::bad_request("the request body was truncated"));
        }
        body.extend_from_slice(&chunk[..read]);
        if body.len() > length {
            body.truncate(length);
        }
    }
    Ok(Some(Request {
        method: method.to_owned(),
        path: path.to_owned(),
        headers,
        body,
    }))
}

/// Writes one response and closes the connection.
pub fn write_response(stream: &mut impl Write, response: &Response) -> ServerResult<()> {
    let reason = reason(response.status);
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len()
    );
    for (name, value) in &response.headers {
        let safe = !name
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b':')
            && !value.bytes().any(|byte| byte.is_ascii_control());
        if safe {
            head.push_str(name);
            head.push_str(": ");
            head.push_str(value);
            head.push_str("\r\n");
        }
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(&response.body))
        .and_then(|()| stream.flush())
        .map_err(|error| ServerError::internal(format!("cannot write the response: {error}")))
}

/// Loads a PEM certificate chain and PKCS#8 private key for HTTPS.
pub fn load_tls(cert: &Path, key: &Path) -> ServerResult<Arc<rustls::ServerConfig>> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| ServerError::internal("cannot install the TLS crypto provider"))?;
    let cert_bytes = std::fs::read(cert).map_err(|error| {
        ServerError::internal(format!("cannot read `{}`: {error}", cert.display()))
    })?;
    let key_bytes = std::fs::read(key).map_err(|error| {
        ServerError::internal(format!("cannot read `{}`: {error}", key.display()))
    })?;
    let certificates = pem_blocks(&cert_bytes, "CERTIFICATE")
        .into_iter()
        .map(CertificateDer::from)
        .collect::<Vec<_>>();
    if certificates.is_empty() {
        return Err(ServerError::internal(format!(
            "`{}` contains no certificate",
            cert.display()
        )));
    }
    let keys = pem_blocks(&key_bytes, "PRIVATE KEY");
    let key = keys.first().ok_or_else(|| {
        ServerError::internal(format!(
            "`{}` contains no PKCS#8 `PRIVATE KEY` block",
            key.display()
        ))
    })?;
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.clone()));
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certificates, key)
        .map_err(|error| ServerError::internal(format!("invalid TLS configuration: {error}")))?;
    Ok(Arc::new(config))
}

/// Handles one TLS connection with the same request/response functions.
pub fn serve_tls(
    stream: TcpStream,
    acceptor: &rustls::ServerConfig,
    max_body: usize,
    route: impl FnOnce(&Request) -> Response,
) -> ServerResult<()> {
    let connection = rustls::ServerConnection::new(Arc::new(acceptor.clone()))
        .map_err(|error| ServerError::internal(format!("TLS handshake failed: {error}")))?;
    let mut stream = rustls::StreamOwned::new(connection, stream);
    let Some(request) = read_request(&mut stream, max_body)? else {
        return Ok(());
    };
    let response = route(&request);
    write_response(&mut stream, &response)
}

fn pem_blocks(bytes: &[u8], label: &str) -> Vec<Vec<u8>> {
    let text = String::from_utf8_lossy(bytes);
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let mut blocks = Vec::new();
    let mut rest = text.as_ref();
    while let Some(start) = rest.find(&begin) {
        let after = &rest[start + begin.len()..];
        let Some(stop) = after.find(&end) else {
            break;
        };
        let encoded: String = after[..stop]
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        if let Ok(decoded) = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            encoded.as_bytes(),
        ) {
            blocks.push(decoded);
        }
        rest = &after[stop + end.len()..];
    }
    blocks
}

fn split_lines(bytes: &[u8]) -> impl Iterator<Item = &str> {
    bytes
        .split(|byte| *byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .filter_map(|line| std::str::from_utf8(line).ok())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// Returns a stable ETag for bytes.
pub fn etag(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("\"{}\"", &hex::encode(hasher.finalize())[..32])
}

/// Chooses a content type from a file extension.
pub fn content_type(path: &str) -> &'static str {
    if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".zst") {
        "application/zstd"
    } else if path.ends_with(".toml") {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> ServerResult<Option<Request>> {
        read_request(&mut std::io::Cursor::new(text.as_bytes()), 1024)
    }

    #[test]
    fn a_json_request_is_parsed() {
        let request = parse(
            "POST /api/v1/publish?x=1 HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}",
        )
        .unwrap()
        .unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/v1/publish");
        assert_eq!(request.body, b"{}");
    }

    #[test]
    fn traversal_and_bad_lengths_are_rejected() {
        assert!(parse("GET /../../etc/passwd HTTP/1.1\r\n\r\n").is_err());
        assert!(parse("POST / HTTP/1.1\r\nContent-Length: 99999\r\n\r\n").is_err());
        assert!(parse("POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n").is_err());
    }

    #[test]
    fn an_empty_connection_is_not_a_request() {
        assert!(parse("").unwrap().is_none());
    }
}
