//! KalangoOS's system service (kalangod, MCP over a local socket): here only for the screen-capture and input
//! permission of a session (KalangoOS's niri gives them to whom kalangod allows; see niri patch 0008).

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};

fn socket() -> std::path::PathBuf {
    std::env::var_os("KALANGOD_SOCKET").map(Into::into).unwrap_or_else(|| "/run/kalango/mcp.sock".into())
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

/// Lets the core (`pid`, our child) capture the screen and, with `control`, use the pointer and keyboard.
pub fn grant(pid: u32, control: bool) -> Result<(), String> {
    let caps: Vec<&str> = if control { vec!["captura", "ponteiro", "teclado"] } else { vec!["captura"] };
    rpc("kalango/controle_remoto", json!({"acao": "permitir", "pid": pid, "capacidades": caps})).map(|_| ())
}

pub fn revoke(pid: u32) -> Result<(), String> {
    rpc("kalango/controle_remoto", json!({"acao": "revogar", "pid": pid})).map(|_| ())
}
