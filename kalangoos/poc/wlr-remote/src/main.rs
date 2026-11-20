//! PoC: can a normal (unsandboxed) client capture niri's screen and inject input without the
//! RemoteDesktop portal? Uses wlr-screencopy for frames and the wlr virtual-pointer / zwp
//! virtual-keyboard protocols for input — the three globals niri exposes to unrestricted clients.
//!
//!   wlr-remote-poc capture out.png
//!   wlr-remote-poc move X Y        (absolute, in output pixels)
//!   wlr-remote-poc click X Y
//!   wlr-remote-poc type TEXT       (lowercase ASCII letters, digits and space)
//!   wlr-remote-poc bench N         (N captures, prints frames per second)

use std::{
    os::fd::{AsFd, OwnedFd},
    time::{Duration, Instant},
};

use wayland_client::{
    protocol::{wl_buffer, wl_output, wl_registry, wl_seat, wl_shm, wl_shm_pool},
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::{
    screencopy::v1::client::{
        zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
        zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
    },
    virtual_pointer::v1::client::{
        zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
        zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
    },
};

#[derive(Default)]
struct State {
    shm: Option<wl_shm::WlShm>,
    seat: Option<wl_seat::WlSeat>,
    output: Option<wl_output::WlOutput>,
    output_size: Option<(i32, i32)>,
    output_transform: Option<wl_output::Transform>,
    screencopy: Option<ZwlrScreencopyManagerV1>,
    vpointer: Option<ZwlrVirtualPointerManagerV1>,
    vkeyboard: Option<ZwpVirtualKeyboardManagerV1>,
    frame: FrameState,
}

#[derive(Default)]
struct FrameState {
    format: Option<wl_shm::Format>,
    width: u32,
    height: u32,
    stride: u32,
    buffer_done: bool,
    ready: bool,
    failed: bool,
    flags_y_invert: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        st: &mut Self,
        reg: &wl_registry::WlRegistry,
        ev: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "wl_shm" => st.shm = Some(reg.bind(name, 1, qh, ())),
                "wl_seat" => st.seat = Some(reg.bind(name, 1, qh, ())),
                "wl_output" if st.output.is_none() => {
                    st.output = Some(reg.bind(name, version.min(2), qh, ()))
                }
                "zwlr_screencopy_manager_v1" => {
                    st.screencopy = Some(reg.bind(name, version.min(3), qh, ()))
                }
                "zwlr_virtual_pointer_manager_v1" => {
                    st.vpointer = Some(reg.bind(name, version.min(2), qh, ()))
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    st.vkeyboard = Some(reg.bind(name, 1, qh, ()))
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        st: &mut Self,
        _: &wl_output::WlOutput,
        ev: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match ev {
            wl_output::Event::Mode { flags: WEnum::Value(f), width, height, .. } if f.contains(wl_output::Mode::Current) => {
                st.output_size = Some((width, height));
            }
            // niri's nested (winit) output is Flipped180: the frame comes bottom-up and must be turned
            wl_output::Event::Geometry { transform: WEnum::Value(t), .. } => st.output_transform = Some(t),
            _ => {}
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        st: &mut Self,
        _: &ZwlrScreencopyFrameV1,
        ev: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_screencopy_frame_v1::Event as E;
        let f = &mut st.frame;
        match ev {
            E::Buffer { format: WEnum::Value(format), width, height, stride } => {
                // the first shm format offered is the one we take
                if f.format.is_none() {
                    f.format = Some(format);
                    f.width = width;
                    f.height = height;
                    f.stride = stride;
                }
            }
            E::BufferDone => f.buffer_done = true,
            E::Flags { flags: WEnum::Value(fl) } => {
                f.flags_y_invert = fl.contains(zwlr_screencopy_frame_v1::Flags::YInvert)
            }
            E::Ready { .. } => f.ready = true,
            E::Failed => f.failed = true,
            _ => {}
        }
    }
}

macro_rules! ignore {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore!(
    wl_shm::WlShm,
    wl_seat::WlSeat,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    ZwlrScreencopyManagerV1,
    ZwlrVirtualPointerManagerV1,
    ZwlrVirtualPointerV1,
    ZwpVirtualKeyboardManagerV1,
    ZwpVirtualKeyboardV1
);

fn now_ms(t0: Instant) -> u32 {
    t0.elapsed().as_millis() as u32
}

fn memfd(size: usize) -> OwnedFd {
    let fd = rustix::fs::memfd_create("wlr-remote", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("memfd_create");
    rustix::fs::ftruncate(&fd, size as u64).expect("ftruncate");
    fd
}

/// One screencopy round trip; returns (width, height, RGBA pixels).
fn capture(conn: &Connection, eq: &mut wayland_client::EventQueue<State>, st: &mut State) -> (u32, u32, Vec<u8>) {
    let qh = eq.handle();
    st.frame = FrameState::default();
    let mgr = st.screencopy.as_ref().expect("no zwlr_screencopy_manager_v1");
    let frame = mgr.capture_output(1, st.output.as_ref().expect("no output"), &qh, ());
    while !(st.frame.buffer_done || st.frame.failed) {
        eq.blocking_dispatch(st).unwrap();
    }
    assert!(!st.frame.failed, "screencopy failed before buffer");
    let (w, h, stride) = (st.frame.width, st.frame.height, st.frame.stride);
    let size = (stride * h) as usize;
    let fd = memfd(size);
    let pool = st.shm.as_ref().unwrap().create_pool(fd.as_fd(), size as i32, &qh, ());
    let buf = pool.create_buffer(0, w as i32, h as i32, stride as i32, st.frame.format.unwrap(), &qh, ());
    frame.copy(&buf);
    while !(st.frame.ready || st.frame.failed) {
        eq.blocking_dispatch(st).unwrap();
    }
    assert!(!st.frame.failed, "screencopy copy failed");
    if std::env::var_os("POC_DEBUG").is_some() { eprintln!("y_invert flag = {} transform = {:?}", st.frame.flags_y_invert, st.output_transform); }
    let map = unsafe {
        rustix::mm::mmap(std::ptr::null_mut(), size, rustix::mm::ProtFlags::READ, rustix::mm::MapFlags::SHARED, &fd, 0)
    }
    .expect("mmap");
    let src = unsafe { std::slice::from_raw_parts(map as *const u8, size) };
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let flipped = st.frame.flags_y_invert != matches!(st.output_transform, Some(wl_output::Transform::Flipped180));
        let row = if flipped { h - 1 - y } else { y };
        let line = &src[(row * stride) as usize..(row * stride + w * 4) as usize];
        for px in line.chunks_exact(4) {
            // wl_shm Xrgb8888/Argb8888 are little-endian B,G,R,X in memory; Xbgr/Abgr are R,G,B,X
            match st.frame.format.unwrap() {
                wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888 => rgba.extend_from_slice(&[px[0], px[1], px[2], 255]),
                _ => rgba.extend_from_slice(&[px[2], px[1], px[0], 255]),
            }
        }
    }
    unsafe { rustix::mm::munmap(map, size).ok() };
    buf.destroy();
    pool.destroy();
    frame.destroy();
    let _ = conn.flush();
    (w, h, rgba)
}

fn keycode(c: char) -> Option<(u32, bool)> {
    // evdev codes for the US layout; (code, needs shift)
    let row = |s: &str, base: u32| s.find(c).map(|i| base + i as u32);
    let code = row("qwertyuiop", 16)
        .or_else(|| row("asdfghjkl", 30))
        .or_else(|| row("zxcvbnm", 44))
        .or_else(|| row("1234567890", 2))
        .or(match c {
            ' ' => Some(57),
            '-' => Some(12),
            '.' => Some(52),
            '\n' => Some(28),
            _ => None,
        });
    code.map(|k| (k, false))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let conn = Connection::connect_to_env().expect("WAYLAND_DISPLAY");
    let mut eq = conn.new_event_queue();
    let qh = eq.handle();
    conn.display().get_registry(&qh, ());
    let mut st = State::default();
    eq.roundtrip(&mut st).unwrap();
    eq.roundtrip(&mut st).unwrap();
    println!(
        "globals: screencopy={} virtual_pointer={} virtual_keyboard={} output={:?}",
        st.screencopy.is_some(),
        st.vpointer.is_some(),
        st.vkeyboard.is_some(),
        st.output_size
    );
    let t0 = Instant::now();
    match args.get(1).map(String::as_str) {
        Some("capture") => {
            let (w, h, rgba) = capture(&conn, &mut eq, &mut st);
            let path = args.get(2).map(String::as_str).unwrap_or("captura.png");
            let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
            let mut enc = png::Encoder::new(file, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&rgba).unwrap();
            println!("captured {w}x{h} -> {path} in {} ms", t0.elapsed().as_millis());
        }
        Some("bench") => {
            let n: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
            let t = Instant::now();
            for _ in 0..n {
                capture(&conn, &mut eq, &mut st);
            }
            let dt = t.elapsed().as_secs_f64();
            println!("{n} frames in {dt:.2}s = {:.1} fps", n as f64 / dt);
        }
        Some(cmd @ ("move" | "click")) => {
            let (ow, oh) = st.output_size.expect("output size");
            let x: u32 = args[2].parse().unwrap();
            let y: u32 = args[3].parse().unwrap();
            let vp = st
                .vpointer
                .as_ref()
                .expect("no virtual pointer")
                .create_virtual_pointer(st.seat.as_ref(), &qh, ());
            vp.motion_absolute(now_ms(t0), x, y, ow as u32, oh as u32);
            vp.frame();
            if cmd == "click" {
                const BTN_LEFT: u32 = 0x110;
                use wayland_client::protocol::wl_pointer::ButtonState;
                vp.button(now_ms(t0), BTN_LEFT, ButtonState::Pressed);
                vp.frame();
                vp.button(now_ms(t0) + 1, BTN_LEFT, ButtonState::Released);
                vp.frame();
            }
            eq.roundtrip(&mut st).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            vp.destroy();
            eq.roundtrip(&mut st).unwrap();
            println!("{cmd} {x},{y} sent (output {ow}x{oh})");
        }
        Some("type") => {
            let text = args[2..].join(" ");
            let vk = st
                .vkeyboard
                .as_ref()
                .expect("no virtual keyboard")
                .create_virtual_keyboard(st.seat.as_ref().expect("no seat"), &qh, ());
            let ctx = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
            let km = xkbcommon::xkb::Keymap::new_from_names(&ctx, "evdev", "pc105", "us", "", None, 0)
                .expect("keymap");
            let s = km.get_as_string(xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1);
            let bytes = s.as_bytes();
            let fd = memfd(bytes.len() + 1);
            rustix::io::write(&fd, bytes).unwrap();
            rustix::io::write(&fd, &[0]).unwrap();
            vk.keymap(1 /* XKB_V1 */, fd.as_fd(), (bytes.len() + 1) as u32);
            // the focused client gets the new keymap first; keys sent before it is applied are lost
            eq.roundtrip(&mut st).unwrap();
            std::thread::sleep(Duration::from_millis(300));
            for c in text.chars() {
                let Some((code, _)) = keycode(c) else { continue };
                vk.key(now_ms(t0), code, 1);
                vk.key(now_ms(t0), code, 0);
                eq.roundtrip(&mut st).unwrap();
                std::thread::sleep(Duration::from_millis(15));
            }
            vk.destroy();
            eq.roundtrip(&mut st).unwrap();
            println!("typed {:?}", text);
        }
        _ => eprintln!("usage: capture FILE | bench N | move X Y | click X Y | type TEXT"),
    }
}
