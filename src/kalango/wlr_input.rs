//! The controller's mouse and keyboard on a compositor without the RemoteDesktop portal (niri):
//! enigo's custom mouse and keyboard, backed by a wlr virtual pointer and a virtual keyboard.
//! Installed by [`setup`] when `wlr_remote::active()`.

use enigo::{Key, KeyboardControllable, MouseButton, MouseControllable};
use hbb_common::log;

use crate::server::uinput::service::map_key;

/// The virtual devices, started on first use: KalangoOS's niri offers them only after kalangod allowed this
/// process (niri patch 0008), which happens when the person approves the access, after the server started.
#[derive(Default)]
pub struct Lazy {
    input: std::sync::Mutex<Option<std::sync::Arc<wlr_remote::Input>>>,
    last_try: std::sync::Mutex<Option<std::time::Instant>>,
}

impl Lazy {
    fn get(&self) -> Option<std::sync::Arc<wlr_remote::Input>> {
        let mut input = self.input.lock().unwrap();
        if input.is_none() {
            let mut last = self.last_try.lock().unwrap();
            // at most one attempt a second: events keep coming while it is not allowed
            if last.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1)) {
                return None;
            }
            *last = Some(std::time::Instant::now());
            match wlr_remote::Input::start() {
                Ok(i) => *input = Some(std::sync::Arc::new(i)),
                Err(e) => log::warn!("wlr input not available yet: {e}"),
            }
        }
        input.clone()
    }
}

pub struct WlrMouse(std::sync::Arc<Lazy>);
pub struct WlrKeyboard(std::sync::Arc<Lazy>);

fn button(b: MouseButton) -> Option<wlr_remote::Button> {
    Some(match b {
        MouseButton::Left => wlr_remote::Button::Left,
        MouseButton::Right => wlr_remote::Button::Right,
        MouseButton::Middle => wlr_remote::Button::Middle,
        MouseButton::Back => wlr_remote::Button::Back,
        MouseButton::Forward => wlr_remote::Button::Forward,
        _ => return None,
    })
}

impl MouseControllable for WlrMouse {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_mut_any(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn mouse_move_to(&mut self, x: i32, y: i32) {
        if let Some(i) = self.0.get() { i.move_to(x, y) }
    }
    fn mouse_move_relative(&mut self, x: i32, y: i32) {
        if let Some(i) = self.0.get() { i.move_by(x, y) }
    }
    fn mouse_down(&mut self, b: MouseButton) -> enigo::ResultType {
        match b {
            MouseButton::ScrollUp => { if let Some(i) = self.0.get() { i.scroll(0, -1) } }
            MouseButton::ScrollDown => { if let Some(i) = self.0.get() { i.scroll(0, 1) } }
            MouseButton::ScrollLeft => { if let Some(i) = self.0.get() { i.scroll(-1, 0) } }
            MouseButton::ScrollRight => { if let Some(i) = self.0.get() { i.scroll(1, 0) } }
            b => {
                if let (Some(b), Some(i)) = (button(b), self.0.get()) {
                    i.button(b, true);
                }
            }
        }
        Ok(())
    }
    fn mouse_up(&mut self, b: MouseButton) {
        if let (Some(b), Some(i)) = (button(b), self.0.get()) {
            i.button(b, false);
        }
    }
    fn mouse_click(&mut self, b: MouseButton) {
        self.mouse_down(b).ok();
        self.mouse_up(b);
    }
    fn mouse_scroll_x(&mut self, length: i32) {
        if let Some(i) = self.0.get() { i.scroll(length, 0) }
    }
    fn mouse_scroll_y(&mut self, length: i32) {
        if let Some(i) = self.0.get() { i.scroll(0, length) }
    }
}

/// evdev code (and whether Shift is needed) for an enigo key on a US layout.
fn evdev(key: &Key) -> Option<(u32, bool)> {
    match key {
        // RustDesk sends X keycodes, which are evdev + 8
        Key::Raw(code) if *code >= 8 => Some((*code as u32 - 8, false)),
        k => map_key(k).ok().map(|(k, shift)| (k.code() as u32, shift)),
    }
}

const KEY_LEFTSHIFT: u32 = 42;

impl WlrKeyboard {
    fn press(&self, code: u32, shift: bool, down: bool) {
        let Some(i) = self.0.get() else { return };
        if shift && down {
            i.key(KEY_LEFTSHIFT, true);
        }
        i.key(code, down);
        if shift && !down {
            i.key(KEY_LEFTSHIFT, false);
        }
    }
}

impl KeyboardControllable for WlrKeyboard {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_mut_any(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn get_key_state(&mut self, _key: Key) -> bool {
        false
    }
    fn key_sequence(&mut self, sequence: &str) {
        for c in sequence.chars() {
            match evdev(&Key::Layout(c)) {
                Some((code, shift)) => {
                    self.press(code, shift, true);
                    self.press(code, shift, false);
                }
                None => log::debug!("wlr keyboard: no US key for a character, skipped"),
            }
        }
    }
    fn key_down(&mut self, key: Key) -> enigo::ResultType {
        if let Some((code, shift)) = evdev(&key) {
            self.press(code, shift, true);
        }
        Ok(())
    }
    fn key_up(&mut self, key: Key) {
        if let Some((code, shift)) = evdev(&key) {
            self.press(code, shift, false);
        }
    }
    fn key_click(&mut self, key: Key) {
        self.key_down(key.clone()).ok();
        self.key_up(key);
    }
}

/// Points RustDesk's input at the virtual devices.
pub fn setup() -> hbb_common::ResultType<()> {
    let input = std::sync::Arc::new(Lazy::default());
    crate::server::input_service::set_custom_input(Box::new(WlrKeyboard(input.clone())), Box::new(WlrMouse(input)));
    log::info!("kalango: input through wlr virtual pointer and keyboard (started on first use)");
    Ok(())
}
