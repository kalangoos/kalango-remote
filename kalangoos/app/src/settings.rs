//! Ajustes: security and allowed devices on the left; network (Cloudflare without a server — the
//! default —, local network only, own RustDesk-compatible server) and screen & quality on the right.
//! Two columns that become one below 1000 px.

use gtk::{glib, prelude::*, Align};
use std::rc::{Rc, Weak};

use crate::{
    i18n::{tr, trf},
    secrets,
    store::{self, Settings},
    ui,
    window::Window,
};

pub struct SettingsPage {
    pub root: gtk::ScrolledWindow,
}

fn set(w: &Weak<Window>, f: impl Fn(&mut Settings)) {
    if let Some(w) = w.upgrade() {
        f(&mut w.settings.borrow_mut());
        w.settings_changed();
    }
}

fn bind_switch(row: (gtk::Box, gtk::Switch, gtk::Label), on: bool, w: &Weak<Window>, f: fn(&mut Settings, bool)) -> gtk::Box {
    let (r, s, _) = row;
    s.set_active(on);
    let w = w.clone();
    s.connect_state_set(move |_, v| {
        set(&w, |st| f(st, v));
        glib::Propagation::Proceed
    });
    r
}

fn dropdown(items: &[&str], selected: u32) -> gtk::DropDown {
    let d = gtk::DropDown::from_strings(items);
    d.add_css_class("kr-dropdown");
    d.set_selected(selected);
    d.set_size_request(240, -1);
    d.set_valign(Align::Center);
    d
}

fn value_row(title: &str, sub: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = ui::hbox(12, "kr-switch-row");
    let t = ui::vbox(2, "");
    t.set_hexpand(true);
    t.append(&ui::para(title, "kr-row-title"));
    if !sub.is_empty() {
        t.append(&ui::para(sub, "kr-row-sub"));
    }
    row.append(&t);
    row.append(control);
    row
}

impl SettingsPage {
    pub fn new(win: &Rc<Window>) -> SettingsPage {
        let w = Rc::downgrade(win);
        let s = win.settings.borrow().clone();
        let page = ui::hbox(16, "kr-page");
        let left = ui::vbox(16, "");
        left.set_hexpand(true);
        left.set_valign(Align::Start);
        let right = ui::vbox(16, "");
        right.set_hexpand(true);
        right.set_valign(Align::Start);
        page.append(&left);
        page.append(&right);

        // security
        let sec = ui::vbox(14, "kr-card");
        sec.append(&ui::card_title("escudo", tr("SECURITY")));
        sec.append(&bind_switch(ui::switch_row(tr("Ask before each access"), tr("You see who it is and choose what they can do")), s.ask_before, &w, |s, v| s.ask_before = v));
        let has_fixed = secrets::exists("fixed-password", "default");
        let (un_row, un_sw, un_sub) = ui::switch_row(tr("Unattended access"), tr("Come in without anyone approving, with a fixed password. For your own computers."));
        un_sw.set_active(s.unattended && has_fixed);
        sec.append(&un_row);
        let set_pw = ui::button(tr("Set password…"), None, "");
        set_pw.set_valign(Align::Center);
        sec.append(&value_row(tr("Fixed password"), tr("Kept in the system's keyring (later: the KalangoOS vault)"), &set_pw));
        let (tfa_row, tfa_sw, _) = ui::switch_row(tr("Two-step verification"), tr("Asks for the authenticator app code on every unattended access"));
        tfa_sw.set_active(s.two_factor && secrets::exists("two-factor", "default"));
        sec.append(&tfa_row);
        let otp = dropdown(&[tr("6 characters, new"), tr("8 characters, new")], if s.otp_length == 8 { 1 } else { 0 });
        sec.append(&value_row(tr("One-time password"), "", &otp));
        sec.append(&bind_switch(ui::switch_row(tr("Lock the screen when the session ends"), ""), s.lock_after, &w, |s, v| s.lock_after = v));
        // privacy mode needs the KalangoOS compositor (niri patch 0012: a cover only the local monitors show, the
        // captures keep the real desktop): elsewhere the switch stays off, with the reason
        let privacy_ok = crate::kalangod::compositor_supports_privacy();
        let (pr_row, pr_sw, _) = ui::switch_row(
            tr("Black screen here during unattended access"),
            if privacy_ok {
                tr("Only when nobody approved the access here. Whoever is nearby does not see what is done; Esc Esc and the Stop button keep working")
            } else {
                tr("Needs the KalangoOS compositor, which this session is not running")
            },
        );
        pr_sw.set_active(privacy_ok && s.privacy_black);
        pr_sw.set_sensitive(privacy_ok);
        if privacy_ok {
            let w2 = w.clone();
            pr_sw.connect_state_set(move |_, v| {
                set(&w2, |st| st.privacy_black = v);
                glib::Propagation::Proceed
            });
        }
        sec.append(&pr_row);
        sec.append(&bind_switch(ui::switch_row(tr("Record every session"), tr("In Videos › Kalango Remote; deleted after 30 days")), s.record, &w, |s, v| s.record = v));
        left.append(&sec);

        // allowed devices
        let dev = ui::vbox(12, "kr-card");
        dev.append(&ui::card_title("check", tr("ALLOWED DEVICES")));
        let list = ui::vbox(10, "");
        dev.append(&list);
        let fill_list = {
            let w = w.clone();
            let list = list.clone();
            Rc::new(move || {
                while let Some(c) = list.first_child() {
                    list.remove(&c);
                }
                let Some(win) = w.upgrade() else { return };
                let allowed = win.settings.borrow().allowed.clone();
                if allowed.is_empty() {
                    list.append(&ui::para(tr("None yet. “Always for this device” in a request adds it here."), "kr-caption"));
                }
                for (i, a) in allowed.iter().enumerate() {
                    let row = ui::hbox(12, "");
                    let t = ui::vbox(2, "");
                    t.set_hexpand(true);
                    t.append(&ui::ellip(&a.name, "kr-row-title"));
                    let when = if a.until == 0 { tr("always").to_owned() } else if a.until < store::now() { tr("expired").to_owned() } else { trf("until {when}", &[("when", fmt_date(a.until))]) };
                    t.append(&ui::ellip(&format!("{when} · {}", trf("since {when}", &[("when", fmt_date(a.since))])), "kr-row-sub"));
                    row.append(&t);
                    let rm = ui::button(tr("Remove"), None, "ghost");
                    let w2 = w.clone();
                    rm.connect_clicked(move |b| {
                        set(&w2, |s| {
                            if i < s.allowed.len() {
                                s.allowed.remove(i);
                            }
                        });
                        if let Some(p) = b.parent() {
                            p.set_visible(false);
                        }
                    });
                    row.append(&rm);
                    list.append(&row);
                }
            })
        };
        fill_list();
        let fl = fill_list.clone();
        list.connect_map(move |_| fl());
        dev.append(&bind_switch(ui::switch_row(tr("Accept only the devices on this list"), tr("The others cannot even ask")), s.allowed_only, &w, |s, v| s.allowed_only = v));
        let ranges = ui::hbox(8, "kr-field");
        ranges.append(&ui::icon("rede-cabo", "k_muted", 16));
        let re = gtk::Entry::new();
        re.set_hexpand(true);
        re.set_text(&s.allowed_ranges);
        re.set_placeholder_text(Some(tr("Addresses allowed on the local network (e.g. 192.168.0.0/24)")));
        ranges.append(&re);
        let w2 = w.clone();
        re.connect_changed(move |e| {
            let t = e.text().to_string();
            set(&w2, |s| s.allowed_ranges = t.clone());
        });
        dev.append(&ranges);
        left.append(&dev);

        // network
        let net = ui::vbox(12, "kr-card");
        net.append(&ui::card_title("rede-cabo", tr("NETWORK")));
        let mk = |title: &str, sub: &str, badge: bool| {
            let card = ui::vbox(6, "kr-option");
            let head = ui::hbox(8, "");
            let rb = gtk::CheckButton::new();
            rb.add_css_class("kr-radio");
            head.append(&rb);
            head.append(&ui::label(title, "kr-row-title"));
            if badge {
                head.append(&ui::label(tr("default"), "kr-tag teal"));
            }
            card.append(&head);
            let p = ui::para(sub, "kr-caption");
            p.set_margin_start(28);
            card.append(&p);
            (card, rb)
        };
        let (c1, r1) = mk(
            tr("Cloudflare (no server)"),
            tr("When you share, KalangoOS opens a temporary Cloudflare tunnel (cloudflared) and closes it when you stop. No account needed and nothing goes through KalangoOS servers; picture and keyboard are encrypted end to end."),
            true,
        );
        let (c2, r2) = mk(tr("Local network only, direct"), tr("Works only when both computers are on the same network. Nothing goes out to the internet."), false);
        let (c3, r3) = mk(tr("Own server (advanced)"), tr("A RustDesk-compatible meeting and relay server, kept by you or your company."), false);
        r2.set_group(Some(&r1));
        r3.set_group(Some(&r1));
        let fields = ui::vbox(8, "");
        fields.set_margin_start(28);
        let field = |ph: &str, v: &str| {
            let b = ui::hbox(8, "kr-field");
            let e = gtk::Entry::new();
            e.set_hexpand(true);
            e.set_text(v);
            e.set_placeholder_text(Some(ph));
            b.append(&e);
            fields.append(&b);
            e
        };
        let id_srv = field(tr("ID server  ·  e.g. remote.mycompany.com"), &s.server_id);
        let relay = field(tr("Relay server (optional)"), &s.server_relay);
        let key = field(tr("Server public key"), &s.server_key);
        let key_warn = ui::label(tr("Without the server's public key its identity is not verified: sharing and connecting through it are refused."), "kr-error-text");
        key_warn.set_wrap(true);
        key_warn.set_xalign(0.0);
        key_warn.set_visible(false);
        fields.append(&key_warn);
        fields.append(&ui::para(tr("Sharing then shows this computer's id instead of a link; connect to others by their id. Restart the share after changing it."), "kr-note"));
        c3.append(&fields);
        net.append(&c1);
        net.append(&c2);
        net.append(&c3);
        let choose = {
            let (c1, c2, c3, fields) = (c1.clone(), c2.clone(), c3.clone(), fields.clone());
            let (kw, ke) = (key_warn.clone(), key.clone());
            move |n: &str| {
                kw.set_visible(n == "server" && ke.text().trim().is_empty());
                for (c, id) in [(&c1, "cloudflare"), (&c2, "lan"), (&c3, "server")] {
                    if id == n {
                        c.add_css_class("selected");
                    } else {
                        c.remove_css_class("selected");
                    }
                }
                fields.set_sensitive(n == "server");
                fields.set_opacity(if n == "server" { 1.0 } else { 0.5 });
            }
        };
        match s.network.as_str() {
            "lan" => r2.set_active(true),
            "server" => r3.set_active(true),
            _ => r1.set_active(true),
        }
        choose(&s.network);
        for (rb, id) in [(&r1, "cloudflare"), (&r2, "lan"), (&r3, "server")] {
            let w2 = w.clone();
            let ch = choose.clone();
            rb.connect_toggled(move |b| {
                if b.is_active() {
                    set(&w2, |s| s.network = id.into());
                    ch(id);
                }
            });
        }
        for (e, f) in [(&id_srv, 0), (&relay, 1), (&key, 2)] {
            let w2 = w.clone();
            let kw = key_warn.clone();
            e.connect_changed(move |e| {
                let t = e.text().to_string();
                if f == 2 {
                    kw.set_visible(w2.upgrade().is_some_and(|w| w.settings.borrow().network == "server") && t.trim().is_empty());
                }
                set(&w2, |s| match f {
                    0 => s.server_id = t.clone(),
                    1 => s.server_relay = t.clone(),
                    _ => s.server_key = t.clone(),
                });
            });
        }
        net.append(&bind_switch(
            ui::switch_row(tr("Also accept connections on the same network"), tr("Opens a port on the local network while you share (Cloudflare mode), so the code is found there; only local addresses get in")),
            s.lan_too,
            &w,
            |s, v| s.lan_too = v,
        ));
        net.append(&bind_switch(ui::switch_row(tr("Accept connection by IP address"), tr("Port 21118 · local network only")), s.accept_ip, &w, |s, v| s.accept_ip = v));
        let cf = ui::hbox(8, "");
        let status = cloudflared_status();
        cf.append(&ui::icon(if status.0 { "check" } else { "alerta" }, if status.0 { "k_teal" } else { "k_gold" }, 16));
        cf.append(&ui::para(&status.1, "kr-caption"));
        net.append(&cf);
        right.append(&net);

        // screen and quality
        let scr = ui::vbox(12, "kr-card");
        scr.append(&ui::card_title("monitor", tr("SCREEN AND QUALITY")));
        let (qseg, qb) = ui::segmented(&[tr("Best picture"), tr("Balanced"), tr("Fast response")], "");
        qseg.set_valign(Align::Center);
        let qi = match s.quality.as_str() {
            "best" => 0,
            "speed" | "low" => 2,
            _ => 1,
        };
        qb[qi].set_active(true);
        for (i, b) in qb.iter().enumerate() {
            let w2 = w.clone();
            b.connect_toggled(move |b| {
                if b.is_active() {
                    set(&w2, |s| s.quality = ["best", "balanced", "speed"][i].into());
                }
            });
        }
        scr.append(&value_row(tr("Default quality"), "", &qseg));
        let scale = dropdown(&[tr("Fit to the window"), tr("100%"), tr("Stretch")], match s.scale.as_str() { "original" => 1, "stretch" => 2, _ => 0 });
        let w2 = w.clone();
        scale.connect_selected_notify(move |d| {
            let v = ["fit", "original", "stretch"][d.selected().min(2) as usize];
            set(&w2, |s| s.scale = v.into());
        });
        scr.append(&value_row(tr("Scale"), "", &scale));
        let codec = dropdown(&[tr("Automatic"), "AV1", "VP9", "H.264"], match s.codec.as_str() { "av1" => 1, "vp9" => 2, "h264" => 3, _ => 0 });
        let w2 = w.clone();
        codec.connect_selected_notify(move |d| {
            let v = ["auto", "av1", "vp9", "h264"][d.selected().min(3) as usize];
            set(&w2, |s| s.codec = v.into());
        });
        scr.append(&value_row(tr("Video codec"), tr("AV1 when both accelerate it; otherwise VP9 or H.264"), &codec));
        scr.append(&bind_switch(ui::switch_row(tr("Show the other computer's cursor"), ""), s.show_cursor, &w, |s, v| s.show_cursor = v));
        scr.append(&bind_switch(ui::switch_row(tr("Audio of the other computer"), ""), s.audio, &w, |s, v| s.audio = v));
        scr.append(&bind_switch(ui::switch_row(tr("Share the clipboard"), ""), s.clipboard, &w, |s, v| s.clipboard = v));
        right.append(&scr);

        // the fixed password and unattended access
        let w2 = w.clone();
        let sub2 = un_sub.clone();
        set_pw.connect_clicked(move |b| fixed_password_dialog(b, &w2, &sub2));
        let w2 = w.clone();
        un_sw.connect_state_set(move |s, on| {
            if on && !secrets::exists("fixed-password", "default") {
                s.set_active(false);
                if let Some(w) = w2.upgrade() {
                    w.toast(tr("Set the fixed password first"));
                }
                return glib::Propagation::Stop;
            }
            set(&w2, |st| st.unattended = on);
            glib::Propagation::Proceed
        });
        let w2 = w.clone();
        tfa_sw.connect_state_set(move |sw, on| {
            if on && !secrets::exists("two-factor", "default") {
                // the switch only turns on once the first code of the authenticator app was right
                two_factor_dialog(sw, &w2);
                return glib::Propagation::Stop;
            }
            if !on {
                secrets::clear("two-factor", "default");
                if let Some(w) = w2.upgrade() {
                    w.toast(tr("Two-step verification turned off"));
                }
            }
            set(&w2, |st| st.two_factor = on);
            glib::Propagation::Proceed
        });
        let w2 = w.clone();
        otp.connect_selected_notify(move |d| {
            let n = if d.selected() == 1 { 8 } else { 6 };
            set(&w2, |s| s.otp_length = n);
        });

        let root = gtk::ScrolledWindow::new();
        root.set_hscrollbar_policy(gtk::PolicyType::Never);
        root.set_child(Some(&page));
        let p = page.clone();
        ui::on_width(&root, move |width| {
            p.set_orientation(if width < 1000 { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
        });
        SettingsPage { root }
    }
}

fn fmt_date(t: u64) -> String {
    glib::DateTime::from_unix_local(t as i64).ok().and_then(|d| d.format("%d/%m %H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
}

/// Whether cloudflared is here, its version, and that no tunnel is open by default.
fn cloudflared_status() -> (bool, String) {
    let bin = std::env::var_os("KALANGO_CLOUDFLARED").map(std::path::PathBuf::from).or_else(|| {
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?.to_path_buf();
        [dir.join("cloudflared"), dir.join("../../kalangoos/.deps/bin/cloudflared")]
            .into_iter()
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|d| d.join("cloudflared")))
            .find(|p| p.is_file())
    });
    match bin.and_then(|b| std::process::Command::new(b).arg("--version").output().ok()) {
        Some(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            let v = s.split_whitespace().skip_while(|w| *w != "version").nth(1).unwrap_or("?").to_owned();
            (true, trf("cloudflared {v} installed · no account · no tunnel open", &[("v", v)]))
        }
        _ => (false, tr("cloudflared is not installed: sharing through Cloudflare will not open").to_owned()),
    }
}

/// Turning two-step verification on: a new secret as a QR code and as text for the authenticator app, and the
/// first code it shows to prove it was set up; only then is the secret kept (in the keyring).
fn two_factor_dialog(anchor: &gtk::Switch, w: &Weak<Window>) {
    use kalango_remote_proto::totp;
    let Ok(secret) = totp::new_secret() else {
        if let Some(w) = w.upgrade() {
            w.toast(tr("Could not make a secret: no source of randomness"));
        }
        return;
    };
    let pop = gtk::Popover::new();
    pop.add_css_class("kr-menu");
    let b = ui::vbox(8, "");
    for set_margin in [gtk::Widget::set_margin_start, gtk::Widget::set_margin_end, gtk::Widget::set_margin_top, gtk::Widget::set_margin_bottom] {
        set_margin(b.upcast_ref(), 8);
    }
    b.set_size_request(300, -1);
    b.append(&ui::label(tr("Two-step verification"), "kr-row-title"));
    b.append(&ui::para(tr("Scan the code with an authenticator app, or type the key in it. Then enter the 6 digits it shows."), "kr-row-sub"));
    let account = glib::host_name().to_string();
    let url = totp::otpauth_url(&secret, &account, "Kalango Remote");
    if let Some(t) = crate::home::qr_texture(&url) {
        let img = gtk::Image::from_paintable(Some(&t));
        img.set_pixel_size(168);
        img.set_halign(Align::Center);
        b.append(&img);
    }
    let text = totp::base32(&secret);
    let spaced: Vec<&str> = text.as_bytes().chunks(4).filter_map(|c| std::str::from_utf8(c).ok()).collect();
    let key = ui::label(&spaced.join(" "), "kr-mono");
    key.set_focusable(false);
    key.set_halign(Align::Center);
    b.append(&key);
    let code = gtk::Entry::new();
    code.set_placeholder_text(Some(tr("6-digit code")));
    code.set_max_length(7);
    code.set_input_purpose(gtk::InputPurpose::Digits);
    b.append(&code);
    let err = ui::label("", "kr-error-text");
    b.append(&err);
    let ok = ui::button(tr("Turn on"), None, "primary");
    b.append(&ok);
    pop.set_child(Some(&b));
    pop.set_parent(anchor);
    let (w, p, sw) = (w.clone(), pop.clone(), anchor.clone());
    let code_entry = code.clone();
    let check = move || {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if !totp::verify(&secret, &code_entry.text(), now) {
            err.set_text(tr("That code is not right. Check the clock of both devices."));
            return;
        }
        match secrets::store("two-factor", "default", "Kalango Remote: two-step verification", &text) {
            Ok(()) => {
                p.popdown();
                sw.set_active(true);
                if let Some(w) = w.upgrade() {
                    w.toast(tr("Two-step verification turned on"));
                }
            }
            Err(e) => err.set_text(&e.to_string()),
        }
    };
    let c = check.clone();
    ok.connect_clicked(move |_| c());
    code.connect_activate(move |_| check());
    pop.popup();
}

fn fixed_password_dialog(anchor: &gtk::Button, w: &Weak<Window>, sub: &gtk::Label) {
    let pop = gtk::Popover::new();
    pop.add_css_class("kr-menu");
    let b = ui::vbox(8, "");
    b.set_margin_start(6);
    b.set_margin_end(6);
    b.set_margin_top(6);
    b.set_margin_bottom(6);
    let e1 = gtk::PasswordEntry::new();
    e1.set_show_peek_icon(true);
    e1.set_property("placeholder-text", tr("New fixed password (8+ characters)"));
    let e2 = gtk::PasswordEntry::new();
    e2.set_property("placeholder-text", tr("Repeat it"));
    let err = ui::label("", "kr-error-text");
    let save = ui::button(tr("Save"), None, "primary");
    b.append(&e1);
    b.append(&e2);
    b.append(&err);
    b.append(&save);
    pop.set_child(Some(&b));
    pop.set_parent(anchor);
    let (w, sub, p) = (w.clone(), sub.clone(), pop.clone());
    save.connect_clicked(move |_| {
        let (a, c) = (e1.text().to_string(), e2.text().to_string());
        if a.chars().count() < 8 {
            err.set_text(tr("Use at least 8 characters"));
        } else if a != c {
            err.set_text(tr("The two do not match"));
        } else {
            match secrets::store("fixed-password", "default", "Kalango Remote: fixed password", &a) {
                Ok(()) => {
                    sub.set_text(tr("Come in without anyone approving, with a fixed password. For your own computers."));
                    if let Some(w) = w.upgrade() {
                        w.settings_changed();
                        w.toast(tr("Fixed password saved in the keyring"));
                    }
                    p.popdown();
                }
                Err(e) => err.set_text(&e.to_string()),
            }
        }
    });
    pop.popup();
}
