//! A tiny static HTTP server for `seal doc --serve`.
//!
//! The server is intentionally read-only and serves only files inside the
//! documentation directory. It exists so documentation works on machines with
//! no Python, Node, or other helper installed.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Component, Path, PathBuf};

use crate::error::{DocError, DocResult};

/// A running documentation server.
pub struct Served {
    /// The bound address.
    pub address: std::net::SocketAddr,
    /// The directory being served.
    pub directory: PathBuf,
}

/// Starts a blocking static server.
///
/// This function never returns while the listener is healthy. Use a background
/// process when calling it from a test harness.
pub fn serve(directory: &Path, address: &str) -> DocResult<Served> {
    let listener = TcpListener::bind(address)
        .map_err(|error| DocError::Invalid(format!("cannot listen on `{address}`: {error}")))?;
    let address = listener.local_addr().map_err(|error| {
        DocError::Invalid(format!("cannot read the listening address: {error}"))
    })?;
    let root = directory.to_path_buf();
    let served = Served {
        address,
        directory: root.clone(),
    };
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };
        let root = root.clone();
        std::thread::spawn(move || {
            let _ = handle(&root, &mut stream);
        });
    }
    Ok(served)
}

fn handle(root: &Path, stream: &mut std::net::TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    let read = reader.by_ref().take(8192).read_line(&mut request)?;
    if read == 8192 && !request.ends_with('\n') {
        return respond(
            stream,
            414,
            "URI Too Long",
            "text/plain",
            "request line too long",
        );
    }
    if !request.starts_with("GET ") {
        return respond(
            stream,
            405,
            "Method Not Allowed",
            "text/plain",
            "method not allowed",
        );
    }
    let target = request
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/");
    let relative = match safe_path(root, target) {
        Some(path) => path,
        None => return respond(stream, 400, "Bad Request", "text/plain", "invalid path"),
    };
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let path = match relative.canonicalize() {
        Ok(path) => path,
        Err(_) => return respond(stream, 404, "Not Found", "text/plain", "not found"),
    };
    let path = if path.is_dir() {
        path.join("index.html")
    } else {
        path
    };
    let path = match path.canonicalize() {
        Ok(path) if path.starts_with(&root) => path,
        _ => return respond(stream, 404, "Not Found", "text/plain", "not found"),
    };
    match std::fs::read(&path) {
        Ok(bytes) => {
            let content_type = match path.extension().and_then(|value| value.to_str()) {
                Some("html") => "text/html; charset=utf-8",
                Some("json") => "application/json",
                Some("css") => "text/css",
                _ => "application/octet-stream",
            };
            respond_bytes(stream, 200, "OK", content_type, &bytes)
        }
        Err(_) => respond(stream, 404, "Not Found", "text/plain", "not found"),
    }
}

fn safe_path(root: &Path, target: &str) -> Option<PathBuf> {
    let target = target.split('#').next().unwrap_or("/");
    let target = percent_decode(target);
    let relative = Path::new(target.trim_start_matches('/'));
    let mut path = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(path)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(value) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                output.push(value);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn respond(
    stream: &mut std::net::TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    respond_bytes(stream, status, reason, content_type, body.as_bytes())
}

fn respond_bytes(
    stream: &mut std::net::TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_components_are_rejected() {
        let root = Path::new("/doc");
        assert!(safe_path(root, "/../seal.toml").is_none());
        assert_eq!(
            safe_path(root, "/index.html"),
            Some(PathBuf::from("/doc/index.html"))
        );
    }

    #[test]
    fn percent_encoding_is_decoded_without_allowing_traversal() {
        assert_eq!(percent_decode("app%20util.html"), "app util.html");
    }
}
