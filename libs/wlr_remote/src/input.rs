//! Pointer and keyboard of the remote person, as a virtual pointer and a virtual keyboard. A thread
//! owns the connection; the handle only sends commands, so it is `Send` and cheap to call.

use std::{
    os::fd::AsFd,
    sync::mpsc::{channel, Receiver, Sender},
    time::Instant,
};

use wayland_client::{
    protocol::{wl_buffer, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1::ZxdgOutputManagerV1, zxdg_output_v1};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::{
    screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
    virtual_pointer::v1::client::{
        zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1, zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
    },
};
use xkbcommon::xkb;

use crate::globals::{self, ignore_events, Globals, HasGlobals, Output};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

impl Button {
    /// linux/input-event-codes.h
    pub fn code(self) -> u32 {
        match self {
            Button::Left => 0x110,
            Button::Right => 0x111,
            Button::Middle => 0x112,
            Button::Back => 0x116,
            Button::Forward => 0x115,
        }
    }
}

enum Cmd {
    MoveTo(i32, i32),
    MoveBy(i32, i32),
    Button(Button, bool),
    Scroll(i32, i32),
    Key(u32, bool),
}

pub struct Input {
    tx: Sender<Cmd>,
    /// the layout when the input started (logical coordinates)
    pub outputs: Vec<Output>,
}

impl Input {
    pub fn start() -> std::io::Result<Input> {
        let outputs = globals::outputs()?;
        let (tx, rx) = channel();
        let (ready_tx, ready_rx) = channel();
        std::thread::Builder::new().name("wlr-input".into()).spawn(move || {
            if let Err(e) = run(rx, &ready_tx) {
                log::warn!("wlr input: {e}");
                ready_tx.send(Err(e.to_string())).ok();
            }
        })?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Input { tx, outputs }),
            Ok(Err(e)) => Err(std::io::Error::other(e)),
            Err(_) => Err(std::io::Error::other("wlr input thread ended")),
        }
    }

    /// Absolute position in the global logical layout.
    pub fn move_to(&self, x: i32, y: i32) {
        self.tx.send(Cmd::MoveTo(x, y)).ok();
    }
    pub fn move_by(&self, dx: i32, dy: i32) {
        self.tx.send(Cmd::MoveBy(dx, dy)).ok();
    }
    pub fn button(&self, b: Button, pressed: bool) {
        self.tx.send(Cmd::Button(b, pressed)).ok();
    }
    /// Wheel steps (positive: down / right).
    pub fn scroll(&self, dx: i32, dy: i32) {
        self.tx.send(Cmd::Scroll(dx, dy)).ok();
    }
    /// A key by its evdev code (`KEY_A` = 30), as on a US keyboard.
    pub fn key(&self, evdev: u32, pressed: bool) {
        self.tx.send(Cmd::Key(evdev, pressed)).ok();
    }
}

#[derive(Default)]
struct State {
    g: Globals,
}

impl HasGlobals for State {
    fn globals(&mut self) -> &mut Globals {
        &mut self.g
    }
}

/// The bounding box of all outputs (niri maps an unbound virtual pointer's absolute motion onto it).
pub(crate) fn bounding_box(outputs: &[Output]) -> (i32, i32, i32, i32) {
    let x0 = outputs.iter().map(|o| o.x).min().unwrap_or(0);
    let y0 = outputs.iter().map(|o| o.y).min().unwrap_or(0);
    let x1 = outputs.iter().map(|o| o.x + o.logical_width).max().unwrap_or(1);
    let y1 = outputs.iter().map(|o| o.y + o.logical_height).max().unwrap_or(1);
    (x0, y0, (x1 - x0).max(1), (y1 - y0).max(1))
}

fn run(rx: Receiver<Cmd>, ready: &Sender<Result<(), String>>) -> std::io::Result<()> {
    let mut st = State::default();
    let (conn, mut eq) = globals::connect(&mut st)?;
    let qh = eq.handle();
    let seat = st.g.seat.clone().ok_or_else(|| std::io::Error::other("no wl_seat"))?;
    let vp = st
        .g
        .vpointer
        .as_ref()
        .ok_or_else(|| std::io::Error::other("no virtual pointer manager"))?
        .create_virtual_pointer(Some(&seat), &qh, ());
    let vk = st
        .g
        .vkeyboard
        .as_ref()
        .ok_or_else(|| std::io::Error::other("no virtual keyboard manager"))?
        .create_virtual_keyboard(&seat, &qh, ());
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(&ctx, "evdev", "pc105", "us", "", None, xkb::COMPILE_NO_FLAGS)
        .ok_or_else(|| std::io::Error::other("no xkb keymap"))?;
    let text = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
    let fd = globals::memfd(text.len() + 1)?;
    rustix::io::write(&fd, text.as_bytes())?;
    rustix::io::write(&fd, &[0])?;
    vk.keymap(xkb::KEYMAP_FORMAT_TEXT_V1, fd.as_fd(), (text.len() + 1) as u32);
    let mut xstate = xkb::State::new(&keymap);
    eq.roundtrip(&mut st).map_err(std::io::Error::other)?;
    // keys sent before the focused client applied the keymap are lost
    std::thread::sleep(std::time::Duration::from_millis(300));
    ready.send(Ok(())).ok();

    let t0 = Instant::now();
    let now = || t0.elapsed().as_millis() as u32;
    let (bx, by, bw, bh) = bounding_box(&st.g.public_outputs());
    for cmd in rx {
        match cmd {
            Cmd::MoveTo(x, y) => {
                let x = (x - bx).clamp(0, bw - 1) as u32;
                let y = (y - by).clamp(0, bh - 1) as u32;
                vp.motion_absolute(now(), x, y, bw as u32, bh as u32);
                vp.frame();
            }
            Cmd::MoveBy(dx, dy) => {
                vp.motion(now(), dx as f64, dy as f64);
                vp.frame();
            }
            Cmd::Button(b, pressed) => {
                let state = if pressed { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released };
                vp.button(now(), b.code(), state);
                vp.frame();
            }
            Cmd::Scroll(dx, dy) => {
                vp.axis_source(wl_pointer::AxisSource::Wheel);
                for (axis, steps) in [(wl_pointer::Axis::VerticalScroll, dy), (wl_pointer::Axis::HorizontalScroll, dx)] {
                    if steps != 0 {
                        vp.axis_discrete(now(), axis, 15.0 * steps as f64, steps);
                    }
                }
                vp.frame();
            }
            Cmd::Key(code, pressed) => {
                vk.key(now(), code, if pressed { 1 } else { 0 });
                // the compositor forwards modifiers only as we state them
                xstate.update_key((code + 8).into(), if pressed { xkb::KeyDirection::Down } else { xkb::KeyDirection::Up });
                vk.modifiers(
                    xstate.serialize_mods(xkb::STATE_MODS_DEPRESSED),
                    xstate.serialize_mods(xkb::STATE_MODS_LATCHED),
                    xstate.serialize_mods(xkb::STATE_MODS_LOCKED),
                    xstate.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE),
                );
            }
        }
        conn.flush().map_err(std::io::Error::other)?;
    }
    vp.destroy();
    vk.destroy();
    conn.flush().ok();
    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(st: &mut Self, reg: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            globals::registry_global(st, reg, name, &interface, version, qh);
        }
    }
}

impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(st: &mut Self, _: &wl_output::WlOutput, ev: wl_output::Event, idx: &usize, _: &Connection, _: &QueueHandle<Self>) {
        globals::output_event(&mut st.g, *idx, ev);
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, usize> for State {
    fn event(st: &mut Self, _: &zxdg_output_v1::ZxdgOutputV1, ev: zxdg_output_v1::Event, idx: &usize, _: &Connection, _: &QueueHandle<Self>) {
        globals::xdg_output_event(&mut st.g, *idx, ev);
    }
}

ignore_events!(State: wl_shm::WlShm, wl_seat::WlSeat, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer, ZxdgOutputManagerV1,
    ZwlrScreencopyManagerV1, ZwlrVirtualPointerManagerV1, ZwlrVirtualPointerV1, ZwpVirtualKeyboardManagerV1, ZwpVirtualKeyboardV1);

#[cfg(test)]
mod tests {
    use super::*;

    fn out(x: i32, y: i32, w: i32, h: i32) -> Output {
        Output { name: String::new(), x, y, width: w, height: h, logical_width: w, logical_height: h, flipped: false }
    }

    #[test]
    fn bounding_box_spans_all_outputs() {
        assert_eq!(bounding_box(&[out(0, 0, 1920, 1080)]), (0, 0, 1920, 1080));
        assert_eq!(bounding_box(&[out(0, 0, 1920, 1080), out(1920, -200, 1280, 1024)]), (0, -200, 3200, 1280));
        assert_eq!(bounding_box(&[]), (0, 0, 1, 1));
    }
}
