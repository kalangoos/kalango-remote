//! The session's sounds on the computer being helped (KalangoOS's theme: `kalango-remote-start` when someone comes in,
//! `kalango-remote-stop` when they leave). KalangoOS's shell plays them — its volume, its theme, the session test's
//! silence — when asked through its app action `play-sound` on the session bus. Without the shell (another desktop),
//! nothing: the call does not start it and any error is dropped.

use gtk::{gio, glib, prelude::*};

const SHELL: &str = "os.kalango.Shell";

/// The shell's application id (the session test may give it another) and its object path.
fn shell() -> (String, String) {
    let app = std::env::var("KALANGO_SHELL_APP_ID").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| SHELL.into());
    let path = format!("/{}", app.replace('.', "/"));
    (app, path)
}

/// org.gtk.Actions.Activate's arguments for an action with a string parameter: `(sava{sv})`.
fn activate_params(action: &str, param: &str) -> glib::Variant {
    let empty: std::collections::HashMap<String, glib::Variant> = Default::default();
    glib::Variant::tuple_from_iter([
        action.to_variant(),
        glib::Variant::array_from_iter_with_type(glib::VariantTy::VARIANT, [param.to_variant().to_variant()]),
        empty.to_variant(),
    ])
}

/// Asks the shell for one of the session's sounds, in the background (fire and forget).
pub fn play(id: &'static str) {
    play_then(id, |_| {});
}

/// The same, and then `done` with whether the shell took it.
fn play_then(id: &'static str, done: impl FnOnce(bool) + 'static) {
    let (app, path) = shell();
    gio::bus_get(gio::BusType::Session, gio::Cancellable::NONE, move |c| {
        let Ok(c) = c else { return done(false) };
        c.call(Some(&app), &path, "org.gtk.Actions", "Activate", Some(&activate_params("play-sound", id)), None,
            gio::DBusCallFlags::NO_AUTO_START, 2000, gio::Cancellable::NONE, move |r| {
                if std::env::var_os("KALANGO_REMOTE_DEBUG").is_some() {
                    eprintln!("debug som: {id} pedido ao shell ({})", if r.is_ok() { "ok" } else { "sem o shell" });
                }
                done(r.is_ok());
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_has_the_actions_signature() {
        let v = activate_params("play-sound", "kalango-remote-start");
        assert_eq!(v.type_().as_str(), "(sava{sv})");
        assert_eq!(v.child_value(1).child_value(0).as_variant().and_then(|x| x.get::<String>()).as_deref(), Some("kalango-remote-start"));
        assert_eq!(shell().1, "/os/kalango/Shell");
    }

    /// Against a shell on the session bus (`kalangoos/scripts/testar-sons.sh` runs it with a stand-in on a bus of its
    /// own): the request arrives; with nobody owning the name it fails at once.
    #[test]
    #[ignore]
    fn asks_the_shell_over_the_session_bus() {
        let ctx = glib::MainContext::default();
        let _g = ctx.acquire().unwrap();
        let got = std::rc::Rc::new(std::cell::Cell::new(None));
        let g = got.clone();
        play_then("kalango-remote-start", move |ok| g.set(Some(ok)));
        let t = std::time::Instant::now();
        while got.get().is_none() && t.elapsed() < std::time::Duration::from_secs(5) {
            ctx.iteration(true);
        }
        let expect = std::env::var("KALANGO_REMOTE_TESTE_SHELL").as_deref() == Ok("sim");
        assert_eq!(got.get(), Some(expect));
        assert!(t.elapsed() < std::time::Duration::from_secs(3), "{:?}", t.elapsed());
    }
}
