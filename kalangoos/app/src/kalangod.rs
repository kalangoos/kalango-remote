//! KalangoOS's system service (kalangod, MCP over a local socket): here only for the screen-capture and input
//! permission of a session (KalangoOS's niri gives them to whom kalangod allows; see niri patch 0008).

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};

fn socket() -> std::path::PathBuf {
    std::env::var_os("KALANGOD_SOCKET").map(Into::into).unwrap_or_else(|| "/run/kalango/mcp.sock".into())
}

/// Whether kalangod is there (KalangoOS): its socket exists.
pub fn present() -> bool {
    socket().exists()
}

pub fn rpc(method: &str, params: Value) -> Result<Value, String> {
    let s = std::os::unix::net::UnixStream::connect(socket()).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
    let mut w = s.try_clone().map_err(|e| e.to_string())?;
    writeln!(w, "{}", json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    match v.get("error") {
        Some(e) => Err(e["message"].as_str().unwrap_or("kalangod").to_owned()),
        None => Ok(v["result"].clone()),
    }
}

/// Lets the core (`pid`, our child) capture the screen and, with `control`, use the pointer and keyboard; with
/// `privacy`, the compositor also covers the local monitors in black (and blocks the local person's input to what is
/// behind the cover) while the screen captures keep showing the real desktop (niri patch 0012).
pub fn grant(pid: u32, control: bool, privacy: bool) -> Result<(), String> {
    let mut caps: Vec<&str> = if control { vec!["captura", "ponteiro", "teclado"] } else { vec!["captura"] };
    if privacy {
        caps.push("privacidade");
    }
    rpc("kalango/controle_remoto", json!({"acao": "permitir", "pid": pid, "capacidades": caps})).map(|_| ())
}

pub fn revoke(pid: u32) -> Result<(), String> {
    rpc("kalango/controle_remoto", json!({"acao": "revogar", "pid": pid})).map(|_| ())
}

/// What the capabilities of a compositor's `kalango-features` answer say: it can cover the local monitors.
pub fn features_have_privacy(json: &str) -> bool {
    serde_json::from_str::<Vec<String>>(json).is_ok_and(|v| v.iter().any(|f| f == "privacy"))
}

/// Whether the running compositor can cover the local monitors during a remote session (KalangoOS's niri,
/// `niri msg kalango-features`). Upstream niri, another compositor or no session at all: no, and the privacy
/// switch stays off with its explanation. Asked once.
pub fn compositor_supports_privacy() -> bool {
    static ANSWER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ANSWER.get_or_init(|| {
        let niri = std::env::var_os("KALANGO_REMOTE_NIRI").unwrap_or_else(|| "niri".into());
        std::process::Command::new(niri)
            .args(["msg", "--json", "kalango-features"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .is_some_and(|o| features_have_privacy(&String::from_utf8_lossy(&o.stdout)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_privacy_feature_counts() {
        assert!(features_have_privacy(r#"["remote-control","privacy"]"#));
        assert!(!features_have_privacy(r#"["remote-control"]"#));
        assert!(!features_have_privacy(""));
        assert!(!features_have_privacy("Error: unknown subcommand"));
    }
}
