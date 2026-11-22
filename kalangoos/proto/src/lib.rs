//! Kalango Remote · what the app and the core process say to each other, and the invite format.
//!
//! The app (GTK 4) and `kalango-remote-core` (RustDesk's core, which links GTK 3) live in separate
//! processes. They talk over the core's stdin/stdout with [`wire`] packets: JSON for events and
//! commands, raw frames for video.

pub mod invite;
pub mod lockout;
pub mod setup;
pub mod totp;
pub mod wire;

pub use invite::{code_for, Invite, InviteError, Target};
