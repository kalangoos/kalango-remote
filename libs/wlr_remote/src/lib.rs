//! Screen capture and input injection through Wayland protocols, for compositors that offer them to
//! ordinary clients but have no RemoteDesktop portal — niri, which KalangoOS uses (see docs/POC.md):
//!
//! - frames: `zwlr_screencopy_manager_v1`, waiting for damage, so an idle screen costs nothing;
//! - pointer: `zwlr_virtual_pointer_manager_v1`;
//! - keyboard: `zwp_virtual_keyboard_manager_v1`, with an xkb state kept here so modifiers reach the
//!   focused window (the compositor forwards a virtual keyboard's modifiers as the client sends them).
//!
//! Each [`Capture`] and [`Input`] owns its own connection and thread, so both are `Send` and can be
//! driven from RustDesk's service threads.

mod capture;
mod globals;
mod input;

pub use capture::{Capture, CapturedFrame};
pub use globals::{available, outputs, Output};

/// Whether the session's compositor is niri (its wlr protocols may be offered only after a permission, so
/// [`available`] can say no before it).
pub fn is_niri() -> bool {
    std::env::var_os("NIRI_SOCKET").is_some()
        || std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_lowercase().contains("niri"))
}
pub use input::{Button, Input};

/// Whether `KALANGO_WLR` turned this backend on (set by Kalango Remote's core after [`available`]).
pub fn active() -> bool {
    std::env::var("KALANGO_WLR").as_deref() == Ok("1")
}
