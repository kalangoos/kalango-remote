//! The side of the computer being helped: the access request (who, what they ask for, for how long,
//! a warning against scams; denies by itself in 30 s, Esc denies, Enter allows) and, while a session
//! lasts, the signals that do not go away — a 2 px outline around the whole screen (coral when
//! controlled, teal when only viewed) and a banner with Chat, Take control back and Stop.

use gtk::{gdk, glib, prelude::*, Align};
use serde_json::{json, Value};
use std::{cell::RefCell, collections::HashMap, rc::{Rc, Weak}, time::{Duration, Instant}};

use crate::{
    home::Home,
    i18n::{tr, trf},
    store::{self, AllowedDevice},
    ui,
    window::Window,
};

const DENY_AFTER: Duration = Duration::from_secs(30);

struct Request {
    dialog: gtk::Window,
}

struct Active {
    name: String,
    keyboard: bool,
    started: Instant,
    outline: Option<gtk::Window>,
    banner: gtk::Window,
    label: gtk::Label,
}

pub struct Incoming {
    home: Weak<Home>,
    requests: RefCell<HashMap<i32, Request>>,
    active: RefCell<HashMap<i32, Active>>,
    /// the sessions nobody approved here (a fixed password or an allowed device): the privacy cover is for them only
    unattended: RefCell<std::collections::HashSet<i32>>,
    /// whether the last permission given to the core included the pointer and the keyboard
    last_control: std::cell::Cell<bool>,
}

impl Incoming {
    pub fn new(home: &Rc<Home>) -> Rc<Incoming> {
        Rc::new(Incoming { home: Rc::downgrade(home), requests: Default::default(), active: Default::default(), unattended: Default::default(), last_control: Default::default() })
    }

    fn send(&self, v: Value) {
        if let Some(h) = self.home.upgrade() {
            h.send_to_core(&v);
        }
    }

    fn core_pid(&self) -> Option<u32> {
        let mut pid = None;
        if let Some(h) = self.home.upgrade() {
            h.with_core(|c| pid = c.pid());
        }
        pid
    }

    /// Before a session sees anything: KalangoOS's niri gives screen capture (and, with `control`, the pointer and
    /// the keyboard) only to processes kalangod allows (niri patch 0008). Outside KalangoOS there is no kalangod
    /// and nothing to ask for.
    /// The refusal, when kalangod is there and did not give it: the session would see nothing, so it does not start.
    fn grant(&self, control: bool) -> Option<String> {
        self.last_control.set(control);
        let privacy = self.wants_privacy();
        let Some(pid) = self.core_pid() else {
            eprintln!("kalango-remote: permissão de controle remoto: o núcleo não está rodando");
            return None;
        };
        let result = crate::kalangod::grant(pid, control, privacy);
        match &result {
            Ok(()) => eprintln!("kalango-remote: permissão de controle remoto dada ao núcleo {pid} (controle: {control}, privacidade: {privacy})"),
            Err(e) => eprintln!("kalango-remote: permissão de controle remoto: {e}"),
        }
        grant_refusal(result, crate::kalangod::present())
    }

    /// A connection kalangod would not let see the screen: it is closed and the person here is told why, instead of
    /// a "controlling this computer" banner over a session that shows nothing.
    fn refuse(&self, id: i32, why: &str) {
        self.send(json!({"cmd": "close", "id": id}));
        if let Some(w) = Window::get() {
            // longer than a short notice: it explains why, and is easy to miss
            w.toast_for(&trf("Access refused by the system: {why}", &[("why", refusal_reason(why))]), std::time::Duration::from_secs(6));
        }
    }

    /// Privacy mode (the local monitors covered, the local person's input blocked behind the cover): the setting is on,
    /// the compositor can do it, a session nobody approved here is going on, and no access request waits for an
    /// answer (its dialog would sit under the cover, out of sight and reach).
    fn wants_privacy(&self) -> bool {
        Window::get().is_some_and(|w| w.settings.borrow().privacy_black)
            && !self.unattended.borrow().is_empty()
            && self.requests.borrow().is_empty()
            && crate::kalangod::compositor_supports_privacy()
    }

    /// Gives the core's permission again with what changed (a switch, a request that came or went, a session that
    /// started or ended): the cover comes and goes at once.
    pub fn refresh_grant(&self) {
        if !self.active.borrow().is_empty() {
            // a refused new grant does not end the sessions by itself: kalangod's `allow` returns the error before it
            // touches the earlier grant, and its once-a-second recheck (`prune`/`still_safe` in kalangod's
            // remote_control.rs) looks only at the core, not at this app. So they are closed here and the grant dropped.
            if let Some(why) = self.grant(self.last_control.get()) {
                let ids: Vec<i32> = self.active.borrow().keys().copied().collect();
                for id in ids {
                    self.refuse(id, &why);
                }
                self.revoke();
            }
        }
    }

    fn revoke(&self) {
        if let Some(pid) = self.core_pid() {
            let _ = crate::kalangod::revoke(pid);
        }
    }

    /// Allows the connection `id`, the permission first; false (and the connection closed) when it was refused.
    fn authorize(&self, id: i32, keyboard: bool, clipboard: bool, audio: bool, file: bool) -> bool {
        if let Some(why) = self.grant(keyboard) {
            self.refuse(id, &why);
            return false;
        }
        self.send(json!({"cmd": "authorize", "id": id, "keyboard": keyboard, "clipboard": clipboard, "audio": audio, "file": file}));
        true
    }

    pub fn on_event(self: &Rc<Incoming>, v: &Value) {
        let id = v["id"].as_i64().unwrap_or(0) as i32;
        match v["event"].as_str().unwrap_or("") {
            "connection" => {
                let authorized = v["authorized"].as_bool().unwrap_or(false);
                if authorized {
                    if let Some(r) = self.requests.borrow_mut().remove(&id) {
                        r.dialog.destroy();
                    }
                    if !self.active.borrow().contains_key(&id) {
                        // allowed without our dialog (a fixed password): the permission now
                        match self.grant(v["keyboard"].as_bool().unwrap_or(true)) {
                            Some(why) => self.refuse(id, &why),
                            None => self.start_indicator(id, v),
                        }
                    }
                } else if !self.requests.borrow().contains_key(&id) {
                    // a device in the allowed list (still in time) goes straight in
                    let peer = v["peer_id"].as_str().unwrap_or("");
                    let allowed = Window::get().is_some_and(|w| {
                        w.settings.borrow().allowed.iter().any(|a| a.id == peer && (a.until == 0 || a.until > store::now()))
                    });
                    if allowed {
                        let _ = self.authorize(id, true, true, false, true);
                    } else {
                        self.show_request(id, v);
                    }
                }
            }
            "removed" => {
                if let Some(r) = self.requests.borrow_mut().remove(&id) {
                    r.dialog.destroy();
                }
                let ended = self.active.borrow_mut().remove(&id);
                self.unattended.borrow_mut().remove(&id);
                if self.active.borrow().is_empty() {
                    self.revoke();
                } else {
                    self.refresh_grant();
                }
                if let Some(a) = ended {
                    crate::shell_sound::play("kalango-remote-stop");
                    if self.active.borrow().is_empty() && Window::get().is_some_and(|w| w.settings.borrow().lock_after) {
                        crate::access::lock_screen();
                    }
                    a.banner.destroy();
                    if let Some(o) = a.outline {
                        o.destroy();
                    }
                    if let Some(w) = Window::get() {
                        w.toast(&trf("Session ended · {who} is no longer connected", &[("who", a.name)]));
                    }
                }
            }
            "chat" => {
                if let Some(a) = self.active.borrow().get(&id) {
                    a.label.set_text(&trf("{who}: {text}", &[("who", a.name.clone()), ("text", v["text"].as_str().unwrap_or("").to_owned())]));
                }
            }
            _ => {}
        }
    }

    /// Ends every session going on (Esc Esc anywhere: the compositor runs `kalango-remote end-sessions`).
    pub fn end_sessions(&self) {
        let ids: Vec<i32> = self.active.borrow().keys().copied().collect();
        for id in ids {
            self.send(json!({"cmd": "close", "id": id}));
        }
    }

    pub fn close_all(&self) {
        self.revoke();
        for (_, r) in self.requests.borrow_mut().drain() {
            r.dialog.destroy();
        }
        for (_, a) in self.active.borrow_mut().drain() {
            a.banner.destroy();
            if let Some(o) = a.outline {
                o.destroy();
            }
        }
    }

    fn show_request(self: &Rc<Incoming>, id: i32, v: &Value) {
        let name = v["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(tr("Someone")).to_owned();
        let peer_id = v["peer_id"].as_str().unwrap_or("").to_owned();
        let dialog = gtk::Window::builder().title(tr("Remote access request")).resizable(false).build();
        dialog.add_css_class("kr-window");
        dialog.add_css_class("kr-dialog");
        if let Some(w) = Window::get() {
            dialog.set_transient_for(Some(&w.window));
        }
        let no_titlebar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        no_titlebar.set_visible(false);
        dialog.set_titlebar(Some(&no_titlebar));
        let root = ui::vbox(14, "kr-dialog-body");
        root.set_size_request(440, -1);
        let head = ui::hbox(8, "");
        head.append(&ui::icon("escudo", "k_accent", 16));
        let ht = ui::label(tr("Remote access request"), "kr-big-title");
        ht.set_hexpand(true);
        head.append(&ht);
        let lan = Window::get().is_some_and(|w| w.settings.borrow().network == "lan");
        let route = Window::get().map(|w| w.settings.borrow().route_text()).unwrap_or(tr("via Cloudflare"));
        head.append(&ui::badge(route, if lan { "blue" } else { "teal" }).0);
        root.append(&head);
        let who = ui::hbox(12, "kr-who");
        let avatar = ui::label(&name.chars().next().unwrap_or('?').to_uppercase().to_string(), "kr-avatar");
        avatar.set_xalign(0.5);
        who.append(&avatar);
        let wt = ui::vbox(2, "");
        wt.set_hexpand(true);
        wt.append(&ui::ellip(&name, "kr-device-name"));
        wt.append(&ui::ellip(&trf("device {id}", &[("id", peer_id.clone())]), "kr-caption"));
        who.append(&wt);
        let first = Window::get().is_none_or(|w| !w.settings.borrow().allowed.iter().any(|a| a.id == peer_id));
        if first {
            who.append(&ui::label(tr("first time"), "kr-tag gold"));
        }
        root.append(&who);
        root.append(&ui::label(tr("They ask to:"), "kr-caption"));
        let perms = ui::vbox(4, "");
        let check = |text: &str, note: &str, on: bool, locked: bool| {
            let row = ui::hbox(8, "kr-perm");
            let c = gtk::CheckButton::with_label(text);
            c.set_active(on);
            c.set_sensitive(!locked);
            c.set_hexpand(true);
            row.append(&c);
            if !note.is_empty() {
                row.append(&ui::label(note, "kr-caption"));
            }
            perms.append(&row);
            c
        };
        check(tr("See the screen"), tr("always included"), true, true);
        let kb = check(tr("Control the keyboard and the mouse"), "", v["keyboard"].as_bool().unwrap_or(true), false);
        let file = check(tr("Transfer files"), tr("to and from the Downloads folder"), false, false);
        let audio = check(tr("Hear this computer's audio"), "", false, false);
        let clip = check(tr("Use the clipboard"), tr("copy and paste between the two"), v["clipboard"].as_bool().unwrap_or(true), false);
        root.append(&perms);
        let drow = ui::hbox(8, "");
        let dl = ui::label(tr("For how long"), "kr-body");
        dl.set_hexpand(true);
        drow.append(&dl);
        let dur = gtk::DropDown::from_strings(&[tr("Only this session"), tr("1 hour"), tr("Always for this device")]);
        dur.add_css_class("kr-dropdown");
        dur.set_size_request(240, -1);
        drow.append(&dur);
        root.append(&drow);
        let warn = ui::hbox(10, "kr-warn");
        warn.append(&ui::icon("alerta", "k_gold", 16));
        warn.append(&ui::para(tr("Only approve if you asked this person for help. Nobody from KalangoOS support asks for access like this."), "kr-caption"));
        root.append(&warn);
        let btns = ui::hbox(8, "");
        let deny = ui::button(tr("Deny"), None, "tall");
        deny.set_hexpand(true);
        let allow = ui::button(tr("Allow"), Some(("check", "k_accent_fg")), "primary tall");
        allow.set_hexpand(true);
        btns.append(&deny);
        btns.append(&allow);
        root.append(&btns);
        let track = ui::hbox(0, "kr-track accent");
        let fill = ui::hbox(0, "fill");
        fill.set_halign(Align::Start);
        track.append(&fill);
        root.append(&track);
        let count = ui::label("", "kr-small-note");
        root.append(&count);
        dialog.set_child(Some(&root));

        let me = Rc::downgrade(self);
        let decide = Rc::new(move |allow: bool| {
            let Some(me) = me.upgrade() else { return };
            let Some(r) = me.requests.borrow_mut().remove(&id) else { return };
            if allow {
                let allowed = me.authorize(id, kb.is_active(), clip.is_active(), audio.is_active(), file.is_active());
                // the core does not announce the connection again after "authorize": the indicator starts here
                if allowed && !me.active.borrow().contains_key(&id) {
                    me.start_indicator(id, &json!({"name": name, "keyboard": kb.is_active()}));
                }
                let until = match dur.selected() {
                    1 => Some(store::now() + 3600),
                    2 => Some(0),
                    _ => None,
                };
                if let (true, Some(until), Some(w)) = (allowed, until, Window::get()) {
                    w.settings.borrow_mut().allowed.push(AllowedDevice { name: name.clone(), id: peer_id.clone(), until, since: store::now() });
                    w.settings_changed();
                }
            } else {
                // the other side is not told why
                me.send(json!({"cmd": "close", "id": id}));
            }
            r.dialog.destroy();
            // (a session going on under the cover gets it back)
            me.refresh_grant();
        });
        let d = decide.clone();
        deny.connect_clicked(move |_| d(false));
        let d = decide.clone();
        allow.connect_clicked(move |_| d(true));
        // Esc denies and Enter allows wherever the focus is (capture: before the focused button)
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let d = decide.clone();
        keys.connect_key_pressed(move |_, k, _, _| match k {
            gdk::Key::Escape => {
                d(false);
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                d(true);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
        dialog.add_controller(keys);
        // the countdown: denies by itself; the last 5 s are coral
        let start = Instant::now();
        let d = decide.clone();
        let dl2 = dialog.clone();
        glib::timeout_add_local(Duration::from_millis(200), move || {
            if dl2.root().is_none() || !dl2.is_visible() {
                return glib::ControlFlow::Break;
            }
            let left = DENY_AFTER.saturating_sub(start.elapsed());
            if left.is_zero() {
                d(false);
                return glib::ControlFlow::Break;
            }
            fill.set_size_request((track.width() as f64 * left.as_secs_f64() / DENY_AFTER.as_secs_f64()) as i32, 3);
            if left.as_secs() < 5 {
                track.add_css_class("coral");
            }
            let s = left.as_secs() + 1;
            count.set_text(&trf("No answer: denies by itself in 0:{s} · Esc denies · Enter allows", &[("s", format!("{s:02}"))]));
            glib::ControlFlow::Continue
        });
        // safe default: the focus starts on Deny
        dialog.present();
        deny.grab_focus();
        self.requests.borrow_mut().insert(id, Request { dialog });
        // the cover lifts while the dialog waits
        self.refresh_grant();
    }

    fn start_indicator(self: &Rc<Incoming>, id: i32, v: &Value) {
        // someone came in: the session's sound (KalangoOS's shell plays it)
        crate::shell_sound::play("kalango-remote-start");
        let name = v["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(tr("Someone")).to_owned();
        let keyboard = v["keyboard"].as_bool().unwrap_or(true);
        let outline = outline_window(keyboard);
        let banner = gtk::Window::builder().title(tr("Remote session")).resizable(false).decorated(false).build();
        banner.add_css_class("kr-window");
        banner.add_css_class("kr-banner-window");
        let row = ui::hbox(12, "kr-controlled");
        let dot = ui::hbox(0, if keyboard { "kr-rec-dot" } else { "kr-rec-dot teal" });
        dot.set_valign(Align::Center);
        row.append(&dot);
        let what = if keyboard { trf("{who} is controlling this computer", &[("who", name.clone())]) } else { trf("{who} is viewing this computer", &[("who", name.clone())]) };
        row.append(&ui::label(&what, "kr-row-title"));
        let unattended = v["unattended"].as_bool().unwrap_or(false);
        let st = Window::get().map(|w| w.settings.borrow().clone()).unwrap_or_default();
        if unattended {
            // what the fixed password lets in, so nobody is surprised by what it can do
            let mut allowed = vec![tr("screen").to_owned(), tr("keyboard and mouse").to_owned()];
            if st.clipboard {
                allowed.push(tr("clipboard").to_owned());
            }
            if st.audio {
                allowed.push(tr("audio").to_owned());
            }
            row.append(&ui::badge(&trf("unattended · {allowed}; no files, terminal or tunnels", &[("allowed", allowed.join(", "))]), "gold").0);
        }
        if st.record {
            row.append(&ui::badge(tr("recording"), "coral").0);
        }
        let label = ui::label("", "kr-caption");
        row.append(&label);
        row.append(&ui::hbox(0, "kr-vdiv").tap_size(1, 18));
        let chat = ui::button(tr("Chat"), Some(("conversa", "k_text")), "ghost chip");
        row.append(&chat);
        let take = ui::button(tr("Take control back"), Some(("olho", "k_text")), "ghost chip");
        take.set_visible(keyboard);
        row.append(&take);
        let stop = ui::button(tr("Stop (Esc Esc)"), Some(("parar", "k_coral")), "danger chip");
        row.append(&stop);
        banner.set_child(Some(&row));
        let me = Rc::downgrade(self);
        stop.connect_clicked(move |_| {
            if let Some(me) = me.upgrade() {
                me.send(json!({"cmd": "close", "id": id}));
            }
        });
        let me = Rc::downgrade(self);
        take.connect_clicked(move |b| {
            if let Some(me) = me.upgrade() {
                // only the picture from now on: the pointer and keyboard permission goes too
                me.grant(false);
                me.send(json!({"cmd": "authorize", "id": id, "keyboard": false}));
                b.set_visible(false);
            }
        });
        let me = Rc::downgrade(self);
        let n = name.clone();
        chat.connect_clicked(move |_| {
            if let Some(me) = me.upgrade() {
                me.reply_chat(id, &n);
            }
        });
        // Esc twice in a row (while the banner has the focus) ends the session
        let keys = gtk::EventControllerKey::new();
        let last_esc = std::cell::Cell::new(None::<Instant>);
        let me = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, k, _, _| {
            if k == gdk::Key::Escape {
                if last_esc.get().is_some_and(|t| t.elapsed() < Duration::from_millis(800)) {
                    if let Some(me) = me.upgrade() {
                        me.send(json!({"cmd": "close", "id": id}));
                    }
                }
                last_esc.set(Some(Instant::now()));
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        banner.add_controller(keys);
        place_banner(&banner);
        banner.present();
        // the clock in the banner
        let lw = label.clone();
        let started = Instant::now();
        let route = Window::get().map(|w| w.settings.borrow().route_text()).unwrap_or(tr("via Cloudflare"));
        glib::timeout_add_seconds_local(1, move || {
            if lw.root().is_none() {
                return glib::ControlFlow::Break;
            }
            let e = started.elapsed().as_secs();
            if !lw.text().contains(':') || lw.text().starts_with('·') {
                lw.set_text(&format!("· {route} · {:02}:{:02}", e / 60, e % 60));
            }
            glib::ControlFlow::Continue
        });
        self.active.borrow_mut().insert(id, Active { name, keyboard, started, outline, banner, label });
        if unattended {
            self.unattended.borrow_mut().insert(id);
        }
        // (the permission now says whether this one is covered)
        self.refresh_grant();
    }

    fn reply_chat(self: &Rc<Incoming>, id: i32, who: &str) {
        let Some(a) = self.active.borrow().get(&id).map(|a| a.banner.clone()) else { return };
        let pop = gtk::Popover::new();
        pop.add_css_class("kr-menu");
        let e = gtk::Entry::new();
        e.set_placeholder_text(Some(&trf("Write to {who}…", &[("who", who.to_owned())])));
        e.set_size_request(320, -1);
        pop.set_child(Some(&e));
        if let Some(child) = a.child() {
            pop.set_parent(&child);
        }
        let me = Rc::downgrade(self);
        let p = pop.clone();
        e.connect_activate(move |e| {
            if let Some(me) = me.upgrade() {
                me.send(json!({"cmd": "chat", "id": id, "text": e.text().to_string()}));
            }
            p.popdown();
        });
        pop.popup();
        let _ = self.active.borrow().get(&id).map(|a| (a.keyboard, a.started));
    }
}

trait TapSize {
    fn tap_size(self, w: i32, h: i32) -> Self;
}
impl TapSize for gtk::Box {
    fn tap_size(self, w: i32, h: i32) -> Self {
        self.set_size_request(w, h);
        self
    }
}

/// The outline around the whole screen, over everything and without taking clicks (layer shell).
#[cfg(feature = "layer-shell")]
fn outline_window(controlling: bool) -> Option<gtk::Window> {
    use gtk4_layer_shell::{Edge, Layer, LayerShell};
    if !gtk4_layer_shell::is_supported() {
        return None;
    }
    let w = gtk::Window::new();
    w.init_layer_shell();
    w.set_layer(Layer::Overlay);
    w.set_namespace(Some("kalango-remote-outline"));
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        w.set_anchor(e, true);
    }
    w.set_exclusive_zone(-1);
    w.add_css_class("kr-outline");
    if !controlling {
        w.add_css_class("teal");
    }
    w.set_can_target(false);
    w.present();
    // no input region: clicks go through to the apps below
    if let Some(s) = w.surface() {
        s.set_input_region(&gtk::cairo::Region::create());
    }
    Some(w)
}

#[cfg(not(feature = "layer-shell"))]
fn outline_window(_controlling: bool) -> Option<gtk::Window> {
    None
}

/// The banner at the top centre of the screen (layer shell when there is one).
fn place_banner(w: &gtk::Window) {
    #[cfg(feature = "layer-shell")]
    {
        use gtk4_layer_shell::{Edge, Layer, LayerShell};
        if gtk4_layer_shell::is_supported() {
            w.init_layer_shell();
            w.set_layer(Layer::Overlay);
            w.set_namespace(Some("kalango-remote-banner"));
            w.set_anchor(Edge::Top, true);
            w.set_margin(Edge::Top, 44);
            w.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::OnDemand);
        }
    }
    let _ = w;
}

impl Home {
    pub fn send_to_core(&self, v: &Value) {
        self.with_core(|c| c.send(v));
    }
}

/// kalangod's reason, at most ~200 characters: the notice must fit in a window.
fn refusal_reason(why: &str) -> String {
    const MAX: usize = 200;
    if why.chars().count() <= MAX {
        return why.to_owned();
    }
    let mut s: String = why.chars().take(MAX).collect();
    s.push('…');
    s
}

/// The refusal of a grant: only when kalangod is there (outside KalangoOS there is none and the compositor decides).
fn grant_refusal(result: Result<(), String>, kalangod_present: bool) -> Option<String> {
    match result {
        Err(e) if kalangod_present => Some(e),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_grant_stops_the_session_only_where_kalangod_decides() {
        assert_eq!(grant_refusal(Ok(()), true), None);
        assert_eq!(grant_refusal(Err("dumpable".into()), true), Some("dumpable".into()));
        // no kalangod (another distribution): nothing to ask, the session goes on
        assert_eq!(grant_refusal(Err("No such file or directory".into()), false), None);
    }

    #[test]
    fn a_long_reason_is_cut() {
        assert_eq!(refusal_reason("dumpable"), "dumpable");
        let long = "x".repeat(500);
        let cut = refusal_reason(&long);
        assert_eq!(cut.chars().count(), 201);
        assert!(cut.ends_with('…'));
    }

    // every refusal closes the connection: a refused session must not stay open waiting for a picture
    #[test]
    fn every_refusal_closes_the_connection() {
        let src = include_str!("incoming.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        let body = |start: &str| {
            let b = &code[code.find(start).unwrap_or_else(|| panic!("{start}"))..];
            b[..b.find("\n    }\n").unwrap()].to_owned()
        };
        assert!(body("    fn refuse(&self, id: i32, why: &str) {").contains(r#"self.send(json!({"cmd": "close", "id": id}));"#), "refuse must close the connection");
        let authorize = body("    fn authorize(&self,");
        assert!(authorize.contains("if let Some(why) = self.grant(keyboard) {\n            self.refuse(id, &why);\n            return false;"), "authorize must refuse before authorizing");
        assert!(code.contains("Some(why) => self.refuse(id, &why),"), "a session allowed without the dialog must be refused too");
        assert!(body("    pub fn refresh_grant(&self) {").contains("self.refuse(id, &why);"), "a refused new grant must close the sessions");
        assert!(code.contains("let allowed = me.authorize(") && code.contains("if allowed && "), "the dialog's banner waits for authorize");
        assert_eq!(code.matches("self.refuse(").count(), 3, "every refusal site is checked here");
    }

    // the banner "… is controlling this computer" only for a session that got the permission
    #[test]
    fn the_banner_waits_for_the_permission() {
        let src = include_str!("incoming.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(code.contains("let allowed = me.authorize("));
        assert!(code.contains("if allowed && !me.active.borrow().contains_key(&id) {"));
        assert!(code.contains("Some(why) => self.refuse(id, &why),\n                            None => self.start_indicator(id, v),"));
        assert_eq!(code.matches("start_indicator(id,").count(), 2, "every place that starts the banner is checked here");
    }
}

