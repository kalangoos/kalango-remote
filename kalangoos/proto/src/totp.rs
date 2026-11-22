//! Time-based one-time codes (RFC 6238, SHA-1, 6 digits, 30 s) for the two-step verification: the same code an
//! authenticator app shows. The app makes the secret and checks the first code when it is turned on; the core
//! asks the same code of whoever connects.

use hmac::{Hmac, Mac};
use sha1::Sha1;

pub const STEP_SECS: u64 = 30;
pub const DIGITS: u32 = 6;
const SECRET_LEN: usize = 20;
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// A new random secret (160 bits, what authenticator apps expect).
pub fn new_secret() -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut s = vec![0u8; SECRET_LEN];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut s)?;
    Ok(s)
}

pub fn base32(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for &b in data {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

pub fn from_base32(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes().filter(|c| *c != b'=' && !c.is_ascii_whitespace()) {
        let v = ALPHABET.iter().position(|a| *a == c.to_ascii_uppercase())? as u32;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// The `otpauth://` address authenticator apps read from the QR code.
pub fn otpauth_url(secret: &[u8], account: &str, issuer: &str) -> String {
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    format!("otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={STEP_SECS}", enc(issuer), enc(account), base32(secret), enc(issuer))
}

fn code_at_step(secret: &[u8], step: u64, digits: u32) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("HMAC takes keys of any length");
    mac.update(&step.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let o = (h[19] & 15) as usize;
    let n = u32::from_be_bytes([h[o] & 0x7f, h[o + 1], h[o + 2], h[o + 3]]);
    format!("{:0width$}", n % 10u32.pow(digits), width = digits as usize)
}

/// The code at `unix_time`.
pub fn code(secret: &[u8], unix_time: u64) -> String {
    code_at_step(secret, unix_time / STEP_SECS, DIGITS)
}

/// Whether `given` is the code now, or one step before or after (clocks are never exact).
pub fn verify(secret: &[u8], given: &str, unix_time: u64) -> bool {
    let given: String = given.chars().filter(|c| !c.is_whitespace()).collect();
    if given.len() != DIGITS as usize || !given.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let now = unix_time / STEP_SECS;
    (now.saturating_sub(1)..=now + 1).any(|s| code_at_step(secret, s, DIGITS) == given)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_vectors() {
        let k = b"12345678901234567890";
        assert_eq!(code_at_step(k, 59 / 30, 8), "94287082");
        assert_eq!(code_at_step(k, 1111111109 / 30, 8), "07081804");
        assert_eq!(code(k, 59), "287082");
    }

    #[test]
    fn base32_round_trip_and_the_known_text() {
        assert_eq!(base32(b"12345678901234567890"), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(from_base32("gezd gnbv").unwrap(), b"12345");
        assert!(from_base32("0189").is_none());
    }

    #[test]
    fn verify_allows_one_step_of_clock_drift_and_nothing_else() {
        let k = new_secret().unwrap();
        let t = 1_700_000_000;
        assert!(verify(&k, &code(&k, t), t));
        assert!(verify(&k, &code(&k, t - 30), t));
        assert!(verify(&k, &code(&k, t + 30), t));
        assert!(!verify(&k, &code(&k, t + 90), t));
        assert!(!verify(&k, "12345", t));
        assert!(!verify(&k, "abcdef", t));
    }

    #[test]
    fn the_address_carries_the_secret_and_escapes_the_name() {
        let u = otpauth_url(b"12345678901234567890", "Meu PC", "Kalango Remote");
        assert!(u.starts_with("otpauth://totp/Kalango%20Remote:Meu%20PC?secret=GEZDGNBV"));
        assert!(u.contains("&issuer=Kalango%20Remote"));
    }
}
