//! OpenSSH's files, read as data: `~/.ssh/config` (Host, HostName, User, Port, IdentityFile, ProxyJump)
//! and `known_hosts` (plain and hashed entries), and key fingerprints the way `ssh-keygen -l` shows them.

use base64::{engine::general_purpose::STANDARD, engine::general_purpose::STANDARD_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConfigHost {
    pub alias: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
}

/// The concrete `Host` blocks of an ssh config (patterns with `*` or `?` and `Match` blocks are skipped:
/// they are rules, not servers).
pub fn parse_config(text: &str) -> Vec<ConfigHost> {
    let mut out: Vec<ConfigHost> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (k.to_ascii_lowercase(), v.trim_start_matches(|c: char| c.is_whitespace() || c == '=').trim().trim_matches('"').to_owned()),
            None => continue,
        };
        match key.as_str() {
            "host" => {
                current.clear();
                for alias in value.split_whitespace() {
                    if alias.contains(['*', '?', '!']) {
                        continue;
                    }
                    out.push(ConfigHost { alias: alias.to_owned(), ..Default::default() });
                    current.push(out.len() - 1);
                }
            }
            "match" => current.clear(),
            _ => {
                for &i in &current {
                    let h = &mut out[i];
                    // the first value wins, as in ssh
                    match key.as_str() {
                        "hostname" if h.hostname.is_none() => h.hostname = Some(value.clone()),
                        "user" if h.user.is_none() => h.user = Some(value.clone()),
                        "port" if h.port.is_none() => h.port = value.parse().ok(),
                        "identityfile" if h.identity_file.is_none() => h.identity_file = Some(value.clone()),
                        "proxyjump" if h.proxy_jump.is_none() && value != "none" => h.proxy_jump = Some(value.clone()),
                        _ => {}
                    }
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct KnownHost {
    /// the patterns of the line (`host`, `[host]:port`), or the hashed `|1|salt|hash`
    pub hosts: Vec<String>,
    pub key_type: String,
    /// base64 of the key blob
    pub key: String,
    pub revoked: bool,
}

pub fn parse_known_hosts(text: &str) -> Vec<KnownHost> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                return None;
            }
            let mut parts = l.split_whitespace();
            let mut first = parts.next()?;
            let mut revoked = false;
            if first.starts_with('@') {
                revoked = first == "@revoked";
                if first == "@cert-authority" {
                    return None;
                }
                first = parts.next()?;
            }
            let key_type = parts.next()?.to_owned();
            let key = parts.next()?.to_owned();
            Some(KnownHost { hosts: first.split(',').map(str::to_owned).collect(), key_type, key, revoked })
        })
        .collect()
}

/// How ssh names a host in known_hosts: `host`, or `[host]:port` when the port is not 22.
pub fn host_pattern(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    }
}

fn matches(pattern: &str, name: &str) -> bool {
    if let Some(rest) = pattern.strip_prefix("|1|") {
        let Some((salt, hash)) = rest.split_once('|') else { return false };
        let (Ok(salt), Ok(hash)) = (STANDARD.decode(salt), STANDARD.decode(hash)) else { return false };
        let Ok(mut mac) = Hmac::<sha1::Sha1>::new_from_slice(&salt) else { return false };
        mac.update(name.as_bytes());
        return mac.verify_slice(&hash).is_ok();
    }
    pattern.eq_ignore_ascii_case(name)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Trust {
    /// the same key was trusted before
    Known,
    /// never seen: show the fingerprint and ask
    Unknown,
    /// a different key of the same type (or a revoked one): hard block
    Changed { before: String, before_type: String },
}

/// What the known_hosts say about this host presenting this key.
pub fn check(known: &[KnownHost], host: &str, port: u16, key_type: &str, key: &str) -> Trust {
    let name = host_pattern(host, port);
    let entries: Vec<&KnownHost> = known.iter().filter(|k| k.hosts.iter().any(|p| matches(p, &name))).collect();
    if entries.iter().any(|k| k.revoked && k.key == key) {
        return Trust::Changed { before: String::new(), before_type: key_type.to_owned() };
    }
    if entries.iter().any(|k| !k.revoked && k.key == key) {
        return Trust::Known;
    }
    // a host that had any key before and now shows another one: blocked (ssh does the same)
    match entries.iter().find(|k| !k.revoked) {
        Some(k) => Trust::Changed { before: fingerprint(&k.key).unwrap_or_default(), before_type: k.key_type.clone() },
        None => Trust::Unknown,
    }
}

/// `SHA256:…` of a key blob (base64), as `ssh-keygen -l` prints it.
pub fn fingerprint(key_b64: &str) -> Option<String> {
    let blob = STANDARD.decode(key_b64).ok()?;
    Some(format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(&blob))))
}

/// `ssh-ed25519` → `ED25519`, `ecdsa-sha2-nistp256` → `ECDSA`, `ssh-rsa` → `RSA`, `sk-ssh-ed25519@…` → `ED25519-SK`.
pub fn short_type(key_type: &str) -> String {
    let sk = key_type.starts_with("sk-");
    let t = if key_type.contains("ed25519") {
        "ED25519"
    } else if key_type.contains("ecdsa") {
        "ECDSA"
    } else if key_type.contains("rsa") {
        "RSA"
    } else {
        key_type
    };
    if sk {
        format!("{t}-SK")
    } else {
        t.to_owned()
    }
}

/// A known_hosts line for a host's key.
pub fn known_hosts_line(host: &str, port: u16, key_type: &str, key: &str) -> String {
    format!("{} {key_type} {key}\n", host_pattern(host, port))
}

/// `SHA256:Qx3pV9tL0aB7…` → `Qx3pV9…wK2M` (the short form the lists use).
pub fn short_fingerprint(fp: &str) -> String {
    let body = fp.trim_start_matches("SHA256:");
    if body.chars().count() <= 12 {
        return fp.to_owned();
    }
    let start: String = body.chars().take(6).collect();
    let end: String = body.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("SHA256:{start}…{end}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIFbOhSNRS9yEb2iY9BNWXu6DFxSSbdA8ScpHG8ZAR0wR";

    #[test]
    fn reads_concrete_hosts_of_a_config() {
        let cfg = "# comment\nHost *\n  ServerAliveInterval 30\nHost servidor-casa casa\n  HostName 192.168.0.10\n  User home\nHost app-prod-01\n  HostName=203.0.113.24\n  User deploy\n  Port 22\n  IdentityFile ~/.ssh/deploy_ed25519\n  ProxyJump bastiao-trabalho\n  User other\nMatch host x\n  User nobody\n";
        let hs = parse_config(cfg);
        assert_eq!(hs.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(), vec!["servidor-casa", "casa", "app-prod-01"]);
        assert_eq!(hs[0].hostname.as_deref(), Some("192.168.0.10"));
        assert_eq!(hs[2].user.as_deref(), Some("deploy"), "first value wins");
        assert_eq!(hs[2].proxy_jump.as_deref(), Some("bastiao-trabalho"));
        assert_eq!(hs[2].port, Some(22));
    }

    #[test]
    fn known_hosts_plain_hashed_changed() {
        // a hashed entry for "[bastiao.example.com]:2222", made with HMAC-SHA1 like ssh-keygen -H
        let name = host_pattern("bastiao.example.com", 2222);
        let salt = [7u8; 20];
        let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&salt).unwrap();
        mac.update(name.as_bytes());
        let hashed = format!("|1|{}|{}", STANDARD.encode(salt), STANDARD.encode(mac.finalize().into_bytes()));
        let text = format!("app.example.com,203.0.113.24 ssh-ed25519 {KEY}\n{hashed} ssh-ed25519 {KEY}\n@cert-authority *.example.com ssh-rsa AAAA\n");
        let k = parse_known_hosts(&text);
        assert_eq!(k.len(), 2);
        assert_eq!(check(&k, "203.0.113.24", 22, "ssh-ed25519", KEY), Trust::Known);
        assert_eq!(check(&k, "bastiao.example.com", 2222, "ssh-ed25519", KEY), Trust::Known, "hashed entry");
        assert_eq!(check(&k, "bastiao.example.com", 22, "ssh-ed25519", KEY), Trust::Unknown, "another port is another host");
        assert!(matches!(check(&k, "app.example.com", 22, "ssh-ed25519", "AAAAother"), Trust::Changed { .. }));
        assert_eq!(check(&k, "new.example.com", 22, "ssh-ed25519", KEY), Trust::Unknown);
    }

    #[test]
    fn fingerprints() {
        // what `ssh-keygen -lf` prints for this key
        assert_eq!(fingerprint(KEY).unwrap(), "SHA256:AESLnQEIIicVRmf7e0wo+Mj9CenCeQmt9/MXlBN1ar4");
        assert_eq!(short_type("ecdsa-sha2-nistp256"), "ECDSA");
        assert_eq!(short_type("sk-ssh-ed25519@openssh.com"), "ED25519-SK");
        assert_eq!(short_fingerprint("SHA256:Qx3pV9tL0aB7mR4ewK2M"), "SHA256:Qx3pV9…wK2M");
        assert_eq!(known_hosts_line("h", 2222, "ssh-ed25519", "AAA"), "[h]:2222 ssh-ed25519 AAA\n");
    }
}
