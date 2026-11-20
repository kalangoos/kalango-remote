//! The connecting side of a Cloudflare quick tunnel, without cloudflared: a local TCP port whose
//! connections are carried as binary WebSocket messages to `wss://<tunnel>`; Cloudflare's edge
//! hands them to the sharer's cloudflared, which opens the TCP connection to RustDesk's server.
//! (What `cloudflared access tcp` does; measured equivalent in docs/POC.md.)

use std::net::SocketAddr;

use hbb_common::{
    futures_util::{SinkExt, StreamExt},
    log,
    tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    },
};
use tokio_tungstenite::tungstenite::Message;

/// Listens on 127.0.0.1 (free port) and returns the address; each accepted connection gets its
/// own WebSocket to `host`.
pub async fn start(host: String) -> std::io::Result<SocketAddr> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let addr = listener.local_addr()?;
    hbb_common::tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let host = host.clone();
            hbb_common::tokio::spawn(async move {
                if let Err(e) = pipe(tcp, &host).await {
                    log::warn!("kalango bridge to {host}: {e}");
                }
            });
        }
    });
    Ok(addr)
}

/// A tunnel created seconds ago may not resolve yet (and a resolver may cache the miss for a
/// while), so the first connection keeps trying for up to 30 s before giving up.
async fn connect_with_retry(
    host: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
    Box<dyn std::error::Error + Send + Sync>,
> {
    let start = std::time::Instant::now();
    let mut wait = std::time::Duration::from_millis(500);
    loop {
        match tokio_tungstenite::connect_async(format!("wss://{host}/")).await {
            Ok((ws, _)) => return Ok(ws),
            Err(e) if start.elapsed() < std::time::Duration::from_secs(30) => {
                log::info!("kalango bridge: {host} not reachable yet ({e}), trying again");
                hbb_common::tokio::time::sleep(wait).await;
                wait = (wait * 2).min(std::time::Duration::from_secs(4));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

async fn pipe(mut tcp: TcpStream, host: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tcp.set_nodelay(true).ok();
    let ws = connect_with_retry(host).await?;
    let (mut ws_tx, mut ws_rx) = ws.split();
    let (mut tcp_rx, mut tcp_tx) = tcp.split();
    let up = async {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = tcp_rx.read(&mut buf).await?;
            if n == 0 {
                ws_tx.close().await.ok();
                return Ok::<(), Box<dyn std::error::Error + Send + Sync>>(());
            }
            ws_tx.send(Message::Binary(buf[..n].to_vec().into())).await?;
        }
    };
    let down = async {
        while let Some(msg) = ws_rx.next().await {
            match msg? {
                Message::Binary(b) => tcp_tx.write_all(&b).await?,
                Message::Close(_) => break,
                _ => {}
            }
        }
        tcp_tx.shutdown().await.ok();
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    };
    hbb_common::tokio::select! {
        r = up => r?,
        r = down => r?,
    }
    Ok(())
}
