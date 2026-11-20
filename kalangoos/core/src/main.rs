//! kalango-remote-core · the process that runs RustDesk's core for the Kalango Remote app.
//!
//!   kalango-remote-core serve [--lan] [--no-tunnel] [--ask] [--port P]
//!       Share this computer. Without --no-tunnel, opens a Cloudflare quick tunnel to a
//!       loopback-only listener; --lan listens on every address instead (same network) and answers
//!       the share code over UDP. --ask: a right password still needs the approval of whoever is
//!       here. Events (JSON packets on stdout): listening, tunnel, tunnel_error, connection,
//!       removed, chat. Commands (JSON packets on stdin): authorize, close, chat, new_password.
//!   kalango-remote-core session <link|code|host:port> --password PW [--view-only|--files]
//!       Control another computer. Events and frames on stdout; commands (mouse, key, text, chat,
//!       quality, close) on stdin. Ends when stdin closes.
//!   kalango-remote-core probe <link|code|host:port> --password PW [--frames N] [--save F.png]
//!       A session without a window, for tests: prints the events, counts frames, exits 0 after N.
//!
//! Packets are `kalango_remote_proto::wire`.

mod bridge;
mod lan;
mod tunnel;

use std::{
    io::Write,
    net::SocketAddr,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

use hbb_common::{config::Config, log, password_security};
use kalango_remote_proto::{
    wire::{self, Packet},
    Invite, Target,
};
use librustdesk::kalango::{self, cm, session};
use serde_json::{json, Value};

type Out = Arc<Mutex<std::io::Stdout>>;

/// `KALANGO_CORE_TEXT=1`: events as JSON lines (tests and people reading along) instead of packets.
fn text_mode() -> bool {
    static T: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *T.get_or_init(|| std::env::var("KALANGO_CORE_TEXT").as_deref() == Ok("1"))
}

fn emit(out: &Out, v: Value) {
    let mut o = out.lock().unwrap();
    let res = if text_mode() { writeln!(o, "{v}").and_then(|_| o.flush()) } else { wire::write_json(&mut *o, &v) };
    if res.is_err() {
        // the app is gone; nothing left to do for
        std::process::exit(0);
    }
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    hbb_common::env_logger::Builder::from_env(hbb_common::env_logger::Env::default().default_filter_or("info"))
        .target(hbb_common::env_logger::Target::Stderr)
        .init();
    // tokio-tungstenite's rustls needs a process-wide crypto provider
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    // KALANGO_REMOTE_INSTANCE: a second instance under the same user (tests that run both ends on
    // one machine) gets its own config and IPC sockets
    match std::env::var("KALANGO_REMOTE_INSTANCE") {
        Ok(i) if !i.is_empty() && i.chars().all(|c| c.is_ascii_alphanumeric()) => kalango::set_app_name(&format!("kalango-remote-{i}")),
        _ => kalango::set_app_name("kalango-remote"),
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("serve") => {
            // on niri: capture and input through wlr protocols (before anything checks for X11)
            if kalango::use_wlr_if_available() {
                log::info!("serve: wlr capture and input");
            }
            serve(&args[1..])
        }
        Some("session") => client(&args[1..], false),
        Some("probe") => client(&args[1..], true),
        _ => {
            eprintln!("usage: kalango-remote-core serve|session|probe … (see the source header)");
            2
        }
    };
    std::process::exit(code);
}

/// Commands from the app on stdin (JSON packets, or JSON lines in text mode) until it closes.
fn commands() -> impl Iterator<Item = Value> {
    let mut stdin = std::io::stdin().lock();
    std::iter::from_fn(move || loop {
        if text_mode() {
            let mut line = String::new();
            if std::io::BufRead::read_line(&mut stdin, &mut line).ok()? == 0 {
                return None;
            }
            if let Ok(v) = serde_json::from_str(&line) {
                return Some(v);
            }
        } else {
            match wire::read_packet(&mut stdin) {
                Ok(Some(Packet::Json(v))) => return Some(v),
                Ok(Some(_)) => continue,
                _ => return None,
            }
        }
    })
}

// ---------------------------------------------------------------------------------------------
// sharing

fn serve(args: &[String]) -> i32 {
    let out: Out = Arc::new(Mutex::new(std::io::stdout()));
    let lan = args.iter().any(|a| a == "--lan");
    let use_tunnel = !args.iter().any(|a| a == "--no-tunnel");
    let ask = args.iter().any(|a| a == "--ask");
    let port: u16 = arg_value(args, "--port").and_then(|p| p.parse().ok()).unwrap_or(if lan { 21118 } else { 0 });

    // one-time password: RustDesk's temporary password, renewed after each session
    Config::set_option("verification-method".into(), "use-temporary-password".into());
    Config::set_option("approve-mode".into(), "password".into());
    Config::set_option(kalango::OPTION_APPROVE_AFTER_PASSWORD.into(), if ask { "Y" } else { "" }.into());
    if password_security::temporary_password().is_empty() {
        password_security::update_temporary_password();
    }

    let (cm_tx, cm_rx) = mpsc::channel();
    cm::start(cm_tx);

    let (id, pk) = kalango::identity();
    let pk: [u8; 32] = match pk.try_into() {
        Ok(pk) => pk,
        Err(_) => {
            emit(&out, json!({"event": "error", "text": "no signing key"}));
            return 1;
        }
    };
    let code = kalango_remote_proto::code_for(&pk);

    let (bound_tx, bound_rx) = mpsc::channel::<SocketAddr>();
    std::thread::spawn(move || {
        let opts = kalango::direct::DirectOptions { port, loopback_only: !lan };
        if let Err(e) = kalango::direct::run(opts, move |a| {
            bound_tx.send(a).ok();
        }) {
            log::error!("direct server: {e}");
            std::process::exit(3);
        }
    });
    let Ok(bound) = bound_rx.recv_timeout(Duration::from_secs(20)) else {
        emit(&out, json!({"event": "error", "text": "the server did not start"}));
        return 1;
    };
    emit(
        &out,
        json!({
            "event": "listening", "addr": bound.to_string(), "id": id, "code": code,
            "pk": kalango_remote_proto::invite::b64url_encode(&pk),
            "password": password_security::temporary_password(),
        }),
    );
    if lan {
        lan::respond(code.clone(), bound.port(), id.clone(), pk);
    }

    let _tunnel = if use_tunnel {
        match tunnel::cloudflared_path() {
            None => {
                emit(&out, json!({"event": "tunnel_error", "kind": "missing", "text": "cloudflared not found"}));
                None
            }
            Some(bin) => {
                let o = out.clone();
                let (id2, pk2) = (id.clone(), pk);
                let version = tunnel::cloudflared_version(&bin);
                emit(&out, json!({"event": "tunnel_starting", "version": version}));
                match tunnel::Tunnel::start(&bin, bound.port(), move |line| {
                    log::debug!("cloudflared: {line}");
                    if let Some(host) = tunnel::parse_tunnel_url(line) {
                        // announce the link once its name resolves (a fresh tunnel takes a few
                        // seconds in DNS), so whoever gets it can connect right away
                        let o = o.clone();
                        let id = id2.clone();
                        std::thread::spawn(move || {
                            let start = Instant::now();
                            while start.elapsed() < Duration::from_secs(30)
                                && std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), 443)).is_err()
                            {
                                std::thread::sleep(Duration::from_millis(500));
                            }
                            let inv = Invite { host: host.clone(), identity: Some((id, pk2)) };
                            emit(&o, json!({"event": "tunnel", "host": host, "link": inv.link()}));
                        });
                    } else if line.contains("Registered tunnel connection") {
                        emit(&o, json!({"event": "tunnel_ready"}));
                    } else if let Some(err) = tunnel::parse_tunnel_error(line) {
                        emit(&o, json!({"event": "tunnel_error", "kind": "failed", "text": err}));
                    }
                }) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        emit(&out, json!({"event": "tunnel_error", "kind": "spawn", "text": e.to_string()}));
                        None
                    }
                }
            }
        }
    } else {
        None
    };

    // commands from the app
    let o = out.clone();
    std::thread::spawn(move || {
        for c in commands() {
            let id = c["id"].as_i64().unwrap_or(0) as i32;
            match c["cmd"].as_str().unwrap_or("") {
                "authorize" => {
                    for perm in ["keyboard", "clipboard", "audio", "file"] {
                        if let Some(v) = c[perm].as_bool() {
                            cm::switch_permission(id, perm.to_owned(), v);
                        }
                    }
                    cm::authorize(id);
                }
                "close" => cm::close(id),
                "chat" => cm::send_chat(id, c["text"].as_str().unwrap_or("").to_owned()),
                "new_password" => {
                    password_security::update_temporary_password();
                    emit(&o, json!({"event": "password", "password": password_security::temporary_password()}));
                }
                other => log::warn!("serve: unknown command {other}"),
            }
        }
        // the app closed: stop sharing
        std::process::exit(0);
    });

    for ev in cm_rx {
        if let Ok(v) = serde_json::to_value(&ev) {
            emit(&out, v);
        }
        // a finished session renews the one-time password
        if let cm::CmEvent::Removed { .. } = ev {
            emit(&out, json!({"event": "password", "password": password_security::temporary_password()}));
        }
    }
    0
}

// ---------------------------------------------------------------------------------------------
// controlling

/// Turns what the person pasted into a direct address RustDesk can dial, pinning the identity.
fn resolve(target: &str, out: &Out) -> Result<(String, bool), String> {
    match Target::parse(target).map_err(|e| format!("{e:?}"))? {
        Target::Code(code) => {
            emit(out, json!({"event": "step", "step": "searching_lan", "code": code}));
            match lan::find(&code, Duration::from_secs(3)).map_err(|e| e.to_string())? {
                Some(f) => {
                    let addr = f.addr.to_string();
                    kalango::pin_peer(&addr, &f.id, f.pk);
                    Ok((addr, true))
                }
                None => Err("code_not_found".into()),
            }
        }
        Target::Invite(inv) => {
            let addr = if inv.is_tunnel() {
                emit(out, json!({"event": "step", "step": "tunnel", "host": inv.host}));
                let host = inv.host.clone();
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let rt = hbb_common::tokio::runtime::Runtime::new().expect("tokio runtime");
                    rt.block_on(async move {
                        tx.send(bridge::start(host).await).ok();
                        std::future::pending::<()>().await;
                    });
                });
                rx.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?.to_string()
            } else {
                inv.host.clone()
            };
            if let Some((id, pk)) = &inv.identity {
                kalango::pin_peer(&addr, id, *pk);
            }
            Ok((addr, inv.identity.is_some()))
        }
    }
}

fn client(args: &[String], probe: bool) -> i32 {
    let out: Out = Arc::new(Mutex::new(std::io::stdout()));
    let Some(target) = args.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("missing target");
        return 2;
    };
    let password = arg_value(args, "--password").unwrap_or_default();
    let kind = if args.iter().any(|a| a == "--files") {
        session::Kind::Files
    } else if args.iter().any(|a| a == "--view-only") {
        session::Kind::ViewOnly
    } else {
        session::Kind::Control
    };
    let (addr, pinned) = match resolve(target, &out) {
        Ok(r) => r,
        Err(e) => {
            emit(&out, json!({"event": "error", "kind": e}));
            return 1;
        }
    };
    emit(&out, json!({"event": "resolved", "addr": addr, "pinned": pinned}));
    let (tx, rx) = mpsc::channel();
    let s = session::start(&addr, &password, kind, tx);

    if probe {
        return run_probe(s, rx, args, &out);
    }

    // events
    let o = out.clone();
    std::thread::spawn(move || {
        for ev in rx {
            if let Ok(v) = serde_json::to_value(&ev) {
                emit(&o, v);
            }
        }
    });
    // frames
    let frames = s.ui_handler.frames.clone();
    let o = out.clone();
    std::thread::spawn(move || loop {
        if let Some(f) = frames.take(Duration::from_millis(500)) {
            let mut w = o.lock().unwrap();
            if wire::write_frame(&mut *w, f.display as u32, f.width as u32, f.height as u32, f.stride as u32, &f.bgra).is_err() {
                std::process::exit(0);
            }
        }
    });
    // commands
    for c in commands() {
        let b = |k: &str| c[k].as_bool().unwrap_or(false);
        let i = |k: &str| c[k].as_i64().unwrap_or(0) as i32;
        let st = |k: &str| c[k].as_str().unwrap_or("").to_owned();
        match c["cmd"].as_str().unwrap_or("") {
            "mouse" => s.send_mouse(i("mask"), i("x"), i("y"), b("alt"), b("ctrl"), b("shift"), b("command")),
            "key" => s.input_key(&st("name"), b("down"), b("press"), b("alt"), b("ctrl"), b("shift"), b("command")),
            "text" => s.input_string(&st("text")),
            "chat" => s.send_chat(st("text")),
            "quality" => s.save_image_quality(st("value")),
            "toggle" => s.toggle_option(st("name")),
            "refresh" => s.refresh_video(-1),
            "close" => break,
            other => log::warn!("session: unknown command {other}"),
        }
    }
    s.close();
    std::thread::sleep(Duration::from_millis(300));
    0
}

/// `--click X,Y` and `--type TEXT` (lowercase ASCII): input sent once the first frame arrived, so a
/// test can check that control reaches the shared computer.
fn send_probe_input(s: &session::KalangoSession, args: &[String]) {
    if let Some((x, y)) = arg_value(args, "--click").and_then(|v| v.split_once(',').map(|(a, b)| (a.parse().ok(), b.parse().ok()))) {
        if let (Some(x), Some(y)) = (x, y) {
            // RustDesk's mouse masks: type in the low 3 bits (1 down, 2 up), button << 3 (1 = left)
            s.send_mouse(0, x, y, false, false, false, false);
            std::thread::sleep(Duration::from_millis(100));
            s.send_mouse(1 | (1 << 3), x, y, false, false, false, false);
            s.send_mouse(2 | (1 << 3), x, y, false, false, false, false);
            println!("{}", json!({"probe": "clicked", "x": x, "y": y}));
            std::thread::sleep(Duration::from_millis(300));
        }
    }
    if let Some(text) = arg_value(args, "--type") {
        for c in text.chars() {
            let name = c.to_string();
            s.input_key(&name, true, false, false, false, false, false);
            s.input_key(&name, false, false, false, false, false, false);
            std::thread::sleep(Duration::from_millis(20));
        }
        println!("{}", json!({"probe": "typed", "chars": text.chars().count()}));
    }
}

fn run_probe(s: session::KalangoSession, rx: mpsc::Receiver<session::Event>, args: &[String], out: &Out) -> i32 {
    let want: usize = arg_value(args, "--frames").and_then(|n| n.parse().ok()).unwrap_or(30);
    let save = arg_value(args, "--save");
    let timeout = Duration::from_secs(arg_value(args, "--timeout").and_then(|n| n.parse().ok()).unwrap_or(60));
    let start = Instant::now();
    let mut first: Option<Instant> = None;
    let mut n = 0usize;
    while start.elapsed() < timeout && n < want {
        while let Ok(ev) = rx.try_recv() {
            let v = serde_json::to_value(&ev).unwrap_or_default();
            println!("{v}");
            if matches!(ev, session::Event::Message { ref msgtype, .. } if msgtype.contains("error")) {
                return 1;
            }
        }
        if let Some(f) = s.ui_handler.frames.take(Duration::from_millis(200)) {
            n += 1;
            let t = *first.get_or_insert_with(Instant::now);
            if n == 1 {
                send_probe_input(&s, args);
                println!("{}", json!({"probe": "first_frame", "width": f.width, "height": f.height, "after_ms": start.elapsed().as_millis()}));
                if let Some(path) = &save {
                    // BGRA rows with padding → tightly packed RGBA
                    let mut rgba = Vec::with_capacity(f.width * f.height * 4);
                    for row in f.bgra.chunks(f.stride).take(f.height) {
                        for px in row[..f.width * 4].chunks_exact(4) {
                            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                        }
                    }
                    if let Ok(mut file) = std::fs::File::create(path) {
                        repng::encode(&mut file, f.width as u32, f.height as u32, &rgba).ok();
                    }
                }
            }
            if n == want {
                let secs = t.elapsed().as_secs_f64().max(0.001);
                println!("{}", json!({"probe": "done", "frames": n, "fps_after_first": ((n - 1) as f64 / secs * 10.0).round() / 10.0, "dropped": s.ui_handler.frames.dropped.load(std::sync::atomic::Ordering::Relaxed)}));
            }
        }
    }
    let _ = std::io::stdout().flush();
    let _ = out;
    s.close();
    std::thread::sleep(Duration::from_millis(300));
    if n >= want {
        0
    } else {
        println!("{}", json!({"probe": "timeout", "frames": n}));
        1
    }
}
