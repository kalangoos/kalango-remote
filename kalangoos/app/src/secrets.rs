//! Secrets (the fixed password, SSH passwords and private keys) in the system's Secret Service through
//! libsecret's `secret-tool`. KalangoOS's plan is its own vault in `kalangod` (secrets bound to the
//! session, agents without access); this module is the one place to switch when that exists.

use std::{io::Write, process::{Command, Stdio}};

const APP: &str = "kalango-remote";

/// `KALANGO_REMOTE_COFRE=arquivo`: tests keep secrets in 0600 files of their own profile instead of the
/// person's real keyring (like KalangoOS's `KALANGO_COFRE=arquivo`).
fn file_store() -> Option<std::path::PathBuf> {
    (std::env::var("KALANGO_REMOTE_COFRE").as_deref() == Ok("arquivo")).then(|| crate::store::config_dir().join("cofre-de-teste"))
}

fn file_of(kind: &str, name: &str) -> Option<std::path::PathBuf> {
    let safe = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>();
    file_store().map(|d| d.join(format!("{}--{}", safe(kind), safe(name))))
}

pub fn store(kind: &str, name: &str, label: &str, secret: &str) -> std::io::Result<()> {
    if let Some(p) = file_of(kind, name) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(p.parent().unwrap_or(&p))?;
        std::fs::write(&p, secret)?;
        return std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    let mut c = Command::new("secret-tool")
        .args(["store", "--label", label, "application", APP, "kind", kind, "name", name])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    // the secret goes through stdin, never in the command line
    c.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?.write_all(secret.as_bytes())?;
    let out = c.wait_with_output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(String::from_utf8_lossy(&out.stderr).trim().to_owned()))
    }
}

pub fn lookup(kind: &str, name: &str) -> Option<String> {
    if let Some(p) = file_of(kind, name) {
        return std::fs::read_to_string(p).ok();
    }
    let out = Command::new("secret-tool").args(["lookup", "application", APP, "kind", kind, "name", name]).output().ok()?;
    (out.status.success() && !out.stdout.is_empty()).then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn clear(kind: &str, name: &str) {
    if let Some(p) = file_of(kind, name) {
        let _ = std::fs::remove_file(p);
        return;
    }
    let _ = Command::new("secret-tool").args(["clear", "application", APP, "kind", kind, "name", name]).status();
}

pub fn exists(kind: &str, name: &str) -> bool {
    lookup(kind, name).is_some()
}
