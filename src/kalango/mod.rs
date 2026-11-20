//! Kalango Remote · the glue between RustDesk's core and KalangoOS's remote-access app.
//!
//! The app's window is GTK 4 and runs in its own process (this library links GTK 3 on Linux, and the
//! two cannot share a process). It drives `kalango-remote-core` (kalangoos/core), which uses this
//! module:
//!
//! - [`direct`]: the sharing side without rendezvous servers — RustDesk's server listening on one TCP
//!   port (loopback when the Cloudflare tunnel carries it, any address for the local network).
//! - pinned peers: a direct address whose identity (RustDesk id + ed25519 public key) came out of
//!   band, in the invite; the handshake then encrypts end to end and fails closed on a mismatch.
//! - [`session`]: a controlling session whose UI events (frames, peer info, chat, transfers) go to a
//!   channel instead of a toolkit.
//! - [`cm`]: the connection manager (incoming requests, approval, the "being controlled" state) as a
//!   channel too.

pub mod cm;
pub mod direct;
pub mod session;
pub mod wlr_input;

use std::{collections::HashMap, sync::Mutex};

use hbb_common::config::Config;

lazy_static::lazy_static! {
    static ref PINNED: Mutex<HashMap<String, (String, [u8; 32])>> = Default::default();
}

/// Pins the identity expected at a direct address (`127.0.0.1:PORT`, `host:port`): the sharer's
/// RustDesk id and its ed25519 signing key, both from the invite.
pub fn pin_peer(addr: &str, id: &str, pk: [u8; 32]) {
    PINNED
        .lock()
        .unwrap()
        .insert(addr.to_owned(), (id.to_owned(), pk));
}

pub fn unpin_peer(addr: &str) {
    PINNED.lock().unwrap().remove(addr);
}

pub(crate) fn pinned_peer(addr: &str) -> Option<(String, [u8; 32])> {
    PINNED.lock().unwrap().get(addr).cloned()
}

/// Option that makes a right password only *ask*: the person at the shared computer approves.
pub const OPTION_APPROVE_AFTER_PASSWORD: &str = "kalango-approve-after-password";

pub(crate) fn approve_after_password() -> bool {
    Config::get_option(OPTION_APPROVE_AFTER_PASSWORD) == "Y"
}

/// This computer's identity as an invite carries it: (RustDesk id, ed25519 public signing key).
/// Creates the key pair on first use.
pub fn identity() -> (String, Vec<u8>) {
    let (_sk, pk) = Config::get_key_pair();
    (Config::get_id(), pk)
}

/// Turns on capture and input through wlr protocols when the compositor offers them (niri has no
/// RemoteDesktop portal). Must run before anything asks whether this is X11: RustDesk then takes its
/// X11-shaped paths (`RUSTDESK_FORCED_DISPLAY_SERVER`), and `scrap`/enigo use the wlr backends.
pub fn use_wlr_if_available() -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() || !wlr_remote::available() {
        return false;
    }
    std::env::set_var("KALANGO_WLR", "1");
    std::env::set_var("RUSTDESK_FORCED_DISPLAY_SERVER", "x11");
    true
}

/// Names this build's config and IPC namespace, so it never shares state with an installed RustDesk.
pub fn set_app_name(name: &str) {
    *hbb_common::config::APP_NAME.write().unwrap() = name.to_owned();
}
