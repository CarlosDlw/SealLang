//! DNS TXT ownership proofs.
//!
//! A namespace owner publishes `seal-verify=<token>` at `_seal.<namespace>`.
//! The resolver uses the first nameserver in `/etc/resolv.conf`, validates the
//! response id and bounds, and accepts only TXT records that contain the token.

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use crate::error::{ServerError, ServerResult};

/// Verifies that `_seal.<namespace>` publishes the expected token.
pub fn verify(namespace: &str, token: &str) -> ServerResult<()> {
    if token.len() < 16 || token.len() > 256 || token.chars().any(char::is_control) {
        return Err(ServerError::bad_request(
            "the DNS verification token must be 16 to 256 printable characters",
        ));
    }
    let name = format!("_seal.{namespace}");
    if name.len() > 253
        || name
            .split('.')
            .any(|label| label.is_empty() || label.len() > 63)
    {
        return Err(ServerError::bad_request(
            "the namespace is not a valid DNS name",
        ));
    }
    let servers = nameservers()?;
    let mut last = None;
    for server in servers.iter().take(3) {
        match query(server, &name, token) {
            Ok(true) => return Ok(()),
            Ok(false) => {
                last = Some(ServerError::forbidden(format!(
                    "the TXT record `_seal.{namespace}` does not contain the verification token"
                )))
            }
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| ServerError::internal("no DNS nameserver is configured")))
}

fn nameservers() -> ServerResult<Vec<SocketAddr>> {
    let text = std::fs::read_to_string("/etc/resolv.conf").map_err(|error| {
        ServerError::internal(format!("cannot read `/etc/resolv.conf`: {error}"))
    })?;
    let mut servers = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(value) = line.strip_prefix("nameserver") else {
            continue;
        };
        let value = value.trim();
        if let Ok(address) = value.parse() {
            servers.push(SocketAddr::new(address, 53));
        } else if let Ok(address) = value.parse::<std::net::IpAddr>() {
            servers.push(SocketAddr::new(address, 53));
        }
    }
    if servers.is_empty() {
        return Err(ServerError::internal(
            "`/etc/resolv.conf` has no nameserver",
        ));
    }
    Ok(servers)
}

fn query(server: &SocketAddr, name: &str, token: &str) -> ServerResult<bool> {
    let id = (crate::http::random_nonce() & 0xffff) as u16;
    let mut packet = Vec::with_capacity(256);
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x00]);
    packet.extend_from_slice(&1u16.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());
    for label in name.split('.') {
        if label.len() > 63 {
            return Err(ServerError::bad_request("a DNS label is too long"));
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&16u16.to_be_bytes());
    packet.extend_from_slice(&1u16.to_be_bytes());

    let bind = if server.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind)
        .map_err(|error| ServerError::internal(format!("cannot bind a DNS socket: {error}")))?;
    socket.set_read_timeout(Some(Duration::from_secs(2))).ok();
    socket
        .send_to(&packet, server)
        .map_err(|error| ServerError::internal(format!("cannot send a DNS query: {error}")))?;
    let mut response = [0u8; 4096];
    let (length, _) = socket
        .recv_from(&mut response)
        .map_err(|error| ServerError::internal(format!("cannot read a DNS response: {error}")))?;
    parse(&response[..length], id, name, token)
}

fn parse(response: &[u8], id: u16, name: &str, token: &str) -> ServerResult<bool> {
    if response.len() < 12 {
        return Err(ServerError::internal("a DNS response is truncated"));
    }
    if u16::from_be_bytes([response[0], response[1]]) != id {
        return Err(ServerError::internal(
            "a DNS response has the wrong transaction id",
        ));
    }
    let flags = u16::from_be_bytes([response[2], response[3]]);
    if flags & 0x8000 == 0 {
        return Err(ServerError::internal("a DNS response is not a response"));
    }
    if flags & 0x000f != 0 {
        return Ok(false);
    }
    let questions = u16::from_be_bytes([response[4], response[5]]) as usize;
    let answers = u16::from_be_bytes([response[6], response[7]]) as usize;
    let mut position = 12;
    for _ in 0..questions {
        position = skip_name(response, position)?;
        if position + 4 > response.len() {
            return Err(ServerError::internal("a DNS question is truncated"));
        }
        position += 4;
    }
    for _ in 0..answers {
        position = skip_name(response, position)?;
        if position + 10 > response.len() {
            return Err(ServerError::internal("a DNS answer is truncated"));
        }
        let kind = u16::from_be_bytes([response[position], response[position + 1]]);
        let class = u16::from_be_bytes([response[position + 2], response[position + 3]]);
        let length = u16::from_be_bytes([response[position + 8], response[position + 9]]) as usize;
        position += 10;
        if position + length > response.len() {
            return Err(ServerError::internal("a DNS record is truncated"));
        }
        if kind == 16 && class == 1 {
            let record = &response[position..position + length];
            if let Some(text) = txt(record)
                && (text == token
                    || text
                        .split_whitespace()
                        .any(|part| part == format!("seal-verify={token}")))
            {
                return Ok(true);
            }
        }
        position += length;
    }
    let _ = name;
    Ok(false)
}

fn skip_name(response: &[u8], mut position: usize) -> ServerResult<usize> {
    loop {
        let Some(&length) = response.get(position) else {
            return Err(ServerError::internal("a DNS name is truncated"));
        };
        if length == 0 {
            return Ok(position + 1);
        }
        if length & 0xc0 == 0xc0 {
            if position + 2 > response.len() {
                return Err(ServerError::internal("a DNS pointer is truncated"));
            }
            return Ok(position + 2);
        }
        if length & 0xc0 != 0 {
            return Err(ServerError::internal("a DNS label is too long"));
        }
        position += 1 + length as usize;
        if position > response.len() {
            return Err(ServerError::internal("a DNS name is truncated"));
        }
    }
}

fn txt(record: &[u8]) -> Option<String> {
    let mut position = 0;
    let mut text = String::new();
    while position < record.len() {
        let length = *record.get(position)? as usize;
        position += 1;
        let end = position.checked_add(length)?;
        let part = record.get(position..end)?;
        text.push_str(std::str::from_utf8(part).ok()?);
        position = end;
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_tokens_are_rejected_before_any_query() {
        assert!(verify("example.com", "short").is_err());
    }

    #[test]
    fn txt_strings_are_concatenated() {
        let record = [3u8, b'a', b'b', b'c', 2, b'd', b'e'];
        assert_eq!(txt(&record).as_deref(), Some("abcde"));
    }
}
