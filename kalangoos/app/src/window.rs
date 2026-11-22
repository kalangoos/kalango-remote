//! The main window: the rail (the app, then its areas), the toolbar (title, search or ask, menu, the
//! window's buttons) and the pages.

use gtk::{gdk, gio, glib, prelude::*, Align, Orientation};
use std::{cell::RefCell, rc::Rc};

use crate::{
    home::Home,
    i18n::{n_, tr},
    plan::{Decision, Plan},
    plan_panel::PlanPanel,
    session::Kind,
    settings::SettingsPage,
    store::{Peers, Settings},
    ui,
};

/// (page id, icon, tooltip)
const PAGES: &[(&str, &str, &str)] = &[
    ("home", "monitor", n_("Remote desktop")),
    ("ssh", "terminal", n_("SSH")),
    ("transfers", "pasta", n_("Transfers")),
    ("history", "relogio", n_("History")),
    ("settings", "engrenagem", n_("Settings")),
];

pub struct Window {
    pub window: gtk::ApplicationWindow,
    pub overlay: gtk::Overlay,
    pub stack: gtk::Stack,
    rail: RefCell<Vec<(String, gtk::Button, gtk::Image, &'static str)>>,
    pub settings: RefCell<Settings>,
    pub peers: RefCell<Peers>,
    pub home: RefCell<Option<Rc<Home>>>,
    pub search: gtk::Entry,
    /// sessions with a window open (the home page lets go of them once connected)
    pub sessions: RefCell<Vec<Rc<crate::session::Session>>>,
    /// the SSH page (its handlers hold weak references: the window keeps it alive)
    plan: RefCell<Option<Rc<PlanPanel>>>,
    pub ssh: RefCell<Option<Rc<crate::ssh::page::SshPage>>>,
}

thread_local! {
    static WINDOW: RefCell<Option<Rc<Window>>> = const { RefCell::new(None) };
}

/// The design's window is 1500×910; smaller outputs get what fits (the layouts follow the space).
fn window_size(widget: &gtk::Widget) -> (i32, i32) {
    let (sw, sh) = gdk::Display::default()
        .and_then(|d| {
            let surface = widget.native().and_then(|n| n.surface());
            surface.and_then(|s| d.monitor_at_surface(&s)).or_else(|| d.monitors().item(0).and_downcast::<gdk::Monitor>())
        })
        .map(|m| (m.geometry().width(), m.geometry().height()))
        .unwrap_or((1920, 1080));
    (1500.min(sw - 48), 910.min(sh - 116))
}

impl Window {
    pub fn get() -> Option<Rc<Window>> {
        WINDOW.with(|w| w.borrow().clone())
    }

    pub fn get_or_create(app: &gtk::Application) -> Rc<Window> {
        if let Some(w) = Self::get() {
            return w;
        }
        let w = Self::new(app);
        WINDOW.with(|s| *s.borrow_mut() = Some(w.clone()));
        w
    }

    fn new(app: &gtk::Application) -> Rc<Window> {
        let window = gtk::ApplicationWindow::builder().application(app).title(tr("Kalango Remote")).build();
        window.add_css_class("kr-window");
        let (w, h) = window_size(window.upcast_ref());
        window.set_default_size(w, h);
        let no_titlebar = gtk::Box::new(Orientation::Horizontal, 0);
        no_titlebar.set_visible(false);
        window.set_titlebar(Some(&no_titlebar));

        // the rail
        let rail_box = ui::vbox(4, "kr-rail");
        let logo = ui::png(include_bytes!("../icons/app-128.png"), 36);
        logo.set_halign(Align::Center);
        rail_box.append(&logo);
        let div = ui::hbox(0, "kr-rail-div");
        div.set_size_request(28, 1);
        div.set_halign(Align::Center);
        rail_box.append(&div);
        let mut rail = Vec::new();
        for (id, glyph, tip) in PAGES {
            let b = gtk::Button::new();
            b.add_css_class("kr-rail-btn");
            let img = ui::icon(glyph, "k_text", 16);
            b.set_child(Some(&img));
            b.set_tooltip_text(Some(tr(tip)));
            rail_box.append(&b);
            rail.push((id.to_string(), b, img, *glyph));
        }

        // the toolbar
        let bar = ui::hbox(10, "kr-toolbar");
        let title = ui::label(tr("Kalango Remote"), "kr-title");
        title.set_hexpand(true);
        bar.append(&title);
        let search_box = ui::hbox(8, "kr-search");
        search_box.set_size_request(340, -1);
        search_box.set_hexpand(false);
        search_box.set_valign(Align::Center);
        search_box.append(&ui::icon("spark", "k_accent", 13));
        let search = gtk::Entry::new();
        search.set_hexpand(true);
        search.set_placeholder_text(Some(tr("Search a device or ask: “connect to mom's laptop”")));
        search_box.append(&search);
        search_box.append(&ui::label("Ctrl K", "kr-kbd"));
        bar.append(&search_box);
        let menu = gtk::MenuButton::new();
        menu.add_css_class("kr-icon-btn");
        menu.set_child(Some(&ui::icon("menu", "k_muted", 16)));
        menu.set_valign(Align::Center);
        menu.set_has_frame(false);
        bar.append(&menu);
        bar.append(&ui::window_controls());
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&bar));

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(180);
        stack.set_vexpand(true);
        stack.set_hexpand(true);
        stack.set_hhomogeneous(false);
        stack.set_vhomogeneous(false);

        let right = ui::vbox(0, "");
        right.append(&handle);
        right.append(&stack);
        let root = ui::hbox(0, "");
        root.append(&rail_box);
        root.append(&right);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&root));
        window.set_child(Some(&overlay));

        let this = Rc::new(Window {
            window,
            overlay,
            stack,
            rail: RefCell::new(rail),
            settings: RefCell::new(Settings::load()),
            peers: RefCell::new(Peers::load()),
            home: RefCell::new(None),
            search,
            sessions: RefCell::new(Vec::new()),
            ssh: RefCell::new(None),
            plan: RefCell::new(None),
        });

        // pages
        let home = Home::new(&this);
        this.stack.add_named(&home.root, Some("home"));
        *this.home.borrow_mut() = Some(home);
        this.stack.add_named(&crate::session::transfers_page(), Some("transfers"));
        this.stack.add_named(&crate::session::history_page(&this), Some("history"));
        this.stack.add_named(&SettingsPage::new(&this).root, Some("settings"));
        let ssh = crate::ssh::page::SshPage::new(&this);
        this.stack.add_named(&ssh.root, Some("ssh"));
        *this.ssh.borrow_mut() = Some(ssh);

        for (id, b, _, _) in this.rail.borrow().iter() {
            let w = Rc::downgrade(&this);
            let id = id.clone();
            b.connect_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    w.show_page(&id);
                }
            });
        }
        this.show_page("home");
        // KALANGO_REMOTE_DEBUG=1: each part's minimum width (a window cannot get narrower than their sum)
        if std::env::var_os("KALANGO_REMOTE_DEBUG").is_some() {
            let root = this.window.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || dump_min(root.upcast_ref(), 0));
        }

        // search: device names filter the home page; a sentence (3+ words) goes to the agent
        let w = Rc::downgrade(&this);
        this.search.connect_changed(move |e| {
            if let Some(h) = w.upgrade().and_then(|w| w.home.borrow().clone()) {
                h.filter(&e.text());
            }
        });
        let w = Rc::downgrade(&this);
        this.search.connect_activate(move |e| {
            let text = e.text().to_string();
            if text.split_whitespace().count() >= 3 {
                if let Some(w) = w.upgrade() {
                    w.ask_agent(&text);
                }
                e.set_text("");
            }
        });
        // Ctrl K
        let keys = gtk::EventControllerKey::new();
        let s = this.search.clone();
        keys.connect_key_pressed(move |_, k, _, m| {
            if m.contains(gdk::ModifierType::CONTROL_MASK) && matches!(k, gdk::Key::k | gdk::Key::K) {
                s.grab_focus();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        this.window.add_controller(keys);
        // closing the window while sharing stops the share (the core ends with its pipe)
        let w = Rc::downgrade(&this);
        this.window.connect_close_request(move |_| {
            if let Some(h) = w.upgrade().and_then(|w| w.home.borrow().clone()) {
                h.stop_sharing();
            }
            glib::Propagation::Proceed
        });
        this
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn show_page(&self, id: &str) {
        let id = match id {
            "ajustes" => "settings",
            other => other,
        };
        if self.stack.child_by_name(id).is_none() {
            return;
        }
        self.stack.set_visible_child_name(id);
        for (pid, b, img, glyph) in self.rail.borrow().iter() {
            let active = pid == id;
            if active {
                b.add_css_class("active");
            } else {
                b.remove_css_class("active");
            }
            ui::set_icon(img, glyph, if active { "k_accent" } else { "k_text" }, 16);
        }
    }

    pub fn toast(&self, text: &str) {
        ui::toast(&self.overlay, text);
    }

    /// Hands a request to KalangoOS's agent (`kalango-shell pedir`), with this app as context; the agent
    /// asks for approval at every step that acts on a computer.
    pub fn ask_agent(&self, request: &str) {
        let text = format!("{} {}", tr("(from Kalango Remote)"), request);
        match std::process::Command::new("kalango-shell").arg("pedir").arg(&text).spawn() {
            Ok(_) => self.toast(tr("Sent to the agent")),
            Err(_) => self.toast(tr("The KalangoOS agent is not available here")),
        }
    }

    /// Shows the agent's plan for approval. The agent's command (`agent-plan`) stays open until the person
    /// decides; then it gets the decision on stdout and its exit status. A plan still waiting is cancelled.
    pub fn show_plan(self: &Rc<Window>, plan: Plan, cmd: gio::ApplicationCommandLine) {
        // (taken first: the answer of the old plan borrows `self.plan` again)
        let old = self.plan.borrow_mut().take();
        if let Some(old) = old {
            old.decide(Decision::Cancelled);
        }
        let me = Rc::downgrade(self);
        let p = plan.clone();
        let cmd_pending = cmd.clone();
        let panel = PlanPanel::new(&plan, move |d| {
            let mut connecting = false;
            if let Some(w) = me.upgrade() {
                connecting = w.on_plan_decision(&p, d);
                if let Some(panel) = w.plan.borrow_mut().take() {
                    w.overlay.remove_overlay(&panel.root);
                }
            }
            cmd.print_literal(&format!("{}\n", p.answer(d, connecting)));
            cmd.set_exit_status(d.exit_status());
        });
        // until a person decides the command fails: closing the window or a crash must never read as "approved"
        cmd_pending.set_exit_status(1);
        self.overlay.add_overlay(&panel.root);
        let root = panel.root.clone();
        ui::on_width(&self.overlay, move |width| {
            // the panel is 560 wide; a narrower window gives it the whole width
            let narrow = width < 560 + 28;
            root.set_halign(if narrow { Align::Fill } else { Align::End });
            root.set_size_request(if narrow { -1 } else { 560 }, -1);
            root.set_margin_start(if narrow { 14 } else { 0 });
        });
        // nobody decided in time: the plan is cancelled here, before the agent gives up waiting (its limit is 10 minutes
        // and it then tells the person nothing was done: a click that came later must not connect anyway)
        let weak_panel = Rc::downgrade(&panel);
        let me = Rc::downgrade(self);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let delay = crate::plan::panel_delay_secs(plan.deadline, now) as u32;
        glib::timeout_add_seconds_local_once(delay, move || {
            let (Some(w), Some(panel)) = (me.upgrade(), weak_panel.upgrade()) else { return };
            let pending = w.plan.borrow().as_ref().is_some_and(|current| Rc::ptr_eq(current, &panel));
            if pending {
                panel.decide(Decision::Cancelled);
            }
        });
        *self.plan.borrow_mut() = Some(panel);
        self.show_page("home");
    }

    /// What the decision does here: approving starts the connection to the device (the one thing the agent
    /// cannot do alone: the other computer's password is asked as usual); adjusting hands the request back to the
    /// search field. Returns whether a connection was started.
    fn on_plan_decision(&self, plan: &Plan, d: Decision) -> bool {
        match d {
            Decision::Approved => {
                let target = if plan.target.is_empty() {
                    let name = plan.device.to_lowercase();
                    self.peers.borrow().list.iter().find(|p| p.name.to_lowercase() == name).map(|p| p.target.clone()).unwrap_or_default()
                } else {
                    plan.target.clone()
                };
                let Some(home) = self.home.borrow().clone() else { return false };
                if target.is_empty() {
                    self.toast(&crate::i18n::trf("Paste the code or link of {device} to connect", &[("device", plan.device.clone())]));
                    return false;
                }
                self.show_page("home");
                home.connect_to(&target, Kind::Control);
                true
            }
            Decision::Adjust => {
                self.search.set_text(&plan.request);
                self.search.grab_focus();
                self.search.set_position(-1);
                false
            }
            Decision::Cancelled => false,
        }
    }

    pub fn settings_changed(&self) {
        self.settings.borrow().save();
        if let Some(h) = self.home.borrow().clone() {
            h.settings_changed();
        }
    }
}


fn dump_min(w: &gtk::Widget, depth: usize) {
    if depth > 16 || !w.is_visible() {
        return;
    }
    let (min, nat, _, _) = w.measure(Orientation::Horizontal, -1);
    if min > 300 {
        eprintln!("debug mínimo: {}{} {} min {min} nat {nat} alocado {}", "  ".repeat(depth), w.type_().name(), w.css_classes().join("."), w.width());
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        c = ch.next_sibling();
        dump_min(&ch, depth + 1);
    }
}

/// `kalango-remote agent-plan FILE|-`: the agent's plan, read from the file or from stdin.
pub fn agent_plan(app: &gtk::Application, cmd: &gio::ApplicationCommandLine, source: Option<&str>) -> i32 {
    let fail = |msg: String| {
        cmd.printerr_literal(&format!("kalango-remote: {msg}\n"));
        2
    };
    let text = match source {
        Some("-") => match cmd.stdin() {
            Some(s) => match s.read_bytes(crate::plan::MAX_BYTES + 1, gio::Cancellable::NONE) {
                Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                Err(e) => return fail(e.to_string()),
            },
            None => return fail("no stdin".into()),
        },
        Some(path) => {
            let p = cmd.cwd().map(|c| c.join(path)).unwrap_or_else(|| path.into());
            match std::fs::File::open(&p).and_then(|f| {
                use std::io::Read;
                let mut s = String::new();
                f.take(crate::plan::MAX_BYTES as u64 + 1).read_to_string(&mut s).map(|_| s)
            }) {
                Ok(s) => s,
                Err(e) => return fail(format!("{}: {e}", p.display())),
            }
        }
        None => return fail("usage: kalango-remote agent-plan FILE|-".into()),
    };
    match Plan::parse(&text) {
        Ok(plan) => {
            let w = Window::get_or_create(app);
            w.present();
            w.show_plan(plan, cmd.clone());
            1 // pending: only a real decision changes it (see show_plan)
        }
        Err(e) => fail(e),
    }
}
