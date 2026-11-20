//! Look and building blocks: the theme (KalangoOS's dark or light tokens, as the system chose), the
//! stylesheet, the UI icons recoloured from the tokens, and small widget helpers used by every screen.

use gtk::{gdk, glib, prelude::*, Align, Orientation};
use std::{cell::RefCell, collections::HashMap};

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

const TOKENS_DARK: &str = include_str!("../style/tokens-dark.css");
const TOKENS_LIGHT: &str = include_str!("../style/tokens-light.css");
const APP_CSS: &str = include_str!("../style/app.css");

/// Light or dark, as KalangoOS's Appearance setting says (`~/.config/kalango/tema.json`: "escuro",
/// "claro" or "automatico" — light from 7h to 19h). `KALANGO_REMOTE_TEMA=claro|escuro` overrides it.
pub fn is_light() -> bool {
    thread_local!(static LIGHT: bool = {
        let style = std::env::var("KALANGO_REMOTE_TEMA").ok().or_else(|| {
            let p = glib::user_config_dir().join("kalango/tema.json");
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(p).ok()?).ok()?;
            v["estilo"].as_str().map(str::to_owned)
        });
        match style.as_deref() {
            Some("claro") => true,
            Some("automatico") => glib::DateTime::now_local().map(|d| (7..19).contains(&d.hour())).unwrap_or(false),
            _ => false,
        }
    });
    LIGHT.with(|l| *l)
}

fn parse_tokens(css: &str) -> HashMap<String, String> {
    css.lines()
        .filter_map(|l| l.strip_prefix("@define-color "))
        .filter_map(|l| {
            let (n, v) = l.split_once(' ')?;
            Some((n.to_owned(), v.trim_end_matches(';').trim().to_owned()))
        })
        .collect()
}

/// A colour token of the current theme (`k_accent` → `#A5A1E2`).
pub fn token(name: &str) -> String {
    thread_local!(static T: HashMap<String, String> = parse_tokens(if is_light() { TOKENS_LIGHT } else { TOKENS_DARK }));
    T.with(|t| t.get(name).cloned().unwrap_or_else(|| "#FF00FF".into()))
}

pub fn load_css() {
    let fonts = if is_light() {
        "@define-color k_qr_dark #0B0C10;\n* { font-family: \"Roboto\", \"Inter\", sans-serif; }\n.display { font-family: \"Ubuntu\", \"URW Gothic\", sans-serif; }\n"
    } else {
        "@define-color k_qr_dark #0B0C10;\n* { font-family: \"Inter\", \"Roboto\", sans-serif; }\n.display { font-family: \"URW Gothic\", sans-serif; }\n"
    };
    let css = format!("{}\n{}\n{}", if is_light() { TOKENS_LIGHT } else { TOKENS_DARK }, APP_CSS, fonts);
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css);
    if let Some(display) = gdk::Display::default() {
        // above the person's own gtk.css (USER priority), as KalangoOS's apps do
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_USER + 1);
    }
}

/// The SVG of a UI icon with every colour replaced by `color` (a token name or `#RRGGBB`).
pub fn icon_svg(name: &str, color: &str) -> Option<String> {
    let src = ICONS.iter().find(|(n, _)| *n == name)?.1;
    let hex = if color.starts_with('#') { color.to_owned() } else { token(color) };
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(i) = rest.find("=\"#") {
        out.push_str(&rest[..i + 2]);
        let after = &rest[i + 2..];
        let end = after[1..].find('"').map(|e| e + 1).unwrap_or(after.len());
        out.push_str(&hex);
        rest = &after[end..];
    }
    out.push_str(rest);
    Some(out)
}

thread_local! {
    static TEXTURES: RefCell<HashMap<(String, String, i32), gdk::Texture>> = RefCell::new(HashMap::new());
}

/// An icon as a texture rendered at twice its size (sharp on scaled outputs).
pub fn icon_texture(name: &str, color: &str, px: i32) -> Option<gdk::Texture> {
    let key = (name.to_owned(), color.to_owned(), px);
    if let Some(t) = TEXTURES.with(|c| c.borrow().get(&key).cloned()) {
        return Some(t);
    }
    let svg = icon_svg(name, color)?;
    let loader = gtk::gdk_pixbuf::PixbufLoader::with_type("svg").ok()?;
    loader.set_size(px * 2, px * 2);
    loader.write(svg.as_bytes()).ok()?;
    loader.close().ok()?;
    let tex = gdk::Texture::for_pixbuf(&loader.pixbuf()?);
    TEXTURES.with(|c| c.borrow_mut().insert(key, tex.clone()));
    Some(tex)
}

pub fn icon(name: &str, color: &str, px: i32) -> gtk::Image {
    let img = gtk::Image::new();
    if let Some(t) = icon_texture(name, color, px) {
        img.set_paintable(Some(&t));
    }
    img.set_pixel_size(px);
    img.set_size_request(px, px);
    img.set_halign(Align::Center);
    img.set_valign(Align::Center);
    img
}

pub fn set_icon(img: &gtk::Image, name: &str, color: &str, px: i32) {
    img.set_paintable(icon_texture(name, color, px).as_ref());
}

/// A picture from the app's PNGs (brand art).
pub fn png(bytes: &'static [u8], px: i32) -> gtk::Picture {
    let pic = gtk::Picture::new();
    if let Ok(t) = gdk::Texture::from_bytes(&glib::Bytes::from_static(bytes)) {
        pic.set_paintable(Some(&t));
    }
    pic.set_can_shrink(true);
    pic.set_content_fit(gtk::ContentFit::Contain);
    pic.set_size_request(px, px);
    pic
}

pub fn hbox(spacing: i32, class: &str) -> gtk::Box {
    let b = gtk::Box::new(Orientation::Horizontal, spacing);
    if !class.is_empty() {
        for c in class.split(' ') {
            b.add_css_class(c);
        }
    }
    b
}

pub fn vbox(spacing: i32, class: &str) -> gtk::Box {
    let b = gtk::Box::new(Orientation::Vertical, spacing);
    if !class.is_empty() {
        for c in class.split(' ') {
            b.add_css_class(c);
        }
    }
    b
}

pub fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    if !class.is_empty() {
        for c in class.split(' ') {
            l.add_css_class(c);
        }
    }
    l
}

/// A label that wraps inside its column instead of widening it.
pub fn para(text: &str, class: &str) -> gtk::Label {
    let l = label(text, class);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l
}

/// A label that cuts with "…" instead of widening its column.
pub fn ellip(text: &str, class: &str) -> gtk::Label {
    let l = label(text, class);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l
}

pub fn spacer() -> gtk::Box {
    let b = gtk::Box::new(Orientation::Horizontal, 0);
    b.set_hexpand(true);
    b
}

/// A text button, optionally with an icon before the text. `class`: `primary`, `danger`, `ghost`,
/// `secondary` (default look) and size classes from the stylesheet.
pub fn button(text: &str, icon_name: Option<(&str, &str)>, class: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("kr-btn");
    for c in class.split(' ').filter(|c| !c.is_empty()) {
        b.add_css_class(c);
    }
    let row = hbox(6, "");
    row.set_halign(Align::Center);
    if let Some((n, color)) = icon_name {
        row.append(&icon(n, color, 16));
    }
    if !text.is_empty() {
        row.append(&gtk::Label::new(Some(text)));
    }
    b.set_child(Some(&row));
    b
}

/// A square icon button (`kr-icon-btn`), with a tooltip.
pub fn icon_button(name: &str, color: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("kr-icon-btn");
    b.set_child(Some(&icon(name, color, 16)));
    if !tip.is_empty() {
        b.set_tooltip_text(Some(tip));
    }
    b.set_valign(Align::Center);
    b
}

/// A route/status badge: dot + text in a colour (`teal`, `blue`, `coral`, `gold`, `dim`).
pub fn badge(text: &str, color: &str) -> (gtk::Box, gtk::Label) {
    let b = hbox(5, &format!("kr-badge {color}"));
    let dot = gtk::Box::new(Orientation::Horizontal, 0);
    dot.add_css_class("dot");
    dot.set_valign(Align::Center);
    b.append(&dot);
    let l = gtk::Label::new(Some(text));
    b.append(&l);
    b.set_valign(Align::Center);
    (b, l)
}

/// Section title of a card: icon + UPPERCASE label.
pub fn card_title(icon_name: &str, text: &str) -> gtk::Box {
    let b = hbox(6, "");
    b.append(&icon(icon_name, "k_muted", 16));
    b.append(&label(text, "kr-section"));
    b
}

/// A row "title / subtitle ... switch".
pub fn switch_row(title: &str, subtitle: &str) -> (gtk::Box, gtk::Switch, gtk::Label) {
    let row = hbox(12, "kr-switch-row");
    let t = vbox(2, "");
    t.set_hexpand(true);
    t.append(&para(title, "kr-row-title"));
    let sub = para(subtitle, "kr-row-sub");
    sub.set_visible(!subtitle.is_empty());
    t.append(&sub);
    row.append(&t);
    let s = gtk::Switch::new();
    s.set_valign(Align::Center);
    row.append(&s);
    (row, s, sub)
}

/// A segmented control; returns the box and its buttons (one active at a time).
pub fn segmented(items: &[&str], class: &str) -> (gtk::Box, Vec<gtk::ToggleButton>) {
    let b = hbox(2, &format!("kr-seg {class}"));
    let mut buttons: Vec<gtk::ToggleButton> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let t = gtk::ToggleButton::with_label(it);
        t.set_hexpand(true);
        if let Some(first) = buttons.first() {
            t.set_group(Some(first));
        }
        if i == 0 {
            t.set_active(true);
        }
        b.append(&t);
        buttons.push(t);
    }
    (b, buttons)
}

/// Copies text to the clipboard.
pub fn copy(widget: &impl IsA<gtk::Widget>, text: &str) {
    widget.as_ref().clipboard().set_text(text);
}

/// The window's own controls (minimize, maximize, close) as round 16 px buttons.
pub fn window_controls() -> gtk::Box {
    let c = hbox(8, "kr-wincontrols");
    c.set_valign(Align::Center);
    for (glyph, action) in [("minimizar", 0), ("maximizar", 1), ("fechar", 2)] {
        let x = gtk::Button::new();
        x.add_css_class("kr-winctl");
        x.set_child(Some(&icon(glyph, "k_text", 16)));
        x.set_valign(Align::Center);
        x.connect_clicked(move |x| {
            let Some(w) = x.root().and_downcast::<gtk::Window>() else { return };
            match action {
                0 => w.minimize(),
                1 => {
                    if w.is_maximized() {
                        w.unmaximize()
                    } else {
                        w.maximize()
                    }
                }
                _ => w.close(),
            }
        });
        c.append(&x);
    }
    c
}

/// Runs `f(width)` whenever the widget's allocated width changes (for layouts that follow the space).
pub fn on_width(widget: &impl IsA<gtk::Widget>, f: impl Fn(i32) + 'static) {
    let last = std::cell::Cell::new(-1);
    widget.as_ref().add_tick_callback(move |w, _| {
        let width = w.width();
        if width > 0 && width != last.get() {
            last.set(width);
            f(width);
        }
        glib::ControlFlow::Continue
    });
}

/// A short message at the bottom of a window ("Código copiado"), gone after 2.4 s.
pub fn toast(overlay: &gtk::Overlay, text: &str) {
    let l = label(text, "kr-toast");
    l.set_halign(Align::Center);
    l.set_valign(Align::End);
    l.set_margin_bottom(24);
    l.set_can_target(false);
    overlay.add_overlay(&l);
    let o = overlay.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(2400), move || o.remove_overlay(&l));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recolours_every_colour_of_an_icon() {
        let s = icon_svg("copiar", "#123456").unwrap();
        assert!(s.contains("#123456"));
        assert!(!s.contains("#8E8F99"));
        assert!(s.contains("fill=\"none\""), "fill=none must stay none");
    }

    #[test]
    fn tokens_parse() {
        let t = parse_tokens("@define-color k_accent #A5A1E2;\n/* x */\n@define-color k_text #E2E2E6;");
        assert_eq!(t["k_accent"], "#A5A1E2");
        assert_eq!(t.len(), 2);
    }
}
