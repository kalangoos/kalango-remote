//! How this computer may be entered, from the settings to what the sharing core is told: the fixed password
//! of unattended access, the one-time password length, the two-step secret and the recording folder, plus
//! the housekeeping those need (recordings are deleted after 30 days; the screen locks when a session ends).

use kalango_remote_proto::setup::{Server, Setup};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use crate::{secrets, store::Settings};

pub const KEEP_RECORDINGS_DAYS: u64 = 30;
const VIDEO_EXTENSIONS: &[&str] = &["webm", "mp4", "mkv"];

/// "Videos › Kalango Remote": where recorded sessions go (`KALANGO_REMOTE_VIDEOS` moves it for tests).
pub fn recordings_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("KALANGO_REMOTE_VIDEOS") {
        return PathBuf::from(d);
    }
    let videos = gtk::glib::user_special_dir(gtk::glib::UserDirectory::Videos).unwrap_or_else(|| gtk::glib::home_dir().join("Videos"));
    videos.join("Kalango Remote")
}

/// What the core needs to start sharing under these settings. `fixed_password` and `two_factor` come from
/// the keyring; a switch that is on without its secret (it was cleared) counts as off.
pub fn setup_for(s: &Settings, fixed_password: Option<String>, two_factor: Option<String>, videos: &Path) -> Setup {
    Setup {
        fixed_password: if s.unattended { fixed_password.filter(|p| !p.is_empty()) } else { None },
        otp_length: if s.otp_length == 8 { 8 } else { 6 },
        two_factor: if s.two_factor { two_factor.filter(|t| !t.is_empty()) } else { None },
        record_dir: s.record.then(|| videos.to_string_lossy().into_owned()),
        clipboard: s.clipboard,
        audio: s.audio,
        // with "own server" chosen it is always asked for; empty fields make the core refuse (never the tunnel instead)
        server: (s.network == "server").then(|| Server {
            id: s.server_id.trim().to_owned(),
            relay: s.server_relay.trim().to_owned(),
            key: s.server_key.trim().to_owned(),
        }),
    }
}

/// The setup for the settings as they are now, reading the secrets from the keyring.
pub fn current_setup(s: &Settings) -> Setup {
    let two = secrets::lookup("two-factor", "default").map(|t| t.trim().to_owned());
    let fixed = secrets::lookup("fixed-password", "default");
    setup_for(s, fixed, two, &recordings_dir())
}

/// Deletes the recordings in `dir` older than `days` (only video files, only in this folder, not below it).
/// Returns how many it deleted.
pub fn prune_recordings(dir: &Path, now: SystemTime, days: u64) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let limit = Duration::from_secs(days * 86_400);
    let mut gone = 0;
    for e in entries.flatten() {
        let p = e.path();
        let video = p.extension().and_then(|x| x.to_str()).is_some_and(|x| VIDEO_EXTENSIONS.contains(&x.to_ascii_lowercase().as_str()));
        let old = e.metadata().ok().filter(|m| m.is_file()).and_then(|m| m.modified().ok()).and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > limit);
        if video && old && std::fs::remove_file(&p).is_ok() {
            gone += 1;
        }
    }
    gone
}

/// Locks this computer's screen (logind: the same path as the power menu).
/// The arguments of `kalango-remote-core serve` for these settings. The code on the card is found on the same network
/// through a UDP broadcast the core answers only when it listens on the network (`--lan`); in the default mode (the
/// Cloudflare tunnel) that used to be off, so typing the code on a computer next door said "nobody on this network".
/// With "prefer a direct connection on the same network" on, the tunnel mode listens on the network too (any port) and the
/// tunnel stays: the password and, with "ask before", the approval are still needed, as in the tunnel.
pub fn serve_args(s: &Settings) -> Vec<String> {
    let mut args = vec!["serve".to_string()];
    if s.network == "lan" {
        args.extend(["--no-tunnel".into(), "--lan".into()]);
    } else if s.network == "server" {
        // through the own server: no tunnel; the core registers there and the id is the code
        args.push("--no-tunnel".into());
    } else if s.prefer_lan {
        args.extend(["--lan".into(), "--port".into(), "0".into()]);
    }
    if s.ask_before {
        args.push("--ask".into());
    }
    args.push("--setup-stdin".into());
    args
}

pub fn lock_screen() {
    // the test profile (`KALANGO_REMOTE_COFRE=arquivo`) never locks the real session of whoever runs the tests
    if std::env::var("KALANGO_REMOTE_COFRE").as_deref() == Ok("arquivo") {
        eprintln!("kalango-remote: debug tela travada (perfil de teste: não trava de verdade)");
        return;
    }
    if let Err(e) = std::process::Command::new("loginctl").arg("lock-session").spawn() {
        eprintln!("kalango-remote: could not lock the screen: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kr-access-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_code_works_on_the_same_network_in_the_default_mode_too() {
        let mut s = Settings::default();
        assert_eq!((s.network.as_str(), s.prefer_lan, s.ask_before), ("cloudflare", true, true));
        // the default: the tunnel AND the network (so the UDP code answers), asked before every access
        assert_eq!(serve_args(&s), ["serve", "--lan", "--port", "0", "--ask", "--setup-stdin"]);
        s.prefer_lan = false;
        assert_eq!(serve_args(&s), ["serve", "--ask", "--setup-stdin"], "tunnel only when the direct connection is not wanted");
        s.network = "lan".into();
        assert_eq!(serve_args(&s), ["serve", "--no-tunnel", "--lan", "--ask", "--setup-stdin"]);
        s.network = "server".into();
        s.ask_before = false;
        s.prefer_lan = true;
        assert_eq!(serve_args(&s), ["serve", "--no-tunnel", "--setup-stdin"], "the own server is never opened to the network");
    }

    #[test]
    fn the_fixed_password_only_counts_while_unattended_access_is_on() {
        let mut s = Settings::default();
        let v = Path::new("/v");
        assert_eq!(setup_for(&s, Some("pw".into()), None, v).fixed_password, None);
        s.unattended = true;
        assert_eq!(setup_for(&s, Some("pw".into()), None, v).fixed_password.as_deref(), Some("pw"));
        assert_eq!(setup_for(&s, None, None, v).fixed_password, None);
    }

    #[test]
    fn two_step_recording_and_server_follow_their_switches() {
        let mut s = Settings::default();
        let v = Path::new("/v/Kalango Remote");
        let off = setup_for(&s, None, Some("ABC".into()), v);
        assert_eq!((off.two_factor, off.record_dir, off.server), (None, None, None));
        s.two_factor = true;
        s.record = true;
        s.network = "server".into();
        s.server_id = " remote.example.com ".into();
        let on = setup_for(&s, None, Some("ABC".into()), v);
        assert_eq!(on.two_factor.as_deref(), Some("ABC"));
        assert_eq!(on.record_dir.as_deref(), Some("/v/Kalango Remote"));
        assert_eq!(on.server.unwrap().id, "remote.example.com");
        s.server_id.clear();
        s.server_key.clear();
        let empty = setup_for(&s, None, None, v);
        assert!(empty.validate().is_err(), "an own server without its address and key is refused, not skipped");
    }

    #[test]
    fn recordings_older_than_the_limit_are_deleted_and_nothing_else() {
        let d = tmp("prune");
        let now = SystemTime::now();
        let old = now - Duration::from_secs(31 * 86_400);
        let make = |name: &str, when: SystemTime| {
            let p = d.join(name);
            std::fs::write(&p, b"x").unwrap();
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(when).unwrap();
        };
        make("velho.webm", old);
        make("novo.webm", now);
        make("velho.txt", old);
        std::fs::create_dir(d.join("pasta.webm")).unwrap();
        assert_eq!(prune_recordings(&d, now, KEEP_RECORDINGS_DAYS), 1);
        assert!(!d.join("velho.webm").exists());
        assert!(d.join("novo.webm").exists() && d.join("velho.txt").exists() && d.join("pasta.webm").exists());
        assert_eq!(prune_recordings(&d.join("nao-existe"), now, 30), 0);
        let _ = std::fs::remove_dir_all(&d);
    }
}
