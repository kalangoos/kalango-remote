//! What the app tells the sharing core before it starts listening (one JSON packet on its stdin, so a password
//! never travels in a command line): how the computer may be entered, and the options that follow from it.

use serde::{Deserialize, Serialize};

/// A meeting and relay server of the person's own (RustDesk-compatible).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Server {
    pub id: String,
    pub relay: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Setup {
    /// Unattended access: this password lets someone in without anyone approving.
    pub fixed_password: Option<String>,
    /// Length of the one-time password: 6 or 8.
    pub otp_length: u8,
    /// Two-step verification: the base32 secret of the authenticator code asked of everyone who comes in.
    pub two_factor: Option<String>,
    /// Record every incoming session into this folder.
    pub record_dir: Option<String>,
    /// Share through the person's own server instead of the tunnel.
    pub server: Option<Server>,
    /// What a session may do by default (files, terminal, port forwarding and remote restart are never on by
    /// default; a request the person approves can add files for that session).
    pub clipboard: bool,
    pub audio: bool,
}

impl Setup {
    /// Refuses what cannot be honoured, so the core never runs with a protection silently off.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(t) = &self.two_factor {
            match crate::totp::from_base32(t) {
                Some(s) if s.len() >= 16 => {}
                _ => return Err("two-step verification is on but its secret is not valid".into()),
            }
        }
        if let Some(s) = &self.server {
            if s.id.trim().is_empty() || s.key.trim().is_empty() {
                return Err("the own server needs its address and its public key (without the key its identity is not verified)".into());
            }
        }
        if self.fixed_password.as_deref().is_some_and(|p| p.chars().count() < 8) {
            return Err("the fixed password is too short".into());
        }
        Ok(())
    }

    /// The RustDesk options this setup means. `ask`: a right one-time password still needs the approval of
    /// whoever is at the computer.
    pub fn options(&self, ask: bool) -> Vec<(&'static str, String)> {
        let mut o = vec![
            ("verification-method", if self.fixed_password.is_some() { "use-both-passwords" } else { "use-temporary-password" }.to_owned()),
            ("approve-mode", "password".to_owned()),
            ("kalango-approve-after-password", if ask { "Y" } else { "" }.to_owned()),
            ("temporary-password-length", if self.otp_length == 8 { "8" } else { "6" }.to_owned()),
            ("allow-auto-record-incoming", if self.record_dir.is_some() { "Y" } else { "N" }.to_owned()),
        ];
        let yn = |on: bool| if on { "" } else { "N" }.to_owned();
        o.push(("enable-file-transfer", "N".into()));
        o.push(("enable-terminal", "N".into()));
        o.push(("enable-tunnel", "N".into()));
        o.push(("enable-remote-restart", "N".into()));
        o.push(("enable-clipboard", yn(self.clipboard)));
        o.push(("enable-audio", yn(self.audio)));
        let s = self.server.clone().unwrap_or_default();
        o.push(("custom-rendezvous-server", s.id));
        o.push(("relay-server", s.relay));
        o.push(("key", s.key));
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(o: &'a [(&'static str, String)], k: &str) -> &'a str {
        o.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str()).unwrap()
    }

    #[test]
    fn a_fixed_password_turns_on_both_passwords_and_nothing_else_does() {
        let off = Setup::default().options(true);
        assert_eq!(get(&off, "verification-method"), "use-temporary-password");
        let on = Setup { fixed_password: Some("segredo-longo".into()), ..Default::default() }.options(true);
        assert_eq!(get(&on, "verification-method"), "use-both-passwords");
        assert_eq!(get(&on, "kalango-approve-after-password"), "Y");
        assert_eq!(get(&Setup::default().options(false), "kalango-approve-after-password"), "");
    }

    #[test]
    fn length_recording_and_server_follow_the_setup() {
        let s = Setup {
            otp_length: 8,
            record_dir: Some("/home/x/Vídeos/Kalango Remote".into()),
            server: Some(Server { id: "remote.example.com".into(), relay: "".into(), key: "AAAA".into() }),
            ..Default::default()
        };
        let o = s.options(false);
        assert_eq!(get(&o, "temporary-password-length"), "8");
        assert_eq!(get(&o, "allow-auto-record-incoming"), "Y");
        assert_eq!(get(&o, "custom-rendezvous-server"), "remote.example.com");
        assert_eq!(get(&o, "key"), "AAAA");
        let o = Setup::default().options(false);
        assert_eq!(get(&o, "temporary-password-length"), "6");
        assert_eq!(get(&o, "allow-auto-record-incoming"), "N");
        assert_eq!(get(&o, "custom-rendezvous-server"), "");
    }

    #[test]
    fn nothing_but_clipboard_and_audio_is_granted_by_default() {
        let o = Setup { fixed_password: Some("segredo-longo".into()), clipboard: true, ..Default::default() }.options(false);
        for k in ["enable-file-transfer", "enable-terminal", "enable-tunnel", "enable-remote-restart", "enable-audio"] {
            assert_eq!(get(&o, k), "N", "{k}");
        }
        assert_eq!(get(&o, "enable-clipboard"), "");
    }

    #[test]
    fn a_setup_that_would_run_unprotected_is_refused() {
        let ok = Setup { two_factor: Some("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into()), ..Default::default() };
        assert!(ok.validate().is_ok());
        assert!(Setup { two_factor: Some("123".into()), ..Default::default() }.validate().is_err());
        assert!(Setup { two_factor: Some("ABC".into()), ..Default::default() }.validate().is_err());
        let srv = |key: &str| Setup { server: Some(Server { id: "s.example".into(), relay: "".into(), key: key.into() }), ..Default::default() };
        assert!(srv("").validate().is_err());
        assert!(srv("AAAA").validate().is_ok());
        assert!(Setup { fixed_password: Some("curta".into()), ..Default::default() }.validate().is_err());
    }

    #[test]
    fn it_round_trips_as_json() {
        let s = Setup { fixed_password: Some("x".into()), otp_length: 6, two_factor: Some("ABC".into()), ..Default::default() };
        assert_eq!(serde_json::from_str::<Setup>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
        assert_eq!(serde_json::from_str::<Setup>("{}").unwrap(), Setup::default());
    }
}
