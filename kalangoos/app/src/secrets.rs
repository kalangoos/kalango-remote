//! Secrets (the fixed password, SSH passwords and private keys) in the system's Secret Service through
//! libsecret's `secret-tool`. KalangoOS's plan is its own vault in `kalangod` (secrets bound to the
//! session, agents without access); this module is the one place to switch when that exists.

use std::{io::Write, process::{Command, Stdio}};

const APP: &str = "kalango-remote";

pub fn store(kind: &str, name: &str, label: &str, secret: &str) -> std::io::Result<()> {
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
    let out = Command::new("secret-tool").args(["lookup", "application", APP, "kind", kind, "name", name]).output().ok()?;
    (out.status.success() && !out.stdout.is_empty()).then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn clear(kind: &str, name: &str) {
    let _ = Command::new("secret-tool").args(["clear", "application", APP, "kind", kind, "name", name]).status();
}

pub fn exists(kind: &str, name: &str) -> bool {
    lookup(kind, name).is_some()
}
