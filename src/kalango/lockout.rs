//! Failed logins counted for the whole share, not per source address: behind the Cloudflare tunnel or a relay
//! every client arrives from the same address, so a per-address budget is shared by everyone or by no one.
//! After `FREE` wrong passwords or codes in a row the share refuses every login for a growing time; a right
//! login ends it. A one-time code is never accepted twice.

use std::{collections::HashMap, sync::Mutex, time::{Duration, Instant}};

const FREE: u32 = 5;
const BASE_SECS: u64 = 30;
const MAX_SECS: u64 = 900;
const CODE_MEMORY: Duration = Duration::from_secs(120);

#[derive(Default)]
pub struct Lockout {
    failures: u32,
    until: Option<Instant>,
    codes: HashMap<String, Instant>,
}

impl Lockout {
    /// Seconds left of the lockout, if any.
    pub fn locked(&self, now: Instant) -> Option<u64> {
        self.until.and_then(|u| u.checked_duration_since(now)).map(|d| d.as_secs() + 1)
    }

    pub fn fail(&mut self, now: Instant) {
        self.failures += 1;
        if self.failures >= FREE {
            let secs = BASE_SECS.saturating_mul(1 << (self.failures - FREE).min(10)).min(MAX_SECS);
            self.until = Some(now + Duration::from_secs(secs));
        }
    }

    pub fn success(&mut self) {
        self.failures = 0;
        self.until = None;
    }

    /// Remembers a one-time code that was right; false if it was already used (replay).
    pub fn use_code(&mut self, code: &str, now: Instant) -> bool {
        self.codes.retain(|_, t| now.duration_since(*t) < CODE_MEMORY);
        self.codes.insert(code.to_owned(), now).is_none()
    }
}

lazy_static::lazy_static! {
    static ref GLOBAL: Mutex<Lockout> = Mutex::new(Lockout::default());
}

pub fn locked() -> Option<u64> {
    GLOBAL.lock().unwrap().locked(Instant::now())
}

pub fn fail() {
    GLOBAL.lock().unwrap().fail(Instant::now());
}

pub fn success() {
    GLOBAL.lock().unwrap().success();
}

pub fn use_code(code: &str) -> bool {
    GLOBAL.lock().unwrap().use_code(code, Instant::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_wrong_logins_lock_everyone_and_the_time_grows() {
        let mut l = Lockout::default();
        let t = Instant::now();
        for _ in 0..4 {
            l.fail(t);
        }
        assert_eq!(l.locked(t), None);
        l.fail(t);
        assert_eq!(l.locked(t), Some(31));
        assert_eq!(l.locked(t + Duration::from_secs(31)), None);
        l.fail(t);
        assert!(l.locked(t).unwrap() > 31);
        for _ in 0..30 {
            l.fail(t);
        }
        assert!(l.locked(t).unwrap() <= MAX_SECS + 1);
        l.success();
        assert_eq!(l.locked(t), None);
    }

    #[test]
    fn a_code_works_once() {
        let mut l = Lockout::default();
        let t = Instant::now();
        assert!(l.use_code("123456", t));
        assert!(!l.use_code("123456", t + Duration::from_secs(30)));
        assert!(l.use_code("654321", t));
        assert!(l.use_code("123456", t + Duration::from_secs(200)));
    }
}
