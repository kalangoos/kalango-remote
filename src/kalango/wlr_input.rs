//! The controller's mouse and keyboard on a compositor without the RemoteDesktop portal (niri):
//! enigo's custom mouse and keyboard, backed by a wlr virtual pointer and a virtual keyboard.
//! Installed by [`setup`] when `wlr_remote::active()`.

use enigo::{Key, KeyboardControllable, MouseButton, MouseControllable};
use hbb_common::log;

use crate::server::uinput::service::map_key;

pub struct WlrMouse(std::sync::Arc<wlr_remote::Input>);
pub struct WlrKeyboard(std::sync::Arc<wlr_remote::Input>);

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
        self.0.move_to(x, y);
    }
    fn mouse_move_relative(&mut self, x: i32, y: i32) {
        self.0.move_by(x, y);
    }
    fn mouse_down(&mut self, b: MouseButton) -> enigo::ResultType {
        match b {
            MouseButton::ScrollUp => self.0.scroll(0, -1),
            MouseButton::ScrollDown => self.0.scroll(0, 1),
            MouseButton::ScrollLeft => self.0.scroll(-1, 0),
            MouseButton::ScrollRight => self.0.scroll(1, 0),
            b => {
                if let Some(b) = button(b) {
                    self.0.button(b, true);
                }
            }
        }
        Ok(())
    }
    fn mouse_up(&mut self, b: MouseButton) {
        if let Some(b) = button(b) {
            self.0.button(b, false);
        }
    }
    fn mouse_click(&mut self, b: MouseButton) {
        self.mouse_down(b).ok();
        self.mouse_up(b);
    }
    fn mouse_scroll_x(&mut self, length: i32) {
        self.0.scroll(length, 0);
    }
    fn mouse_scroll_y(&mut self, length: i32) {
        self.0.scroll(0, length);
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
        if shift && down {
            self.0.key(KEY_LEFTSHIFT, true);
        }
        self.0.key(code, down);
        if shift && !down {
            self.0.key(KEY_LEFTSHIFT, false);
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
    let input = std::sync::Arc::new(wlr_remote::Input::start()?);
    crate::server::input_service::set_custom_input(Box::new(WlrKeyboard(input.clone())), Box::new(WlrMouse(input)));
    log::info!("kalango: input through wlr virtual pointer and keyboard");
    Ok(())
}
