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

/// One IPv4 address of this computer: address, netmask, `IFF_*` flags and the interface name.
#[derive(Debug, Clone, PartialEq)]
struct IfAddr {
    addr: Ipv4Addr,
    mask: Ipv4Addr,
    flags: u32,
    name: String,
}

/// The broadcast address of a network: the address with the host bits of the mask set.
fn directed_broadcast(addr: Ipv4Addr, mask: Ipv4Addr) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(addr) | !u32::from(mask))
}

/// Where to send the query: this computer and the broadcast address of every network it is on. The limited broadcast
/// (255.255.255.255) leaves only through the interface of the default route, so a computer with a second network
/// (a cable next to Wi-Fi, a bridge) never reached the one the code's owner is on. Interfaces that are down, loopback
/// or without broadcast (tun/wg point-to-point links), /31 and /32 networks (no broadcast address of their own) and
/// the docker bridges (nobody who shares a code is behind them) are skipped.
fn targets_from(ifaces: &[IfAddr]) -> Vec<Ipv4Addr> {
    let mut out = vec![Ipv4Addr::BROADCAST, Ipv4Addr::LOCALHOST];
    for i in ifaces {
        let wanted = (libc::IFF_UP | libc::IFF_BROADCAST) as u32;
        let skipped = (libc::IFF_LOOPBACK | libc::IFF_POINTOPOINT) as u32;
        if i.flags & wanted != wanted || i.flags & skipped != 0 || u32::from(i.mask) >= 0xffff_fffe {
            continue;
        }
        if i.name == "docker0" || i.name.starts_with("br-") || i.name.starts_with("veth") {
            continue;
        }
        let b = directed_broadcast(i.addr, i.mask);
        if !out.contains(&b) {
            out.push(b);
        }
    }
    out
}

fn interfaces() -> Vec<IfAddr> {
    let mut out = Vec::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list` with a linked list that freeifaddrs releases; the entries are only read here.
    unsafe {
        if libc::getifaddrs(&mut list) != 0 {
            return out;
        }
        let mut it = list;
        while !it.is_null() {
            let i = &*it;
            if !i.ifa_addr.is_null() && !i.ifa_netmask.is_null() && (*i.ifa_addr).sa_family as i32 == libc::AF_INET {
                let ip = |s: *const libc::sockaddr| Ipv4Addr::from(u32::from_be((*(s as *const libc::sockaddr_in)).sin_addr.s_addr));
                out.push(IfAddr {
                    addr: ip(i.ifa_addr),
                    mask: ip(i.ifa_netmask),
                    flags: i.ifa_flags,
                    name: std::ffi::CStr::from_ptr(i.ifa_name).to_string_lossy().into_owned(),
                });
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
    let targets = targets_from(&interfaces());
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

    fn nic(name: &str, addr: &str, mask: &str, flags: i32) -> IfAddr {
        IfAddr { addr: addr.parse().unwrap(), mask: mask.parse().unwrap(), flags: flags as u32, name: name.into() }
    }

    #[test]
    fn the_query_goes_to_every_real_network() {
        let up = libc::IFF_UP | libc::IFF_BROADCAST | libc::IFF_RUNNING;
        let t = targets_from(&[
            nic("enp0s1", "10.0.2.15", "255.255.255.0", up),
            // a second network that is not on the default route: the case the limited broadcast never reached
            nic("enp0s5", "192.168.77.1", "255.255.255.0", up),
            nic("lo", "127.0.0.1", "255.0.0.0", libc::IFF_UP | libc::IFF_LOOPBACK),
            // (a point-to-point link may carry the broadcast flag too: it is skipped for being point to point)
            nic("wg0", "10.9.0.2", "255.255.255.0", libc::IFF_UP | libc::IFF_BROADCAST | libc::IFF_POINTOPOINT),
            nic("eth9", "172.16.0.5", "255.255.255.0", libc::IFF_BROADCAST),
            nic("p2p", "10.1.1.1", "255.255.255.254", up),
            nic("host", "10.2.2.2", "255.255.255.255", up),
            nic("docker0", "172.17.0.1", "255.255.0.0", up),
            nic("br-1234", "172.18.0.1", "255.255.0.0", up),
            nic("br0", "192.168.5.2", "255.255.255.0", up),
        ]);
        let ip = |s: &str| s.parse::<Ipv4Addr>().unwrap();
        assert_eq!(t, vec![Ipv4Addr::BROADCAST, Ipv4Addr::LOCALHOST, ip("10.0.2.255"), ip("192.168.77.255"), ip("192.168.5.255")]);
    }

    #[test]
    fn broadcast_address_of_a_network() {
        let b = |a: &str, m: &str| directed_broadcast(a.parse().unwrap(), m.parse().unwrap());
        assert_eq!(b("192.168.77.1", "255.255.255.0"), Ipv4Addr::new(192, 168, 77, 255));
        assert_eq!(b("10.0.2.15", "255.255.0.0"), Ipv4Addr::new(10, 0, 255, 255));
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
