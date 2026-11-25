//! What the app remembers: its settings and the computers it connected to (favourites and recents),
//! as JSON under `~/.config/kalango/remote/` (XDG), thumbnails under `~/.cache/kalango/remote/`.

use gtk::glib;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::i18n::{tr, trf};

pub fn config_dir() -> PathBuf {
    glib::user_config_dir().join("kalango/remote")
}

pub fn cache_dir() -> PathBuf {
    glib::user_cache_dir().join("kalango/remote")
}

fn read<T: for<'a> Deserialize<'a> + Default>(name: &str) -> T {
    std::fs::read(config_dir().join(name)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write<T: Serialize>(name: &str, v: &T) {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(s) = serde_json::to_vec_pretty(v) {
        // written whole and renamed, so a crash never leaves half a file
        let tmp = dir.join(format!(".{name}.tmp"));
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(tmp, dir.join(name));
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct AllowedDevice {
    pub name: String,
    /// RustDesk id of the device
    pub id: String,
    /// unix seconds; 0 = always
    pub until: u64,
    pub since: u64,
}

impl Default for AllowedDevice {
    fn default() -> Self {
        AllowedDevice { name: String::new(), id: String::new(), until: 0, since: 0 }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub ask_before: bool,
    pub unattended: bool,
    pub lock_after: bool,
    pub privacy_black: bool,
    pub record: bool,
    /// two-step verification (the secret is in the keyring)
    pub two_factor: bool,
    pub otp_length: u8,
    /// "cloudflare", "lan" or "server"
    pub network: String,
    pub server_id: String,
    pub server_relay: String,
    pub server_key: String,
    /// Sharing through the tunnel also listens on the local network (so the code is found there): off unless the
    /// person turns it on, because it opens a port on whatever network the computer is on. (Replaces `prefer_lan`,
    /// which did nothing in 0.1.0 and was on by default: an old settings file does not turn this on.)
    pub lan_too: bool,
    pub accept_ip: bool,
    pub allowed_only: bool,
    pub allowed: Vec<AllowedDevice>,
    pub allowed_ranges: String,
    /// "best", "balanced" or "speed"
    pub quality: String,
    /// "fit", "original" or "stretch"
    pub scale: String,
    pub codec: String,
    pub show_cursor: bool,
    pub audio: bool,
    pub clipboard: bool,
    /// how long a share stays open
    pub share_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ask_before: true,
            unattended: false,
            lock_after: true,
            privacy_black: true,
            record: false,
            two_factor: false,
            otp_length: 6,
            network: "cloudflare".into(),
            server_id: String::new(),
            server_relay: String::new(),
            server_key: String::new(),
            lan_too: false,
            accept_ip: false,
            allowed_only: false,
            allowed: Vec::new(),
            allowed_ranges: String::new(),
            quality: "balanced".into(),
            scale: "fit".into(),
            codec: "auto".into(),
            show_cursor: true,
            audio: false,
            clipboard: true,
            share_minutes: 60,
        }
    }
}

impl Settings {
    /// How the connections of a share arrive, for the badges and banners.
    pub fn route_text(&self) -> &'static str {
        match self.network.as_str() {
            "lan" => tr("on the same network"),
            "server" if !self.server_id.trim().is_empty() => tr("via your server"),
            _ => tr("via Cloudflare"),
        }
    }

    pub fn load() -> Settings {
        read("settings.json")
    }
    pub fn save(&self) {
        write("settings.json", self)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Peer {
    /// what the person sees: the remote hostname, or the name they gave it
    pub name: String,
    /// what reconnects: the invite link, `host:port` or a code
    pub target: String,
    pub code: String,
    /// "cloudflare" or "lan"
    pub route: String,
    pub last_used: u64,
    pub latency_ms: u32,
    pub favorite: bool,
    pub view_only: bool,
    pub thumb: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Peers {
    pub list: Vec<Peer>,
}

impl Peers {
    pub fn load() -> Peers {
        read("peers.json")
    }
    pub fn save(&self) {
        write("peers.json", self)
    }
    pub fn favorites(&self) -> Vec<Peer> {
        let mut v: Vec<_> = self.list.iter().filter(|p| p.favorite).cloned().collect();
        v.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        v
    }
    pub fn recents(&self) -> Vec<Peer> {
        let mut v: Vec<_> = self.list.iter().filter(|p| !p.favorite).cloned().collect();
        v.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        v
    }
    /// Records a session to `target` (updates it, or adds it on top).
    pub fn remember(&mut self, p: Peer) {
        if let Some(old) = self.list.iter_mut().find(|o| o.target == p.target || (!p.code.is_empty() && o.code == p.code)) {
            let fav = old.favorite;
            *old = Peer { favorite: fav, ..p };
        } else {
            self.list.push(p);
        }
    }
    pub fn clear_recents(&mut self) {
        self.list.retain(|p| p.favorite);
    }
    pub fn toggle_favorite(&mut self, target: &str) {
        if let Some(p) = self.list.iter_mut().find(|p| p.target == target) {
            p.favorite = !p.favorite;
        }
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// "agora", "há 12 min", "há 3 h", "ontem", "há 4 dias" — how long ago, short.
pub fn ago(then: u64, now: u64) -> String {
    let d = now.saturating_sub(then);
    if d < 60 {
        tr("now").to_owned()
    } else if d < 3600 {
        trf("{n} min ago", &[("n", (d / 60).to_string())])
    } else if d < 86_400 {
        trf("{n} h ago", &[("n", (d / 3600).to_string())])
    } else if d < 2 * 86_400 {
        tr("yesterday").to_owned()
    } else {
        trf("{n} days ago", &[("n", (d / 86_400).to_string())])
    }
}

/// "Aberto por mais 54 min" — minutes left, rounded up.
pub fn minutes_left(secs: u64) -> u64 {
    secs.div_ceil(60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages() {
        assert_eq!(ago(1000, 1030), tr("now"));
        assert_eq!(ago(0, 12 * 60), trf("{n} min ago", &[("n", "12".into())]));
        assert_eq!(ago(0, 3 * 3600 + 5), trf("{n} h ago", &[("n", "3".into())]));
        assert_eq!(ago(0, 30 * 3600), tr("yesterday"));
        assert_eq!(ago(0, 4 * 86_400), trf("{n} days ago", &[("n", "4".into())]));
        assert_eq!(minutes_left(54 * 60 - 1), 54);
        assert_eq!(minutes_left(0), 0);
    }

    #[test]
    fn remember_keeps_favorite_and_updates() {
        let mut ps = Peers::default();
        ps.remember(Peer { name: "a".into(), target: "x.trycloudflare.com".into(), last_used: 1, ..Default::default() });
        ps.toggle_favorite("x.trycloudflare.com");
        ps.remember(Peer { name: "a2".into(), target: "x.trycloudflare.com".into(), last_used: 5, ..Default::default() });
        assert_eq!(ps.list.len(), 1);
        assert!(ps.list[0].favorite);
        assert_eq!(ps.list[0].name, "a2");
        ps.remember(Peer { name: "b".into(), target: "10.0.0.2:21118".into(), last_used: 9, ..Default::default() });
        assert_eq!(ps.recents().len(), 1);
        ps.clear_recents();
        assert_eq!(ps.list.len(), 1);
    }

    #[test]
    fn settings_tolerate_old_files() {
        let s: Settings = serde_json::from_str(r#"{"network":"lan"}"#).unwrap();
        assert_eq!(s.network, "lan");
        assert!(s.ask_before, "missing fields take the defaults");
    }
}
