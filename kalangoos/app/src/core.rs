//! The core process (`kalango-remote-core`, RustDesk's core) seen from the window: spawn it, read its
//! packets on a thread and hand them to the GTK main loop, write commands to it.

use std::{
    io::{BufRead, BufReader, Write},
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
    /// the process ended
    Exited(Exit),
}

/// How the core ended.
#[derive(Debug, Default)]
pub struct Exit {
    /// its exit status, when known
    pub code: Option<i32>,
    /// the first line it wrote to stderr (the cause when it could not even start, e.g. a missing library)
    pub first_error: String,
    /// the last line it wrote to stderr (the cause when it stopped later)
    pub last_error: String,
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
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
        // stderr still reaches ours line by line; the first line is kept for the error state
        let first_error = Arc::new(Mutex::new(None::<String>));
        let last_error = Arc::new(Mutex::new(String::new()));
        if let Some(err) = child.stderr.take() {
            let first = first_error.clone();
            let last = last_error.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    // never eprintln! here: if our stderr breaks it panics, this thread ends, and the core
                    // dies of SIGPIPE at its next log line
                    let _ = writeln!(std::io::stderr(), "{line}");
                    if line.trim().is_empty() {
                        continue;
                    }
                    let mut f = first.lock().unwrap();
                    if f.is_none() {
                        *f = Some(line.clone());
                    }
                    *last.lock().unwrap() = line;
                }
            });
        }
        let child = Arc::new(Mutex::new(Some(child)));
        let reaped = child.clone();
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
            // reap it (no zombie), unless stop() already took it to do the same
            let taken = reaped.lock().unwrap().take();
            let code = taken.and_then(|mut c| c.wait().ok()).and_then(|st| st.code());
            // the stderr thread may still be on its last line
            for _ in 0..10 {
                if first_error.lock().unwrap().is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let first_error = first_error.lock().unwrap().clone().unwrap_or_default();
            let last_error = last_error.lock().unwrap().clone();
            tx.send_blocking(Msg::Exited(Exit { code, first_error, last_error })).ok();
        });
        glib::spawn_future_local(async move {
            while let Ok(m) = rx.recv().await {
                on_msg(m);
            }
        });
        Ok(Core { stdin: Arc::new(Mutex::new(stdin)), child })
    }

    /// The process id (for kalangod's permission), while it runs.
    pub fn pid(&self) -> Option<u32> {
        self.child.lock().unwrap().as_ref().map(|c| c.id())
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
