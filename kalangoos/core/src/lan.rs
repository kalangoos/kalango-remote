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

/// The broadcast address of a network: the address with the host bits of the mask set.
fn directed_broadcast(addr: Ipv4Addr, mask: Ipv4Addr) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(addr) | !u32::from(mask))
}

/// Where to send the query: this computer and the broadcast address of every network it is on. The limited broadcast
/// (255.255.255.255) leaves only through the interface of the default route, so a computer with a second network (a
/// cable next to Wi-Fi, a VPN, a docker bridge) never reached the one the code's owner is on.
fn broadcast_targets() -> Vec<Ipv4Addr> {
    let mut out = vec![Ipv4Addr::BROADCAST, Ipv4Addr::LOCALHOST];
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list` with a linked list that freeifaddrs releases; the entries are only read here.
    unsafe {
        if libc::getifaddrs(&mut list) != 0 {
            return out;
        }
        let mut it = list;
        while !it.is_null() {
            let i = &*it;
            let up = i.ifa_flags & (libc::IFF_UP | libc::IFF_BROADCAST | libc::IFF_LOOPBACK) as u32
                == (libc::IFF_UP | libc::IFF_BROADCAST) as u32;
            if up && !i.ifa_addr.is_null() && !i.ifa_netmask.is_null() && (*i.ifa_addr).sa_family as i32 == libc::AF_INET {
                let ip = |s: *const libc::sockaddr| Ipv4Addr::from(u32::from_be((*(s as *const libc::sockaddr_in)).sin_addr.s_addr));
                let b = directed_broadcast(ip(i.ifa_addr), ip(i.ifa_netmask));
                if !out.contains(&b) {
                    out.push(b);
                }
            }
            it = i.ifa_next;
        }
        libc::freeifaddrs(list);
    }
    out
}

/// Broadcasts a query for `code` and waits up to `wait` for the answer.
pub fn find(code: &str, wait: Duration) -> std::io::Result<Option<Found>> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    sock.set_broadcast(true)?;
    sock.set_read_timeout(Some(Duration::from_millis(300)))?;
    let q = query_packet(code);
    let targets = broadcast_targets();
    let start = Instant::now();
    let mut buf = [0u8; 256];
    while start.elapsed() < wait {
        for dst in &targets {
            sock.send_to(q.as_bytes(), (*dst, PORT)).ok();
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
    fn broadcast_of_each_network() {
        let b = |a: &str, m: &str| directed_broadcast(a.parse().unwrap(), m.parse().unwrap());
        assert_eq!(b("192.168.77.1", "255.255.255.0"), Ipv4Addr::new(192, 168, 77, 255));
        assert_eq!(b("10.0.2.15", "255.255.0.0"), Ipv4Addr::new(10, 0, 255, 255));
        // every computer asks on the loopback and on the limited broadcast too, and nothing is listed twice
        let t = broadcast_targets();
        assert!(t.contains(&Ipv4Addr::LOCALHOST) && t.contains(&Ipv4Addr::BROADCAST));
        assert_eq!(t.len(), t.iter().collect::<std::collections::HashSet<_>>().len());
    }

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
