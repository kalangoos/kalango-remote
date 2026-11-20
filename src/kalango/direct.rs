//! The sharing side without rendezvous: RustDesk's server, reachable on one TCP port only.
//!
//! Stock RustDesk's `start_server` also registers with rendezvous servers (public ones by default),
//! asks a STUN server for the NAT type and syncs with an API server. Kalango Remote's default
//! promises that nothing goes through anyone's servers but Cloudflare's tunnel, so this starts only
//! the pieces a direct connection needs: the config IPC, the services and a listener.

use std::net::{Ipv4Addr, SocketAddr};

use hbb_common::{allow_err, log, tcp, tokio, ResultType, Stream};

use crate::server::ConnectionMeta;

pub struct DirectOptions {
    /// 0 picks a free port (reported through `on_listening`).
    pub port: u16,
    /// Listen on 127.0.0.1 only (the tunnel's cloudflared connects locally) or on every address
    /// (same local network).
    pub loopback_only: bool,
}

/// Runs until the process ends. `on_listening` gets the bound address once.
#[tokio::main]
pub async fn run(opts: DirectOptions, on_listening: impl FnOnce(SocketAddr)) -> ResultType<()> {
    allow_err!(crate::trusted_devices::initialize());
    crate::common::set_server_running(true);
    std::thread::spawn(|| {
        if let Err(err) = crate::ipc::start("") {
            log::error!("kalango direct: config ipc failed: {err}");
        }
    });
    if wlr_remote::active() {
        if let Err(e) = super::wlr_input::setup() {
            log::error!("kalango direct: no wlr input ({e}); the screen can be seen but not controlled");
        }
    }
    let server = crate::server::new();
    let ip = if opts.loopback_only {
        Ipv4Addr::LOCALHOST
    } else {
        Ipv4Addr::UNSPECIFIED
    };
    let listener = tcp::new_listener(SocketAddr::from((ip, opts.port)), false).await?;
    let local = listener.local_addr()?;
    log::info!("kalango direct: listening on {local}");
    on_listening(local);
    loop {
        let (stream, addr) = listener.accept().await?;
        stream.set_nodelay(true).ok();
        let local_addr = stream.local_addr().unwrap_or(local);
        log::info!("kalango direct: connection from {addr}");
        let server = server.clone();
        tokio::spawn(async move {
            // secure = true: send our signed identity so a client that pinned it (from the invite)
            // encrypts end to end; the tunnel only ever sees ciphertext.
            allow_err!(
                crate::server::create_tcp_connection(
                    server,
                    Stream::from(stream, local_addr),
                    addr,
                    true,
                    ConnectionMeta::default(),
                )
                .await
            );
        });
    }
}
