//! SSH servers, nested groups with inherited settings, identities and port forwards — the file
//! `~/.config/kalango/remote/hosts.json` that KalangoOS's Terminal reads too (schema: docs/HOSTS.md).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// None: inherit from the group (and the groups above it)
    pub user: Option<String>,
    pub identity: Option<String>,
    /// another host's id
    pub jump: Option<String>,
    /// seconds; 0 = off
    pub keepalive: Option<u32>,
    pub forward_agent: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub defaults: Settings,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ConnType {
    Ssh,
    Mosh,
}

impl Default for ConnType {
    fn default() -> Self {
        ConnType::Ssh
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub group: Option<String>,
    pub tags: Vec<String>,
    #[serde(rename = "type")]
    pub conn: ConnType,
    #[serde(flatten)]
    pub own: Settings,
    /// ask before connecting (production servers)
    pub confirm: bool,
    pub last_used: u64,
}

impl Default for Host {
    fn default() -> Self {
        Host {
            id: String::new(),
            name: String::new(),
            address: String::new(),
            port: 22,
            group: None,
            tags: Vec::new(),
            conn: ConnType::Ssh,
            own: Settings::default(),
            confirm: false,
            last_used: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityKind {
    /// a private key kept in the keyring (later: the kalangod vault)
    VaultKey,
    /// a password kept in the keyring
    VaultPassword,
    /// a key the system's ssh-agent holds (hardware keys too)
    Agent,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Identity {
    pub id: String,
    pub kind: IdentityKind,
    pub public_key: String,
    /// `SHA256:…`
    pub fingerprint: String,
    pub key_type: String,
    pub origin: String,
    pub confirm_each_use: bool,
    pub created: u64,
}

impl Default for Identity {
    fn default() -> Self {
        Identity {
            id: String::new(),
            kind: IdentityKind::VaultKey,
            public_key: String::new(),
            fingerprint: String::new(),
            key_type: String::new(),
            origin: String::new(),
            confirm_each_use: true,
            created: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ForwardKind {
    Local,
    Remote,
    Dynamic,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Forward {
    pub id: String,
    pub name: String,
    pub kind: ForwardKind,
    /// `127.0.0.1:5433` (local, dynamic) or the remote side's `host:port` (remote)
    pub listen: String,
    /// `db-staging:5432` (empty for dynamic)
    pub target: String,
    pub host: String,
    /// opens with the host's connection
    pub with_host: bool,
    /// listens on 127.0.0.1 only
    pub loopback_only: bool,
}

impl Default for Forward {
    fn default() -> Self {
        Forward {
            id: String::new(),
            name: String::new(),
            kind: ForwardKind::Local,
            listen: "127.0.0.1:0".into(),
            target: String::new(),
            host: String::new(),
            with_host: true,
            loopback_only: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct HostsFile {
    pub version: u32,
    pub groups: Vec<Group>,
    pub hosts: Vec<Host>,
    pub identities: Vec<Identity>,
    pub forwards: Vec<Forward>,
}

impl Default for HostsFile {
    fn default() -> Self {
        HostsFile { version: VERSION, groups: Vec::new(), hosts: Vec::new(), identities: Vec::new(), forwards: Vec::new() }
    }
}

/// Where a value of the effective settings came from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Host,
    /// the group's name
    Group(String),
    Default,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Effective {
    pub user: (String, Source),
    pub identity: (Option<String>, Source),
    pub jump: (Option<String>, Source),
    pub keepalive: (u32, Source),
    pub forward_agent: (bool, Source),
}

pub fn path() -> PathBuf {
    crate::store::config_dir().join("hosts.json")
}

pub fn known_hosts_path() -> PathBuf {
    crate::store::config_dir().join("known_hosts")
}

impl HostsFile {
    pub fn load() -> HostsFile {
        std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, p)
    }

    pub fn host(&self, id: &str) -> Option<&Host> {
        self.hosts.iter().find(|h| h.id == id)
    }

    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// The group and the groups above it, nearest first (a cycle stops the walk).
    pub fn group_chain(&self, start: Option<&str>) -> Vec<&Group> {
        let mut out: Vec<&Group> = Vec::new();
        let mut cur = start.and_then(|g| self.group(g));
        while let Some(g) = cur {
            if out.iter().any(|o| o.id == g.id) {
                break;
            }
            out.push(g);
            cur = g.parent.as_deref().and_then(|p| self.group(p));
        }
        out
    }

    /// The host's settings with what it inherits, and where each value came from.
    pub fn effective(&self, h: &Host) -> Effective {
        let chain = self.group_chain(h.group.as_deref());
        fn pick<T: Clone>(own: &Option<T>, chain: &[&Group], get: impl Fn(&Settings) -> Option<T>) -> Option<(T, Source)> {
            if let Some(v) = own {
                return Some((v.clone(), Source::Host));
            }
            chain.iter().find_map(|g| get(&g.defaults).map(|v| (v, Source::Group(g.name.clone()))))
        }
        let user = pick(&h.own.user, &chain, |s| s.user.clone()).unwrap_or_else(|| (local_user(), Source::Default));
        let identity = pick(&h.own.identity, &chain, |s| s.identity.clone())
            .map(|(v, s)| (Some(v), s))
            .unwrap_or((None, Source::Default));
        let jump = pick(&h.own.jump, &chain, |s| s.jump.clone()).map(|(v, s)| (Some(v), s)).unwrap_or((None, Source::Default));
        let keepalive = pick(&h.own.keepalive, &chain, |s| s.keepalive).unwrap_or((0, Source::Default));
        let forward_agent = pick(&h.own.forward_agent, &chain, |s| s.forward_agent).unwrap_or((false, Source::Default));
        Effective { user, identity, jump, keepalive, forward_agent }
    }

    /// The jump chain to reach a host: `[bastion, …]`, outermost first (cycles stop).
    pub fn jump_chain(&self, h: &Host) -> Vec<&Host> {
        let mut out: Vec<&Host> = Vec::new();
        let mut cur = self.effective(h).jump.0;
        while let Some(id) = cur {
            let Some(j) = self.host(&id) else { break };
            if j.id == h.id || out.iter().any(|o| o.id == j.id) {
                break;
            }
            out.insert(0, j);
            cur = self.effective(j).jump.0;
        }
        out
    }

    /// `user@address:port` of a host with its effective user.
    pub fn destination(&self, h: &Host) -> String {
        let user = self.effective(h).user.0;
        if h.port == 22 {
            format!("{user}@{}", h.address)
        } else {
            format!("{user}@{}:{}", h.address, h.port)
        }
    }

    /// An id not used yet, from a name ("App Prod 01" → "app-prod-01", then "-2", "-3"…).
    pub fn new_id(&self, name: &str) -> String {
        let base: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let base = if base.is_empty() { "servidor".to_owned() } else { base };
        let taken = |id: &str| self.hosts.iter().any(|h| h.id == id) || self.groups.iter().any(|g| g.id == id) || self.forwards.iter().any(|f| f.id == id) || self.identities.iter().any(|i| i.id == id);
        if !taken(&base) {
            return base;
        }
        (2..).map(|n| format!("{base}-{n}")).find(|c| !taken(c)).unwrap_or(base)
    }
}

pub fn local_user() -> String {
    std::env::var("USER").unwrap_or_else(|_| "root".into())
}

/// `usuario@endereço[:porta]` typed in "quick connect".
pub fn parse_quick(s: &str) -> Option<(Option<String>, String, u16)> {
    let s = s.trim();
    if s.is_empty() || s.contains(char::is_whitespace) {
        return None;
    }
    let (user, rest) = match s.rsplit_once('@') {
        Some((u, r)) if !u.is_empty() => (Some(u.to_owned()), r),
        Some(_) => return None,
        None => (None, s),
    };
    // [v6]:port, host:port, host
    let (host, port) = if let Some(r) = rest.strip_prefix('[') {
        let (h, after) = r.split_once(']')?;
        (h.to_owned(), after.strip_prefix(':').map(|p| p.parse().ok()).unwrap_or(Some(22))?)
    } else if rest.matches(':').count() == 1 {
        let (h, p) = rest.split_once(':')?;
        (h.to_owned(), p.parse().ok()?)
    } else {
        (rest.to_owned(), 22)
    };
    if host.is_empty() || !(user.is_some() || host.contains('.') || host.contains(':') || host == "localhost") {
        return None;
    }
    Some((user, host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HostsFile {
        HostsFile {
            groups: vec![
                Group { id: "trabalho".into(), name: "Trabalho".into(), parent: None, defaults: Settings { user: Some("deploy".into()), identity: Some("deploy-ed25519".into()), jump: Some("bastiao".into()), keepalive: Some(30), forward_agent: None } },
                Group { id: "producao".into(), name: "produção".into(), parent: Some("trabalho".into()), defaults: Settings { keepalive: Some(15), ..Default::default() } },
            ],
            hosts: vec![
                Host { id: "bastiao".into(), name: "bastiao-trabalho".into(), address: "bastiao.example.com".into(), port: 2222, group: Some("trabalho".into()), own: Settings { user: Some("ops".into()), jump: None, ..Default::default() }, ..Default::default() },
                Host { id: "app".into(), name: "app-prod-01".into(), address: "203.0.113.24".into(), group: Some("producao".into()), ..Default::default() },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn inherits_through_nested_groups_and_says_from_where() {
        let f = sample();
        let app = f.host("app").unwrap();
        let e = f.effective(app);
        assert_eq!(e.user, ("deploy".into(), Source::Group("Trabalho".into())));
        assert_eq!(e.keepalive, (15, Source::Group("produção".into())), "the nearest group wins");
        assert_eq!(e.identity.0.as_deref(), Some("deploy-ed25519"));
        assert_eq!(f.destination(app), "deploy@203.0.113.24");
        // a host's own value is an exception to the group
        let mut a2 = app.clone();
        a2.own.user = Some("root".into());
        assert_eq!(f.effective(&a2).user.1, Source::Host);
    }

    #[test]
    fn jump_chains_stop_at_cycles() {
        let mut f = sample();
        let app = f.host("app").unwrap().clone();
        assert_eq!(f.jump_chain(&app).iter().map(|h| h.id.as_str()).collect::<Vec<_>>(), vec!["bastiao"]);
        // the bastion inherits its own group's jump (itself): no loop
        assert!(f.jump_chain(f.host("bastiao").unwrap()).is_empty());
        // a → b → a
        f.hosts[0].own.jump = Some("app".into());
        assert_eq!(f.jump_chain(&app).len(), 1);
        // a group that is its own ancestor
        f.groups[0].parent = Some("producao".into());
        assert_eq!(f.group_chain(Some("producao")).len(), 2);
    }

    #[test]
    fn quick_connect_and_ids() {
        assert_eq!(parse_quick("deploy@10.0.2.15"), Some((Some("deploy".into()), "10.0.2.15".into(), 22)));
        assert_eq!(parse_quick("ops@bastiao.example.com:2222"), Some((Some("ops".into()), "bastiao.example.com".into(), 2222)));
        assert_eq!(parse_quick("pi@[fe80::1]:22"), Some((Some("pi".into()), "fe80::1".into(), 22)));
        assert_eq!(parse_quick("servidor"), None, "a bare word is a search, not an address");
        assert_eq!(parse_quick("a b@c"), None);
        let f = sample();
        assert_eq!(f.new_id("App Prod 01!"), "app-prod-01");
        assert_eq!(f.new_id("bastiao"), "bastiao-2");
    }

    #[test]
    fn file_round_trip_and_old_files() {
        let f = sample();
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("\"type\":\"ssh\""));
        let back: HostsFile = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
        let partial: HostsFile = serde_json::from_str(r#"{"hosts":[{"id":"x","address":"x.example"}]}"#).unwrap();
        assert_eq!(partial.hosts[0].port, 22);
        assert_eq!(partial.version, VERSION);
    }
}
