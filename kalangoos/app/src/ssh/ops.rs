//! What the SSH area does with OpenSSH: read a server's key before any secret leaves (fingerprint
//! check), trust it into the app's own known_hosts, generate and import keys into the keyring, list the
//! ssh-agent's keys, build the command the Terminal runs, import `~/.ssh`, run port forwards.

use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
};

use super::{
    model::{ConnType, Forward, ForwardKind, Host, HostsFile, Identity, IdentityKind},
    openssh::{self, Trust},
};
use crate::{secrets, store};

/// A server's host key as it presents it now.
#[derive(Debug, Clone, PartialEq)]
pub struct ScannedKey {
    pub key_type: String,
    pub key: String,
    pub fingerprint: String,
}

/// `-o ProxyCommand=ssh … -W %h:%p jump` reaching `h` through its jump (recursively through the jump's
/// own jump). ProxyJump would not do: the inner ssh does not get our known_hosts nor our key.
fn proxy_args(f: &HostsFile, h: &Host, materialize: bool) -> Result<Vec<String>, String> {
    let e = f.effective(h);
    let Some(jid) = e.jump.0 else { return Ok(Vec::new()) };
    let Some(j) = f.host(&jid) else { return Ok(Vec::new()) };
    if f.jump_chain(h).is_empty() {
        return Ok(Vec::new());
    }
    let mut inner: Vec<String> = vec!["ssh".into()];
    inner.extend(host_opts(f, j, materialize, true)?);
    inner.extend(["-W".into(), "%h:%p".into(), "-p".into(), j.port.to_string(), format!("{}@{}", f.effective(j).user.0, j.address)]);
    Ok(vec!["-o".into(), format!("ProxyCommand={}", inner.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" "))])
}

/// The options every ssh to `h` takes: our known_hosts (strict or not), its identity, keepalive, agent
/// forwarding and how to reach it.
fn host_opts(f: &HostsFile, h: &Host, materialize: bool, strict: bool) -> Result<Vec<String>, String> {
    let e = f.effective(h);
    let kh = super::model::known_hosts_path().display().to_string();
    let mut o: Vec<String> = vec!["-F".into(), "/dev/null".into()];
    o.extend(common_opts(&kh));
    if strict {
        o.extend(["-o".into(), "StrictHostKeyChecking=yes".into()]);
    }
    if e.keepalive.0 > 0 {
        o.extend(["-o".into(), format!("ServerAliveInterval={}", e.keepalive.0)]);
    }
    if e.forward_agent.0 {
        o.push("-A".into());
    }
    if let Some(id) = &e.identity.0 {
        match f.identities.iter().find(|i| &i.id == id).map(|i| &i.kind) {
            Some(IdentityKind::VaultKey) if materialize => {
                o.extend(["-i".into(), materialize_key(id)?.display().to_string(), "-o".into(), "IdentitiesOnly=yes".into()]);
            }
            Some(IdentityKind::VaultKey) => o.extend(["-i".into(), format!("<keyring:{id}>")]),
            _ => {}
        }
    }
    o.extend(proxy_args(f, h, materialize)?);
    Ok(o)
}

fn common_opts(known_hosts: &str) -> Vec<String> {
    vec![
        "-o".into(),
        format!("UserKnownHostsFile={known_hosts}"),
        "-o".into(),
        "GlobalKnownHostsFile=/dev/null".into(),
    ]
}

/// Reads the server's key without authenticating: ssh writes the key it saw into a throwaway
/// known_hosts, then fails at "none" authentication — no password or key is offered.
pub fn scan(f: &HostsFile, h: &Host) -> Result<ScannedKey, String> {
    let dir = std::env::temp_dir().join(format!("kalango-remote-scan-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let kh = dir.join("known_hosts");
    let _ = std::fs::remove_file(&kh);
    let mut cmd = Command::new("ssh");
    cmd.args(common_opts(&kh.display().to_string()))
        .args(["-o", "StrictHostKeyChecking=accept-new", "-o", "BatchMode=yes", "-o", "PreferredAuthentications=none", "-o", "ConnectTimeout=8", "-o", "LogLevel=ERROR"])
        .args(["-F", "/dev/null"])
        .args(proxy_args(f, h, true)?)
        .args(["-p", &h.port.to_string()])
        .arg(format!("{}@{}", f.effective(h).user.0, h.address))
        .arg("true")
        .stdin(Stdio::null());
    let out = cmd.output().map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&kh).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    let k = openssh::parse_known_hosts(&text).into_iter().next().ok_or_else(|| {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        if err.is_empty() { "no answer".to_owned() } else { err }
    })?;
    let fingerprint = openssh::fingerprint(&k.key).unwrap_or_default();
    Ok(ScannedKey { key_type: k.key_type, key: k.key, fingerprint })
}

pub fn known_hosts() -> Vec<openssh::KnownHost> {
    std::fs::read_to_string(super::model::known_hosts_path()).map(|t| openssh::parse_known_hosts(&t)).unwrap_or_default()
}

pub fn trust_of(h: &Host, k: &ScannedKey) -> Trust {
    openssh::check(&known_hosts(), &h.address, h.port, &k.key_type, &k.key)
}

/// Remembers a server's key (the person confirmed the fingerprint).
pub fn trust(h: &Host, k: &ScannedKey) -> std::io::Result<()> {
    use std::io::Write;
    let p = super::model::known_hosts_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&p)?;
    file.write_all(openssh::known_hosts_line(&h.address, h.port, &k.key_type, &k.key).as_bytes())
}

/// Removes every key remembered for a host (the only way to accept a changed key).
pub fn forget_host_key(address: &str, port: u16) -> std::io::Result<()> {
    let p = super::model::known_hosts_path();
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let name = openssh::host_pattern(address, port);
    let kept: String = text
        .lines()
        .filter(|l| {
            let k = openssh::parse_known_hosts(l);
            !k.iter().any(|k| k.hosts.iter().any(|h| h == &name))
        })
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(p, kept)
}

/// Generates an ed25519 key, keeps the private part in the keyring and returns the identity.
pub fn generate_key(f: &HostsFile, name: &str, passphrase: &str) -> Result<Identity, String> {
    let dir = runtime_dir().join(format!("gen-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok();
    let key = dir.join("key");
    let comment = format!("{}@kalango", name);
    let st = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", passphrase, "-C", &comment, "-f"])
        .arg(&key)
        .status()
        .map_err(|e| e.to_string())?;
    if !st.success() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err("ssh-keygen failed".into());
    }
    let private = std::fs::read_to_string(&key).map_err(|e| e.to_string())?;
    let public = std::fs::read_to_string(key.with_extension("pub")).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    let id = f.new_id(name);
    secrets::store("ssh-key", &id, &format!("Kalango Remote: SSH key {id}"), &private).map_err(|e| e.to_string())?;
    Ok(identity_from_public(&id, public.trim(), IdentityKind::VaultKey, "generated here"))
}

/// Imports a private key file (e.g. from ~/.ssh) into the keyring.
pub fn import_key(f: &HostsFile, path: &std::path::Path) -> Result<Identity, String> {
    let private = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if !private.contains("PRIVATE KEY") {
        return Err("not a private key".into());
    }
    let public = std::fs::read_to_string(path.with_extension("pub"))
        .or_else(|_| {
            let o = Command::new("ssh-keygen").args(["-y", "-P", "", "-f"]).arg(path).output()?;
            Ok::<String, std::io::Error>(String::from_utf8_lossy(&o.stdout).to_string())
        })
        .map_err(|e: std::io::Error| e.to_string())?;
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "chave".into());
    let id = f.new_id(&name);
    secrets::store("ssh-key", &id, &format!("Kalango Remote: SSH key {id}"), &private).map_err(|e| e.to_string())?;
    Ok(identity_from_public(&id, public.trim(), IdentityKind::VaultKey, &format!("imported from {}", path.display())))
}

pub fn identity_from_public(id: &str, public: &str, kind: IdentityKind, origin: &str) -> Identity {
    let mut parts = public.split_whitespace();
    let key_type = parts.next().unwrap_or("").to_owned();
    let blob = parts.next().unwrap_or("");
    Identity {
        id: id.to_owned(),
        kind,
        public_key: public.to_owned(),
        fingerprint: openssh::fingerprint(blob).unwrap_or_default(),
        key_type: openssh::short_type(&key_type),
        origin: origin.to_owned(),
        confirm_each_use: true,
        created: store::now(),
    }
}

/// The keys the system's ssh-agent holds (`ssh-add -L`).
pub fn agent_keys() -> Vec<Identity> {
    let Ok(o) = Command::new("ssh-add").arg("-L").output() else { return Vec::new() };
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|l| l.starts_with("ssh-") || l.starts_with("sk-") || l.starts_with("ecdsa-"))
        .map(|l| {
            let comment = l.split_whitespace().nth(2).unwrap_or("agent").to_owned();
            identity_from_public(&comment, l, IdentityKind::Agent, "ssh-agent")
        })
        .collect()
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("kalango-remote")
}

/// Writes a keyring key to a 0600 file in the runtime dir (tmpfs) for ssh to read; the Terminal deletes
/// it when the connection ends.
pub fn materialize_key(id: &str) -> Result<PathBuf, String> {
    let private = secrets::lookup("ssh-key", id).ok_or_else(|| format!("the key {id} is not in the keyring"))?;
    let dir = runtime_dir().join("keys");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok();
    let p = dir.join(id);
    std::fs::write(&p, private).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    Ok(p)
}

/// The command that opens a host, for the Terminal (`kalango-remote ssh-command <host-id>`): strict
/// host key checking against this app's known_hosts, the jump chain, the identity, keepalive.
pub fn command(f: &HostsFile, h: &Host, materialize: bool) -> Result<Vec<String>, String> {
    let e = f.effective(h);
    let mut ssh: Vec<String> = vec!["ssh".into()];
    ssh.extend(host_opts(f, h, materialize, true)?);
    for fw in f.forwards.iter().filter(|fw| fw.host == h.id && fw.with_host) {
        ssh.extend(forward_args(fw));
    }
    let dest = format!("{}@{}", e.user.0, h.address);
    match h.conn {
        ConnType::Ssh => {
            ssh.extend(["-p".into(), h.port.to_string(), dest]);
            Ok(ssh)
        }
        ConnType::Mosh => {
            // mosh-client runs as its own program, authenticated by ssh first
            let ssh_cmd = ssh[..].iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ") + &format!(" -p {}", h.port);
            Ok(vec!["mosh".into(), format!("--ssh={ssh_cmd}"), dest])
        }
    }
}

fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@,%".contains(c)) {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub fn forward_args(fw: &Forward) -> Vec<String> {
    let bind = |listen: &str| -> String {
        if fw.loopback_only && !listen.contains(':') {
            format!("127.0.0.1:{listen}")
        } else if fw.loopback_only {
            listen.to_owned()
        } else {
            listen.rsplit(':').next().map(|p| format!("0.0.0.0:{p}")).unwrap_or_else(|| listen.to_owned())
        }
    };
    match fw.kind {
        ForwardKind::Local => vec!["-L".into(), format!("{}:{}", bind(&fw.listen), fw.target)],
        ForwardKind::Remote => vec!["-R".into(), format!("{}:{}", fw.listen, fw.target)],
        ForwardKind::Dynamic => vec!["-D".into(), bind(&fw.listen)],
    }
}

/// Starts a forward on its own (`ssh -N`), in the background.
pub fn start_forward(f: &HostsFile, fw: &Forward) -> Result<Child, String> {
    let h = f.host(&fw.host).ok_or("unknown server")?;
    let mut argv = command(f, h, true)?;
    if argv.first().map(String::as_str) != Some("ssh") {
        return Err("forwards need SSH, not Mosh".into());
    }
    // ssh -N <forward> … user@host
    let dest = argv.pop().unwrap_or_default();
    argv.extend(["-N".into(), "-o".into(), "ExitOnForwardFailure=yes".into(), "-o".into(), "BatchMode=yes".into()]);
    argv.extend(forward_args(fw));
    argv.push(dest);
    Command::new(&argv[0]).args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())
}

/// Hosts from `~/.ssh/config` and the keys of `~/.ssh/known_hosts` into this app's files. Returns
/// (hosts added, known keys added).
pub fn import_openssh(f: &mut HostsFile, ssh_dir: &std::path::Path) -> (usize, usize) {
    let mut added = 0;
    let cfg = std::fs::read_to_string(ssh_dir.join("config")).unwrap_or_default();
    let parsed = openssh::parse_config(&cfg);
    for c in &parsed {
        if f.hosts.iter().any(|h| h.name == c.alias) {
            continue;
        }
        let id = f.new_id(&c.alias);
        let mut h = Host { id, name: c.alias.clone(), address: c.hostname.clone().unwrap_or_else(|| c.alias.clone()), port: c.port.unwrap_or(22), ..Default::default() };
        h.own.user = c.user.clone();
        f.hosts.push(h);
        added += 1;
    }
    // ProxyJump by alias, once every host exists
    for c in &parsed {
        if let Some(j) = &c.proxy_jump {
            let first = j.split(',').next_back().unwrap_or(j).split('@').next_back().unwrap_or(j).to_owned();
            let jid = f.hosts.iter().find(|h| h.name == first).map(|h| h.id.clone());
            if let (Some(jid), Some(h)) = (jid, f.hosts.iter_mut().find(|h| h.name == c.alias)) {
                h.own.jump = Some(jid);
            }
        }
    }
    // keys: the identity files become keyring identities
    for c in &parsed {
        let Some(path) = &c.identity_file else { continue };
        let p = PathBuf::from(path.replacen('~', &ssh_dir.parent().map(|p| p.display().to_string()).unwrap_or_default(), 1));
        let existing = f.identities.iter().find(|i| i.origin.ends_with(&p.display().to_string())).map(|i| i.id.clone());
        let id = match existing {
            Some(id) => Some(id),
            None => import_key(f, &p).ok().map(|i| {
                let id = i.id.clone();
                f.identities.push(i);
                id
            }),
        };
        if let (Some(id), Some(h)) = (id, f.hosts.iter_mut().find(|h| h.name == c.alias)) {
            h.own.identity = Some(id);
        }
    }
    // known_hosts lines go to ours (hashed ones too: ssh reads them the same way)
    let mut known = 0;
    if let Ok(text) = std::fs::read_to_string(ssh_dir.join("known_hosts")) {
        let ours = std::fs::read_to_string(super::model::known_hosts_path()).unwrap_or_default();
        let mut add = String::new();
        for l in text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
            if !ours.contains(l) {
                add.push_str(l);
                add.push('\n');
                known += 1;
            }
        }
        if !add.is_empty() {
            let p = super::model::known_hosts_path();
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(&p, ours + &add);
        }
    }
    (added, known)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::model::{Group, Settings};

    #[test]
    fn terminal_command_has_strict_checks_jumps_and_forwards() {
        let mut f = HostsFile::default();
        f.groups.push(Group { id: "t".into(), name: "Trabalho".into(), parent: None, defaults: Settings { user: Some("deploy".into()), jump: Some("b".into()), keepalive: Some(30), ..Default::default() } });
        f.hosts.push(Host { id: "b".into(), name: "bastiao".into(), address: "bastiao.example.com".into(), port: 2222, own: Settings { user: Some("ops".into()), ..Default::default() }, ..Default::default() });
        f.hosts.push(Host { id: "a".into(), name: "app".into(), address: "203.0.113.24".into(), group: Some("t".into()), ..Default::default() });
        f.forwards.push(Forward { id: "fw".into(), name: "db".into(), kind: ForwardKind::Local, listen: "5433".into(), target: "db-staging:5432".into(), host: "a".into(), with_host: true, loopback_only: true });
        let a = f.host("a").unwrap().clone();
        let argv = command(&f, &a, false).unwrap();
        let s = argv.join(" ");
        assert!(s.contains("StrictHostKeyChecking=yes"));
        assert!(s.contains("ProxyCommand=ssh -F /dev/null"), "{s}");
        assert!(s.contains("-W %h:%p -p 2222 ops@bastiao.example.com"), "{s}");
        assert!(s.contains("ServerAliveInterval=30"));
        assert!(s.contains("-L 127.0.0.1:5433:db-staging:5432"));
        assert!(s.ends_with("-p 22 deploy@203.0.113.24"), "{s}");
        let mut m = a.clone();
        m.conn = ConnType::Mosh;
        let mosh = command(&f, &m, false).unwrap();
        assert_eq!(mosh[0], "mosh");
        assert!(mosh[1].starts_with("--ssh=ssh -F /dev/null"), "{}", mosh[1]);
        assert_eq!(mosh[2], "deploy@203.0.113.24");
    }

    #[test]
    fn forward_arguments() {
        let mut fw = Forward { kind: ForwardKind::Dynamic, listen: "1080".into(), ..Default::default() };
        assert_eq!(forward_args(&fw), vec!["-D", "127.0.0.1:1080"]);
        fw.loopback_only = false;
        assert_eq!(forward_args(&fw), vec!["-D", "0.0.0.0:1080"]);
        fw.kind = ForwardKind::Remote;
        fw.listen = "8080".into();
        fw.target = "localhost:5173".into();
        assert_eq!(forward_args(&fw), vec!["-R", "8080:localhost:5173"]);
    }
}
