//! Finding a sharing computer on the same local network by its code, without any server: the
//! connecting side broadcasts `KR1?<code>` on UDP port 21120; the computer whose key gives that
//! code answers `KR1!<code> <port> <id> <pk>`. The answer carries the identity, so the session is
//! pinned (encrypted end to end) exactly as with a link. Nothing leaves the local network.

use std::{
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

use kalango_remote_proto::invite::{b64url_decode, b64url_encode};

pub const PORT: u16 = 21120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub addr: SocketAddr,
    pub id: String,
    pub pk: [u8; 32],
}

pub fn query_packet(code: &str) -> String {
    format!("KR1?{code}")
}

pub fn answer_packet(code: &str, port: u16, id: &str, pk: &[u8; 32]) -> String {
    format!("KR1!{code} {port} {id} {}", b64url_encode(pk))
}

/// The code a query asks for.
pub fn parse_query(p: &str) -> Option<&str> {
    p.strip_prefix("KR1?").filter(|c| c.len() == 9)
}

/// An answer for `code`, from `from` (the answer's own IP is not trusted: only its port is used).
pub fn parse_answer(p: &str, code: &str, from: SocketAddr) -> Option<Found> {
    let rest = p.strip_prefix("KR1!")?;
    let mut it = rest.split(' ');
    if it.next()? != code {
        return None;
    }
    let port: u16 = it.next()?.parse().ok()?;
    let id = it.next()?.to_owned();
    let pk: [u8; 32] = b64url_decode(it.next()?)?.try_into().ok()?;
    // the key must really give this code, or anyone on the network could answer for it
    if kalango_remote_proto::code_for(&pk) != code {
        return None;
    }
    Some(Found { addr: SocketAddr::new(from.ip(), port), id, pk })
}

/// Answers queries for `code` until the process ends.
pub fn respond(code: String, port: u16, id: String, pk: [u8; 32]) {
    std::thread::spawn(move || {
        let Ok(sock) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PORT)) else {
            hbb_common::log::warn!("kalango lan: port {PORT} busy, the code will not be found on this network");
            return;
        };
        let answer = answer_packet(&code, port, &id, &pk);
        let mut buf = [0u8; 128];
        while let Ok((n, from)) = sock.recv_from(&mut buf) {
            if std::str::from_utf8(&buf[..n]).ok().and_then(parse_query) == Some(code.as_str()) {
                sock.send_to(answer.as_bytes(), from).ok();
            }
        }
    });
}

/// Broadcasts a query for `code` and waits up to `wait` for the answer.
pub fn find(code: &str, wait: Duration) -> std::io::Result<Option<Found>> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    sock.set_broadcast(true)?;
    sock.set_read_timeout(Some(Duration::from_millis(300)))?;
    let q = query_packet(code);
    let start = Instant::now();
    let mut buf = [0u8; 256];
    while start.elapsed() < wait {
        for dst in [Ipv4Addr::BROADCAST, Ipv4Addr::LOCALHOST] {
            sock.send_to(q.as_bytes(), (dst, PORT)).ok();
        }
        while let Ok((n, from)) = sock.recv_from(&mut buf) {
            if let Some(f) = std::str::from_utf8(&buf[..n]).ok().and_then(|p| parse_answer(p, code, from)) {
                return Ok(Some(f));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_checked_against_the_code() {
        let pk = [3u8; 32];
        let code = kalango_remote_proto::code_for(&pk);
        let from: SocketAddr = "192.168.0.14:5555".parse().unwrap();
        let a = answer_packet(&code, 21118, "123456789", &pk);
        assert_eq!(
            parse_answer(&a, &code, from),
            Some(Found { addr: "192.168.0.14:21118".parse().unwrap(), id: "123456789".into(), pk })
        );
        // someone else's key cannot answer for this code
        let forged = answer_packet(&code, 21118, "123456789", &[4u8; 32]);
        assert_eq!(parse_answer(&forged, &code, from), None);
        assert_eq!(parse_query(&query_packet(&code)), Some(code.as_str()));
        assert_eq!(parse_query("KR1?short"), None);
    }
}
