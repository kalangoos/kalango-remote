//! The lockout of the whole share (rules and tests: kalangoos/proto/src/lockout.rs, included here so that
//! this crate does not depend on the proto crate).

#[path = "../../kalangoos/proto/src/lockout.rs"]
mod rules;

use rules::Lockout;
use std::{sync::Mutex, time::Instant};

lazy_static::lazy_static! {
    static ref GLOBAL: Mutex<Lockout> = Mutex::new(Lockout::default());
}

/// Counts this attempt and checks the lockout in one step (no `await` between): `Some(secs)` = refused.
pub fn reserve() -> Option<u64> {
    GLOBAL.lock().unwrap().reserve(Instant::now())
}

pub fn release() {
    GLOBAL.lock().unwrap().release();
}

pub fn success() {
    GLOBAL.lock().unwrap().success();
}

pub fn locked() -> Option<u64> {
    GLOBAL.lock().unwrap().locked(Instant::now())
}

pub fn failures() -> u32 {
    GLOBAL.lock().unwrap().failures()
}

pub fn use_code(code: &str) -> bool {
    GLOBAL.lock().unwrap().use_code(code, Instant::now())
}
