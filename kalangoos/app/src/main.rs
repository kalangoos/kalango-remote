//! Kalango Remote · KalangoOS's remote-access app (GTK 4). RustDesk's core runs in
//! `kalango-remote-core` (see kalangoos/core and docs/POC.md for why it is a separate process).
//!
//!   kalango-remote                      the window
//!   kalango-remote --page ssh|settings  opening on a page (also for tests and captures)
//!   kalango-remote end-sessions         ends the sessions going on in the running app (KalangoOS's niri runs it
//!                                       on Esc Esc while someone is connected)
//!   kalango-remote agent-plan FILE|-    shows the agent's plan for approval and answers with the decision (plan.rs)
//!   kalango-remote ssh-command HOST-ID  the command that opens a saved SSH server, as a JSON array
//!                                       (for KalangoOS's Terminal; docs/HOSTS.md)

mod access;
mod core;
mod home;
mod i18n;
mod incoming;
mod kalangod;
mod plan;
mod plan_panel;
mod secrets;
mod session;
mod settings;
mod shell_sound;
mod ssh;
mod store;
mod ui;
mod window;

use gtk::{gio, glib, prelude::*};

const APP_ID: &str = "os.kalango.Remote";

/// The first thing the process does: not dumpable. A dumpable process can be ptrace-attached by anything of the
/// same user (Yama's ptrace_scope 1 only limits that to its ancestors) and its memory and environment read through
/// /proc; kalangod, which gives this app and its core the screen, the keyboard and the privacy cover, refuses them
/// unless they are out of that reach, and takes the permission back when they stop being.
fn harden_self() {
    // SAFETY: prctl(PR_SET_DUMPABLE, 0) only changes this process's own flag.
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong);
    }
}

fn main() -> glib::ExitCode {
    harden_self();
    // the Terminal's side of the SSH contract: no window, an answer on stdout
    let args: Vec<String> = std::env::args().skip(1).collect();
    // scripts and tests: import ~/.ssh (config, keys, known_hosts); read a server's key and what the
    // known_hosts say about it (prints `known`, `unknown` or `changed` and the fingerprint)
    if args.first().map(String::as_str) == Some("ssh-import") {
        let mut f = ssh::model::HostsFile::load();
        let (h, k) = ssh::ops::import_openssh(&mut f, &glib::home_dir().join(".ssh"));
        if let Err(e) = f.save() {
            eprintln!("kalango-remote: {e}");
            return glib::ExitCode::FAILURE;
        }
        println!("{h} servidores, {k} chaves conhecidas, {} identidades", f.identities.len());
        return glib::ExitCode::SUCCESS;
    }
    if args.first().map(String::as_str) == Some("ssh-scan") {
        let f = ssh::model::HostsFile::load();
        let Some(h) = args.get(1).and_then(|id| f.host(id)) else { return glib::ExitCode::from(2) };
        return match ssh::ops::scan(&f, h) {
            Ok(k) => {
                let state = match ssh::ops::trust_of(h, &k) {
                    ssh::openssh::Trust::Known => "known",
                    ssh::openssh::Trust::Unknown => "unknown",
                    ssh::openssh::Trust::Changed { .. } => "changed",
                };
                println!("{state} {} {}", ssh::openssh::short_type(&k.key_type), k.fingerprint);
                glib::ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("kalango-remote: {e}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if args.first().map(String::as_str) == Some("ssh-command") {
        let f = ssh::model::HostsFile::load();
        let Some(h) = args.get(1).and_then(|id| f.host(id)) else {
            eprintln!("kalango-remote: unknown server {:?}", args.get(1));
            return glib::ExitCode::from(2);
        };
        return match ssh::ops::command(&f, h, true) {
            Ok(argv) => {
                println!("{}", serde_json::to_string(&argv).unwrap_or_default());
                glib::ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("kalango-remote: {e}");
                glib::ExitCode::FAILURE
            }
        };
    }
    let app = gtk::Application::builder()
        .application_id(std::env::var("KALANGO_REMOTE_APP_ID").unwrap_or_else(|_| APP_ID.into()))
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(|_| ui::load_css());
    app.connect_command_line(|app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().skip(1).filter_map(|a| a.to_str().map(str::to_owned)).collect();
        if args.first().map(String::as_str) == Some("end-sessions") {
            if let Some(h) = window::Window::get().and_then(|w| w.home.borrow().clone()) {
                h.end_sessions();
            }
            return 0;
        }
        if args.first().map(String::as_str) == Some("agent-plan") {
            return window::agent_plan(app, cmd, args.get(1).map(String::as_str));
        }
        let page = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)).cloned();
        let w = window::Window::get_or_create(app);
        if let Some(p) = page {
            w.show_page(&p);
        }
        w.present();
        0
    });
    app.run()
}

#[cfg(test)]
mod harden_tests {
    #[test]
    fn the_process_stops_being_dumpable() {
        use std::os::unix::fs::MetadataExt;
        super::harden_self();
        // the kernel makes the files of a non-dumpable process root's: how kalangod sees it from outside
        assert_eq!(std::fs::metadata("/proc/self/status").map(|m| m.uid()).ok(), Some(0));
        assert!(std::fs::read("/proc/self/environ").is_err() || nix_is_root());
    }

    // the first thing `main` does is `harden_self()`: nothing starts, reads input or opens a socket before that
    #[test]
    fn main_hardens_the_process_first() {
        let src = include_str!("main.rs");
        let body = &src[src.find("fn main() -> glib::ExitCode {").unwrap() + "fn main() -> glib::ExitCode {".len()..];
        let first = body.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("//")).unwrap_or("");
        assert_eq!(first, "harden_self();", "harden_self() must be the first statement of main");
    }
    fn nix_is_root() -> bool {
        // SAFETY: getuid has no preconditions.
        unsafe { libc::getuid() == 0 }
    }
}
