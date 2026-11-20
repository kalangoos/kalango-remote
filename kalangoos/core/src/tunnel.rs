//! The Cloudflare quick tunnel: `cloudflared tunnel --url tcp://127.0.0.1:PORT` gives a random
//! `*.trycloudflare.com` address, no account, closed when the process ends. The connecting side
//! reaches it with a plain WSS connection (see `bridge`), so only the sharer needs cloudflared.

use std::{
    io::{BufRead, BufReader},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
};

/// Where cloudflared is: `KALANGO_CLOUDFLARED`, next to this program, the dev copy fetched by
/// `kalangoos/scripts/cloudflared.sh`, then `PATH`.
pub fn cloudflared_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KALANGO_CLOUDFLARED") {
        return Some(PathBuf::from(p));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let mut candidates = vec![dir.join("cloudflared")];
    // target/<profile>/kalango-remote-core → kalangoos/.deps/bin/cloudflared
    if let Some(root) = dir.parent().and_then(|t| t.parent()) {
        candidates.push(root.join("kalangoos/.deps/bin/cloudflared"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join("cloudflared")));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// The version line cloudflared prints (`cloudflared version 2026.10.7 (built …)` → `2026.10.7`).
pub fn cloudflared_version(bin: &std::path::Path) -> Option<String> {
    let out = Command::new(bin).arg("--version").output().ok()?;
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_version(s: &str) -> Option<String> {
    s.split_whitespace().skip_while(|w| *w != "version").nth(1).map(str::to_owned)
}

/// The tunnel address in one of cloudflared's log lines.
pub fn parse_tunnel_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start + 8..];
    let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '.')).unwrap_or(rest.len());
    let host = &rest[..end];
    // the banner also prints https://www.cloudflare.com/... links; only the tunnel matters
    (host.ends_with(".trycloudflare.com") && host != "api.trycloudflare.com").then(|| host.to_owned())
}

/// A log line that means the tunnel will not open (no network, quick tunnels refused…).
pub fn parse_tunnel_error(line: &str) -> Option<String> {
    let l = line.to_ascii_lowercase();
    let fatal = ["failed to request quick tunnel", "failed to unmarshal quick tunnel", "error parsing", "no such host", "network is unreachable"];
    fatal.iter().any(|f| l.contains(f)).then(|| {
        line.split_once(" ERR ").map(|(_, m)| m).unwrap_or(line).trim().to_owned()
    })
}

pub struct Tunnel {
    child: Child,
}

impl Tunnel {
    /// Starts cloudflared for `port`; `on_line` sees each log line (url and errors are parsed by the
    /// caller with the functions above). The child dies with this process (PR_SET_PDEATHSIG).
    pub fn start(bin: &std::path::Path, port: u16, mut on_line: impl FnMut(&str) + Send + 'static) -> std::io::Result<Tunnel> {
        let mut cmd = Command::new(bin);
        cmd.args(["tunnel", "--no-autoupdate", "--protocol", "auto", "--url"])
            .arg(format!("tcp://127.0.0.1:{port}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let mut child = cmd.spawn()?;
        if let Some(err) = child.stderr.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    on_line(&line);
                }
            });
        }
        Ok(Tunnel { child })
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_tunnel_in_the_banner() {
        let banner = "2026-11-20T00:56:41Z INF |  https://positive-installed-july-dispatch.trycloudflare.com                                |";
        assert_eq!(parse_tunnel_url(banner).as_deref(), Some("positive-installed-july-dispatch.trycloudflare.com"));
        assert_eq!(parse_tunnel_url("INF Requesting new quick Tunnel on trycloudflare.com..."), None);
        assert_eq!(parse_tunnel_url("see https://www.cloudflare.com/website-terms/"), None);
        assert_eq!(parse_tunnel_url("https://api.trycloudflare.com/tunnel"), None);
    }

    #[test]
    fn recognizes_fatal_errors_and_version() {
        let e = "2026-11-20T01:00:00Z ERR Error unmarshaling QuickTunnel response: failed to request quick Tunnel: Post \"https://api.trycloudflare.com/tunnel\": dial tcp: lookup api.trycloudflare.com: no such host";
        assert!(parse_tunnel_error(e).is_some());
        assert!(parse_tunnel_error("INF Registered tunnel connection connIndex=0").is_none());
        assert_eq!(parse_version("cloudflared version 2026.10.7 (built 2026-10-30-17:05 UTC)\n").as_deref(), Some("2026.10.7"));
    }
}
