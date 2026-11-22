//! What one person passes to the other to connect.
//!
//! - **Link**: `https://fresh-otter-42.trycloudflare.com/#k=<id>.<pk>` — the tunnel's address, and in
//!   the fragment (which browsers and Cloudflare never send anywhere) the sharer's RustDesk id and
//!   ed25519 public key. With them the connecting side pins the identity: the session is encrypted
//!   end to end and refuses any other computer, the tunnel included.
//! - **Code**: `7Q4M-29XR`, the first 40 bits of SHA-256 of the public key in Crockford base32. It is
//!   what people read aloud to check they reached the right computer, and what finds the computer
//!   on the same local network (the core answers the code over UDP broadcast). Across the internet
//!   the code alone cannot be resolved without a server, so there the link is needed.

use sha2::{Digest, Sha256};

const B32: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    /// `name.trycloudflare.com`, `host:port` or `ip:port`.
    pub host: String,
    /// The sharer's identity, when the invite carries it.
    pub identity: Option<(String, [u8; 32])>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Invite(Invite),
    /// A share code (normalized `XXXX-XXXX`), resolved on the local network.
    Code(String),
    /// A RustDesk id (9 or 10 digits): reached through the person's own server (Settings › Network).
    Id(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteError {
    Empty,
    NotALink,
    BadIdentity,
}

/// The share code for a public key: 8 Crockford base32 characters from SHA-256(pk).
pub fn code_for(pk: &[u8; 32]) -> String {
    let h = Sha256::digest(pk);
    let bits = u64::from_be_bytes([0, 0, 0, h[0], h[1], h[2], h[3], h[4]]);
    let mut s = String::with_capacity(9);
    for i in (0..8).rev() {
        s.push(B32[((bits >> (i * 5)) & 31) as usize] as char);
        if i == 4 {
            s.push('-');
        }
    }
    s
}

/// `7q4m 29xr`, `7Q4M29XR`, `7q4m-29xr` → `7Q4M-29XR`; Crockford's look-alikes (O→0, I/L→1) too.
pub fn normalize_code(s: &str) -> Option<String> {
    let mut out = String::new();
    for c in s.chars().filter(|c| !matches!(c, '-' | ' ' | '\u{2010}'..='\u{2015}')) {
        let c = match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !B32.contains(&(c as u8)) || !c.is_ascii() {
            return None;
        }
        out.push(c);
    }
    if out.len() != 8 {
        return None;
    }
    out.insert(4, '-');
    Some(out)
}

pub fn b64url_encode(data: &[u8]) -> String {
    let mut s = String::new();
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (*b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            s.push(B64URL[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    s
}

pub fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let vals: Option<Vec<u32>> = s.bytes().map(|c| B64URL.iter().position(|x| *x == c).map(|p| p as u32)).collect();
    let vals = vals?;
    if vals.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::new();
    for chunk in vals.chunks(4) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, v)| acc | v << (18 - 6 * i));
        for i in 0..chunk.len() - 1 {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

impl Invite {
    pub fn link(&self) -> String {
        let mut s = format!("https://{}/", self.host);
        if let Some((id, pk)) = &self.identity {
            s.push_str(&format!("#k={}.{}", id, b64url_encode(pk)));
        }
        s
    }

    pub fn code(&self) -> Option<String> {
        self.identity.as_ref().map(|(_, pk)| code_for(pk))
    }

    /// Is this a Cloudflare quick tunnel (reached over WSS) rather than a direct `host:port`?
    pub fn is_tunnel(&self) -> bool {
        !self.host.contains(':')
    }

    pub fn parse(input: &str) -> Result<Invite, InviteError> {
        let s = input.trim();
        if s.is_empty() {
            return Err(InviteError::Empty);
        }
        let (rest, fragment) = match s.split_once('#') {
            Some((a, b)) => (a, Some(b)),
            None => (s, None),
        };
        let rest = rest.strip_prefix("https://").or_else(|| rest.strip_prefix("wss://")).unwrap_or(rest);
        let host = rest.split('/').next().unwrap_or("").to_ascii_lowercase();
        let valid_host = !host.is_empty()
            && host.contains(['.', ':'])
            && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
        if !valid_host {
            return Err(InviteError::NotALink);
        }
        let identity = match fragment.and_then(|f| f.strip_prefix("k=")) {
            None => None,
            Some(k) => {
                let (id, pk) = k.split_once('.').ok_or(InviteError::BadIdentity)?;
                let pk = b64url_decode(pk).ok_or(InviteError::BadIdentity)?;
                let pk: [u8; 32] = pk.try_into().map_err(|_| InviteError::BadIdentity)?;
                if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
                    return Err(InviteError::BadIdentity);
                }
                Some((id.to_owned(), pk))
            }
        };
        Ok(Invite { host, identity })
    }
}

impl Target {
    /// What the person typed or pasted in "Paste the code or the link".
    pub fn parse(input: &str) -> Result<Target, InviteError> {
        if let Some(code) = normalize_code(input) {
            return Ok(Target::Code(code));
        }
        let digits: String = input.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
        if (9..=10).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()) {
            return Ok(Target::Id(digits));
        }
        Invite::parse(input).map(Target::Invite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PK: [u8; 32] = [7; 32];

    #[test]
    fn link_round_trip_keeps_identity() {
        let inv = Invite { host: "fresh-otter-42.trycloudflare.com".into(), identity: Some(("123456789".into(), PK)) };
        let link = inv.link();
        assert!(link.starts_with("https://fresh-otter-42.trycloudflare.com/#k=123456789."));
        assert_eq!(Invite::parse(&link).unwrap(), inv);
        assert!(inv.is_tunnel());
        assert_eq!(inv.code(), Some(code_for(&PK)));
    }

    #[test]
    fn bare_hosts_and_lan_addresses() {
        let i = Invite::parse("  Fresh-Otter-42.trycloudflare.com ").unwrap();
        assert_eq!(i.host, "fresh-otter-42.trycloudflare.com");
        assert_eq!(i.identity, None);
        let lan = Invite::parse("192.168.0.14:21118").unwrap();
        assert!(!lan.is_tunnel());
        assert_eq!(Invite::parse(""), Err(InviteError::Empty));
        assert_eq!(Invite::parse("hello"), Err(InviteError::NotALink));
        assert_eq!(Invite::parse("https://a.b/#k=1.@@"), Err(InviteError::BadIdentity));
        assert_eq!(Invite::parse("https://a.b/#k=1.AAAA"), Err(InviteError::BadIdentity), "key must be 32 bytes");
    }

    #[test]
    fn a_rustdesk_id_is_its_own_kind_of_target() {
        assert_eq!(Target::parse("123 456 789").unwrap(), Target::Id("123456789".into()));
        assert_eq!(Target::parse("1234567890").unwrap(), Target::Id("1234567890".into()));
        assert!(matches!(Target::parse("12345678").unwrap(), Target::Code(_)), "8 characters stay a share code");
        assert!(Target::parse("12345678901").is_err());
    }

    #[test]
    fn codes() {
        let c = code_for(&PK);
        assert_eq!(c.len(), 9);
        assert_eq!(&c[4..5], "-");
        assert_eq!(normalize_code(&c.to_lowercase().replace('-', " ")), Some(c.clone()));
        assert_eq!(normalize_code("7q4m-29xr"), Some("7Q4M-29XR".into()));
        assert_eq!(normalize_code("OIL1-2222"), Some("0111-2222".into()), "Crockford look-alikes");
        assert_eq!(normalize_code("7Q4M-29X"), None);
        assert_eq!(normalize_code("7Q4M-29XU"), None, "U is not Crockford base32");
        assert_eq!(Target::parse("7Q4M-29XR"), Ok(Target::Code("7Q4M-29XR".into())));
        assert!(matches!(Target::parse("x.trycloudflare.com"), Ok(Target::Invite(_))));
        assert_ne!(code_for(&[8; 32]), c);
    }

    #[test]
    fn base64url() {
        for n in 0..40u8 {
            let data: Vec<u8> = (0..n).collect();
            assert_eq!(b64url_decode(&b64url_encode(&data)).unwrap(), data);
        }
        assert_eq!(b64url_encode(b"\xfb\xff"), "-_8");
    }
}

#[cfg(test)]
mod real_link {
    #[test]
    fn link_from_a_real_share() {
        let l = "https://127.0.0.1:41555/#k=237070713.yVFQ5Su7BmwP2pFMk9YUefDjmt4q_TardtBixjd382c";
        let i = super::Invite::parse(l).unwrap();
        assert_eq!(i.identity.unwrap().0, "237070713");
    }
}
