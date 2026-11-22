//! (shared by file: `src/kalango/lockout.rs` includes this file, so the rules are tested in this crate)
//! Failed logins counted for the whole share, not per source address: behind the Cloudflare tunnel or a relay
//! every client arrives from the same address, so a per-address budget is shared by everyone or by no one.
//! After `FREE` wrong passwords or codes in a row the share refuses every login for a growing time; a right
//! login ends it. A one-time code is never accepted twice.

use std::{collections::HashMap, time::{Duration, Instant}};

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

    /// Counts an attempt before it is judged, in one step with the check: `Some(secs)` when the share is
    /// locked (nothing is counted), `None` when the attempt may go on (and is counted). A burst of parallel
    /// guesses therefore cannot get more than `FREE` tries in. A right login calls `success` (or `release`
    /// when more steps, such as the two-step code, are still to come).
    pub fn reserve(&mut self, now: Instant) -> Option<u64> {
        if let Some(s) = self.locked(now) {
            return Some(s);
        }
        self.failures += 1;
        if self.failures >= FREE {
            let secs = BASE_SECS.saturating_mul(1 << (self.failures - FREE).min(10)).min(MAX_SECS);
            self.until = Some(now + Duration::from_secs(secs));
        }
        None
    }

    /// Gives back the slot of an attempt that turned out right but is not finished.
    pub fn release(&mut self) {
        self.failures = self.failures.saturating_sub(1);
        if self.failures < FREE {
            self.until = None;
        }
    }

    pub fn failures(&self) -> u32 {
        self.failures
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_wrong_logins_lock_everyone_and_the_time_grows() {
        let mut l = Lockout::default();
        let t = Instant::now();
        for _ in 0..4 {
            assert_eq!(l.reserve(t), None);
        }
        assert_eq!(l.locked(t), None);
        assert_eq!(l.reserve(t), None, "the fifth is the last one let in");
        assert_eq!(l.locked(t), Some(31));
        assert_eq!(l.reserve(t), Some(31), "the sixth, even in the same instant, is refused");
        assert_eq!(l.locked(t + Duration::from_secs(31)), None);
    }

    #[test]
    fn a_burst_cannot_get_more_than_five_guesses() {
        let mut l = Lockout::default();
        let t = Instant::now();
        let let_in = (0..50).filter(|_| l.reserve(t).is_none()).count();
        assert_eq!(let_in, 5);
    }

    #[test]
    fn a_right_login_resets_the_count() {
        let mut l = Lockout::default();
        let t = Instant::now();
        for _ in 0..4 {
            l.reserve(t);
        }
        l.success();
        for _ in 0..4 {
            assert_eq!(l.reserve(t), None);
        }
        assert_eq!(l.locked(t), None, "after success four more failures leave it unlocked");
    }

    #[test]
    fn a_released_slot_is_given_back() {
        let mut l = Lockout::default();
        let t = Instant::now();
        for _ in 0..5 {
            l.reserve(t);
        }
        assert!(l.locked(t).is_some());
        l.release();
        assert_eq!(l.locked(t), None);
        assert_eq!(l.failures(), 4);
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
