//! Início: on the left what is about THIS computer (share it, ask the agent), on the right what goes TO
//! another one (connect, favourites, recents). Two columns from 1100 px (left 440), the left one shrinking
//! to 360 down to 900, one column below; device grids take as many 200–280 px columns as fit.

use gtk::{gdk, glib, prelude::*, Align};
use serde_json::{json, Value};
use std::{cell::{Cell, RefCell}, rc::{Rc, Weak}, time::Duration};

use crate::{
    core::{Core, Msg},
    i18n::{tr, trf},
    incoming::Incoming,
    session::{self, Kind},
    store::{self, Peer},
    ui,
    window::Window,
};

#[derive(Clone, Debug, PartialEq)]
enum ShareState {
    Idle,
    Opening,
    Sharing,
    NoInternet,
    Failed(String),
    /// the core exited before answering (the first line of its stderr, or its exit code)
    CoreFailed(String),
    /// another Kalango Remote already shares this computer (its core holds the IPC sockets)
    AlreadySharing,
}

#[derive(Default)]
struct ShareInfo {
    code: String,
    link: String,
    password: String,
    addr: String,
    lan: bool,
    /// unix seconds when the share closes by itself
    until: u64,
}

struct ShareCard {
    root: gtk::Box,
    badge: gtk::Box,
    badge_label: gtk::Label,
    stack: gtk::Stack,
    code: gtk::Label,
    link: gtk::Label,
    qr: gtk::Picture,
    pass: gtk::Label,
    pass_shown: Cell<bool>,
    time_label: gtk::Label,
    time_track: gtk::Box,
    time_fill: gtk::Box,
    failed_text: gtk::Label,
    noinet_text: gtk::Label,
    core_title: gtk::Label,
    core_text: gtk::Label,
    core_retry: gtk::Button,
    opening_step: gtk::Label,
    ask: gtk::Switch,
    unattended: gtk::Switch,
    unattended_sub: gtk::Label,
}

pub struct Home {
    pub root: gtk::ScrolledWindow,
    win: Weak<Window>,
    page: gtk::Box,
    left: gtk::Box,
    right: gtk::Box,
    share: ShareCard,
    state: RefCell<ShareState>,
    /// the core of this share said something (it started); an exit before that is a core error
    core_answered: Cell<bool>,
    /// core errors in a row (each new try waits longer)
    core_failures: Cell<u32>,
    /// unix seconds before which "Try again" stays off after a core error
    retry_at: Rc<Cell<u64>>,
    info: RefCell<ShareInfo>,
    core: RefCell<Option<Core>>,
    incoming: RefCell<Option<Rc<Incoming>>>,
    // connect
    target: gtk::Entry,
    target_box: gtk::Box,
    kind: Vec<gtk::ToggleButton>,
    connect_btn: gtk::Button,
    connect_stack: gtk::Stack,
    steps: gtk::Box,
    password_box: gtk::Box,
    password: gtk::Entry,
    password_title: gtk::Label,
    error_label: gtk::Label,
    lan_badge: gtk::Box,
    lan_label: gtk::Label,
    pending: RefCell<Option<Rc<session::Session>>>,
    // devices
    favorites: gtk::Box,
    recents: gtk::Box,
    fav_section: gtk::Box,
    rec_section: gtk::Box,
    filter: RefCell<String>,
    grid_width: Cell<i32>,
}

fn qr_texture(text: &str) -> Option<gdk::Texture> {
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M).ok()?;
    let w = code.width();
    let quiet = 2usize;
    let scale = 4usize;
    let side = (w + quiet * 2) * scale;
    let colors = code.to_colors();
    // light: cor/realce (white), dark: cor/scrim — the same in both themes (contrast)
    let mut px = vec![255u8; side * side * 3];
    for y in 0..w {
        for x in 0..w {
            if colors[y * w + x] == qrcode::Color::Dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let (xx, yy) = ((x + quiet) * scale + dx, (y + quiet) * scale + dy);
                        let i = (yy * side + xx) * 3;
                        px[i..i + 3].copy_from_slice(&[0x0B, 0x0C, 0x10]);
                    }
                }
            }
        }
    }
    let bytes = glib::Bytes::from_owned(px);
    Some(gdk::MemoryTexture::new(side as i32, side as i32, gdk::MemoryFormat::R8g8b8, &bytes, side * 3).upcast())
}

/// This computer's address on the local network (no packet is sent: the route lookup picks it).
pub fn local_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    s.connect(("192.0.2.1", 9)).ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

fn switch_rows(card: &gtk::Box) -> (gtk::Switch, gtk::Switch, gtk::Label) {
    let (r1, ask, _) = ui::switch_row(tr("Ask before each access"), tr("You approve what the person can do"));
    let (r2, unattended, sub) =
        ui::switch_row(tr("Unattended access"), tr("Off · fixed password and allowed devices in Settings"));
    card.append(&r1);
    card.append(&r2);
    (ask, unattended, sub)
}

impl ShareCard {
    fn new() -> ShareCard {
        let root = ui::vbox(14, "kr-card");
        let head = ui::hbox(8, "");
        let title = ui::card_title("monitor", tr("THIS COMPUTER"));
        title.set_hexpand(true);
        head.append(&title);
        let (badge, badge_label) = ui::badge(tr("not shared"), "dim");
        head.append(&badge);
        root.append(&head);

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(220);
        stack.set_interpolate_size(true);
        // only the visible state counts (a hidden one must not widen the column)
        stack.set_vhomogeneous(false);
        stack.set_hhomogeneous(false);
        root.append(&stack);

        // idle (nothing shared)
        let idle = ui::vbox(10, "");
        let logo = ui::png(include_bytes!("../icons/logo-256.png"), 140);
        logo.set_halign(Align::Center);
        idle.append(&logo);
        let t = ui::label(tr("Help from afar, without a server of ours"), "kr-row-title");
        t.set_halign(Align::Center);
        t.set_xalign(0.5);
        t.add_css_class("kr-strong");
        idle.append(&t);
        idle.append(&ui::para(
            tr("To get help, share: KalangoOS opens a temporary tunnel and shows a code for you to give the other person."),
            "kr-body",
        ));
        let share_btn = ui::button(tr("Share this computer"), Some(("compartilhar", "k_accent_fg")), "primary");
        share_btn.set_widget_name("share");
        idle.append(&share_btn);
        stack.add_named(&idle, Some("idle"));

        // opening the tunnel
        let opening = ui::vbox(8, "");
        opening.set_margin_top(10);
        let spin = gtk::Spinner::new();
        spin.set_spinning(true);
        spin.set_size_request(16, 16);
        opening.append(&spin);
        let ot = ui::label(tr("Opening a secure tunnel…"), "kr-big-title");
        ot.set_halign(Align::Center);
        opening.append(&ot);
        let opening_step = ui::label(tr("cloudflared · asking for a temporary address"), "kr-caption");
        opening_step.set_halign(Align::Center);
        opening.append(&opening_step);
        let ind = gtk::ProgressBar::new();
        ind.add_css_class("kr-indeterminate");
        ind.set_margin_start(70);
        ind.set_margin_end(70);
        let indc = ind.clone();
        glib::timeout_add_local(Duration::from_millis(60), move || {
            indc.pulse();
            glib::ControlFlow::Continue
        });
        opening.append(&ind);
        let cancel = ui::button(tr("Cancel"), None, "");
        cancel.set_widget_name("cancel");
        cancel.set_margin_top(10);
        opening.append(&cancel);
        stack.add_named(&opening, Some("opening"));

        // sharing
        let sharing = ui::vbox(14, "");
        let top = ui::hbox(14, "");
        let left = ui::vbox(4, "");
        left.set_hexpand(true);
        left.append(&ui::label(tr("Code to share"), "kr-caption"));
        let code_row = ui::hbox(4, "");
        let code = ui::label("", "kr-code display");
        code.set_hexpand(true);
        code_row.append(&code);
        let copy_code = ui::icon_button("copiar", "k_muted", tr("Copy the code"));
        copy_code.set_widget_name("copy-code");
        code_row.append(&copy_code);
        left.append(&code_row);
        let link_row = ui::hbox(6, "kr-linkbox");
        let link = ui::ellip("", "kr-linktext");
        link_row.append(&link);
        let copy_link = ui::icon_button("copiar", "k_muted", tr("Copy the link"));
        copy_link.add_css_class("small");
        copy_link.set_widget_name("copy-link");
        link_row.append(&copy_link);
        left.append(&link_row);
        left.append(&ui::label(tr("One-time password"), "kr-caption"));
        let pass_row = ui::hbox(4, "");
        let pass = ui::label("●●●●●●", "kr-dots");
        pass.set_hexpand(true);
        pass_row.append(&pass);
        let eye = ui::icon_button("olho", "k_muted", tr("Show for 10 s"));
        eye.set_widget_name("show-password");
        pass_row.append(&eye);
        let renew = ui::icon_button("reiniciar", "k_muted", tr("New password"));
        renew.set_widget_name("new-password");
        pass_row.append(&renew);
        left.append(&pass_row);
        top.append(&left);
        let qrbox = ui::vbox(6, "");
        let qr = gtk::Picture::new();
        qr.add_css_class("kr-qr");
        qr.set_size_request(104, 104);
        qr.set_can_shrink(true);
        qr.set_valign(Align::Start);
        qrbox.append(&qr);
        let qrl = ui::label(tr("Open on the phone"), "kr-small-note");
        qrl.set_halign(Align::Center);
        qrbox.append(&qrl);
        top.append(&qrbox);
        sharing.append(&top);
        let time = ui::vbox(6, "");
        let trow = ui::hbox(8, "");
        let time_label = ui::label("", "kr-text2");
        time_label.set_hexpand(true);
        trow.append(&time_label);
        let extend = gtk::Button::with_label(tr("+30 min"));
        extend.add_css_class("kr-link-btn");
        extend.set_widget_name("extend");
        trow.append(&extend);
        time.append(&trow);
        let time_track = ui::hbox(0, "kr-track");
        let time_fill = ui::hbox(0, "fill");
        time_fill.set_halign(Align::Start);
        time_track.append(&time_fill);
        time.append(&time_track);
        time.append(&ui::para(
            tr("Temporary Cloudflare tunnel, no account and no server of ours. Closes by itself at the end of the time or when you stop."),
            "kr-note",
        ));
        sharing.append(&time);
        let actions = ui::hbox(8, "");
        let stop = ui::button(tr("Stop sharing"), Some(("parar", "k_coral")), "danger");
        stop.set_hexpand(true);
        stop.set_widget_name("stop");
        actions.append(&stop);
        let invite = ui::button(tr("Copy invite"), Some(("compartilhar", "k_text")), "");
        invite.set_widget_name("copy-invite");
        actions.append(&invite);
        sharing.append(&actions);
        stack.add_named(&sharing, Some("sharing"));

        // no internet
        let noinet = ui::vbox(10, "");
        noinet.append(&ui::label(tr("No internet, the tunnel does not open."), "kr-big-title"));
        let noinet_text = ui::para("", "kr-caption");
        noinet.append(&noinet_text);
        let nrow = ui::hbox(8, "");
        let retry1 = ui::button(tr("Try again"), Some(("reiniciar", "k_text")), "");
        retry1.set_hexpand(true);
        retry1.set_widget_name("retry");
        nrow.append(&retry1);
        let agent1 = ui::button(tr("Ask the agent"), Some(("spark", "k_accent")), "agent");
        agent1.set_widget_name("ask-agent");
        nrow.append(&agent1);
        noinet.append(&nrow);
        stack.add_named(&noinet, Some("noinet"));

        // the tunnel did not open
        let failed = ui::vbox(10, "");
        let fh = ui::hbox(8, "");
        fh.append(&ui::icon("alerta", "k_coral", 16));
        fh.append(&ui::label(tr("Could not open the tunnel"), "kr-big-title"));
        failed.append(&fh);
        let failed_text = ui::para("", "kr-caption");
        failed.append(&failed_text);
        let frow = ui::hbox(8, "");
        let retry2 = ui::button(tr("Try again"), Some(("reiniciar", "k_text")), "");
        retry2.set_widget_name("retry");
        frow.append(&retry2);
        let agent2 = ui::button(tr("Ask the agent"), Some(("spark", "k_accent")), "agent");
        agent2.set_widget_name("ask-agent");
        frow.append(&agent2);
        failed.append(&frow);
        stack.add_named(&failed, Some("failed"));

        // the core did not open
        let nocore = ui::vbox(10, "");
        let core_title = ui::label(tr("The Kalango Remote core did not open."), "kr-big-title");
        nocore.append(&core_title);
        let core_text = ui::para("", "kr-caption");
        nocore.append(&core_text);
        let crow = ui::hbox(8, "");
        let core_retry = ui::button(tr("Try again"), Some(("reiniciar", "k_text")), "");
        core_retry.set_hexpand(true);
        core_retry.set_widget_name("retry");
        crow.append(&core_retry);
        let agent3 = ui::button(tr("Ask the agent"), Some(("spark", "k_accent")), "agent");
        agent3.set_widget_name("ask-agent");
        crow.append(&agent3);
        nocore.append(&crow);
        stack.add_named(&nocore, Some("nocore"));

        root.append(&ui::hbox(0, "kr-div"));
        let (ask, unattended, unattended_sub) = switch_rows(&root);
        ShareCard {
            root,
            badge,
            badge_label,
            stack,
            code,
            link,
            qr,
            pass,
            pass_shown: Cell::new(false),
            time_label,
            time_track,
            time_fill,
            failed_text,
            noinet_text,
            core_title,
            core_text,
            core_retry,
            opening_step,
            ask,
            unattended,
            unattended_sub,
        }
    }

    fn set_badge(&self, text: &str, color: &str) {
        self.badge_label.set_text(text);
        for c in ["teal", "blue", "coral", "gold", "dim"] {
            self.badge.remove_css_class(c);
        }
        self.badge.add_css_class(color);
    }

    /// Every button of the card with this name (they live in several states).
    fn buttons(&self, name: &str) -> Vec<gtk::Button> {
        let mut out = Vec::new();
        let mut stack: Vec<gtk::Widget> = vec![self.root.clone().upcast()];
        while let Some(w) = stack.pop() {
            if w.widget_name() == name {
                if let Ok(b) = w.clone().downcast::<gtk::Button>() {
                    out.push(b);
                }
            }
            let mut c = w.first_child();
            while let Some(ch) = c {
                c = ch.next_sibling();
                stack.push(ch);
            }
        }
        out
    }
}

impl Home {
    pub fn new(win: &Rc<Window>) -> Rc<Home> {
        let page = ui::hbox(16, "kr-page");
        let left = ui::vbox(16, "");
        left.set_valign(Align::Start);
        let right = ui::vbox(20, "");
        right.set_hexpand(true);
        right.set_valign(Align::Start);
        page.append(&left);
        page.append(&right);
        let root = gtk::ScrolledWindow::new();
        root.set_hscrollbar_policy(gtk::PolicyType::Never);
        root.set_child(Some(&page));

        let share = ShareCard::new();
        left.append(&share.root);

        // ask the agent
        let agent = ui::vbox(10, "kr-agent");
        agent.append(&{
            let b = ui::hbox(6, "");
            b.append(&ui::icon("spark", "k_accent", 16));
            b.append(&ui::label(tr("ASK THE AGENT"), "kr-section"));
            b
        });
        agent.append(&ui::para(
            tr("Say what you need. The agent connects and does it for you, and every step on the other computer asks for its approval."),
            "kr-body",
        ));
        let req = ui::hbox(8, "kr-field");
        req.append(&ui::icon("spark", "k_muted", 16));
        let req_entry = gtk::Entry::new();
        req_entry.set_hexpand(true);
        req_entry.set_placeholder_text(Some(tr("“connect to mom's laptop and install the printer”")));
        req.append(&req_entry);
        agent.append(&req);
        left.append(&agent);

        // connect to another computer
        let connect = ui::vbox(12, "kr-card");
        let ch = ui::hbox(8, "");
        let ct = ui::card_title("rede-cabo", tr("CONNECT TO ANOTHER COMPUTER"));
        ct.set_hexpand(true);
        ch.append(&ct);
        let (lan_badge, lan_label) = ui::badge("", "blue");
        lan_badge.set_visible(false);
        ch.append(&lan_badge);
        connect.append(&ch);
        let line = ui::hbox(8, "kr-connect-line");
        let target_box = ui::hbox(8, "kr-field");
        target_box.set_hexpand(true);
        target_box.append(&ui::icon("busca", "k_muted", 16));
        let target = gtk::Entry::new();
        target.set_hexpand(true);
        target.set_placeholder_text(Some(tr("Paste the code or the link  ·  e.g. 7Q4M-29XR or …trycloudflare.com")));
        target_box.append(&target);
        line.append(&target_box);
        let (seg, kind) = ui::segmented(&[tr("Control"), tr("View only"), tr("Files")], "kr-kind");
        seg.set_size_request(300, -1);
        seg.set_valign(Align::Center);
        line.append(&seg);
        let connect_btn = ui::button(tr("Connect"), Some(("monitor", "k_accent_fg")), "primary tall");
        line.append(&connect_btn);
        connect.append(&line);
        let connect_stack = gtk::Stack::new();
        connect_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        connect_stack.set_interpolate_size(true);
        connect_stack.set_vhomogeneous(false);
        connect_stack.set_hhomogeneous(false);
        connect_stack.add_named(
            &ui::para(
                tr("Whoever shares gives a code or a link. On the same network the connection goes direct; outside it, through the Cloudflare tunnel. Nothing goes through KalangoOS servers."),
                "kr-caption",
            ),
            Some("hint"),
        );
        let steps = ui::vbox(8, "");
        connect_stack.add_named(&steps, Some("steps"));
        let password_box = ui::vbox(10, "");
        let password_title = ui::label("", "kr-row-title");
        password_box.append(&password_title);
        let prow = ui::hbox(8, "kr-field");
        prow.append(&ui::icon("cadeado", "k_muted", 16));
        let password = gtk::Entry::new();
        password.set_visibility(false);
        password.set_hexpand(true);
        password.set_placeholder_text(Some(tr("One-time password")));
        prow.append(&password);
        password_box.append(&prow);
        let error_label = ui::para("", "kr-error-text");
        error_label.set_visible(false);
        password_box.append(&error_label);
        let pbtns = ui::hbox(8, "");
        let pcancel = ui::button(tr("Cancel"), None, "");
        pcancel.set_hexpand(true);
        pbtns.append(&pcancel);
        let penter = ui::button(tr("Enter"), None, "primary");
        penter.set_hexpand(true);
        pbtns.append(&penter);
        password_box.append(&pbtns);
        connect_stack.add_named(&password_box, Some("password"));
        connect.append(&connect_stack);
        right.append(&connect);

        let (fav_section, favorites) = device_section(tr("FAVORITES"), tr("See all"));
        let (rec_section, recents) = device_section(tr("RECENT"), tr("Clear"));
        right.append(&fav_section);
        right.append(&rec_section);

        let home = Rc::new(Home {
            root,
            win: Rc::downgrade(win),
            page,
            left,
            right,
            share,
            state: RefCell::new(ShareState::Idle),
            core_answered: Cell::new(false),
            core_failures: Cell::new(0),
            retry_at: Rc::new(Cell::new(0)),
            info: RefCell::new(ShareInfo::default()),
            core: RefCell::new(None),
            incoming: RefCell::new(None),
            target,
            target_box,
            kind,
            connect_btn,
            connect_stack,
            steps,
            password_box,
            password,
            password_title,
            error_label,
            lan_badge,
            lan_label,
            pending: RefCell::new(None),
            favorites,
            recents,
            fav_section,
            rec_section,
            filter: RefCell::new(String::new()),
            grid_width: Cell::new(0),
        });

        // wiring: share card
        let actions: [(&str, fn(&Rc<Home>)); 11] = [
            ("share", Home::start_sharing),
            ("retry", Home::start_sharing),
            ("cancel", Home::stop_sharing_rc),
            ("stop", Home::stop_sharing_rc),
            ("copy-code", Home::copy_code),
            ("copy-link", Home::copy_link),
            ("copy-invite", Home::copy_invite),
            ("show-password", Home::show_password),
            ("new-password", Home::new_password),
            ("extend", Home::extend),
            ("ask-agent", Home::ask_agent_tunnel),
        ];
        for (name, f) in actions {
            for b in home.share.buttons(name) {
                let h = Rc::downgrade(&home);
                b.connect_clicked(move |_| {
                    if let Some(h) = h.upgrade() {
                        f(&h);
                    }
                });
            }
        }
        {
            let h = Rc::downgrade(&home);
            home.share.ask.connect_state_set(move |_, on| {
                if let Some(h) = h.upgrade() {
                    if let Some(w) = h.win.upgrade() {
                        w.settings.borrow_mut().ask_before = on;
                        w.settings_changed();
                    }
                }
                glib::Propagation::Proceed
            });
            let h = Rc::downgrade(&home);
            home.share.unattended.connect_state_set(move |s, on| {
                if let Some(w) = h.upgrade().and_then(|h| h.win.upgrade()) {
                    if on {
                        // turning it on happens in Settings (fixed password, allowed devices)
                        s.set_active(false);
                        w.show_page("settings");
                        return glib::Propagation::Stop;
                    }
                    w.settings.borrow_mut().unattended = false;
                    w.settings_changed();
                }
                glib::Propagation::Proceed
            });
        }
        // agent request
        {
            let h = Rc::downgrade(&home);
            req_entry.connect_activate(move |e| {
                let t = e.text().to_string();
                if t.trim().is_empty() {
                    return;
                }
                if let Some(w) = h.upgrade().and_then(|h| h.win.upgrade()) {
                    w.ask_agent(&t);
                }
                e.set_text("");
            });
        }
        // connect
        {
            let h = Rc::downgrade(&home);
            let go = move || {
                if let Some(h) = h.upgrade() {
                    h.connect();
                }
            };
            let g = go.clone();
            home.connect_btn.connect_clicked(move |_| g());
            let g = go.clone();
            home.target.connect_activate(move |_| g());
            let h2 = Rc::downgrade(&home);
            penter.connect_clicked(move |_| {
                if let Some(h) = h2.upgrade() {
                    h.connect();
                }
            });
            let h2 = Rc::downgrade(&home);
            home.password.connect_activate(move |_| {
                if let Some(h) = h2.upgrade() {
                    h.connect();
                }
            });
            let h2 = Rc::downgrade(&home);
            pcancel.connect_clicked(move |_| {
                if let Some(h) = h2.upgrade() {
                    h.cancel_connect();
                }
            });
            let tb = home.target_box.clone();
            home.target.connect_changed(move |_| tb.remove_css_class("error"));
        }
        // layout follows the width
        {
            let h = Rc::downgrade(&home);
            ui::on_width(&home.root, move |w| {
                if let Some(h) = h.upgrade() {
                    h.relayout(w);
                }
            });
        }
        home.settings_changed();
        home.refresh_devices();
        home.apply_state();
        home
    }

    pub fn with_core(&self, f: impl FnOnce(&Core)) {
        if let Some(c) = self.core.borrow().as_ref() {
            f(c);
        }
    }

    fn win(&self) -> Option<Rc<Window>> {
        self.win.upgrade()
    }

    pub fn settings_changed(&self) {
        let Some(w) = self.win() else { return };
        let s = w.settings.borrow().clone();
        self.share.ask.set_active(s.ask_before);
        self.share.unattended.set_active(s.unattended);
        self.share.unattended_sub.set_text(if s.unattended {
            tr("On · fixed password and allowed devices in Settings")
        } else {
            tr("Off · fixed password and allowed devices in Settings")
        });
    }

    // -------------------------------------------------------------------------------------- layout

    fn relayout(&self, width: i32) {
        // the page's own padding is 24 on each side
        let inner = width - 48;
        if width < 900 {
            self.page.set_orientation(gtk::Orientation::Vertical);
            self.left.set_size_request(-1, -1);
            self.left.set_hexpand(true);
        } else {
            self.page.set_orientation(gtk::Orientation::Horizontal);
            let lw = if width >= 1100 { 440 } else { 360 + (width - 900) * 80 / 200 };
            self.left.set_size_request(lw, -1);
            self.left.set_hexpand(false);
        }
        let grid = if width < 900 { inner } else { inner - self.left.width_request() - 16 };
        if grid != self.grid_width.get() {
            self.grid_width.set(grid);
            self.refresh_devices();
        }
        // narrow: the connect line wraps (the field on its own row)
        if let Some(line) = self.target_box.parent().and_downcast::<gtk::Box>() {
            line.set_orientation(if grid < 700 { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
        }
    }

    pub fn filter(&self, text: &str) {
        *self.filter.borrow_mut() = text.to_lowercase();
        self.refresh_devices();
    }

    pub fn refresh_devices(self: &Home) {
        let Some(w) = self.win() else { return };
        let peers = w.peers.borrow().clone();
        let f = self.filter.borrow().clone();
        let keep = |p: &Peer| f.is_empty() || p.name.to_lowercase().contains(&f) || p.target.to_lowercase().contains(&f);
        let favs: Vec<Peer> = peers.favorites().into_iter().filter(keep).collect();
        let recs: Vec<Peer> = peers.recents().into_iter().filter(keep).collect();
        let width = self.grid_width.get().max(200);
        fill_grid(&self.favorites, &favs, width, &self.win, tr("Mark a computer with the star to keep it here."));
        fill_grid(&self.recents, &recs, width, &self.win, tr("The computers you connect to appear here."));
        let _ = (&self.fav_section, &self.rec_section);
    }

    // ------------------------------------------------------------------------------------- sharing

    fn apply_state(&self) {
        let st = self.state.borrow().clone();
        let lan = self.info.borrow().lan;
        match &st {
            ShareState::Idle => {
                self.share.stack.set_visible_child_name("idle");
                self.share.set_badge(tr("not shared"), "dim");
            }
            ShareState::Opening => {
                self.share.stack.set_visible_child_name("opening");
                self.share.set_badge(tr("opening tunnel…"), "gold");
            }
            ShareState::Sharing => {
                self.share.stack.set_visible_child_name("sharing");
                if lan {
                    self.share.set_badge(tr("on the same network"), "blue");
                } else {
                    self.share.set_badge(tr("via Cloudflare"), "teal");
                }
                self.tick();
            }
            ShareState::NoInternet => {
                self.share.stack.set_visible_child_name("noinet");
                self.share.set_badge(tr("no internet"), "coral");
                let ip = local_ip().unwrap_or_else(|| "—".into());
                self.share.noinet_text.set_text(&trf("On the same network you can still get a direct connection: your local address is {ip}.",
                    &[("ip", ip)],
                ));
            }
            ShareState::CoreFailed(why) => {
                self.share.stack.set_visible_child_name("nocore");
                self.share.set_badge(tr("core stopped"), "coral");
                self.share.core_title.set_text(tr("The Kalango Remote core did not open."));
                self.share.core_text.set_text(why);
                self.retry_countdown();
            }
            ShareState::AlreadySharing => {
                self.share.stack.set_visible_child_name("nocore");
                self.share.set_badge(tr("already sharing"), "gold");
                self.share.core_title.set_text(tr("This computer is already shared by another Kalango Remote."));
                self.share.core_text.set_text(tr("Only one can receive connections at a time. Stop sharing in the other window (or close it) and try again here."));
                self.retry_countdown();
            }
            ShareState::Failed(why) => {
                self.share.stack.set_visible_child_name("failed");
                self.share.set_badge(tr("tunnel failed"), "coral");
                self.share.failed_text.set_text(&format!(
                    "{} {}",
                    tr("Cloudflare did not answer (the network may block port 7844). You can still get a connection on the same network, or try again."),
                    if why.is_empty() { String::new() } else { format!("({why})") }
                ));
            }
        }
    }

    fn set_state(&self, s: ShareState) {
        *self.state.borrow_mut() = s;
        self.apply_state();
    }

    pub fn start_sharing(self: &Rc<Home>) {
        let Some(w) = self.win() else { return };
        self.stop_sharing();
        let s = w.settings.borrow().clone();
        let lan = s.network == "lan";
        let mut args = vec!["serve".to_string()];
        if lan {
            args.extend(["--no-tunnel".into(), "--lan".into()]);
        }
        if s.ask_before {
            args.push("--ask".into());
        }
        *self.info.borrow_mut() = ShareInfo { lan, ..Default::default() };
        self.core_answered.set(false);
        let h = Rc::downgrade(self);
        match Core::spawn(&args, move |m| {
            if let Some(h) = h.upgrade() {
                h.on_core(m);
            }
        }) {
            Ok(c) => {
                *self.core.borrow_mut() = Some(c);
                self.share.opening_step.set_text(if lan {
                    tr("starting the direct connection on this network")
                } else {
                    tr("cloudflared · asking for a temporary address")
                });
                self.set_state(ShareState::Opening);
            }
            Err(e) => self.core_failed(format!("{}: {e}", crate::core::core_path().display())),
        }
    }

    /// The core exited before answering: say so with its own words, and make each new try wait longer
    /// (2, 4, 8… s up to 30) so a core that cannot start is not restarted in a loop.
    fn core_failed(&self, why: String) {
        let n = self.core_failures.get().saturating_add(1);
        self.core_failures.set(n);
        let wait = 2u64.saturating_pow(n.min(5)).min(30);
        self.retry_at.set(store::now() + wait);
        self.set_state(ShareState::CoreFailed(why));
    }

    /// The "Try again" of the core error waits for `retry_at`, counting down on the button.
    fn retry_countdown(&self) {
        let b = self.share.core_retry.clone();
        let at = self.retry_at.clone();
        let update = move || {
            let left = at.get().saturating_sub(store::now());
            let label = b.child().and_then(|r| r.last_child()).and_then(|l| l.downcast::<gtk::Label>().ok());
            b.set_sensitive(left == 0);
            if let Some(l) = label {
                if left == 0 {
                    l.set_text(tr("Try again"));
                } else {
                    l.set_text(&trf("Try again in {n} s", &[("n", left.to_string())]));
                }
            }
            if left == 0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        };
        if update() == glib::ControlFlow::Continue {
            glib::timeout_add_seconds_local(1, update);
        }
    }

    fn stop_sharing_rc(self: &Rc<Home>) {
        self.stop_sharing();
    }

    pub fn end_sessions(&self) {
        if let Some(i) = self.incoming.borrow().as_ref() {
            i.end_sessions();
        }
    }

    pub fn stop_sharing(&self) {
        if let Some(c) = self.core.borrow_mut().take() {
            c.stop();
        }
        if let Some(i) = self.incoming.borrow_mut().take() {
            i.close_all();
        }
        if *self.state.borrow() != ShareState::Idle {
            self.set_state(ShareState::Idle);
        }
    }

    fn on_core(self: &Rc<Home>, m: Msg) {
        let v = match m {
            Msg::Event(v) => v,
            Msg::Frame(_) => return,
            Msg::Exited(exit) => {
                let ours = self.core.borrow_mut().take().is_some();
                if ours {
                    eprintln!("kalango-remote: o núcleo saiu (código {:?}): {}", exit.code, exit.last_error);
                }
                // still opening (with or without a first answer): never back to the start without a word
                if ours && *self.state.borrow() == ShareState::Opening {
                    // before any answer the first line says why (a missing library…); later, the last one
                    let line = if self.core_answered.get() { exit.last_error } else { exit.first_error };
                    let why = if line.is_empty() {
                        trf("It exited with code {code}.", &[("code", exit.code.map_or("?".into(), |c| c.to_string()))])
                    } else {
                        line
                    };
                    self.core_failed(why);
                    return;
                }
                if matches!(*self.state.borrow(), ShareState::Sharing | ShareState::Opening) {
                    self.set_state(ShareState::Idle);
                }
                return;
            }
        };
        self.core_answered.set(true);
        let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
        match v["event"].as_str().unwrap_or("") {
            "listening" => {
                let mut i = self.info.borrow_mut();
                i.code = s("code");
                i.password = s("password");
                i.addr = s("addr");
                if i.lan {
                    let port = i.addr.rsplit(':').next().unwrap_or("21118").to_owned();
                    let ip = local_ip().unwrap_or_else(|| "127.0.0.1".into());
                    let inv = kalango_remote_proto::Invite {
                        host: format!("{ip}:{port}"),
                        identity: kalango_remote_proto::invite::b64url_decode(&s("pk"))
                            .and_then(|pk| pk.try_into().ok())
                            .map(|pk: [u8; 32]| (s("id"), pk)),
                    };
                    i.link = inv.link();
                    drop(i);
                    self.begin_shared();
                }
            }
            // the tunnel is up; its address still has to answer in DNS before the link works (up to ~30 s): say so,
            // or the card looks stuck on "asking for a temporary address"
            "tunnel_ready" if *self.state.borrow() == ShareState::Opening => {
                self.share.opening_step.set_text(tr("tunnel open · waiting for its address to answer"));
            }
            "tunnel" => {
                self.info.borrow_mut().link = s("link");
                self.begin_shared();
            }
            "error" if v["kind"] == "already_sharing" => {
                if let Some(c) = self.core.borrow_mut().take() {
                    c.stop();
                }
                let n = self.core_failures.get().saturating_add(1);
                self.core_failures.set(n);
                self.retry_at.set(store::now() + 2u64.saturating_pow(n.min(5)).min(30));
                self.set_state(ShareState::AlreadySharing);
            }
            "tunnel_error" => {
                let text = s("text");
                let offline = local_ip().is_none() || text.contains("no such host") || text.contains("unreachable");
                if let Some(c) = self.core.borrow_mut().take() {
                    c.stop();
                }
                self.set_state(if offline { ShareState::NoInternet } else { ShareState::Failed(text) });
            }
            "password" => {
                self.info.borrow_mut().password = s("password");
                self.show_password_value();
            }
            "connection" | "removed" | "chat" => {
                let existing = self.incoming.borrow().clone();
                let inc = match existing {
                    Some(i) => i,
                    None => {
                        let i = Incoming::new(self);
                        *self.incoming.borrow_mut() = Some(i.clone());
                        i
                    }
                };
                inc.on_event(&v);
            }
            _ => {}
        }
    }

    fn begin_shared(self: &Rc<Home>) {
        let Some(w) = self.win() else { return };
        let minutes = w.settings.borrow().share_minutes.max(5) as u64;
        {
            let mut i = self.info.borrow_mut();
            i.until = store::now() + minutes * 60;
        }
        let i = self.info.borrow();
        self.share.code.set_text(&i.code);
        self.share.link.set_text(i.link.trim_start_matches("https://").split('/').next().unwrap_or(""));
        self.share.link.set_tooltip_text(Some(&i.link));
        if let Some(t) = qr_texture(&i.link) {
            self.share.qr.set_paintable(Some(&t));
        }
        // KALANGO_REMOTE_DEBUG: tests read the link from the log; the one-time password only with
        // KALANGO_REMOTE_DEBUG=senha (a test profile's throwaway password, never in normal use)
        match std::env::var("KALANGO_REMOTE_DEBUG").as_deref() {
            Ok("senha") => eprintln!("debug compartilhando: código {} link {} senha {}", i.code, i.link, i.password),
            Ok(_) => eprintln!("debug compartilhando: código {} link {}", i.code, i.link),
            Err(_) => {}
        }
        drop(i);
        self.share.pass_shown.set(false);
        self.show_password_value();
        self.core_failures.set(0);
        self.set_state(ShareState::Sharing);
        // the clock: once a second while sharing
        let h = Rc::downgrade(self);
        glib::timeout_add_seconds_local(1, move || {
            let Some(h) = h.upgrade() else { return glib::ControlFlow::Break };
            if *h.state.borrow() != ShareState::Sharing {
                return glib::ControlFlow::Break;
            }
            h.tick();
            glib::ControlFlow::Continue
        });
    }

    fn tick(&self) {
        let Some(w) = self.win() else { return };
        let total = w.settings.borrow().share_minutes.max(5) as u64 * 60;
        let left = self.info.borrow().until.saturating_sub(store::now());
        if left == 0 {
            self.stop_sharing();
            w.toast(tr("The share closed at the end of its time"));
            return;
        }
        self.share.time_label.set_text(&trf("Open for {n} more min", &[("n", store::minutes_left(left).to_string())]));
        let full = self.share.time_track.width().max(1);
        let frac = (left as f64 / total.max(left) as f64).clamp(0.0, 1.0);
        self.share.time_fill.set_size_request((full as f64 * frac) as i32, 3);
        // the last 5 minutes are gold
        if left <= 300 {
            self.share.time_track.add_css_class("gold");
        } else {
            self.share.time_track.remove_css_class("gold");
        }
    }

    fn extend(self: &Rc<Home>) {
        let mut i = self.info.borrow_mut();
        i.until += 30 * 60;
        drop(i);
        self.tick();
    }

    fn show_password_value(&self) {
        let i = self.info.borrow();
        if self.share.pass_shown.get() {
            self.share.pass.set_text(&i.password);
            self.share.pass.remove_css_class("kr-dots");
            self.share.pass.add_css_class("kr-pass");
        } else {
            self.share.pass.set_text(&"●".repeat(i.password.chars().count().max(6)));
            self.share.pass.add_css_class("kr-dots");
            self.share.pass.remove_css_class("kr-pass");
        }
    }

    fn show_password(self: &Rc<Home>) {
        self.share.pass_shown.set(true);
        self.show_password_value();
        let h = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_secs(10), move || {
            if let Some(h) = h.upgrade() {
                h.share.pass_shown.set(false);
                h.show_password_value();
            }
        });
    }

    fn new_password(self: &Rc<Home>) {
        if let Some(c) = self.core.borrow().as_ref() {
            c.send(&json!({"cmd": "new_password"}));
        }
    }

    fn copy_code(self: &Rc<Home>) {
        ui::copy(&self.root, &self.info.borrow().code);
        if let Some(w) = self.win() {
            w.toast(tr("Code copied"));
        }
    }

    fn copy_link(self: &Rc<Home>) {
        ui::copy(&self.root, &self.info.borrow().link);
        if let Some(w) = self.win() {
            w.toast(tr("Link copied"));
        }
    }

    fn copy_invite(self: &Rc<Home>) {
        let i = self.info.borrow();
        let text = trf("To help me on my computer, open Kalango Remote and paste this link: {link}\nCheck that the code is {code}. I'll tell you the password.",
            &[("link", i.link.clone()), ("code", i.code.clone())],
        );
        ui::copy(&self.root, &text);
        if let Some(w) = self.win() {
            w.toast(tr("Invite copied"));
        }
    }

    fn ask_agent_tunnel(self: &Rc<Home>) {
        if let Some(w) = self.win() {
            w.ask_agent(tr("Kalango Remote could not open the Cloudflare tunnel on this computer. Check the network (port 7844) and tell me what blocks it."));
        }
    }

    // ----------------------------------------------------------------------------------- connecting

    fn selected_kind(&self) -> Kind {
        if self.kind[1].is_active() {
            Kind::ViewOnly
        } else if self.kind[2].is_active() {
            Kind::Files
        } else {
            Kind::Control
        }
    }

    pub fn connect_to(self: &Rc<Home>, target: &str, kind: Kind) {
        self.target.set_text(target);
        self.kind[match kind {
            Kind::Control => 0,
            Kind::ViewOnly => 1,
            Kind::Files => 2,
        }]
        .set_active(true);
        self.connect();
    }

    fn connect(self: &Rc<Home>) {
        let target = self.target.text().trim().to_string();
        if kalango_remote_proto::Target::parse(&target).is_err() {
            self.target_box.add_css_class("error");
            return;
        }
        // first the password (the one-time password the other person sees)
        let pw = self.password.text().to_string();
        if self.connect_stack.visible_child_name().as_deref() != Some("password") || pw.is_empty() {
            let who = match kalango_remote_proto::Target::parse(&target) {
                Ok(kalango_remote_proto::Target::Invite(i)) => i.code().unwrap_or(i.host),
                _ => target.clone(),
            };
            self.password_title.set_text(&trf("Password of {who}", &[("who", who)]));
            self.error_label.set_visible(false);
            self.connect_stack.set_visible_child_name("password");
            self.password.grab_focus();
            return;
        }
        self.show_steps(&target);
        let h = Rc::downgrade(self);
        let s = session::Session::start(self.win.clone(), &target, &pw, self.selected_kind(), move |ev| {
            if let Some(h) = h.upgrade() {
                h.on_connect_event(ev);
            }
        });
        *self.pending.borrow_mut() = Some(s);
    }

    fn cancel_connect(self: &Rc<Home>) {
        if let Some(s) = self.pending.borrow_mut().take() {
            s.close();
        }
        self.password.set_text("");
        self.connect_stack.set_visible_child_name("hint");
    }

    fn show_steps(&self, target: &str) {
        while let Some(c) = self.steps.first_child() {
            self.steps.remove(&c);
        }
        let is_code = matches!(kalango_remote_proto::Target::parse(target), Ok(kalango_remote_proto::Target::Code(_)));
        let first = if is_code { tr("found by the code") } else { tr("found by the link") };
        self.add_step("done", tr("Searching"), first);
        self.add_step(
            "spin",
            if is_code { tr("Trying a direct connection on the same network") } else { tr("Connecting through the Cloudflare tunnel") },
            "",
        );
        let cancel = ui::button(tr("Cancel"), None, "");
        let h = Rc::downgrade(&self.win.upgrade().and_then(|w| w.home.borrow().clone()).unwrap_or_else(|| unreachable_home()));
        cancel.connect_clicked(move |_| {
            if let Some(h) = h.upgrade() {
                h.cancel_connect();
            }
        });
        self.steps.append(&cancel);
        self.connect_stack.set_visible_child_name("steps");
    }

    fn add_step(&self, state: &str, title: &str, sub: &str) {
        let row = ui::hbox(10, "");
        match state {
            "spin" => {
                let s = gtk::Spinner::new();
                s.set_spinning(true);
                s.set_size_request(16, 16);
                s.set_valign(Align::Start);
                row.append(&s);
            }
            _ => {
                let i = ui::icon("check", "k_teal", 16);
                i.set_valign(Align::Start);
                row.append(&i);
            }
        }
        let t = ui::vbox(2, "");
        t.append(&ui::label(title, "kr-step-title"));
        if !sub.is_empty() {
            t.append(&ui::label(sub, "kr-step-sub"));
        }
        row.append(&t);
        // before the Cancel button, if there is one
        match self.steps.last_child().filter(|c| c.is::<gtk::Button>()) {
            Some(b) => row.insert_before(&self.steps, Some(&b)),
            None => self.steps.append(&row),
        }
    }

    fn on_connect_event(self: &Rc<Home>, ev: session::ConnectEvent) {
        match ev {
            session::ConnectEvent::Waiting(text) => self.add_step("spin", &text, ""),
            session::ConnectEvent::Connected => {
                self.pending.borrow_mut().take();
                self.password.set_text("");
                self.target.set_text("");
                self.connect_stack.set_visible_child_name("hint");
                self.refresh_devices();
            }
            session::ConnectEvent::WrongPassword => {
                self.pending.borrow_mut().take();
                self.connect_stack.set_visible_child_name("password");
                self.error_label.set_text(tr("Wrong password. Ask the person to read it again."));
                self.error_label.set_visible(true);
                self.password.select_region(0, -1);
                self.password.grab_focus();
            }
            session::ConnectEvent::Failed(text) => {
                self.pending.borrow_mut().take();
                self.connect_stack.set_visible_child_name("password");
                self.error_label.set_text(&text);
                self.error_label.set_visible(true);
                self.target_box.add_css_class("error");
            }
        }
    }
}

fn unreachable_home() -> Rc<Home> {
    panic!("home page missing")
}


fn device_section(title: &str, action: &str) -> (gtk::Box, gtk::Box) {
    let sec = ui::vbox(10, "");
    let head = ui::hbox(8, "");
    let t = ui::label(title, "kr-section");
    t.set_hexpand(true);
    head.append(&t);
    let a = gtk::Button::with_label(action);
    a.add_css_class("kr-link-btn");
    a.set_widget_name(if action == tr("Clear") { "clear-recents" } else { "see-all" });
    a.connect_clicked(|b| {
        if b.widget_name() == "clear-recents" {
            if let Some(w) = Window::get() {
                w.peers.borrow_mut().clear_recents();
                w.peers.borrow().save();
                if let Some(h) = w.home.borrow().clone() {
                    h.refresh_devices();
                }
            }
        } else if let Some(w) = Window::get() {
            w.show_page("history");
        }
    });
    head.append(&a);
    sec.append(&head);
    let grid = ui::vbox(12, "");
    sec.append(&grid);
    (sec, grid)
}

/// Rows of device cards: as many columns as fit (200–280 px), filled row by row.
fn fill_grid(grid: &gtk::Box, peers: &[Peer], width: i32, win: &Weak<Window>, empty: &str) {
    while let Some(c) = grid.first_child() {
        grid.remove(&c);
    }
    if peers.is_empty() {
        grid.append(&ui::label(empty, "kr-empty"));
        return;
    }
    let cols = columns_for(width) as usize;
    // the design's thumbnail is 206×117 inside a card with 9 px around it
    let card_w = (width - 12 * (cols as i32 - 1)) / cols as i32;
    let thumb_h = ((card_w - 18) * 117 / 206).max(60);
    // rows share the width evenly (homogeneous): every card is between 200 and ~255 px
    for chunk in peers.chunks(cols) {
        let row = ui::hbox(12, "");
        row.set_homogeneous(true);
        for p in chunk {
            row.append(&device_card(p, win, thumb_h));
        }
        for _ in chunk.len()..cols {
            row.append(&gtk::Box::new(gtk::Orientation::Horizontal, 0));
        }
        grid.append(&row);
    }
}

/// How many device columns fit in `width` (cards 200 to 280 px, 12 px apart).
pub fn columns_for(width: i32) -> i32 {
    ((width + 12) / (200 + 12)).max(1)
}

fn device_card(p: &Peer, win: &Weak<Window>, thumb_h: i32) -> gtk::Widget {
    let card = ui::vbox(8, "kr-device");
    let thumb = gtk::Overlay::new();
    thumb.add_css_class("kr-thumb");
    thumb.set_overflow(gtk::Overflow::Hidden);
    let pic = gtk::Picture::new();
    pic.set_content_fit(gtk::ContentFit::Cover);
    pic.set_can_shrink(true);
    if let Some(path) = p.thumb.as_ref().filter(|p| p.is_file()) {
        pic.set_filename(Some(path));
    }
    thumb.set_child(Some(&pic));
    let state = ui::hbox(5, "kr-state");
    let dot = ui::hbox(0, "dot");
    dot.set_valign(Align::Center);
    state.append(&dot);
    // the route it took last time (whether it is online now is not known without a server)
    state.append(&gtk::Label::new(Some(if p.route == "lan" { tr("same network") } else { tr("via link") })));
    if p.route != "lan" {
        state.add_css_class("off");
    }
    state.set_halign(Align::Start);
    state.set_valign(Align::Start);
    state.set_margin_start(6);
    state.set_margin_top(6);
    thumb.add_overlay(&state);
    thumb.set_size_request(-1, thumb_h);
    card.append(&thumb);
    let info = ui::hbox(8, "");
    info.set_margin_start(4);
    info.set_margin_end(4);
    info.set_margin_bottom(4);
    let t = ui::vbox(2, "");
    t.set_hexpand(true);
    t.append(&ui::ellip(&p.name, "kr-device-name"));
    let sub = if p.latency_ms > 0 {
        format!("{} · {} ms", if p.route == "lan" { tr("on the same network") } else { tr("via Cloudflare") }, p.latency_ms)
    } else {
        store::ago(p.last_used, store::now())
    };
    t.append(&ui::ellip(&sub, "kr-caption"));
    info.append(&t);
    if p.view_only {
        info.append(&ui::label(tr("view only"), "kr-tag gold"));
    }
    // favourites: the gold star takes it out; recents: a menu (keep as favourite, forget)
    let refresh = || {
        if let Some(w) = Window::get() {
            w.peers.borrow().save();
            let home = w.home.borrow().clone();
            if let Some(h) = home {
                h.refresh_devices();
            }
        }
    };
    if p.favorite {
        let star = ui::icon_button("estrela-n", "k_gold", tr("Remove from favorites"));
        star.add_css_class("small");
        let target = p.target.clone();
        star.connect_clicked(move |_| {
            if let Some(w) = Window::get() {
                w.peers.borrow_mut().toggle_favorite(&target);
            }
            refresh();
        });
        info.append(&star);
    } else {
        let more = gtk::MenuButton::new();
        more.add_css_class("kr-icon-btn");
        more.add_css_class("small");
        more.set_has_frame(false);
        more.set_valign(Align::Center);
        more.set_child(Some(&ui::icon("pontos-v", "k_muted", 16)));
        let pop = gtk::Popover::new();
        pop.add_css_class("kr-menu");
        let col = ui::vbox(2, "");
        let fav = gtk::Button::with_label(tr("Add to favorites"));
        fav.add_css_class("kr-menu-item");
        let forget = gtk::Button::with_label(tr("Forget"));
        forget.add_css_class("kr-menu-item");
        col.append(&fav);
        col.append(&forget);
        pop.set_child(Some(&col));
        more.set_popover(Some(&pop));
        let target = p.target.clone();
        fav.connect_clicked(move |_| {
            if let Some(w) = Window::get() {
                w.peers.borrow_mut().toggle_favorite(&target);
            }
            refresh();
        });
        let target = p.target.clone();
        forget.connect_clicked(move |_| {
            if let Some(w) = Window::get() {
                w.peers.borrow_mut().list.retain(|p| p.target != target);
            }
            refresh();
        });
        info.append(&more);
    }
    card.append(&info);
    // a click on the card connects again (same kind as last time)
    let click = gtk::GestureClick::new();
    let target = p.target.clone();
    let view_only = p.view_only;
    let win = win.clone();
    click.connect_released(move |g, n, _, _| {
        if n == 1 {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(h) = win.upgrade().and_then(|w| w.home.borrow().clone()) {
                h.connect_to(&target, if view_only { Kind::ViewOnly } else { Kind::Control });
            }
        }
    });
    thumb.add_controller(click);
    let _ = Value::Null;
    card.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_columns_follow_the_width() {
        // the design: 932 px → 4 columns of 224; the narrow window's 560 → 2
        assert_eq!(columns_for(932), 4);
        assert_eq!(columns_for(560), 2);
        assert_eq!(columns_for(150), 1);
        assert_eq!(columns_for(211), 1);
        assert_eq!(columns_for(412), 2);
    }
}
