//! The SSH page (rail › terminal icon): tabs Servidores · Portas · Chaves.

use gtk::{glib, prelude::*, Align};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
    time::Duration,
};

use super::{
    model::{ConnType, Forward, ForwardKind, Host, HostsFile, IdentityKind, Settings, Source},
    openssh::{self, Trust},
    ops,
};
use crate::{
    i18n::{tr, trf},
    secrets, store, ui,
    window::Window,
};

/// Reachability of a host: None = not tested yet.
type Reach = HashMap<String, bool>;

pub struct SshPage {
    pub root: gtk::Box,
    win: Weak<Window>,
    data: RefCell<HostsFile>,
    tab: gtk::Stack,
    search: gtk::Entry,
    tag_filter: RefCell<Option<String>>,
    tags_row: gtk::Box,
    recents_row: gtk::Box,
    groups_box: gtk::Box,
    editor: gtk::Revealer,
    editor_box: gtk::Box,
    selected: RefCell<Option<String>>,
    reach: RefCell<Reach>,
    // ports
    ports_list: gtk::Box,
    port_editor: gtk::Revealer,
    port_editor_box: gtk::Box,
    running: RefCell<HashMap<String, std::process::Child>>,
    // keys
    keys_box: gtk::Box,
    key_detail: gtk::Box,
}

fn section(text: &str) -> gtk::Label {
    ui::label(text, "kr-section")
}

fn tag_class(tag: &str) -> &'static str {
    match tag {
        "produção" | "producao" | "production" | "prod" => "coral",
        "staging" | "homologação" => "gold",
        "mosh" => "teal",
        "casa" | "home" => "blue",
        _ => "accent",
    }
}

fn field(placeholder: &str, value: &str) -> (gtk::Box, gtk::Entry) {
    let b = ui::hbox(8, "kr-field");
    let e = gtk::Entry::new();
    e.set_hexpand(true);
    e.set_text(value);
    e.set_placeholder_text(Some(placeholder));
    b.append(&e);
    (b, e)
}

fn mono(text: &str) -> gtk::Label {
    let l = ui::label(text, "kr-mono");
    l.set_selectable(true);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::Char);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l
}

impl SshPage {
    pub fn new(win: &Rc<Window>) -> Rc<SshPage> {
        let root = ui::vbox(0, "");
        let body = ui::hbox(16, "kr-page");
        body.set_vexpand(true);
        let main = ui::vbox(14, "");
        main.set_hexpand(true);

        // top bar: tabs, search/quick connect, import, new
        let top = ui::hbox(10, "");
        let (tabs, tab_buttons) = ui::segmented(&[tr("Servers"), tr("Ports"), tr("Keys")], "");
        tabs.set_size_request(300, -1);
        tabs.set_valign(Align::Center);
        top.append(&tabs);
        let (sbox, search) = field(tr("Quick connect (user@address) or search by name and tag"), "");
        sbox.prepend(&ui::icon("busca", "k_muted", 16));
        sbox.set_hexpand(true);
        top.append(&sbox);
        let import = ui::button(tr("Import config and known_hosts"), None, "");
        top.append(&import);
        let new_btn = ui::button(tr("New server"), Some(("mais", "k_accent_fg")), "primary");
        top.append(&new_btn);
        main.append(&top);

        let tab = gtk::Stack::new();
        tab.set_hhomogeneous(false);
        tab.set_vhomogeneous(false);
        tab.set_transition_type(gtk::StackTransitionType::Crossfade);
        tab.set_vexpand(true);

        // servers
        let servers = ui::vbox(12, "");
        let tags_row = ui::hbox(6, "");
        servers.append(&tags_row);
        let recents_row = ui::hbox(6, "");
        servers.append(&recents_row);
        let groups_box = ui::vbox(14, "");
        servers.append(&groups_box);
        let ss = gtk::ScrolledWindow::new();
        ss.set_hscrollbar_policy(gtk::PolicyType::Never);
        ss.set_vexpand(true);
        ss.set_child(Some(&servers));
        tab.add_named(&ss, Some("servers"));

        // ports
        let ports = ui::vbox(12, "");
        ports.append(&ui::para(
            tr("Bring a port from there to here (Local), from here to there (Remote) or use the server as a SOCKS proxy (Dynamic). Each one opens with the server's connection, or on its own."),
            "kr-caption",
        ));
        let ports_list = ui::vbox(0, "kr-card kr-table");
        ports.append(&ports_list);
        let ps = gtk::ScrolledWindow::new();
        ps.set_hscrollbar_policy(gtk::PolicyType::Never);
        ps.set_vexpand(true);
        ps.set_child(Some(&ports));
        tab.add_named(&ps, Some("ports"));

        // keys
        let keys = ui::vbox(12, "");
        let keys_box = ui::vbox(10, "");
        keys.append(&keys_box);
        let ks = gtk::ScrolledWindow::new();
        ks.set_hscrollbar_policy(gtk::PolicyType::Never);
        ks.set_vexpand(true);
        ks.set_child(Some(&keys));
        tab.add_named(&ks, Some("keys"));
        main.append(&tab);
        body.append(&main);

        // the editors on the right
        let editor = gtk::Revealer::new();
        editor.set_transition_type(gtk::RevealerTransitionType::SlideLeft);
        editor.set_transition_duration(220);
        let editor_box = ui::vbox(10, "kr-card kr-editor");
        editor_box.set_size_request(400, -1);
        let es = gtk::ScrolledWindow::new();
        es.set_hscrollbar_policy(gtk::PolicyType::Never);
        es.set_propagate_natural_height(true);
        es.set_child(Some(&editor_box));
        editor.set_child(Some(&es));
        body.append(&editor);
        let port_editor = gtk::Revealer::new();
        port_editor.set_transition_type(gtk::RevealerTransitionType::SlideLeft);
        let port_editor_box = ui::vbox(10, "kr-card kr-editor");
        port_editor_box.set_size_request(380, -1);
        port_editor_box.set_valign(Align::Start);
        port_editor.set_child(Some(&port_editor_box));
        body.append(&port_editor);
        let key_detail = ui::vbox(10, "kr-card kr-editor");
        key_detail.set_size_request(380, -1);
        key_detail.set_valign(Align::Start);
        key_detail.set_visible(false);
        body.append(&key_detail);
        root.append(&body);

        let page = Rc::new(SshPage {
            root,
            win: Rc::downgrade(win),
            data: RefCell::new(HostsFile::load()),
            tab: tab.clone(),
            search: search.clone(),
            tag_filter: RefCell::new(None),
            tags_row,
            recents_row,
            groups_box,
            editor,
            editor_box,
            selected: RefCell::new(None),
            reach: RefCell::new(HashMap::new()),
            ports_list,
            port_editor,
            port_editor_box,
            running: RefCell::new(HashMap::new()),
            keys_box,
            key_detail,
        });

        for (i, b) in tab_buttons.iter().enumerate() {
            let p = Rc::downgrade(&page);
            b.connect_toggled(move |b| {
                if !b.is_active() {
                    return;
                }
                if let Some(p) = p.upgrade() {
                    p.tab.set_visible_child_name(["servers", "ports", "keys"][i]);
                    p.editor.set_reveal_child(false);
                    p.port_editor.set_reveal_child(false);
                    p.key_detail.set_visible(false);
                    match i {
                        1 => p.fill_ports(),
                        2 => p.fill_keys(),
                        _ => p.fill_servers(),
                    }
                }
            });
        }
        let p = Rc::downgrade(&page);
        search.connect_changed(move |_| {
            if let Some(p) = p.upgrade() {
                p.fill_servers();
            }
        });
        let p = Rc::downgrade(&page);
        search.connect_activate(move |e| {
            if let Some(p) = p.upgrade() {
                p.quick_connect(&e.text());
            }
        });
        let p = Rc::downgrade(&page);
        import.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                p.import();
            }
        });
        let p = Rc::downgrade(&page);
        new_btn.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                match p.tab.visible_child_name().as_deref() {
                    Some("ports") => p.edit_forward(None),
                    Some("keys") => p.generate_key(),
                    _ => p.edit_host(None),
                }
            }
        });
        page.fill_servers();
        page.start_reachability();
        page
    }

    fn win(&self) -> Option<Rc<Window>> {
        self.win.upgrade()
    }

    fn toast(&self, t: &str) {
        if let Some(w) = self.win() {
            w.toast(t);
        }
    }

    fn save(&self) {
        if let Err(e) = self.data.borrow().save() {
            self.toast(&e.to_string());
        }
    }

    // ------------------------------------------------------------------------------------- servers

    pub fn fill_servers(self: &Rc<Self>) {
        let data = self.data.borrow().clone();
        let q = self.search.text().to_lowercase();
        let tag = self.tag_filter.borrow().clone();
        // tag chips: Todos · N, then every tag
        while let Some(c) = self.tags_row.first_child() {
            self.tags_row.remove(&c);
        }
        let mut tags: Vec<String> = data.hosts.iter().flat_map(|h| h.tags.clone()).collect();
        tags.sort();
        tags.dedup();
        let mut chips: Vec<(String, Option<String>)> = vec![(trf("All · {n}", &[("n", data.hosts.len().to_string())]), None)];
        chips.extend(tags.into_iter().map(|t| (t.clone(), Some(t))));
        for (label, value) in chips {
            let b = gtk::ToggleButton::with_label(&label);
            b.add_css_class("kr-chip");
            b.set_active(value == tag);
            let p = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                let (p, value) = (p.clone(), value.clone());
                glib::idle_add_local_once(move || {
                    if let Some(p) = p.upgrade() {
                        *p.tag_filter.borrow_mut() = value;
                        p.fill_servers();
                    }
                });
            });
            self.tags_row.append(&b);
        }
        // recents
        while let Some(c) = self.recents_row.first_child() {
            self.recents_row.remove(&c);
        }
        let mut recent: Vec<&Host> = data.hosts.iter().filter(|h| h.last_used > 0).collect();
        recent.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        self.recents_row.set_visible(!recent.is_empty());
        if !recent.is_empty() {
            self.recents_row.append(&section(tr("RECENT")));
            for h in recent.into_iter().take(3) {
                let b = ui::button(&h.name, Some(("terminal", "k_text")), "chip");
                let id = h.id.clone();
                let p = Rc::downgrade(self);
                b.connect_clicked(move |_| {
                    if let Some(p) = p.upgrade() {
                        p.open_in_terminal(&id);
                    }
                });
                self.recents_row.append(&b);
            }
        }
        // groups (and the servers without one at the end)
        while let Some(c) = self.groups_box.first_child() {
            self.groups_box.remove(&c);
        }
        let visible = |h: &Host| {
            (q.is_empty() || h.name.to_lowercase().contains(&q) || h.address.to_lowercase().contains(&q) || h.tags.iter().any(|t| t.to_lowercase().contains(&q)))
                && tag.as_ref().is_none_or(|t| h.tags.contains(t))
        };
        if data.hosts.is_empty() {
            self.groups_box.append(&ui::para(
                tr("No servers yet. Import your ~/.ssh/config and known_hosts, or add a new server."),
                "kr-empty",
            ));
            return;
        }
        // top-level groups, with their subgroups summed in the title ("TRABALHO · 3 › produção 1 · staging 1")
        let mut order: Vec<(Option<String>, String)> = data.groups.iter().filter(|g| g.parent.is_none()).map(|g| (Some(g.id.clone()), g.name.clone())).collect();
        order.push((None, tr("No group").to_owned()));
        for (gid, gname) in order {
            let in_group = |h: &Host| -> bool {
                match &gid {
                    None => h.group.as_ref().is_none_or(|g| data.group(g).is_none()),
                    Some(g) => data.group_chain(h.group.as_deref()).last().is_some_and(|top| &top.id == g),
                }
            };
            let hosts: Vec<&Host> = data.hosts.iter().filter(|h| in_group(h) && visible(h)).collect();
            if hosts.is_empty() {
                continue;
            }
            let mut title = format!("{} · {}", gname.to_uppercase(), hosts.len());
            let subs: Vec<String> = data
                .groups
                .iter()
                .filter(|g| g.parent == gid && gid.is_some())
                .map(|g| format!("{} {}", g.name, hosts.iter().filter(|h| h.group.as_deref() == Some(&g.id)).count()))
                .collect();
            if !subs.is_empty() {
                title = format!("{title}  ›  {}", subs.join(" · "));
            }
            self.groups_box.append(&section(&title));
            let card = ui::vbox(0, "kr-card kr-list");
            for h in hosts {
                card.append(&self.host_row(&data, h));
            }
            self.groups_box.append(&card);
        }
    }

    fn host_row(self: &Rc<Self>, data: &HostsFile, h: &Host) -> gtk::Widget {
        let selected = self.selected.borrow().as_deref() == Some(&h.id);
        let row = ui::hbox(10, if selected { "kr-host-row selected" } else { "kr-host-row" });
        let dot = ui::hbox(0, "kr-reach");
        match self.reach.borrow().get(&h.id) {
            Some(true) => dot.add_css_class("ok"),
            Some(false) => dot.add_css_class("down"),
            None => {}
        }
        dot.set_valign(Align::Center);
        row.append(&dot);
        row.append(&ui::icon("terminal", "k_muted", 16));
        let t = ui::vbox(2, "");
        t.set_hexpand(true);
        t.append(&ui::ellip(&h.name, "kr-device-name"));
        let mut sub = data.destination(h);
        if let Some(j) = data.jump_chain(h).last() {
            sub = format!("{sub} · {}", trf("via {j}", &[("j", j.name.clone())]));
        } else if h.port != 22 {
            sub = format!("{} · {}", data.destination(&Host { port: 22, ..h.clone() }), trf("port {p}", &[("p", h.port.to_string())]));
        } else {
            sub = format!("{sub} · {}", trf("port {p}", &[("p", "22".into())]));
        }
        t.append(&ui::ellip(&sub, "kr-caption"));
        row.append(&t);
        if selected {
            let open = ui::button(tr("Open in Terminal"), Some(("terminal", "k_accent_fg")), "primary chip");
            let id = h.id.clone();
            let p = Rc::downgrade(self);
            open.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.open_in_terminal(&id);
                }
            });
            row.append(&open);
            let files = ui::button(tr("Files"), Some(("pasta", "k_text")), "chip");
            let id = h.id.clone();
            let p = Rc::downgrade(self);
            files.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.open_files(&id);
                }
            });
            row.append(&files);
        } else {
            for tg in &h.tags {
                let l = ui::label(tg, &format!("kr-tag {}", tag_class(tg)));
                l.set_valign(Align::Center);
                row.append(&l);
            }
            if h.conn == ConnType::Mosh {
                let l = ui::label("mosh", "kr-tag teal");
                l.set_valign(Align::Center);
                row.append(&l);
            }
            let e = data.effective(h);
            let auth = match e.identity.0.as_ref().and_then(|i| data.identities.iter().find(|x| &x.id == i)) {
                Some(i) if i.kind == IdentityKind::VaultPassword => tr("password (vault)").to_owned(),
                Some(i) if i.kind == IdentityKind::Agent => tr("ssh agent").to_owned(),
                Some(i) => trf("key {t}", &[("t", i.key_type.to_lowercase())]),
                None => tr("no key chosen").to_owned(),
            };
            let a = ui::hbox(4, "");
            a.append(&ui::icon("cadeado", "k_muted", 12));
            a.append(&ui::label(&auth, "kr-caption"));
            row.append(&a);
            let when = if h.last_used > 0 { store::ago(h.last_used, store::now()) } else { "—".into() };
            let w = ui::label(&when, "kr-caption");
            w.set_size_request(70, -1);
            row.append(&w);
        }
        let click = gtk::GestureClick::new();
        let id = h.id.clone();
        let p = Rc::downgrade(self);
        click.connect_pressed(move |_, n, _, _| {
            let Some(p) = p.upgrade() else { return };
            if std::env::var_os("KALANGO_REMOTE_DEBUG").is_some() {
                eprintln!("debug ssh: clique {n} em {id}");
            }
            if n == 2 {
                let id = id.clone();
                glib::idle_add_local_once(move || p.open_in_terminal(&id));
                return;
            }
            // the list is rebuilt: never inside the handler of a gesture of one of its rows
            let id = id.clone();
            glib::idle_add_local_once(move || {
                *p.selected.borrow_mut() = Some(id.clone());
                p.fill_servers();
                p.edit_host(Some(&id));
            });
        });
        row.add_controller(click);
        row.upcast()
    }

    fn quick_connect(self: &Rc<Self>, text: &str) {
        let Some((user, host, port)) = super::model::parse_quick(text) else { return };
        // connects without saving (a host that is not kept)
        let mut data = self.data.borrow_mut();
        let id = data.new_id(&format!("{host}"));
        let mut h = Host { id: id.clone(), name: host.clone(), address: host, port, ..Default::default() };
        h.own.user = user;
        data.hosts.push(h);
        drop(data);
        self.search.set_text("");
        self.save();
        self.fill_servers();
        self.open_in_terminal(&id);
    }

    fn import(self: &Rc<Self>) {
        let ssh_dir = glib::home_dir().join(".ssh");
        let (hosts, known) = ops::import_openssh(&mut self.data.borrow_mut(), &ssh_dir);
        self.save();
        self.fill_servers();
        self.toast(&trf("{h} servers and {k} known keys imported", &[("h", hosts.to_string()), ("k", known.to_string())]));
    }

    /// Before anything leaves here: the server's key. Known → go on; new → ask; changed → blocked.
    fn check_then(self: &Rc<Self>, id: &str, then: impl Fn(&Rc<SshPage>) + 'static) {
        let data = self.data.borrow().clone();
        let Some(h) = data.host(id).cloned() else { return };
        self.toast(tr("Checking the server's key…"));
        let (tx, rx) = async_channel::bounded(1);
        let d2 = data.clone();
        let h2 = h.clone();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(ops::scan(&d2, &h2));
        });
        let me = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(res) = rx.recv().await else { return };
            let Some(p) = me.upgrade() else { return };
            match res {
                Err(e) => p.toast(&trf("Could not reach {h}: {e}", &[("h", h.name.clone()), ("e", e)])),
                Ok(k) => match ops::trust_of(&h, &k) {
                    Trust::Known => then(&p),
                    Trust::Unknown => p.first_connection(&data, &h, &k, Box::new(then)),
                    Trust::Changed { before, before_type } => p.key_changed(&h, &k, &before, &before_type),
                },
            }
        });
    }

    fn open_in_terminal(self: &Rc<Self>, id: &str) {
        let data = self.data.borrow().clone();
        let Some(h) = data.host(id).cloned() else { return };
        let id = id.to_owned();
        let go = move |p: &Rc<SshPage>| {
            if let Some(hh) = p.data.borrow_mut().hosts.iter_mut().find(|x| x.id == id) {
                hh.last_used = store::now();
            }
            p.save();
            p.fill_servers();
            match std::process::Command::new("kalango-shell").args(["terminal-ssh", &id]).spawn() {
                Ok(_) => p.toast(&trf("Opening {h} in the Terminal", &[("h", id.clone())])),
                Err(_) => p.toast(tr("KalangoOS's Terminal is not available here (kalango-shell terminal-ssh)")),
            }
        };
        if h.confirm {
            let me = Rc::downgrade(self);
            confirm_dialog(
                &self.root,
                &trf("Connect to {h}?", &[("h", h.name.clone())]),
                tr("This server is marked as production: commands that delete or restart ask for confirmation in the Terminal."),
                tr("Connect"),
                move || {
                    if let Some(p) = me.upgrade() {
                        let g = go.clone();
                        p.check_then(&h.id, move |p| g(p));
                    }
                },
            );
        } else {
            self.check_then(&h.id, move |p| go(p));
        }
    }

    fn open_files(self: &Rc<Self>, id: &str) {
        let id = id.to_owned();
        self.check_then(&id.clone(), move |p| {
            match std::process::Command::new("kalango-shell").args(["terminal-ssh", &id, "--arquivos"]).spawn() {
                Ok(_) => {}
                Err(_) => p.toast(tr("KalangoOS's Terminal is not available here (kalango-shell terminal-ssh)")),
            }
        });
    }

    fn first_connection(self: &Rc<Self>, data: &HostsFile, h: &Host, k: &ops::ScannedKey, then: Box<dyn Fn(&Rc<SshPage>)>) {
        let d = dialog(&self.root);
        let body = ui::vbox(14, "kr-dialog-body");
        body.set_size_request(480, -1);
        let head = ui::hbox(8, "");
        head.append(&ui::icon("escudo", "k_accent", 16));
        head.append(&ui::label(&trf("First connection to {h}", &[("h", h.name.clone())]), "kr-big-title"));
        body.append(&head);
        body.append(&ui::para(tr("KalangoOS does not know this server yet. Check the fingerprint with whoever runs the server (or in the hosting panel) before trusting it."), "kr-body"));
        let grid = gtk::Grid::new();
        grid.add_css_class("kr-kv");
        grid.set_column_spacing(24);
        grid.set_row_spacing(8);
        let jump = data.jump_chain(h).last().map(|j| j.name.clone()).unwrap_or_else(|| "—".into());
        for (i, (k_, v, accent)) in [
            (tr("Server"), format!("{}:{}", data.destination(&Host { port: 22, ..h.clone() }), h.port), false),
            (tr("Through the jump"), jump, false),
            (tr("Key"), openssh::short_type(&k.key_type), false),
            ("SHA256", k.fingerprint.trim_start_matches("SHA256:").to_owned(), true),
        ]
        .into_iter()
        .enumerate()
        {
            grid.attach(&ui::label(k_, "kr-caption"), 0, i as i32, 1, 1);
            let v = mono(&v);
            if accent {
                v.add_css_class("accent");
            }
            grid.attach(&v, 1, i as i32, 1, 1);
        }
        body.append(&grid);
        let remember = gtk::CheckButton::with_label(tr("Remember this server (keep it in known_hosts)"));
        remember.set_active(true);
        body.append(&remember);
        let row = ui::hbox(8, "");
        let cancel = ui::button(tr("Cancel"), None, "tall");
        cancel.set_hexpand(true);
        let ok = ui::button(tr("Trust and connect"), None, "primary tall");
        ok.set_hexpand(true);
        row.append(&cancel);
        row.append(&ok);
        body.append(&row);
        d.set_child(Some(&body));
        let dd = d.clone();
        cancel.connect_clicked(move |_| dd.destroy());
        let (dd, h, k, me) = (d.clone(), h.clone(), k.clone(), Rc::downgrade(self));
        ok.connect_clicked(move |_| {
            let Some(p) = me.upgrade() else { return };
            if remember.is_active() {
                if let Err(e) = ops::trust(&h, &k) {
                    p.toast(&e.to_string());
                    return;
                }
            }
            dd.destroy();
            then(&p);
        });
        d.present();
    }

    fn key_changed(self: &Rc<Self>, h: &Host, k: &ops::ScannedKey, before: &str, before_type: &str) {
        let d = dialog(&self.root);
        d.add_css_class("kr-danger");
        let body = ui::vbox(14, "kr-dialog-body");
        body.set_size_request(480, -1);
        let head = ui::hbox(8, "");
        head.append(&ui::icon("alerta", "k_coral", 16));
        let t = ui::label(tr("The fingerprint CHANGED"), "kr-big-title");
        t.set_hexpand(true);
        head.append(&t);
        head.append(&ui::label(tr("high risk"), "kr-tag coral"));
        body.append(&head);
        body.append(&ui::para(
            &trf("The key of the server {h} is not the same as last time. It may be a reinstall, or someone in the middle. For safety the connection was blocked: neither you here nor the agent accept the new key in this dialog.", &[("h", h.name.clone())]),
            "kr-body",
        ));
        let grid = gtk::Grid::new();
        grid.add_css_class("kr-kv");
        grid.set_column_spacing(24);
        grid.set_row_spacing(8);
        grid.attach(&ui::label(tr("Before"), "kr-caption"), 0, 0, 1, 1);
        grid.attach(&mono(&openssh::short_fingerprint(before)), 1, 0, 1, 1);
        grid.attach(&ui::label(tr("Now"), "kr-caption"), 0, 1, 1, 1);
        let now = mono(&openssh::short_fingerprint(&k.fingerprint));
        now.add_css_class("coral");
        grid.attach(&now, 1, 1, 1, 1);
        grid.attach(&ui::label(tr("Key"), "kr-caption"), 0, 2, 1, 1);
        grid.attach(&mono(&format!("{} → {}", openssh::short_type(before_type), openssh::short_type(&k.key_type))), 1, 2, 1, 1);
        body.append(&grid);
        let how = ui::vbox(4, "kr-howto");
        how.append(&ui::label(tr("How to solve"), "kr-row-title"));
        how.append(&ui::para(tr("1. Confirm the new fingerprint with whoever runs the server.  2. In SSH › Keys › Known servers, remove the old key (asks for the administrator password).  3. Connect again: the first-connection dialog appears."), "kr-caption"));
        body.append(&how);
        let row = ui::hbox(8, "");
        let known = ui::button(tr("Open known servers"), None, "tall");
        known.set_hexpand(true);
        let no = ui::button(tr("Do not connect"), None, "primary tall");
        no.set_hexpand(true);
        row.append(&known);
        row.append(&no);
        body.append(&row);
        d.set_child(Some(&body));
        let dd = d.clone();
        no.connect_clicked(move |_| dd.destroy());
        let (dd, me) = (d.clone(), Rc::downgrade(self));
        known.connect_clicked(move |_| {
            dd.destroy();
            if let Some(p) = me.upgrade() {
                p.show_tab(2);
            }
        });
        d.present();
    }

    fn show_tab(self: &Rc<Self>, i: usize) {
        // the segmented buttons are the first child of the top bar
        let names = ["servers", "ports", "keys"];
        self.tab.set_visible_child_name(names[i]);
        if i == 2 {
            self.fill_keys();
        }
    }

    // --------------------------------------------------------------------------------------- editor

    fn edit_host(self: &Rc<Self>, id: Option<&str>) {
        let data = self.data.borrow().clone();
        let host = id.and_then(|i| data.host(i).cloned()).unwrap_or_else(|| Host { id: data.new_id(tr("new-server")), name: String::new(), ..Default::default() });
        let is_new = id.is_none();
        let b = &self.editor_box;
        while let Some(c) = b.first_child() {
            b.remove(&c);
        }
        let head = ui::hbox(8, "");
        let t = ui::ellip(if is_new { tr("New server") } else { &host.name }, "kr-big-title");
        head.append(&t);
        for tg in host.tags.iter().filter(|t| tag_class(t) == "coral") {
            head.append(&ui::label(tg, "kr-tag coral"));
        }
        let close = ui::icon_button("fechar", "k_muted", tr("Close"));
        close.add_css_class("small");
        head.append(&close);
        b.append(&head);
        let label = |t: &str| ui::label(t, "kr-caption");
        b.append(&label(tr("Name")));
        let (nb, name) = field("app-prod-01", &host.name);
        b.append(&nb);
        let addr_row = ui::hbox(8, "");
        let ac = ui::vbox(6, "");
        ac.set_hexpand(true);
        ac.append(&label(tr("Address")));
        let (ab, address) = field("203.0.113.24", &host.address);
        ab.prepend(&ui::icon("rede-cabo", "k_muted", 16));
        ac.append(&ab);
        addr_row.append(&ac);
        let pc = ui::vbox(6, "");
        pc.append(&label(tr("Port")));
        let (pb, port) = field("22", &host.port.to_string());
        pb.set_size_request(90, -1);
        pc.append(&pb);
        addr_row.append(&pc);
        b.append(&addr_row);
        let e = data.effective(&host);
        let from = |s: &Source| match s {
            Source::Group(g) => trf("inherited from the group {g}", &[("g", g.clone())]),
            Source::Host => String::new(),
            Source::Default => String::new(),
        };
        let user_label = if from(&e.user.1).is_empty() { tr("User").to_owned() } else { format!("{} · {}", tr("User"), from(&e.user.1)) };
        b.append(&label(&user_label));
        let (ub, user) = field(&e.user.0, host.own.user.as_deref().unwrap_or(""));
        ub.prepend(&ui::icon("usuario", "k_muted", 16));
        b.append(&ub);
        b.append(&label(tr("Connection")));
        let (cseg, cbtn) = ui::segmented(&["SSH", "Mosh"], "");
        cseg.set_halign(Align::Start);
        cseg.set_size_request(220, -1);
        cbtn[if host.conn == ConnType::Mosh { 1 } else { 0 }].set_active(true);
        b.append(&cseg);
        b.append(&ui::para(tr("Mosh: better on unstable connections (weak Wi-Fi, phone); uses the system's mosh-client. Needs mosh-server on the server."), "kr-caption"));
        b.append(&label(tr("Authentication")));
        let ids: Vec<(String, String)> = data
            .identities
            .iter()
            .map(|i| {
                let kind = match i.kind {
                    IdentityKind::VaultKey => tr("from the keyring"),
                    IdentityKind::VaultPassword => tr("password"),
                    IdentityKind::Agent => tr("ssh agent"),
                };
                (i.id.clone(), format!("{} · {kind}", i.id))
            })
            .collect();
        let mut id_items: Vec<String> = vec![if e.identity.1 != Source::Host && e.identity.0.is_some() { format!("{} ({})", e.identity.0.clone().unwrap_or_default(), from(&e.identity.1)) } else { tr("(inherit / none)").to_owned() }];
        id_items.extend(ids.iter().map(|(_, l)| l.clone()));
        let id_refs: Vec<&str> = id_items.iter().map(String::as_str).collect();
        let ident = gtk::DropDown::from_strings(&id_refs);
        ident.add_css_class("kr-dropdown");
        if let Some(own) = &host.own.identity {
            if let Some(pos) = ids.iter().position(|(i, _)| i == own) {
                ident.set_selected(pos as u32 + 1);
            }
        }
        b.append(&ident);
        let jump_label = if from(&e.jump.1).is_empty() { tr("Jump server").to_owned() } else { format!("{} · {}", tr("Jump server"), from(&e.jump.1)) };
        b.append(&label(&jump_label));
        let others: Vec<&Host> = data.hosts.iter().filter(|x| x.id != host.id).collect();
        let mut j_items = vec![if e.jump.1 != Source::Host && e.jump.0.is_some() { format!("{} ({})", e.jump.0.clone().unwrap_or_default(), from(&e.jump.1)) } else { tr("(inherit / none)").to_owned() }];
        j_items.extend(others.iter().map(|x| x.name.clone()));
        let j_refs: Vec<&str> = j_items.iter().map(String::as_str).collect();
        let jump = gtk::DropDown::from_strings(&j_refs);
        jump.add_css_class("kr-dropdown");
        if let Some(own) = &host.own.jump {
            if let Some(pos) = others.iter().position(|x| &x.id == own) {
                jump.set_selected(pos as u32 + 1);
            }
        }
        b.append(&jump);
        let chain = data.jump_chain(&host);
        if !chain.is_empty() {
            let mut path = vec![tr("this computer").to_owned()];
            path.extend(chain.iter().map(|j| j.name.clone()));
            path.push(if host.name.is_empty() { "…".into() } else { host.name.clone() });
            b.append(&mono(&path.join("  >  ")).tap_class("kr-path"));
        }
        b.append(&label(tr("Group and tags")));
        let grow = ui::hbox(8, "");
        let mut g_items = vec![tr("(no group)").to_owned()];
        g_items.extend(data.groups.iter().map(|g| g.name.clone()));
        let g_refs: Vec<&str> = g_items.iter().map(String::as_str).collect();
        let group = gtk::DropDown::from_strings(&g_refs);
        group.add_css_class("kr-dropdown");
        group.set_hexpand(true);
        if let Some(g) = &host.group {
            if let Some(pos) = data.groups.iter().position(|x| &x.id == g) {
                group.set_selected(pos as u32 + 1);
            }
        }
        grow.append(&group);
        let (tb, tags) = field(tr("tags, separated by commas"), &host.tags.join(", "));
        tb.set_hexpand(true);
        grow.append(&tb);
        b.append(&grow);
        let (r1, confirm, _) = ui::switch_row(tr("Ask for confirmation before connecting"), tr("For production servers"));
        confirm.set_active(host.confirm);
        b.append(&r1);
        let (r2, keep, keep_sub) = ui::switch_row(tr("Keep the connection alive"), "");
        keep.set_active(e.keepalive.0 > 0);
        keep_sub.set_text(&if e.keepalive.0 > 0 { trf("Every {s} s", &[("s", e.keepalive.0.to_string())]) } else { tr("Off").to_owned() });
        keep_sub.set_visible(true);
        b.append(&r2);
        let (r3, fwd, _) = ui::switch_row(tr("Forward the SSH agent"), tr("Lets your keys be used inside"));
        fwd.set_active(e.forward_agent.0);
        b.append(&r3);
        // the fingerprint already trusted
        let known = ops::known_hosts();
        let pat = openssh::host_pattern(&host.address, host.port);
        if let Some(k) = known.iter().find(|k| k.hosts.iter().any(|p| p == &pat)) {
            let fp = ui::hbox(10, "kr-known");
            fp.append(&ui::icon("escudo", "k_teal", 16));
            let c = ui::vbox(2, "");
            c.append(&ui::label(tr("Known fingerprint"), "kr-row-title"));
            c.append(&ui::label(&format!("{}  ·  {}", openssh::short_type(&k.key_type), openssh::short_fingerprint(&openssh::fingerprint(&k.key).unwrap_or_default())), "kr-mono"));
            fp.append(&c);
            b.append(&fp);
        }
        let status = ui::para("", "kr-caption");
        b.append(&status);
        let actions = ui::hbox(8, "");
        let test = ui::button(tr("Test connection"), None, "");
        test.set_hexpand(true);
        let save = ui::button(tr("Save"), None, "primary");
        save.set_hexpand(true);
        actions.append(&test);
        actions.append(&save);
        if !is_new {
            let del = ui::button(tr("Delete"), None, "danger");
            let (me, hid) = (Rc::downgrade(self), host.id.clone());
            del.connect_clicked(move |_| {
                if let Some(p) = me.upgrade() {
                    p.data.borrow_mut().hosts.retain(|h| h.id != hid);
                    p.save();
                    p.editor.set_reveal_child(false);
                    p.fill_servers();
                }
            });
            actions.prepend(&del);
        }
        b.append(&actions);

        let me = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(p) = me.upgrade() {
                p.editor.set_reveal_child(false);
                *p.selected.borrow_mut() = None;
                p.fill_servers();
            }
        });
        let collect = {
            let (name, address, port, user, ident, jump, group, tags, confirm, keep, fwd) =
                (name.clone(), address.clone(), port.clone(), user.clone(), ident.clone(), jump.clone(), group.clone(), tags.clone(), confirm.clone(), keep.clone(), fwd.clone());
            let cbtn = cbtn.clone();
            let base = host.clone();
            let ids = ids.clone();
            let other_ids: Vec<String> = others.iter().map(|x| x.id.clone()).collect();
            let group_ids: Vec<String> = data.groups.iter().map(|g| g.id.clone()).collect();
            let inherited_keep = e.keepalive.clone();
            let inherited_fwd = e.forward_agent.clone();
            move || -> Result<Host, String> {
                let mut h = base.clone();
                h.name = name.text().trim().to_owned();
                h.address = address.text().trim().to_owned();
                if h.name.is_empty() {
                    h.name = h.address.clone();
                }
                if h.address.is_empty() {
                    return Err(tr("The address is missing").to_owned());
                }
                h.port = port.text().trim().parse().map_err(|_| tr("The port must be a number").to_owned())?;
                let u = user.text().trim().to_owned();
                h.own.user = (!u.is_empty()).then_some(u);
                h.conn = if cbtn[1].is_active() { ConnType::Mosh } else { ConnType::Ssh };
                h.own.identity = match ident.selected() {
                    0 => None,
                    n => ids.get(n as usize - 1).map(|(i, _)| i.clone()),
                };
                h.own.jump = match jump.selected() {
                    0 => None,
                    n => other_ids.get(n as usize - 1).cloned(),
                };
                h.group = match group.selected() {
                    0 => None,
                    n => group_ids.get(n as usize - 1).cloned(),
                };
                h.tags = tags.text().split(',').map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()).collect();
                h.confirm = confirm.is_active();
                // an exception only when it differs from what is inherited
                h.own.keepalive = match (keep.is_active(), inherited_keep.0 > 0) {
                    (on, inh) if on == inh => h.own.keepalive,
                    (true, _) => Some(30),
                    (false, _) => Some(0),
                };
                h.own.forward_agent = if fwd.is_active() == inherited_fwd.0 { h.own.forward_agent } else { Some(fwd.is_active()) };
                Ok(h)
            }
        };
        let collect = Rc::new(collect);
        let (me, c2, st) = (Rc::downgrade(self), collect.clone(), status.clone());
        save.connect_clicked(move |_| {
            let Some(p) = me.upgrade() else { return };
            match c2() {
                Err(e) => st.set_text(&e),
                Ok(h) => {
                    let id = h.id.clone();
                    {
                        let mut d = p.data.borrow_mut();
                        match d.hosts.iter_mut().find(|x| x.id == id) {
                            Some(x) => *x = h,
                            None => d.hosts.push(h),
                        }
                    }
                    p.save();
                    *p.selected.borrow_mut() = Some(id.clone());
                    p.fill_servers();
                    p.edit_host(Some(&id));
                    p.toast(tr("Saved"));
                }
            }
        });
        let (me, c3, st) = (Rc::downgrade(self), collect.clone(), status.clone());
        test.connect_clicked(move |b| {
            let Some(p) = me.upgrade() else { return };
            let Ok(h) = c3() else { return };
            let data = {
                let mut d = p.data.borrow().clone();
                d.hosts.retain(|x| x.id != h.id);
                d.hosts.push(h.clone());
                d
            };
            st.set_text(tr("Testing…"));
            b.set_sensitive(false);
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let r = ops::scan(&data, &h).map(|k| (ops::trust_of(&h, &k), k));
                let _ = tx.send_blocking(r);
            });
            let (st, b) = (st.clone(), b.clone());
            glib::spawn_future_local(async move {
                let Ok(r) = rx.recv().await else { return };
                b.set_sensitive(true);
                st.remove_css_class("kr-error-text");
                match r {
                    Ok((Trust::Known, k)) => st.set_text(&trf("✓ Answered · {t} · known key", &[("t", openssh::short_type(&k.key_type))])),
                    Ok((Trust::Unknown, k)) => st.set_text(&trf("✓ Answered · {t} {fp} · not trusted yet (asked at the first connection)", &[("t", openssh::short_type(&k.key_type)), ("fp", openssh::short_fingerprint(&k.fingerprint))])),
                    Ok((Trust::Changed { .. }, _)) => {
                        st.add_css_class("kr-error-text");
                        st.set_text(tr("The server's key CHANGED: connection blocked"));
                    }
                    Err(e) => {
                        st.add_css_class("kr-error-text");
                        st.set_text(&e);
                    }
                }
            });
        });
        self.port_editor.set_reveal_child(false);
        self.editor.set_reveal_child(true);
    }

    // --------------------------------------------------------------------------------------- ports

    fn fill_ports(self: &Rc<Self>) {
        let data = self.data.borrow().clone();
        let l = &self.ports_list;
        while let Some(c) = l.first_child() {
            l.remove(&c);
        }
        let head = ui::hbox(12, "kr-table-head");
        for (t, w) in [(tr("Type"), 90), (tr("Name"), 190), (tr("From"), 170), ("", 24), (tr("To"), 200), (tr("Through the server"), 150), (tr("State"), 120)] {
            let lb = ui::label(t, "kr-caption");
            lb.set_size_request(w, -1);
            head.append(&lb);
        }
        l.append(&head);
        if data.forwards.is_empty() {
            l.append(&ui::label(tr("No forwards yet."), "kr-empty kr-pad"));
            return;
        }
        for fw in &data.forwards {
            let row = ui::hbox(12, "kr-table-row");
            let (kind, class) = match fw.kind {
                ForwardKind::Local => (tr("Local"), "blue"),
                ForwardKind::Remote => (tr("Remote"), "accent"),
                ForwardKind::Dynamic => (tr("Dynamic"), "gold"),
            };
            let k = ui::label(kind, &format!("kr-tag {class}"));
            k.set_valign(Align::Center);
            let kb = ui::hbox(0, "");
            kb.set_size_request(90, -1);
            kb.append(&k);
            row.append(&kb);
            let n = ui::ellip(&fw.name, "kr-row-title");
            n.set_size_request(190, -1);
            n.set_hexpand(false);
            row.append(&n);
            let from = ui::ellip(&fw.listen, "kr-mono");
            from.set_size_request(170, -1);
            from.set_hexpand(false);
            row.append(&from);
            row.append(&ui::icon("seta-direita", "k_muted", 16));
            let to = ui::ellip(if fw.kind == ForwardKind::Dynamic { "SOCKS5" } else { &fw.target }, "kr-mono");
            to.set_size_request(200, -1);
            to.set_hexpand(false);
            row.append(&to);
            let via = ui::hbox(6, "");
            via.set_size_request(150, -1);
            via.append(&ui::icon("terminal", "k_muted", 16));
            via.append(&ui::ellip(&data.host(&fw.host).map(|h| h.name.clone()).unwrap_or_else(|| "—".into()), "kr-caption"));
            row.append(&via);
            let running = self.running.borrow_mut().get_mut(&fw.id).is_some_and(|c| matches!(c.try_wait(), Ok(None)));
            let (st, dot) = if running { (tr("active"), "ok") } else { (tr("stopped"), "") };
            let sb = ui::hbox(6, "");
            sb.set_hexpand(true);
            let d = ui::hbox(0, &format!("kr-reach {dot}"));
            d.set_valign(Align::Center);
            sb.append(&d);
            sb.append(&ui::label(st, "kr-caption"));
            row.append(&sb);
            let sw = gtk::Switch::new();
            sw.set_active(running);
            sw.set_valign(Align::Center);
            let (me, fid) = (Rc::downgrade(self), fw.id.clone());
            sw.connect_state_set(move |_, on| {
                if let Some(p) = me.upgrade() {
                    p.toggle_forward(&fid, on);
                }
                glib::Propagation::Proceed
            });
            row.append(&sw);
            let click = gtk::GestureClick::new();
            let (me, fid) = (Rc::downgrade(self), fw.id.clone());
            click.connect_released(move |_, _, _, _| {
                if let Some(p) = me.upgrade() {
                    p.edit_forward(Some(&fid));
                }
            });
            row.add_controller(click);
            l.append(&row);
        }
    }

    fn toggle_forward(self: &Rc<Self>, id: &str, on: bool) {
        if !on {
            if let Some(mut c) = self.running.borrow_mut().remove(id) {
                let _ = c.kill();
                let _ = c.wait();
            }
            self.fill_ports();
            return;
        }
        let data = self.data.borrow().clone();
        let Some(fw) = data.forwards.iter().find(|f| f.id == id).cloned() else { return };
        let Some(h) = data.host(&fw.host).cloned() else { return };
        let me = Rc::downgrade(self);
        let id = id.to_owned();
        self.check_then(&h.id, move |p| match ops::start_forward(&p.data.borrow(), &fw) {
            Ok(c) => {
                p.running.borrow_mut().insert(id.clone(), c);
                let me2 = me.clone();
                glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                    if let Some(p) = me2.upgrade() {
                        p.fill_ports();
                    }
                });
            }
            Err(e) => p.toast(&e),
        });
    }

    fn edit_forward(self: &Rc<Self>, id: Option<&str>) {
        let data = self.data.borrow().clone();
        let fw = id.and_then(|i| data.forwards.iter().find(|f| f.id == i).cloned()).unwrap_or_else(|| Forward { id: data.new_id("encaminhamento"), ..Default::default() });
        let b = &self.port_editor_box;
        while let Some(c) = b.first_child() {
            b.remove(&c);
        }
        let head = ui::hbox(8, "");
        let t = ui::ellip(if fw.name.is_empty() { tr("New forward") } else { &fw.name }, "kr-big-title");
        head.append(&t);
        let close = ui::icon_button("fechar", "k_muted", tr("Close"));
        close.add_css_class("small");
        head.append(&close);
        b.append(&head);
        b.append(&ui::label(tr("Type"), "kr-caption"));
        let (seg, kb) = ui::segmented(&[tr("Local"), tr("Remote"), tr("Dynamic")], "");
        kb[match fw.kind {
            ForwardKind::Local => 0,
            ForwardKind::Remote => 1,
            ForwardKind::Dynamic => 2,
        }]
        .set_active(true);
        b.append(&seg);
        b.append(&ui::label(tr("Name"), "kr-caption"));
        let (nb, name) = field(tr("Staging database"), &fw.name);
        b.append(&nb);
        b.append(&ui::label(tr("Port on this computer"), "kr-caption"));
        let (lb, listen) = field("localhost:5433", &fw.listen);
        b.append(&lb);
        b.append(&ui::label(tr("Destination (as the server sees it)"), "kr-caption"));
        let (tb, target) = field("db-staging:5432", &fw.target);
        b.append(&tb);
        b.append(&ui::label(tr("Through the server"), "kr-caption"));
        let names: Vec<String> = data.hosts.iter().map(|h| match data.jump_chain(h).last() {
            Some(j) => format!("{} (via {})", h.name, j.name),
            None => h.name.clone(),
        }).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let host = gtk::DropDown::from_strings(&refs);
        host.add_css_class("kr-dropdown");
        if let Some(pos) = data.hosts.iter().position(|h| h.id == fw.host) {
            host.set_selected(pos as u32);
        }
        b.append(&host);
        let (r1, with, _) = ui::switch_row(tr("Open with the server"), "");
        with.set_active(fw.with_host);
        b.append(&r1);
        let (r2, lo, _) = ui::switch_row(tr("Only this computer can use it"), tr("Listens on 127.0.0.1, not on the network"));
        lo.set_active(fw.loopback_only);
        b.append(&r2);
        let err = ui::para("", "kr-error-text");
        b.append(&err);
        let row = ui::hbox(8, "");
        let del = ui::button(tr("Delete"), None, "danger");
        let save = ui::button(tr("Save"), None, "primary");
        row.append(&del);
        row.append(&ui::spacer());
        row.append(&save);
        b.append(&row);
        let me = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(p) = me.upgrade() {
                p.port_editor.set_reveal_child(false);
            }
        });
        let (me, fid) = (Rc::downgrade(self), fw.id.clone());
        del.connect_clicked(move |_| {
            if let Some(p) = me.upgrade() {
                p.data.borrow_mut().forwards.retain(|f| f.id != fid);
                p.save();
                p.port_editor.set_reveal_child(false);
                p.fill_ports();
            }
        });
        let me = Rc::downgrade(self);
        let host_ids: Vec<String> = data.hosts.iter().map(|h| h.id.clone()).collect();
        save.connect_clicked(move |_| {
            let Some(p) = me.upgrade() else { return };
            let mut f = fw.clone();
            f.kind = if kb[1].is_active() { ForwardKind::Remote } else if kb[2].is_active() { ForwardKind::Dynamic } else { ForwardKind::Local };
            f.name = name.text().trim().to_owned();
            f.listen = listen.text().trim().to_owned();
            f.target = target.text().trim().to_owned();
            f.host = host_ids.get(host.selected() as usize).cloned().unwrap_or_default();
            f.with_host = with.is_active();
            f.loopback_only = lo.is_active();
            // checked while typing would be nicer; here before saving
            if let Some(e) = forward_problem(&f) {
                err.set_text(&e);
                return;
            }
            {
                let mut d = p.data.borrow_mut();
                match d.forwards.iter_mut().find(|x| x.id == f.id) {
                    Some(x) => *x = f,
                    None => d.forwards.push(f),
                }
            }
            p.save();
            p.port_editor.set_reveal_child(false);
            p.fill_ports();
        });
        self.editor.set_reveal_child(false);
        self.port_editor.set_reveal_child(true);
    }

    // ---------------------------------------------------------------------------------------- keys

    fn fill_keys(self: &Rc<Self>) {
        let data = self.data.borrow().clone();
        let b = &self.keys_box;
        while let Some(c) = b.first_child() {
            b.remove(&c);
        }
        let note = ui::hbox(10, "kr-note-box");
        note.append(&ui::icon("cadeado", "k_muted", 16));
        note.append(&ui::para(tr("Passwords and private keys stay in the system keyring (later: the KalangoOS vault, kalangod), encrypted and bound to your session. Kalango Remote asks to use each one at connection time. Agents have no access to this vault: they cannot read, copy or use it."), "kr-caption"));
        b.append(&note);
        let actions = ui::hbox(8, "");
        actions.append(&ui::spacer());
        let imp = ui::button(tr("Import existing key"), Some(("baixar", "k_text")), "");
        let gen = ui::button(tr("Generate ed25519 key"), Some(("mais", "k_accent_fg")), "primary");
        actions.append(&imp);
        actions.append(&gen);
        b.append(&actions);
        let me = Rc::downgrade(self);
        gen.connect_clicked(move |_| {
            if let Some(p) = me.upgrade() {
                p.generate_key();
            }
        });
        let me = Rc::downgrade(self);
        imp.connect_clicked(move |_| {
            if let Some(p) = me.upgrade() {
                p.import_key_dialog();
            }
        });
        let used_by = |id: &str| data.hosts.iter().filter(|h| data.effective(h).identity.0.as_deref() == Some(id)).count();
        let list = |title: &str, rows: Vec<gtk::Widget>| {
            b.append(&section(title));
            let card = ui::vbox(0, "kr-card kr-list");
            if rows.is_empty() {
                card.append(&ui::label("—", "kr-empty kr-pad"));
            }
            for r in rows {
                card.append(&r);
            }
            b.append(&card);
        };
        let key_row = |name: &str, sub: &str, tag: (&str, &str), right: &str| -> gtk::Box {
            let r = ui::hbox(10, "kr-host-row");
            r.append(&ui::icon("cadeado", "k_muted", 16));
            let c = ui::vbox(2, "");
            c.set_hexpand(true);
            c.append(&ui::ellip(name, "kr-device-name"));
            c.append(&ui::ellip(sub, "kr-mono"));
            r.append(&c);
            let tl = ui::label(tag.0, &format!("kr-tag {}", tag.1));
            tl.set_valign(Align::Center);
            r.append(&tl);
            let rl = ui::label(right, "kr-caption");
            rl.set_size_request(150, -1);
            r.append(&rl);
            r
        };
        let mut vault = Vec::new();
        let mut passwords = Vec::new();
        for i in &data.identities {
            let n = used_by(&i.id);
            let right = match n {
                0 => tr("not used").to_owned(),
                1 => tr("used by 1 server").to_owned(),
                n => trf("used by {n} servers", &[("n", n.to_string())]),
            };
            match i.kind {
                IdentityKind::VaultKey => {
                    let old = i.key_type == "RSA";
                    let r = key_row(&i.id, &format!("{}  ·  {}  ·  {}", i.key_type, openssh::short_fingerprint(&i.fingerprint), origin_text(&i.origin)), if old { (tr("old"), "gold") } else { (tr("vault"), "accent") }, &right);
                    let (me, id) = (Rc::downgrade(self), i.id.clone());
                    let c = gtk::GestureClick::new();
                    c.connect_released(move |_, _, _, _| {
                        if let Some(p) = me.upgrade() {
                            p.show_key(&id);
                        }
                    });
                    r.add_controller(c);
                    vault.push(r.upcast());
                }
                IdentityKind::VaultPassword => passwords.push(key_row(&i.id, tr("password"), (tr("vault"), "accent"), &right).upcast()),
                IdentityKind::Agent => {}
            }
        }
        list(tr("KEYS IN THE VAULT"), vault);
        let agent: Vec<gtk::Widget> = ops::agent_keys()
            .iter()
            .map(|i| key_row(&i.id, &format!("{}  ·  {}", i.key_type, openssh::short_fingerprint(&i.fingerprint)), ("ssh-agent", "teal"), if i.key_type.ends_with("-SK") { tr("touch to use") } else { "" }).upcast())
            .collect();
        list(tr("SYSTEM SSH AGENT"), agent);
        list(tr("PASSWORDS"), passwords);
        // known servers
        let known: Vec<gtk::Widget> = ops::known_hosts()
            .iter()
            .filter(|k| !k.hosts.iter().all(|h| h.starts_with("|1|")))
            .map(|k| {
                let name = k.hosts.join(", ");
                let host = data.hosts.iter().find(|h| k.hosts.iter().any(|p| p == &openssh::host_pattern(&h.address, h.port)));
                let label = host.map(|h| format!("{} · {}", h.name, name)).unwrap_or(name.clone());
                let r = key_row(&label, &format!("{}  ·  {}", openssh::short_type(&k.key_type), openssh::short_fingerprint(&openssh::fingerprint(&k.key).unwrap_or_default())), (tr("trusted"), "teal"), "");
                let rm = ui::button(tr("Remove"), None, "ghost chip");
                let (me, pat) = (Rc::downgrade(self), k.hosts[0].clone());
                rm.connect_clicked(move |_| {
                    let Some(p) = me.upgrade() else { return };
                    // removing a trusted key is the only way to accept a new one: it asks the administrator
                    let ok = std::process::Command::new("pkexec").arg("true").status().map(|s| s.success()).unwrap_or(false);
                    if !ok {
                        p.toast(tr("Removal needs the administrator password"));
                        return;
                    }
                    let (host, port) = match pat.strip_prefix('[').and_then(|r| r.split_once("]:")) {
                        Some((h, port)) => (h.to_owned(), port.parse().unwrap_or(22)),
                        None => (pat.clone(), 22),
                    };
                    if let Err(e) = ops::forget_host_key(&host, port) {
                        p.toast(&e.to_string());
                    }
                    p.fill_keys();
                });
                r.append(&rm);
                r.upcast()
            })
            .collect();
        list(tr("KNOWN SERVERS (KNOWN_HOSTS)"), known);
    }

    fn show_key(self: &Rc<Self>, id: &str) {
        let data = self.data.borrow().clone();
        let Some(i) = data.identities.iter().find(|x| x.id == id).cloned() else { return };
        let b = &self.key_detail;
        while let Some(c) = b.first_child() {
            b.remove(&c);
        }
        let head = ui::hbox(8, "");
        let t = ui::ellip(&i.id, "kr-big-title");
        head.append(&t);
        head.append(&ui::label(tr("vault"), "kr-tag accent"));
        b.append(&head);
        b.append(&ui::label(tr("Public key"), "kr-caption"));
        let pk = mono(&i.public_key);
        pk.add_css_class("kr-codebox");
        b.append(&pk);
        let row = ui::hbox(8, "");
        let copy = ui::button(tr("Copy public"), None, "");
        copy.set_hexpand(true);
        let install = ui::button(tr("Install on a server…"), None, "");
        install.set_hexpand(true);
        row.append(&copy);
        row.append(&install);
        b.append(&row);
        let (r1, confirm, _) = ui::switch_row(tr("Ask for confirmation at each use"), tr("A system notification before signing"));
        confirm.set_active(i.confirm_each_use);
        b.append(&r1);
        let users: Vec<String> = data.hosts.iter().filter(|h| data.effective(h).identity.0.as_deref() == Some(&i.id)).map(|h| h.name.clone()).collect();
        if !users.is_empty() {
            b.append(&ui::para(&trf("Used by: {list}", &[("list", users.join(", "))]), "kr-caption"));
        }
        let del = ui::button(tr("Delete key…"), None, "danger");
        b.append(&del);
        let pkt = i.public_key.clone();
        let me = Rc::downgrade(self);
        copy.connect_clicked(move |w| {
            ui::copy(w, &pkt);
            if let Some(p) = me.upgrade() {
                p.toast(tr("Public key copied"));
            }
        });
        let (me, iid) = (Rc::downgrade(self), i.id.clone());
        confirm.connect_state_set(move |_, on| {
            if let Some(p) = me.upgrade() {
                if let Some(x) = p.data.borrow_mut().identities.iter_mut().find(|x| x.id == iid) {
                    x.confirm_each_use = on;
                }
                p.save();
            }
            glib::Propagation::Proceed
        });
        let (me, iid) = (Rc::downgrade(self), i.id.clone());
        del.connect_clicked(move |b| {
            let Some(p) = me.upgrade() else { return };
            let iid2 = iid.clone();
            let me2 = Rc::downgrade(&p);
            confirm_dialog(b, &trf("Delete the key {k}?", &[("k", iid.clone())]), tr("The servers that use it stop accepting this computer until you install another key."), tr("Delete"), move || {
                if let Some(p) = me2.upgrade() {
                    secrets::clear("ssh-key", &iid2);
                    p.data.borrow_mut().identities.retain(|x| x.id != iid2);
                    p.save();
                    p.key_detail.set_visible(false);
                    p.fill_keys();
                }
            });
        });
        // a command to paste in a session already open on the server (that one still uses its old login)
        let (me, pk2) = (Rc::downgrade(self), i.public_key.clone());
        install.connect_clicked(move |w| {
            let cmd = format!("mkdir -p ~/.ssh && chmod 700 ~/.ssh && echo '{pk2}' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys");
            ui::copy(w, &cmd);
            if let Some(p) = me.upgrade() {
                p.toast(tr("Install command copied: paste it in a Terminal session on the server"));
            }
        });
        b.set_visible(true);
    }

    fn generate_key(self: &Rc<Self>) {
        let d = dialog(&self.root);
        let body = ui::vbox(12, "kr-dialog-body");
        body.set_size_request(420, -1);
        body.append(&ui::label(tr("Generate ed25519 key"), "kr-big-title"));
        let (nb, name) = field(tr("Name (e.g. deploy-ed25519)"), "");
        body.append(&nb);
        let pass = gtk::PasswordEntry::new();
        pass.set_show_peek_icon(true);
        pass.set_property("placeholder-text", tr("Passphrase (optional, on top of the vault)"));
        body.append(&pass);
        let err = ui::label("", "kr-error-text");
        body.append(&err);
        let row = ui::hbox(8, "");
        let cancel = ui::button(tr("Cancel"), None, "");
        cancel.set_hexpand(true);
        let ok = ui::button(tr("Generate"), None, "primary");
        ok.set_hexpand(true);
        row.append(&cancel);
        row.append(&ok);
        body.append(&row);
        d.set_child(Some(&body));
        let dd = d.clone();
        cancel.connect_clicked(move |_| dd.destroy());
        let (dd, me) = (d.clone(), Rc::downgrade(self));
        ok.connect_clicked(move |_| {
            let Some(p) = me.upgrade() else { return };
            let n = name.text().trim().to_owned();
            if n.is_empty() {
                err.set_text(tr("Give the key a name"));
                return;
            }
            let res = ops::generate_key(&p.data.borrow(), &n, &pass.text());
            match res {
                Ok(i) => {
                    let id = i.id.clone();
                    p.data.borrow_mut().identities.insert(0, i);
                    p.save();
                    dd.destroy();
                    p.fill_keys();
                    p.show_key(&id);
                }
                Err(e) => err.set_text(&e),
            }
        });
        d.present();
    }

    fn import_key_dialog(self: &Rc<Self>) {
        let chooser = gtk::FileDialog::builder().title(tr("Import existing key")).initial_folder(&gtk::gio::File::for_path(glib::home_dir().join(".ssh"))).build();
        let me = Rc::downgrade(self);
        let parent = self.root.root().and_downcast::<gtk::Window>();
        chooser.open(parent.as_ref(), gtk::gio::Cancellable::NONE, move |r| {
            let (Some(p), Ok(file)) = (me.upgrade(), r) else { return };
            let Some(path) = file.path() else { return };
            let res = ops::import_key(&p.data.borrow(), &path);
            match res {
                Ok(i) => {
                    p.data.borrow_mut().identities.push(i);
                    p.save();
                    p.fill_keys();
                }
                Err(e) => p.toast(&e),
            }
        });
    }

    // --------------------------------------------------------------------------------- reachability

    /// Tests every server's port in the background, every 60 s (jump-only servers stay untested).
    fn start_reachability(self: &Rc<Self>) {
        let me = Rc::downgrade(self);
        let run = move || {
            let Some(p) = me.upgrade() else { return glib::ControlFlow::Break };
            let data = p.data.borrow().clone();
            let targets: Vec<(String, String, u16)> = data.hosts.iter().filter(|h| data.jump_chain(h).is_empty()).map(|h| (h.id.clone(), h.address.clone(), h.port)).collect();
            let (tx, rx) = async_channel::unbounded();
            std::thread::spawn(move || {
                for (id, a, port) in targets {
                    let ok = std::net::ToSocketAddrs::to_socket_addrs(&(a.as_str(), port))
                        .ok()
                        .and_then(|mut it| it.next())
                        .is_some_and(|sa| std::net::TcpStream::connect_timeout(&sa, Duration::from_secs(3)).is_ok());
                    let _ = tx.send_blocking((id, ok));
                }
            });
            let me2 = Rc::downgrade(&p);
            glib::spawn_future_local(async move {
                let mut changed = false;
                while let Ok((id, ok)) = rx.recv().await {
                    if let Some(p) = me2.upgrade() {
                        changed |= p.reach.borrow_mut().insert(id, ok) != Some(ok);
                    }
                }
                if changed {
                    if let Some(p) = me2.upgrade() {
                        if p.tab.visible_child_name().as_deref() == Some("servers") {
                            p.fill_servers();
                        }
                    }
                }
            });
            glib::ControlFlow::Continue
        };
        let r2 = run.clone();
        glib::idle_add_local_once(move || {
            r2();
        });
        glib::timeout_add_seconds_local(60, run);
    }
}

/// Where a key came from, in the person's language (stored in English in hosts.json).
fn origin_text(o: &str) -> String {
    match o {
        "generated here" => tr("generated here").to_owned(),
        "imported" => tr("imported").to_owned(),
        o => match o.strip_prefix("imported from ") {
            Some(path) => trf("imported from {path}", &[("path", path.to_owned())]),
            None => o.to_owned(),
        },
    }
}

/// What is wrong with a forward, if anything (checked before saving).
pub fn forward_problem(f: &Forward) -> Option<String> {
    let port_of = |s: &str| s.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()).filter(|p| *p > 0);
    if f.host.is_empty() {
        return Some(tr("Choose the server").to_owned());
    }
    if port_of(&f.listen).is_none() {
        return Some(tr("The port here needs a number (e.g. localhost:5433)").to_owned());
    }
    if f.kind != ForwardKind::Dynamic && (port_of(&f.target).is_none() || !f.target.contains(':')) {
        return Some(tr("The destination needs host:port").to_owned());
    }
    if f.kind == ForwardKind::Local {
        if let Some(p) = port_of(&f.listen) {
            if std::net::TcpListener::bind(("127.0.0.1", p)).is_err() {
                return Some(trf("Port {p} is busy", &[("p", p.to_string())]));
            }
        }
    }
    None
}

fn dialog(anchor: &impl IsA<gtk::Widget>) -> gtk::Window {
    let d = gtk::Window::builder().resizable(false).modal(true).build();
    d.add_css_class("kr-window");
    d.add_css_class("kr-dialog");
    if let Some(w) = anchor.as_ref().root().and_downcast::<gtk::Window>() {
        d.set_transient_for(Some(&w));
    }
    let no_titlebar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    no_titlebar.set_visible(false);
    d.set_titlebar(Some(&no_titlebar));
    let keys = gtk::EventControllerKey::new();
    let dd = d.clone();
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            dd.destroy();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    d.add_controller(keys);
    d
}

fn confirm_dialog(anchor: &impl IsA<gtk::Widget>, title: &str, text: &str, action: &str, on_ok: impl Fn() + 'static) {
    let d = dialog(anchor);
    let body = ui::vbox(12, "kr-dialog-body");
    body.set_size_request(400, -1);
    body.append(&ui::label(title, "kr-big-title"));
    body.append(&ui::para(text, "kr-body"));
    let row = ui::hbox(8, "");
    let cancel = ui::button(tr("Cancel"), None, "");
    cancel.set_hexpand(true);
    let ok = ui::button(action, None, "primary");
    ok.set_hexpand(true);
    row.append(&cancel);
    row.append(&ok);
    body.append(&row);
    d.set_child(Some(&body));
    let dd = d.clone();
    cancel.connect_clicked(move |_| dd.destroy());
    let dd = d.clone();
    ok.connect_clicked(move |_| {
        dd.destroy();
        on_ok();
    });
    d.present();
}

trait TapClass {
    fn tap_class(self, c: &str) -> Self;
}
impl TapClass for gtk::Label {
    fn tap_class(self, c: &str) -> Self {
        self.add_css_class(c);
        self
    }
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        self == &Settings::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_are_checked() {
        let mut f = Forward { host: "h".into(), listen: "localhost:0".into(), target: "db:5432".into(), ..Default::default() };
        assert!(forward_problem(&f).is_some(), "port 0");
        f.listen = "localhost:5433x".into();
        assert!(forward_problem(&f).is_some());
        f.listen = "127.0.0.1:59377".into();
        f.target = "db".into();
        assert!(forward_problem(&f).is_some(), "destination without a port");
        f.target = "db:5432".into();
        assert_eq!(forward_problem(&f), None);
        // a busy port is reported
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        f.listen = format!("127.0.0.1:{}", l.local_addr().unwrap().port());
        assert!(forward_problem(&f).is_some());
        f.kind = ForwardKind::Dynamic;
        f.target.clear();
        f.listen = "1080".into();
        assert_eq!(forward_problem(&f), None);
    }
}
