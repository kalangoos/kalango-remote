//! A session controlling another computer: the core process (`kalango-remote-core session`), and its
//! window — the remote screen fitted to the window, a thin title bar (who, by which route, latency,
//! for how long), the floating toolbar that hides itself, chat, file transfer, and the lost-connection
//! state that reconnects by itself.

use gtk::{gdk, glib, prelude::*, Align, Orientation};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use crate::{
    core::{Core, Msg},
    i18n::{tr, trf},
    store::{self, Peer},
    ui,
    window::Window,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Control,
    ViewOnly,
    Files,
}

pub enum ConnectEvent {
    Waiting(String),
    Connected,
    WrongPassword,
    Failed(String),
}

/// What a RustDesk message box means for the person connecting.
pub fn classify_message(msgtype: &str, title: &str, text: &str) -> Option<ConnectEvent> {
    let t = text.to_lowercase();
    if msgtype.contains("password") || t.contains("wrong password") {
        return Some(ConnectEvent::WrongPassword);
    }
    if msgtype.starts_with("wait") || t.contains("accept") {
        return Some(ConnectEvent::Waiting(tr("Waiting for the other person to approve").to_owned()));
    }
    if msgtype.contains("error") {
        if t.contains("closed manually") || t.contains("denied") || t.contains("rejected") {
            return Some(ConnectEvent::Failed(tr("The person denied the access.").to_owned()));
        }
        if t.contains("identity does not match") {
            return Some(ConnectEvent::Failed(
                tr("This is not the computer of the invite (its key is different). The connection was refused.").to_owned(),
            ));
        }
        if t.contains("reset by the peer") || t.contains("refused") || t.contains("timed out") || t.contains("timeout") || t.contains("failed") {
            return Some(ConnectEvent::Failed(
                tr("This link expired or the person stopped sharing. Ask for a new code.").to_owned(),
            ));
        }
        return Some(ConnectEvent::Failed(format!("{title}: {text}")));
    }
    None
}

struct View {
    window: gtk::Window,
    picture: gtk::Picture,
    screen: gtk::Box,
    route: gtk::Box,
    route_label: gtk::Label,
    route_icon: gtk::Image,
    name: gtk::Label,
    clock: gtk::Label,
    stats: gtk::Label,
    toolbar: gtk::Revealer,
    handle: gtk::Box,
    quality_label: gtk::Label,
    scale_label: gtk::Label,
    chat: gtk::Revealer,
    chat_list: gtk::Box,
    chat_scroll: gtk::ScrolledWindow,
    weak: gtk::Revealer,
    weak_label: gtk::Label,
    lost: gtk::Box,
    lost_label: gtk::Label,
    transfer: gtk::Revealer,
    files: RefCell<Option<Rc<Transfer>>>,
}

pub struct Session {
    main: Weak<Window>,
    target: String,
    password: String,
    kind: Kind,
    core: RefCell<Option<Core>>,
    view: RefCell<Option<Rc<View>>>,
    on_event: Box<dyn Fn(ConnectEvent)>,
    connected: Cell<bool>,
    started: Cell<Option<Instant>>,
    frame_size: Cell<(i32, i32)>,
    last_frame: RefCell<Option<gdk::Texture>>,
    peer_name: RefCell<String>,
    route_lan: Cell<bool>,
    delay: Cell<i32>,
    lowered: Cell<bool>,
    scale: RefCell<String>,
    retries: Cell<u32>,
    last_motion: Cell<Option<Instant>>,
}

impl Session {
    pub fn start(main: Weak<Window>, target: &str, password: &str, kind: Kind, on_event: impl Fn(ConnectEvent) + 'static) -> Rc<Session> {
        let scale = main.upgrade().map(|w| w.settings.borrow().scale.clone()).unwrap_or_else(|| "fit".into());
        let s = Rc::new(Session {
            main,
            target: target.to_owned(),
            password: password.to_owned(),
            kind,
            core: RefCell::new(None),
            view: RefCell::new(None),
            on_event: Box::new(on_event),
            connected: Cell::new(false),
            started: Cell::new(None),
            frame_size: Cell::new((0, 0)),
            last_frame: RefCell::new(None),
            peer_name: RefCell::new(String::new()),
            route_lan: Cell::new(false),
            delay: Cell::new(0),
            lowered: Cell::new(false),
            scale: RefCell::new(scale),
            retries: Cell::new(0),
            last_motion: Cell::new(None),
        });
        s.spawn();
        s
    }

    fn spawn(self: &Rc<Session>) {
        let mut args = vec!["session".to_string(), self.target.clone(), "--password".into(), self.password.clone()];
        match self.kind {
            Kind::ViewOnly => args.push("--view-only".into()),
            Kind::Files => args.push("--files".into()),
            Kind::Control => {}
        }
        if matches!(kalango_remote_proto::Target::parse(&self.target), Ok(kalango_remote_proto::Target::Id(_))) {
            // a RustDesk id is reached through the own server of Settings › Network
            let s = self.main.upgrade().map(|w| w.settings.borrow().clone()).unwrap_or_default();
            if s.server_id.trim().is_empty() || s.server_key.trim().is_empty() {
                (self.on_event)(ConnectEvent::Failed(tr("Set your own server (address and public key) in Settings › Network to connect by id.").to_owned()));
                return;
            }
            args.extend(["--server".into(), s.server_id.trim().to_owned(), "--relay".into(), s.server_relay.trim().to_owned(), "--key".into(), s.server_key.trim().to_owned()]);
        }
        if self.auto_record() {
            args.extend(["--record-dir".into(), crate::access::recordings_dir().to_string_lossy().into_owned()]);
        }
        let me = Rc::downgrade(self);
        match Core::spawn(&args, move |m| {
            if let Some(s) = me.upgrade() {
                s.on_core(m);
            }
        }) {
            Ok(c) => *self.core.borrow_mut() = Some(c),
            Err(e) => (self.on_event)(ConnectEvent::Failed(e.to_string())),
        }
    }

    /// The shared computer asks for its authenticator code (two-step verification): a small window over this
    /// app, the 6 digits go to the core; cancelling ends the attempt.
    fn ask_two_factor(self: &Rc<Session>, wrong: bool) {
        let dialog = gtk::Window::builder().title(tr("Verification code")).resizable(false).modal(true).build();
        dialog.add_css_class("kr-window");
        dialog.add_css_class("kr-dialog");
        if let Some(w) = self.main.upgrade() {
            dialog.set_transient_for(Some(&w.window));
        }
        let no_titlebar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        no_titlebar.set_visible(false);
        dialog.set_titlebar(Some(&no_titlebar));
        let root = ui::vbox(12, "kr-dialog-body");
        root.set_size_request(380, -1);
        let head = ui::hbox(8, "");
        head.append(&ui::icon("escudo", "k_accent", 16));
        head.append(&ui::label(tr("Two-step verification"), "kr-big-title"));
        root.append(&head);
        root.append(&ui::para(tr("This computer asks for the code of its authenticator app. Enter the 6 digits it shows now."), "kr-caption"));
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(tr("6-digit code")));
        entry.set_max_length(7);
        entry.set_input_purpose(gtk::InputPurpose::Digits);
        root.append(&entry);
        let err = ui::label(if wrong { tr("That code is not right. Try the next one.") } else { "" }, "kr-error-text");
        root.append(&err);
        let row = ui::hbox(8, "");
        row.append(&ui::spacer());
        let cancel = ui::button(tr("Cancel"), None, "ghost");
        let ok = ui::button(tr("Confirm"), None, "primary");
        row.append(&cancel);
        row.append(&ok);
        root.append(&row);
        dialog.set_child(Some(&root));
        let (me, d) = (Rc::downgrade(self), dialog.clone());
        let e = entry.clone();
        let send = move || {
            let code: String = e.text().chars().filter(|c| c.is_ascii_digit()).collect();
            if code.len() != 6 {
                err.set_text(tr("Enter the 6 digits."));
                return;
            }
            if let Some(s) = me.upgrade() {
                s.send(json!({"cmd": "2fa", "code": code}));
            }
            d.destroy();
        };
        let s2 = send.clone();
        ok.connect_clicked(move |_| s2());
        entry.connect_activate(move |_| send());
        let (me, d) = (Rc::downgrade(self), dialog.clone());
        cancel.connect_clicked(move |_| {
            d.destroy();
            if let Some(s) = me.upgrade() {
                (s.on_event)(ConnectEvent::Failed(tr("Cancelled: the code of the authenticator app was not entered.").to_owned()));
                s.close();
            }
        });
        dialog.present();
        entry.grab_focus();
    }

    /// "Record every session" is on, so this one is recorded from its first frame.
    fn auto_record(&self) -> bool {
        self.main.upgrade().is_some_and(|w| w.settings.borrow().record)
    }

    fn send(&self, v: Value) {
        if let Some(c) = self.core.borrow().as_ref() {
            c.send(&v);
        }
    }

    pub fn close(&self) {
        if let Some(w) = self.main.upgrade() {
            let me = self as *const Session;
            w.sessions.borrow_mut().retain(|s| !std::ptr::eq(Rc::as_ptr(s), me));
        }
        self.save_thumb();
        if let Some(c) = self.core.borrow_mut().take() {
            c.send(&json!({"cmd": "close"}));
            c.stop();
        }
        if let Some(v) = self.view.borrow_mut().take() {
            v.window.destroy();
        }
    }

    fn on_core(self: &Rc<Session>, m: Msg) {
        match m {
            Msg::Frame(f) => self.on_frame(f),
            Msg::Exited(_) => {
                self.core.borrow_mut().take();
                if self.connected.get() {
                    self.lost();
                }
            }
            Msg::Event(v) => self.on_event_json(&v),
        }
    }

    fn on_event_json(self: &Rc<Session>, v: &Value) {
        let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
        match v["event"].as_str().unwrap_or("") {
            "error" => {
                let kind = s("kind");
                (self.on_event)(ConnectEvent::Failed(if kind == "code_not_found" {
                    tr("Nobody on this network is sharing with that code. Whoever shares needs to turn on “Also accept connections on the same network” or choose “Local network only, direct”; outside the network, ask for the link.").to_owned()
                } else {
                    kind
                }));
            }
            "resolved" => self.route_lan.set(!s("addr").starts_with("127.0.0.1")),
            "peer_info" => {
                let host = s("hostname");
                *self.peer_name.borrow_mut() = if host.is_empty() { self.target.clone() } else { host };
                if let Some(v) = self.view.borrow().as_ref() {
                    v.name.set_text(&self.peer_name.borrow());
                }
            }
            "quality" => {
                if let Some(d) = v["delay"].as_i64() {
                    self.delay.set(d as i32);
                }
                self.update_route(v["fps"].as_i64());
            }
            "message" if s("msgtype") == "input-2fa" => self.ask_two_factor(s("title") == "Wrong 2FA Code"),
            "message" => {
                if let Some(ev) = classify_message(&s("msgtype"), &s("title"), &s("text")) {
                    if self.connected.get() {
                        if matches!(ev, ConnectEvent::Failed(_)) {
                            self.lost();
                        }
                    } else {
                        let failed = !matches!(ev, ConnectEvent::Waiting(_));
                        (self.on_event)(ev);
                        if failed {
                            self.close();
                        }
                    }
                }
            }
            "chat" => self.chat_message(&s("text"), false),
            "closed" if self.connected.get() => self.lost(),
            "folder_files" => {
                if let Some(t) = self.view.borrow().as_ref().and_then(|v| v.files.borrow().clone()) {
                    t.on_remote_dir(v);
                }
            }
            "job_progress" | "job_done" | "job_error" | "job_started" => {
                if let Some(t) = self.view.borrow().as_ref().and_then(|v| v.files.borrow().clone()) {
                    t.on_job(v);
                }
            }
            _ => {}
        }
    }

    fn on_frame(self: &Rc<Session>, f: kalango_remote_proto::wire::Frame) {
        let bytes = glib::Bytes::from_owned(f.bgra);
        let tex: gdk::Texture =
            gdk::MemoryTexture::new(f.width as i32, f.height as i32, gdk::MemoryFormat::B8g8r8a8, &bytes, f.stride as usize).upcast();
        self.frame_size.set((f.width as i32, f.height as i32));
        if !self.connected.get() {
            self.connected.set(true);
            self.retries.set(0);
            self.started.set(Some(Instant::now()));
            self.build_view();
            if let Some(w) = self.main.upgrade() {
                w.sessions.borrow_mut().push(self.clone());
            }
            (self.on_event)(ConnectEvent::Connected);
            self.remember();
        }
        if let Some(v) = self.view.borrow().as_ref() {
            v.picture.set_paintable(Some(&tex));
            v.lost.set_visible(false);
            v.picture.set_opacity(1.0);
        }
        *self.last_frame.borrow_mut() = Some(tex);
    }

    fn remember(&self) {
        let Some(w) = self.main.upgrade() else { return };
        let code = match kalango_remote_proto::Target::parse(&self.target) {
            Ok(kalango_remote_proto::Target::Invite(i)) => i.code().unwrap_or_default(),
            Ok(kalango_remote_proto::Target::Code(c)) => c,
            Ok(kalango_remote_proto::Target::Id(_)) => String::new(),
            Err(_) => String::new(),
        };
        w.peers.borrow_mut().remember(Peer {
            name: self.peer_name.borrow().clone(),
            target: self.target.clone(),
            code,
            route: if self.route_lan.get() { "lan".into() } else { "cloudflare".into() },
            last_used: store::now(),
            latency_ms: self.delay.get().max(0) as u32,
            favorite: false,
            view_only: self.kind == Kind::ViewOnly,
            thumb: None,
        });
        w.peers.borrow().save();
    }

    /// The last frame, small, as the device card's picture.
    fn save_thumb(&self) {
        let (Some(tex), Some(w)) = (self.last_frame.borrow().clone(), self.main.upgrade()) else { return };
        let dir = store::cache_dir().join("thumbs");
        let _ = std::fs::create_dir_all(&dir);
        let name: String = self.target.chars().filter(|c| c.is_ascii_alphanumeric()).take(48).collect();
        let path: PathBuf = dir.join(format!("{name}.png"));
        let bytes = tex.save_to_png_bytes();
        if let Ok(pb) = gtk::gdk_pixbuf::Pixbuf::from_stream(&gtk::gio::MemoryInputStream::from_bytes(&bytes), gtk::gio::Cancellable::NONE) {
            let h = 117 * 2;
            let wpx = (pb.width() * h / pb.height().max(1)).max(1);
            if let Some(small) = pb.scale_simple(wpx, h, gtk::gdk_pixbuf::InterpType::Bilinear) {
                let _ = small.savev(&path, "png", &[]);
            }
        }
        let mut peers = w.peers.borrow_mut();
        if let Some(p) = peers.list.iter_mut().find(|p| p.target == self.target) {
            p.thumb = Some(path);
            p.last_used = store::now();
            p.latency_ms = self.delay.get().max(0) as u32;
        }
        peers.save();
        drop(peers);
        let home = w.home.borrow().clone();
        if let Some(h) = home {
            h.refresh_devices();
        }
    }

    fn update_route(&self, fps: Option<i64>) {
        let Some(v) = self.view.borrow().clone() else { return };
        let d = self.delay.get();
        let route = if self.route_lan.get() { tr("on the same network") } else { tr("via Cloudflare") };
        v.route_label.set_text(&format!("{route} · {d} ms"));
        let weak = d > 200;
        for c in ["teal", "gold", "blue"] {
            v.route.remove_css_class(c);
        }
        v.route.add_css_class(if weak { "gold" } else if self.route_lan.get() { "blue" } else { "teal" });
        ui::set_icon(&v.route_icon, if weak { "wifi-fraco" } else { "wifi" }, if weak { "k_gold" } else if self.route_lan.get() { "k_blue" } else { "k_teal" }, 16);
        let fps = fps.map(|f| f.to_string()).unwrap_or_else(|| "—".into());
        v.stats.set_text(&trf("{d} ms · {f} fps", &[("d", d.to_string()), ("f", fps)]));
        // weak connection: lower the quality once, say so, offer the local network
        if weak && !self.lowered.get() {
            self.lowered.set(true);
            self.send(json!({"cmd": "quality", "value": "low"}));
            v.quality_label.set_text(tr("Fast response"));
            v.weak_label.set_text(&trf("Weak connection ({d} ms). I lowered the quality so the picture does not freeze.", &[("d", d.to_string())]));
            v.weak.set_reveal_child(true);
            let w = Rc::downgrade(&v);
            glib::timeout_add_local_once(Duration::from_secs(6), move || {
                if let Some(v) = w.upgrade() {
                    v.weak.set_reveal_child(false);
                }
            });
        }
    }

    fn lost(self: &Rc<Session>) {
        let Some(v) = self.view.borrow().clone() else { return };
        v.picture.set_opacity(0.3);
        v.lost.set_visible(true);
        let n = self.retries.get() + 1;
        if n > 5 {
            v.lost_label.set_text(tr("Could not reconnect."));
            return;
        }
        self.retries.set(n);
        v.lost_label.set_text(&trf("Trying again in 3 s ({n} of 5)", &[("n", n.to_string())]));
        let me = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_secs(3), move || {
            if let Some(s) = me.upgrade() {
                s.reconnect();
            }
        });
    }

    fn reconnect(self: &Rc<Session>) {
        if let Some(c) = self.core.borrow_mut().take() {
            c.stop();
        }
        self.connected.set(true);
        self.spawn();
    }

    fn chat_message(&self, text: &str, mine: bool) {
        let Some(v) = self.view.borrow().clone() else { return };
        let bubble = ui::vbox(2, if mine { "kr-bubble mine" } else { "kr-bubble" });
        bubble.append(&ui::para(text, "kr-body"));
        let now = glib::DateTime::now_local().ok().and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default();
        bubble.append(&ui::label(&now, "kr-small-note"));
        bubble.set_halign(if mine { Align::End } else { Align::Start });
        v.chat_list.append(&bubble);
        if !mine {
            v.chat.set_reveal_child(true);
        }
        let adj = v.chat_scroll.vadjustment();
        glib::idle_add_local_once(move || adj.set_value(adj.upper()));
    }

    // ------------------------------------------------------------------------------------ the window

    fn build_view(self: &Rc<Session>) {
        let window = gtk::Window::builder().title(tr("Kalango Remote")).build();
        if let Some(app) = self.main.upgrade().and_then(|w| w.window.application()) {
            window.set_application(Some(&app));
        }
        window.add_css_class("kr-window");
        window.add_css_class("kr-session");
        let (fw, fh) = self.frame_size.get();
        window.set_default_size((fw + 2).clamp(640, 1900), (fh + 46).clamp(420, 1000));
        let no_titlebar = gtk::Box::new(Orientation::Horizontal, 0);
        no_titlebar.set_visible(false);
        window.set_titlebar(Some(&no_titlebar));

        // title bar
        let bar = ui::hbox(10, "kr-session-bar");
        let back = ui::icon_button("seta-esquerda", "k_muted", tr("Back to the start"));
        bar.append(&back);
        bar.append(&ui::icon("monitor", "k_muted", 16));
        let name = ui::label(&self.peer_name.borrow(), "kr-title");
        bar.append(&name);
        let route = ui::hbox(6, "kr-route teal");
        let route_icon = ui::icon("wifi", "k_teal", 16);
        route.append(&route_icon);
        let route_label = gtk::Label::new(Some(if self.route_lan.get() { tr("on the same network") } else { tr("via Cloudflare") }));
        route.append(&route_label);
        bar.append(&route);
        bar.append(&ui::label(
            match self.kind {
                Kind::Control => tr("controlling"),
                Kind::ViewOnly => tr("viewing"),
                Kind::Files => tr("files"),
            },
            "kr-tag accent",
        ));
        bar.append(&ui::spacer());
        let clock = ui::label("00:00:00", "kr-caption");
        bar.append(&clock);
        bar.append(&ui::window_controls());
        let handle_bar = gtk::WindowHandle::new();
        handle_bar.set_child(Some(&bar));

        // the remote screen
        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.set_focusable(true);
        picture.set_content_fit(if *self.scale.borrow() == "stretch" { gtk::ContentFit::Fill } else { gtk::ContentFit::Contain });
        let screen = ui::vbox(0, "kr-screen");
        screen.append(&picture);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&screen));

        // the floating toolbar and its handle
        let (toolbar_box, quality_label, scale_label, stats) = self.toolbar();
        let toolbar = gtk::Revealer::new();
        toolbar.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        toolbar.set_transition_duration(220);
        toolbar.set_child(Some(&toolbar_box));
        toolbar.set_halign(Align::Center);
        toolbar.set_valign(Align::Start);
        toolbar.set_margin_top(12);
        toolbar.set_reveal_child(true);
        overlay.add_overlay(&toolbar);
        let handle = ui::hbox(0, "kr-handle");
        handle.set_size_request(48, 4);
        handle.set_halign(Align::Center);
        handle.set_valign(Align::Start);
        handle.set_margin_top(4);
        handle.set_visible(false);
        overlay.add_overlay(&handle);

        // weak connection banner
        let weak = gtk::Revealer::new();
        weak.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        let wb = ui::hbox(10, "kr-banner");
        wb.append(&ui::icon("wifi-fraco", "k_gold", 16));
        let weak_label = ui::label("", "kr-body");
        wb.append(&weak_label);
        let try_lan = gtk::Button::with_label(tr("Try the local network"));
        try_lan.add_css_class("kr-link-btn");
        wb.append(&try_lan);
        let wclose = ui::icon_button("fechar", "k_muted", tr("Close"));
        wclose.add_css_class("small");
        wb.append(&wclose);
        weak.set_child(Some(&wb));
        weak.set_halign(Align::Center);
        weak.set_valign(Align::Start);
        weak.set_margin_top(12);
        overlay.add_overlay(&weak);
        {
            let wk = weak.clone();
            wclose.connect_clicked(move |_| wk.set_reveal_child(false));
            let main = self.main.clone();
            try_lan.connect_clicked(move |_| {
                if let Some(w) = main.upgrade() {
                    w.toast(tr("Ask the person for the code: on the same network it finds the computer directly"));
                }
            });
        }

        // chat
        let chat = gtk::Revealer::new();
        chat.set_transition_type(gtk::RevealerTransitionType::SlideLeft);
        chat.set_transition_duration(220);
        chat.set_halign(Align::End);
        chat.set_margin_top(14);
        chat.set_margin_bottom(14);
        chat.set_margin_end(14);
        let cp = ui::vbox(0, "kr-chat");
        cp.set_size_request(360, -1);
        let ch = ui::hbox(8, "kr-chat-head");
        ch.append(&ui::icon("conversa", "k_muted", 16));
        let ct = ui::ellip(&trf("Chat with {who}", &[("who", self.peer_name.borrow().clone())]), "kr-title");
        ch.append(&ct);
        let cclose = ui::icon_button("fechar", "k_muted", tr("Close"));
        cclose.add_css_class("small");
        ch.append(&cclose);
        cp.append(&ch);
        let chat_list = ui::vbox(10, "kr-chat-list");
        let chat_scroll = gtk::ScrolledWindow::new();
        chat_scroll.set_vexpand(true);
        chat_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        chat_scroll.set_child(Some(&chat_list));
        cp.append(&chat_scroll);
        let ce = gtk::Entry::new();
        ce.add_css_class("kr-chat-entry");
        ce.set_placeholder_text(Some(&trf("Write to {who}…", &[("who", self.peer_name.borrow().clone())])));
        cp.append(&ce);
        chat.set_child(Some(&cp));
        overlay.add_overlay(&chat);
        {
            let c = chat.clone();
            cclose.connect_clicked(move |_| c.set_reveal_child(false));
            let me = Rc::downgrade(self);
            ce.connect_activate(move |e| {
                let t = e.text().to_string();
                if t.trim().is_empty() {
                    return;
                }
                if let Some(s) = me.upgrade() {
                    s.send(json!({"cmd": "chat", "text": t}));
                    s.chat_message(&t, true);
                }
                e.set_text("");
            });
        }

        // lost connection
        let lost = ui::vbox(10, "kr-lost");
        lost.set_halign(Align::Center);
        lost.set_valign(Align::Center);
        let sp = gtk::Spinner::new();
        sp.set_spinning(true);
        lost.append(&sp);
        let lt = ui::label(tr("Connection lost"), "kr-big-title");
        lt.set_halign(Align::Center);
        lost.append(&lt);
        let lost_label = ui::label("", "kr-caption");
        lost_label.set_halign(Align::Center);
        lost.append(&lost_label);
        let lrow = ui::hbox(8, "");
        let rnow = ui::button(tr("Reconnect now"), None, "");
        let lend = ui::button(tr("End"), None, "danger");
        lrow.append(&rnow);
        lrow.append(&lend);
        lost.append(&lrow);
        lost.set_visible(false);
        overlay.add_overlay(&lost);
        {
            let me = Rc::downgrade(self);
            rnow.connect_clicked(move |_| {
                if let Some(s) = me.upgrade() {
                    s.reconnect();
                }
            });
            let me = Rc::downgrade(self);
            lend.connect_clicked(move |_| {
                if let Some(s) = me.upgrade() {
                    s.close();
                }
            });
        }

        // file transfer
        let transfer = gtk::Revealer::new();
        transfer.set_transition_type(gtk::RevealerTransitionType::Crossfade);
        transfer.set_halign(Align::Fill);
        transfer.set_valign(Align::Fill);
        // closed, it still covers the whole picture and would take every click and move (the mouse never reached
        // the other computer): it takes the pointer only while open
        transfer.set_can_target(false);
        overlay.add_overlay(&transfer);

        let root = ui::vbox(0, "");
        root.append(&handle_bar);
        root.append(&overlay);
        window.set_child(Some(&root));

        let view = Rc::new(View {
            window: window.clone(),
            picture: picture.clone(),
            screen,
            route,
            route_label,
            route_icon,
            name,
            clock: clock.clone(),
            stats,
            toolbar: toolbar.clone(),
            handle: handle.clone(),
            quality_label,
            scale_label,
            chat: chat.clone(),
            chat_list,
            chat_scroll,
            weak,
            weak_label,
            lost,
            lost_label,
            transfer: transfer.clone(),
            files: RefCell::new(None),
        });
        *self.view.borrow_mut() = Some(view.clone());

        // back: the start window comes forward, the session stays
        let main = self.main.clone();
        back.connect_clicked(move |_| {
            if let Some(w) = main.upgrade() {
                w.present();
            }
        });
        let me = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(s) = me.upgrade() {
                s.view.borrow_mut().take();
                s.close();
            }
            glib::Propagation::Proceed
        });

        // the clock
        let me = Rc::downgrade(self);
        glib::timeout_add_seconds_local(1, move || {
            let Some(s) = me.upgrade() else { return glib::ControlFlow::Break };
            let Some(v) = s.view.borrow().clone() else { return glib::ControlFlow::Break };
            let e = s.started.get().map(|t| t.elapsed().as_secs()).unwrap_or(0);
            v.clock.set_text(&format!("{:02}:{:02}:{:02}", e / 3600, e / 60 % 60, e % 60));
            glib::ControlFlow::Continue
        });

        self.input(&picture, &window);
        self.autohide(&overlay, &toolbar, &handle);
        if self.kind == Kind::Files {
            self.open_transfer();
        }
        window.present();
        picture.grab_focus();
        self.update_route(None);
    }

    fn toolbar(self: &Rc<Session>) -> (gtk::Box, gtk::Label, gtk::Label, gtk::Label) {
        let tb = ui::hbox(2, "kr-floatbar");
        let menu_button = |icon_name: &str, text: &str, pop: &gtk::Popover| -> (gtk::MenuButton, gtk::Label) {
            let m = gtk::MenuButton::new();
            m.add_css_class("kr-tool");
            m.set_has_frame(false);
            let row = ui::hbox(6, "");
            row.append(&ui::icon(icon_name, "k_text", 16));
            let l = gtk::Label::new(Some(text));
            if !text.is_empty() {
                row.append(&l);
            }
            row.append(&ui::icon("chevron-baixo", "k_muted", 12));
            m.set_child(Some(&row));
            pop.add_css_class("kr-menu");
            m.set_popover(Some(pop));
            (m, l)
        };
        // screen
        let screen_pop = gtk::Popover::new();
        let sl = ui::vbox(2, "");
        let refresh = menu_item(tr("Refresh the picture"), "");
        sl.append(&refresh);
        screen_pop.set_child(Some(&sl));
        let (screen_btn, _) = menu_button("monitor", tr("Screen 1"), &screen_pop);
        tb.append(&screen_btn);
        // quality
        let qpop = gtk::Popover::new();
        let ql = ui::vbox(2, "");
        let profiles = [
            ("best", tr("Best picture"), tr("sharper, uses more network")),
            ("balanced", tr("Balanced"), tr("adjusts itself to the connection")),
            ("low", tr("Fast response"), tr("less delay, softer picture")),
        ];
        let current = self.main.upgrade().map(|w| w.settings.borrow().quality.clone()).unwrap_or_else(|| "balanced".into());
        let mut qlabel_text = tr("Balanced");
        for (id, t, sub) in profiles {
            if (id == "low" && current == "speed") || id == current {
                qlabel_text = t;
            }
            ql.append(&menu_item(t, sub).tap(|b| b.set_widget_name(id)));
        }
        ql.append(&ui::hbox(0, "kr-div"));
        let (cur_row, cur_sw, _) = ui::switch_row(tr("Show the cursor from there"), "");
        cur_sw.set_active(true);
        ql.append(&cur_row);
        let (aud_row, aud_sw, _) = ui::switch_row(tr("Audio of the other computer"), "");
        ql.append(&aud_row);
        qpop.set_child(Some(&ql));
        let (qbtn, quality_label) = menu_button("ajustes", qlabel_text, &qpop);
        tb.append(&qbtn);
        // scale
        let spop = gtk::Popover::new();
        let sbox = ui::vbox(2, "");
        for (id, t) in [("fit", tr("Fit")), ("original", tr("100%")), ("stretch", tr("Stretch"))] {
            sbox.append(&menu_item(t, "").tap(|b| b.set_widget_name(id)));
        }
        spop.set_child(Some(&sbox));
        let scale_text = match self.scale.borrow().as_str() {
            "original" => tr("100%"),
            "stretch" => tr("Stretch"),
            _ => tr("Fit"),
        };
        let (sbtn, scale_label) = menu_button("expandir-altura", scale_text, &spop);
        tb.append(&sbtn);
        // keyboard
        let kpop = gtk::Popover::new();
        let kbox = ui::vbox(2, "");
        let cad = menu_item(tr("Send Ctrl+Alt+Del"), "");
        let lock = menu_item(tr("Lock the other computer"), "");
        kbox.append(&cad);
        kbox.append(&lock);
        kpop.set_child(Some(&kbox));
        let (kbtn, _) = menu_button("teclado", "", &kpop);
        tb.append(&kbtn);
        let clip = ui::icon_button("clipe", "k_text", tr("Clipboard: synced both ways"));
        clip.add_css_class("kr-tool");
        tb.append(&clip);
        let files = ui::icon_button("pasta", "k_text", tr("Transfer files"));
        files.add_css_class("kr-tool");
        tb.append(&files);
        let chat = ui::icon_button("conversa", "k_text", tr("Chat (Ctrl+Alt+C)"));
        chat.add_css_class("kr-tool");
        tb.append(&chat);
        let rec = ui::icon_button("anel", "k_text", tr("Record"));
        rec.add_css_class("kr-tool");
        tb.append(&rec);
        tb.append(&ui::hbox(0, "kr-vdiv").tap(|d| d.set_size_request(1, 20)));
        let stats = ui::hbox(6, "kr-stats");
        stats.append(&ui::icon("wifi", "k_teal", 16));
        let stats_label = gtk::Label::new(Some("— ms"));
        stats.append(&stats_label);
        tb.append(&stats);
        let full = ui::icon_button("maximizar", "k_text", tr("Full screen"));
        full.add_css_class("kr-tool");
        tb.append(&full);
        let end = ui::button(tr("End"), Some(("parar", "k_coral")), "danger");
        tb.append(&end);

        // wiring
        let me = Rc::downgrade(self);
        refresh.connect_clicked(move |_| {
            if let Some(s) = me.upgrade() {
                s.send(json!({"cmd": "refresh"}));
            }
        });
        for b in children_buttons(&ql) {
            let me = Rc::downgrade(self);
            let ql2 = quality_label.clone();
            let qpop2 = qpop.clone();
            b.connect_clicked(move |b| {
                let id = b.widget_name().to_string();
                if let Some(s) = me.upgrade() {
                    s.send(json!({"cmd": "quality", "value": id}));
                    ql2.set_text(match id.as_str() {
                        "best" => tr("Best picture"),
                        "low" => tr("Fast response"),
                        _ => tr("Balanced"),
                    });
                }
                qpop2.popdown();
            });
        }
        let me = Rc::downgrade(self);
        cur_sw.connect_state_set(move |_, _| {
            if let Some(s) = me.upgrade() {
                s.send(json!({"cmd": "toggle", "name": "show-remote-cursor"}));
            }
            glib::Propagation::Proceed
        });
        let me = Rc::downgrade(self);
        aud_sw.connect_state_set(move |_, _| {
            if let Some(s) = me.upgrade() {
                s.send(json!({"cmd": "toggle", "name": "disable-audio"}));
            }
            glib::Propagation::Proceed
        });
        for b in children_buttons(&sbox) {
            let me = Rc::downgrade(self);
            let sl2 = scale_label.clone();
            let spop2 = spop.clone();
            b.connect_clicked(move |b| {
                let id = b.widget_name().to_string();
                if let Some(s) = me.upgrade() {
                    s.set_scale(&id);
                    sl2.set_text(match id.as_str() {
                        "original" => tr("100%"),
                        "stretch" => tr("Stretch"),
                        _ => tr("Fit"),
                    });
                }
                spop2.popdown();
            });
        }
        let me = Rc::downgrade(self);
        cad.connect_clicked(move |_| {
            if let Some(s) = me.upgrade() {
                s.send(json!({"cmd": "ctrl_alt_del"}));
            }
        });
        let me = Rc::downgrade(self);
        lock.connect_clicked(move |_| {
            if let Some(s) = me.upgrade() {
                s.send(json!({"cmd": "lock_screen"}));
            }
        });
        let me = Rc::downgrade(self);
        files.connect_clicked(move |_| {
            if let Some(s) = me.upgrade() {
                s.open_transfer();
            }
        });
        let me = Rc::downgrade(self);
        chat.connect_clicked(move |_| {
            if let Some(v) = me.upgrade().and_then(|s| s.view.borrow().clone()) {
                v.chat.set_reveal_child(!v.chat.reveals_child());
            }
        });
        let me = Rc::downgrade(self);
        let recording = Cell::new(self.auto_record());
        if recording.get() {
            self.send(json!({"cmd": "record", "on": true}));
            if let Some(img) = rec.child().and_downcast::<gtk::Image>() {
                ui::set_icon(&img, "anel", "k_coral", 16);
            }
        }
        rec.connect_clicked(move |b| {
            if let Some(s) = me.upgrade() {
                recording.set(!recording.get());
                s.send(json!({"cmd": "record", "on": recording.get()}));
                if let Some(img) = b.child().and_downcast::<gtk::Image>() {
                    ui::set_icon(&img, "anel", if recording.get() { "k_coral" } else { "k_text" }, 16);
                }
            }
        });
        let me = Rc::downgrade(self);
        full.connect_clicked(move |_| {
            if let Some(v) = me.upgrade().and_then(|s| s.view.borrow().clone()) {
                if v.window.is_fullscreen() {
                    v.window.unfullscreen()
                } else {
                    v.window.fullscreen()
                }
            }
        });
        let me = Rc::downgrade(self);
        end.connect_clicked(move |_| {
            if let Some(s) = me.upgrade() {
                s.close();
            }
        });
        (tb, quality_label, scale_label, stats_label)
    }

    fn set_scale(&self, id: &str) {
        *self.scale.borrow_mut() = id.to_owned();
        let Some(v) = self.view.borrow().clone() else { return };
        v.picture.set_content_fit(match id {
            "stretch" => gtk::ContentFit::Fill,
            "original" => gtk::ContentFit::ScaleDown,
            _ => gtk::ContentFit::Contain,
        });
        if id == "original" {
            let (w, h) = self.frame_size.get();
            v.window.set_default_size(w + 2, h + 46);
        }
    }

    /// Where the frame is drawn inside the picture, to turn pointer positions into remote pixels.
    fn to_remote(&self, pic: &gtk::Picture, x: f64, y: f64) -> Option<(i32, i32)> {
        let (fw, fh) = self.frame_size.get();
        if fw == 0 || fh == 0 {
            return None;
        }
        let (pw, ph) = (pic.width() as f64, pic.height() as f64);
        let (sx, sy) = if *self.scale.borrow() == "stretch" {
            (pw / fw as f64, ph / fh as f64)
        } else {
            let s = (pw / fw as f64).min(ph / fh as f64);
            let s = if *self.scale.borrow() == "original" { s.min(1.0) } else { s };
            (s, s)
        };
        let (ox, oy) = ((pw - fw as f64 * sx) / 2.0, (ph - fh as f64 * sy) / 2.0);
        let (rx, ry) = ((x - ox) / sx, (y - oy) / sy);
        if rx < 0.0 || ry < 0.0 || rx >= fw as f64 || ry >= fh as f64 {
            return None;
        }
        Some((rx as i32, ry as i32))
    }

    fn input(self: &Rc<Session>, pic: &gtk::Picture, window: &gtk::Window) {
        if self.kind == Kind::ViewOnly {
            return;
        }
        let motion = gtk::EventControllerMotion::new();
        let me = Rc::downgrade(self);
        let p = pic.clone();
        motion.connect_motion(move |_, x, y| {
            let Some(s) = me.upgrade() else { return };
            // at most ~120 moves a second
            if s.last_motion.get().is_some_and(|t| t.elapsed() < Duration::from_millis(8)) {
                return;
            }
            s.last_motion.set(Some(Instant::now()));
            if let Some((rx, ry)) = s.to_remote(&p, x, y) {
                s.send(json!({"cmd": "mouse", "mask": 0, "x": rx, "y": ry}));
            }
        });
        pic.add_controller(motion);
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let me = Rc::downgrade(self);
        let p = pic.clone();
        let mask_of = |b: u32| match b {
            2 => 4,
            3 => 2,
            8 => 8,
            9 => 16,
            _ => 1,
        };
        click.connect_pressed(move |g, _, x, y| {
            let Some(s) = me.upgrade() else { return };
            p.grab_focus();
            if let Some((rx, ry)) = s.to_remote(&p, x, y) {
                s.send(json!({"cmd": "mouse", "mask": 1 | (mask_of(g.current_button()) << 3), "x": rx, "y": ry}));
            }
        });
        let me = Rc::downgrade(self);
        let p = pic.clone();
        click.connect_released(move |g, _, x, y| {
            let Some(s) = me.upgrade() else { return };
            if let Some((rx, ry)) = s.to_remote(&p, x, y) {
                s.send(json!({"cmd": "mouse", "mask": 2 | (mask_of(g.current_button()) << 3), "x": rx, "y": ry}));
            }
        });
        pic.add_controller(click);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES | gtk::EventControllerScrollFlags::DISCRETE);
        let me = Rc::downgrade(self);
        scroll.connect_scroll(move |_, dx, dy| {
            if let Some(s) = me.upgrade() {
                s.send(json!({"cmd": "mouse", "mask": 3, "x": -(dx as i32), "y": -(dy as i32)}));
            }
            glib::Propagation::Stop
        });
        pic.add_controller(scroll);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let me = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, k, _, m| {
            let Some(s) = me.upgrade() else { return glib::Propagation::Proceed };
            // Ctrl+Alt+C: the chat stays on this side
            let (ctrl, alt, shift) = (m.contains(gdk::ModifierType::CONTROL_MASK), m.contains(gdk::ModifierType::ALT_MASK), m.contains(gdk::ModifierType::SHIFT_MASK));
            if ctrl && alt && matches!(k, gdk::Key::c | gdk::Key::C) {
                if let Some(v) = s.view.borrow().clone() {
                    v.chat.set_reveal_child(!v.chat.reveals_child());
                }
                return glib::Propagation::Stop;
            }
            if let Some(v) = s.view.borrow().clone() {
                if v.chat.reveals_child() && v.chat.focus_child().is_some() {
                    return glib::Propagation::Proceed;
                }
            }
            s.key(k, true, ctrl, alt, shift);
            glib::Propagation::Stop
        });
        let me = Rc::downgrade(self);
        keys.connect_key_released(move |_, k, _, m| {
            if let Some(s) = me.upgrade() {
                s.key(k, false, m.contains(gdk::ModifierType::CONTROL_MASK), m.contains(gdk::ModifierType::ALT_MASK), m.contains(gdk::ModifierType::SHIFT_MASK));
            }
        });
        window.add_controller(keys);
    }

    fn key(&self, k: gdk::Key, down: bool, ctrl: bool, alt: bool, shift: bool) {
        if let Some(name) = vk_name(k) {
            self.send(json!({"cmd": "key", "name": name, "down": down, "press": false, "ctrl": ctrl, "alt": alt, "shift": shift}));
        } else if let Some(c) = k.to_unicode().filter(|c| !c.is_control()) {
            // a character: one click on press (the other side's layout makes the character)
            if down {
                let c = if ctrl || alt { c.to_ascii_lowercase() } else { c };
                self.send(json!({"cmd": "key", "name": c.to_string(), "down": true, "press": true, "ctrl": ctrl, "alt": alt, "shift": shift && (ctrl || alt)}));
            }
        }
    }

    /// The toolbar hides 2.5 s after the pointer leaves it, leaving a 48×4 handle; the pointer at the
    /// top brings it back.
    fn autohide(self: &Rc<Session>, overlay: &gtk::Overlay, toolbar: &gtk::Revealer, handle: &gtk::Box) {
        let hide_at: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(Some(Instant::now() + Duration::from_millis(2500))));
        let m = gtk::EventControllerMotion::new();
        let (t, h, ha) = (toolbar.clone(), handle.clone(), hide_at.clone());
        m.connect_motion(move |_, _, y| {
            if y < 60.0 {
                t.set_reveal_child(true);
                h.set_visible(false);
                ha.set(None);
            } else if ha.get().is_none() {
                ha.set(Some(Instant::now() + Duration::from_millis(2500)));
            }
        });
        overlay.add_controller(m);
        let (t, h) = (toolbar.clone(), handle.clone());
        glib::timeout_add_local(Duration::from_millis(250), move || {
            if t.root().is_none() {
                return glib::ControlFlow::Break;
            }
            if hide_at.get().is_some_and(|at| Instant::now() >= at) && !t.child().is_some_and(|c| c.has_focus() || has_open_popover(&c)) {
                t.set_reveal_child(false);
                h.set_visible(true);
                hide_at.set(None);
            }
            glib::ControlFlow::Continue
        });
    }

    fn open_transfer(self: &Rc<Session>) {
        let Some(v) = self.view.borrow().clone() else { return };
        if v.files.borrow().is_none() {
            let t = Transfer::new(self);
            v.transfer.set_child(Some(&t.root));
            *v.files.borrow_mut() = Some(t);
        }
        show_transfer(&v.transfer, true);
        let files = v.files.borrow().clone();
        if let Some(t) = files {
            let p = t.remote_path.borrow().clone();
            t.load_remote(&p);
        }
    }
}

/// Opens or closes the file transfer area over the picture; only an open one takes the pointer.
fn show_transfer(transfer: &gtk::Revealer, open: bool) {
    transfer.set_can_target(open);
    transfer.set_reveal_child(open);
}

fn has_open_popover(w: &gtk::Widget) -> bool {
    let mut stack = vec![w.clone()];
    while let Some(w) = stack.pop() {
        if let Some(m) = w.downcast_ref::<gtk::MenuButton>() {
            if m.is_active() {
                return true;
            }
        }
        let mut c = w.first_child();
        while let Some(ch) = c {
            c = ch.next_sibling();
            stack.push(ch);
        }
    }
    false
}

trait Tap: Sized {
    fn tap(self, f: impl FnOnce(&Self)) -> Self {
        f(&self);
        self
    }
}
impl<T> Tap for T {}

fn menu_item(title: &str, sub: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("kr-menu-item");
    let v = ui::vbox(2, "");
    v.append(&ui::label(title, "kr-menu-title"));
    if !sub.is_empty() {
        v.append(&ui::label(sub, "kr-menu-sub"));
    }
    b.set_child(Some(&v));
    b
}

fn children_buttons(b: &gtk::Box) -> Vec<gtk::Button> {
    let mut out = Vec::new();
    let mut c = b.first_child();
    while let Some(ch) = c {
        c = ch.next_sibling();
        if let Ok(btn) = ch.downcast::<gtk::Button>() {
            if !btn.widget_name().is_empty() && btn.widget_name() != "GtkButton" {
                out.push(btn);
            }
        }
    }
    out
}

/// RustDesk's name for a key that is not a plain character.
pub fn vk_name(k: gdk::Key) -> Option<&'static str> {
    use gdk::Key as K;
    Some(match k {
        K::Return | K::KP_Enter => "VK_RETURN",
        K::BackSpace => "VK_BACK",
        K::Tab | K::ISO_Left_Tab => "VK_TAB",
        K::Escape => "VK_ESCAPE",
        K::space => "VK_SPACE",
        K::Delete => "VK_DELETE",
        K::Insert => "VK_INSERT",
        K::Home => "VK_HOME",
        K::End => "VK_END",
        K::Page_Up => "VK_PRIOR",
        K::Page_Down => "VK_NEXT",
        K::Left => "VK_LEFT",
        K::Right => "VK_RIGHT",
        K::Up => "VK_UP",
        K::Down => "VK_DOWN",
        K::Shift_L | K::Shift_R => "VK_SHIFT",
        K::Control_L | K::Control_R => "VK_CONTROL",
        K::Alt_L | K::Alt_R => "VK_MENU",
        K::Caps_Lock => "VK_CAPITAL",
        K::F1 => "VK_F1",
        K::F2 => "VK_F2",
        K::F3 => "VK_F3",
        K::F4 => "VK_F4",
        K::F5 => "VK_F5",
        K::F6 => "VK_F6",
        K::F7 => "VK_F7",
        K::F8 => "VK_F8",
        K::F9 => "VK_F9",
        K::F10 => "VK_F10",
        K::F11 => "VK_F11",
        K::F12 => "VK_F12",
        _ => return None,
    })
}

// ------------------------------------------------------------------------------------- file transfer

struct Transfer {
    root: gtk::Box,
    session: Weak<Session>,
    local_path: RefCell<PathBuf>,
    remote_path: RefCell<String>,
    local_list: gtk::ListBox,
    remote_list: gtk::ListBox,
    local_crumb: gtk::Label,
    remote_crumb: gtk::Label,
    selected_label: gtk::Label,
    queue: gtk::Box,
    queue_title: gtk::Label,
    jobs: RefCell<Vec<(i32, String, u64, gtk::Box, gtk::Label, gtk::Box)>>,
}

impl Transfer {
    fn new(s: &Rc<Session>) -> Rc<Transfer> {
        let root = ui::vbox(0, "kr-transfer");
        root.set_margin_start(60);
        root.set_margin_end(60);
        root.set_margin_top(60);
        root.set_margin_bottom(60);
        let head = ui::hbox(10, "kr-transfer-head");
        head.append(&ui::icon("pasta", "k_muted", 16));
        let ht = ui::label(tr("Transfer files"), "kr-big-title");
        ht.set_hexpand(true);
        head.append(&ht);
        let (b, _) = ui::badge(if s.route_lan.get() { tr("on the same network") } else { tr("via Cloudflare") }, if s.route_lan.get() { "blue" } else { "teal" });
        head.append(&b);
        let close = ui::icon_button("fechar", "k_muted", tr("Close"));
        head.append(&close);
        root.append(&head);
        let panes = ui::hbox(0, "");
        panes.set_vexpand(true);
        let pane = |title: &str, icon_name: &str| {
            let p = ui::vbox(8, "kr-pane");
            p.set_hexpand(true);
            let h = ui::hbox(8, "");
            h.append(&ui::icon(icon_name, "k_muted", 16));
            let t = ui::label(title, "kr-row-title");
            t.set_hexpand(true);
            h.append(&t);
            p.append(&h);
            let crumb_row = ui::hbox(6, "");
            let up = ui::icon_button("anterior", "k_muted", tr("Up"));
            up.add_css_class("small");
            crumb_row.append(&up);
            let crumb = ui::ellip("", "kr-caption");
            crumb_row.append(&crumb);
            p.append(&crumb_row);
            let list = gtk::ListBox::new();
            list.add_css_class("kr-files");
            list.set_selection_mode(gtk::SelectionMode::Multiple);
            let sc = gtk::ScrolledWindow::new();
            sc.set_vexpand(true);
            sc.set_child(Some(&list));
            p.append(&sc);
            (p, h, up, crumb, list)
        };
        let (lp, lh, lup, local_crumb, local_list) = pane(tr("This computer"), "casa");
        let selected_label = ui::label("", "kr-link-text");
        lh.append(&selected_label);
        let (rp, _, rup, remote_crumb, remote_list) = pane(&s.peer_name.borrow(), "monitor");
        let mid = ui::vbox(8, "kr-transfer-mid");
        mid.set_valign(Align::Center);
        let send = ui::button(tr("Send →"), None, "primary");
        let recv = ui::button(tr("← Receive"), None, "");
        mid.append(&send);
        mid.append(&recv);
        panes.append(&lp);
        panes.append(&mid);
        panes.append(&rp);
        root.append(&panes);
        let qbox = ui::vbox(6, "kr-queue");
        let qh = ui::hbox(8, "");
        let queue_title = ui::label(tr("QUEUE"), "kr-section");
        queue_title.set_hexpand(true);
        qh.append(&queue_title);
        qbox.append(&qh);
        let queue = ui::vbox(6, "");
        qbox.append(&queue);
        root.append(&qbox);

        let home = glib::home_dir();
        let t = Rc::new(Transfer {
            root,
            session: Rc::downgrade(s),
            local_path: RefCell::new(home),
            remote_path: RefCell::new(String::new()),
            local_list,
            remote_list,
            local_crumb,
            remote_crumb,
            selected_label,
            queue,
            queue_title,
            jobs: RefCell::new(Vec::new()),
        });
        let me = Rc::downgrade(s);
        close.connect_clicked(move |_| {
            if let Some(v) = me.upgrade().and_then(|s| s.view.borrow().clone()) {
                show_transfer(&v.transfer, false);
            }
        });
        let tw = Rc::downgrade(&t);
        lup.connect_clicked(move |_| {
            if let Some(t) = tw.upgrade() {
                let p = t.local_path.borrow().parent().map(|p| p.to_path_buf());
                if let Some(p) = p {
                    *t.local_path.borrow_mut() = p;
                    t.load_local();
                }
            }
        });
        let tw = Rc::downgrade(&t);
        rup.connect_clicked(move |_| {
            if let Some(t) = tw.upgrade() {
                let p = t.remote_path.borrow().clone();
                let parent = std::path::Path::new(&p).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                t.load_remote(&parent);
            }
        });
        let tw = Rc::downgrade(&t);
        t.local_list.connect_row_activated(move |_, row| {
            if let Some(t) = tw.upgrade() {
                let name = row.widget_name().to_string();
                if let Some(dir) = name.strip_prefix("d:") {
                    let p = t.local_path.borrow().join(dir);
                    *t.local_path.borrow_mut() = p;
                    t.load_local();
                }
            }
        });
        let tw = Rc::downgrade(&t);
        t.remote_list.connect_row_activated(move |_, row| {
            if let Some(t) = tw.upgrade() {
                let name = row.widget_name().to_string();
                if let Some(dir) = name.strip_prefix("d:") {
                    let base = t.remote_path.borrow().clone();
                    t.load_remote(&join_remote(&base, dir));
                }
            }
        });
        let tw = Rc::downgrade(&t);
        t.local_list.connect_selected_rows_changed(move |l| {
            if let Some(t) = tw.upgrade() {
                let n = l.selected_rows().len();
                t.selected_label.set_text(&if n == 0 { String::new() } else { trf("{n} selected", &[("n", n.to_string())]) });
            }
        });
        let tw = Rc::downgrade(&t);
        send.connect_clicked(move |_| {
            if let Some(t) = tw.upgrade() {
                t.send_selected();
            }
        });
        let tw = Rc::downgrade(&t);
        recv.connect_clicked(move |_| {
            if let Some(t) = tw.upgrade() {
                t.receive_selected();
            }
        });
        t.load_local();
        t
    }

    fn row(name: &str, is_dir: bool, size: u64) -> gtk::ListBoxRow {
        let r = gtk::ListBoxRow::new();
        r.set_widget_name(&format!("{}:{}", if is_dir { "d" } else { "f" }, name));
        let b = ui::hbox(8, "kr-file-row");
        b.append(&ui::icon(if is_dir { "pasta" } else { "doc" }, "k_muted", 16));
        b.append(&ui::ellip(name, "kr-body"));
        b.append(&ui::label(&if is_dir { "—".into() } else { human_size(size) }, "kr-caption"));
        r.set_child(Some(&b));
        r
    }

    fn load_local(&self) {
        let p = self.local_path.borrow().clone();
        self.local_crumb.set_text(&p.display().to_string());
        while let Some(c) = self.local_list.first_child() {
            self.local_list.remove(&c);
        }
        let mut entries: Vec<(String, bool, u64)> = std::fs::read_dir(&p)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let n = e.file_name().into_string().ok()?;
                        if n.starts_with('.') {
                            return None;
                        }
                        let m = e.metadata().ok()?;
                        Some((n, m.is_dir(), m.len()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        for (n, d, s) in entries {
            self.local_list.append(&Self::row(&n, d, s));
        }
    }

    fn load_remote(&self, path: &str) {
        if let Some(s) = self.session.upgrade() {
            s.send(json!({"cmd": "read_dir", "path": path}));
        }
    }

    fn on_remote_dir(&self, v: &Value) {
        if v["is_local"].as_bool() == Some(true) {
            return;
        }
        let path = v["path"].as_str().unwrap_or("").to_owned();
        self.remote_crumb.set_text(&path);
        *self.remote_path.borrow_mut() = path;
        while let Some(c) = self.remote_list.first_child() {
            self.remote_list.remove(&c);
        }
        let mut entries: Vec<(String, bool, u64)> = v["entries"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|e| (e["name"].as_str().unwrap_or("").to_owned(), e["is_dir"].as_bool().unwrap_or(false), e["size"].as_u64().unwrap_or(0)))
                    .filter(|e| !e.0.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        for (n, d, s) in entries {
            self.remote_list.append(&Self::row(&n, d, s));
        }
    }

    fn send_selected(&self) {
        let Some(s) = self.session.upgrade() else { return };
        let to = self.remote_path.borrow().clone();
        for r in self.local_list.selected_rows() {
            let name = r.widget_name().to_string();
            let file = &name[2..];
            let path = self.local_path.borrow().join(file).display().to_string();
            s.send(json!({"cmd": "transfer", "path": path, "to": join_remote(&to, file), "from_remote": false}));
        }
    }

    fn receive_selected(&self) {
        let Some(s) = self.session.upgrade() else { return };
        let base = self.remote_path.borrow().clone();
        for r in self.remote_list.selected_rows() {
            let name = r.widget_name().to_string();
            let file = &name[2..];
            let to = self.local_path.borrow().join(file).display().to_string();
            s.send(json!({"cmd": "transfer", "path": join_remote(&base, file), "to": to, "from_remote": true}));
        }
    }

    fn on_job(&self, v: &Value) {
        let id = v["id"].as_i64().unwrap_or(-1) as i32;
        match v["event"].as_str().unwrap_or("") {
            "job_started" => {
                let path = v["path"].as_str().unwrap_or("");
                let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let size = if v["from_remote"].as_bool() == Some(true) { 0 } else { std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) };
                let row = ui::hbox(10, "kr-job");
                row.append(&ui::icon("doc", "k_muted", 16));
                let col = ui::vbox(4, "");
                col.set_hexpand(true);
                let top = ui::hbox(8, "");
                top.append(&ui::ellip(&name, "kr-body"));
                let state = ui::label(tr("in the queue"), "kr-caption");
                top.append(&state);
                col.append(&top);
                let track = ui::hbox(0, "kr-track accent");
                let fill = ui::hbox(0, "fill");
                fill.set_halign(Align::Start);
                track.append(&fill);
                col.append(&track);
                row.append(&col);
                self.queue.append(&row);
                self.jobs.borrow_mut().push((id, name, size, track, state, fill));
            }
            "job_progress" => {
                let done = v["finished_size"].as_f64().unwrap_or(0.0) as u64;
                let speed = v["speed"].as_f64().unwrap_or(0.0);
                for (jid, _, size, track, state, fill) in self.jobs.borrow().iter() {
                    if *jid == id {
                        let frac = if *size > 0 { (done as f64 / *size as f64).min(1.0) } else { 0.0 };
                        state.set_text(&format!("{} · {}/s", human_size(done), human_size(speed as u64)));
                        fill.set_size_request((track.width() as f64 * frac) as i32, 3);
                    }
                }
            }
            "job_done" => {
                for (jid, _, size, track, state, fill) in self.jobs.borrow().iter() {
                    if *jid == id {
                        state.set_text(&trf("sent · {s}", &[("s", human_size(*size))]));
                        track.remove_css_class("accent");
                        fill.set_size_request(track.width(), 3);
                    }
                }
                self.load_local();
                let p = self.remote_path.borrow().clone();
                self.load_remote(&p);
            }
            "job_error" => {
                for (jid, _, _, track, state, _) in self.jobs.borrow().iter() {
                    if *jid == id {
                        state.set_text(v["err"].as_str().unwrap_or(""));
                        track.add_css_class("coral");
                    }
                }
            }
            _ => {}
        }
        let n = self.jobs.borrow().len();
        self.queue_title.set_text(&trf("QUEUE · {n}", &[("n", n.to_string())]));
    }
}

fn join_remote(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_owned()
    } else if base.ends_with('/') || base.ends_with('\\') {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

pub fn human_size(b: u64) -> String {
    let b = b as f64;
    if b < 1024.0 {
        format!("{} B", b as u64)
    } else if b < 1024.0 * 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", b / 1048576.0).replace('.', ",")
    } else {
        format!("{:.1} GB", b / 1073741824.0).replace('.', ",")
    }
}

// --------------------------------------------------------------------------- pages of the main window

/// Transfers (rail): files move inside a session; this page says how.
pub fn transfers_page() -> gtk::Widget {
    let page = ui::vbox(16, "kr-page");
    let card = ui::vbox(10, "kr-card");
    card.append(&ui::card_title("pasta", tr("TRANSFERS")));
    card.append(&ui::para(
        tr("Connect in the Files mode (or open Transfer files during a session) to send and receive files. Each transfer resumes where it stopped if the connection drops."),
        "kr-caption",
    ));
    card.set_valign(Align::Start);
    page.append(&card);
    page.upcast()
}

/// History (rail): every computer this app connected to.
pub fn history_page(w: &Rc<Window>) -> gtk::Widget {
    let page = ui::vbox(10, "kr-page");
    page.append(&ui::label(tr("HISTORY"), "kr-section"));
    let list = ui::vbox(8, "");
    page.append(&list);
    let wk = Rc::downgrade(w);
    let fill = move |list: &gtk::Box| {
        while let Some(c) = list.first_child() {
            list.remove(&c);
        }
        let Some(w) = wk.upgrade() else { return };
        let mut all = w.peers.borrow().list.clone();
        all.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        if all.is_empty() {
            list.append(&ui::label(tr("The computers you connect to appear here."), "kr-empty"));
        }
        for p in all {
            let row = ui::hbox(10, "kr-card kr-history-row");
            row.append(&ui::icon("monitor", "k_muted", 16));
            let t = ui::vbox(2, "");
            t.set_hexpand(true);
            t.append(&ui::ellip(&p.name, "kr-device-name"));
            t.append(&ui::ellip(&format!("{} · {}", if p.route == "lan" { tr("on the same network") } else { tr("via Cloudflare") }, store::ago(p.last_used, store::now())), "kr-caption"));
            row.append(&t);
            list.append(&row);
        }
    };
    fill(&list);
    let l = list.clone();
    page.connect_map(move |_| fill(&l));
    let sc = gtk::ScrolledWindow::new();
    sc.set_child(Some(&page));
    sc.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_become_states() {
        assert!(matches!(classify_message("re-input-password", "Password Error", "Wrong Password"), Some(ConnectEvent::WrongPassword)));
        assert!(matches!(classify_message("error", "Connection Error", "Reset by the peer"), Some(ConnectEvent::Failed(_))));
        assert!(matches!(
            classify_message("error", "Connection Error", "Kalango Remote: the peer's identity does not match the invite (x)"),
            Some(ConnectEvent::Failed(t)) if t == tr("This is not the computer of the invite (its key is different). The connection was refused.")
        ));
        assert!(matches!(classify_message("wait-remote-accept-nook", "Prompt", "Please wait for the remote side to accept"), Some(ConnectEvent::Waiting(_))));
        assert!(classify_message("success", "Successful", "Connected, waiting for image...").is_none());
        assert!(matches!(classify_message("error", "Connection Error", "Closed manually by the peer"), Some(ConnectEvent::Failed(t)) if t == tr("The person denied the access.")));
    }

    // the transfer area is an overlay over the whole picture: while closed it must not take the pointer, or the mouse of
    // whoever controls never reaches the other computer (found in two VMs; there is no display in the tests)
    #[test]
    fn the_closed_transfer_area_lets_the_pointer_through() {
        let src = include_str!("session.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        let build = &code[code.find("let transfer = gtk::Revealer::new();").unwrap()..];
        let build = &build[..build.find("overlay.add_overlay(&transfer);").unwrap()];
        assert!(build.contains("transfer.set_can_target(false);"), "the transfer revealer must start without the pointer");
        let helper = "fn show_transfer(transfer: &gtk::Revealer, open: bool) {\n    transfer.set_can_target(open);\n    transfer.set_reveal_child(open);";
        assert!(code.contains(helper), "show_transfer must switch the pointer together with the reveal");
        assert_eq!(code.matches("transfer.set_reveal_child(").count(), 1, "open and close the transfer area only through show_transfer");
    }

    #[test]
    fn sizes_and_paths() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(4 * 1024 * 1024 + 200 * 1024), "4,2 MB");
        assert_eq!(join_remote("/home/x", "a.jpg"), "/home/x/a.jpg");
        assert_eq!(join_remote("C:\\Users\\", "a.jpg"), "C:\\Users\\a.jpg");
        assert_eq!(join_remote("", "a"), "a");
    }
}
