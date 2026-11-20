//! The core process (`kalango-remote-core`, RustDesk's core) seen from the window: spawn it, read its
//! packets on a thread and hand them to the GTK main loop, write commands to it.

use std::{
    io::Write,
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
};

use gtk::glib;
use kalango_remote_proto::wire::{self, Frame, Packet};
use serde_json::Value;

pub enum Msg {
    Event(Value),
    Frame(Frame),
    /// the process ended (its exit status, when known)
    Exited(Option<i32>),
}

/// Where the core is: `KALANGO_REMOTE_CORE`, or next to this program.
pub fn core_path() -> PathBuf {
    if let Some(p) = std::env::var_os("KALANGO_REMOTE_CORE") {
        return PathBuf::from(p);
    }
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("kalango-remote-core")))
        .unwrap_or_else(|| PathBuf::from("kalango-remote-core"))
}

#[derive(Clone)]
pub struct Core {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Core {
    /// Starts `kalango-remote-core ARGS`; `on_msg` runs on the main loop for every packet.
    pub fn spawn(args: &[String], on_msg: impl Fn(Msg) + 'static) -> std::io::Result<Core> {
        let mut child = Command::new(core_path())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
        let (tx, rx) = async_channel::bounded::<Msg>(64);
        std::thread::spawn(move || {
            loop {
                match wire::read_packet(&mut stdout) {
                    Ok(Some(Packet::Json(v))) => {
                        if tx.send_blocking(Msg::Event(v)).is_err() {
                            break;
                        }
                    }
                    Ok(Some(Packet::Frame(f))) => {
                        // a full channel means the window is behind: drop this frame, keep the newest
                        if tx.is_full() {
                            continue;
                        }
                        if tx.send_blocking(Msg::Frame(f)).is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            tx.send_blocking(Msg::Exited(None)).ok();
        });
        glib::spawn_future_local(async move {
            while let Ok(m) = rx.recv().await {
                on_msg(m);
            }
        });
        Ok(Core { stdin: Arc::new(Mutex::new(stdin)), child: Arc::new(Mutex::new(Some(child))) })
    }

    pub fn send(&self, cmd: &Value) {
        if let Some(w) = self.stdin.lock().unwrap().as_mut() {
            let _ = wire::write_json(w, cmd).and_then(|_| w.flush());
        }
    }

    /// Ends the process: closing its stdin tells it the window is gone; a kill follows if needed.
    pub fn stop(&self) {
        self.stdin.lock().unwrap().take();
        if let Some(mut c) = self.child.lock().unwrap().take() {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(800));
                if matches!(c.try_wait(), Ok(None)) {
                    let _ = c.kill();
                }
                let _ = c.wait();
            });
        }
    }
}
