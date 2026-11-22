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
}

impl Setup {
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
    fn it_round_trips_as_json() {
        let s = Setup { fixed_password: Some("x".into()), otp_length: 6, two_factor: Some("ABC".into()), ..Default::default() };
        assert_eq!(serde_json::from_str::<Setup>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
        assert_eq!(serde_json::from_str::<Setup>("{}").unwrap(), Setup::default());
    }
}
