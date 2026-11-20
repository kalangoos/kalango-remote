use std::os::fd::OwnedFd;

use wayland_client::{
    protocol::{wl_buffer, wl_output, wl_registry, wl_seat, wl_shm, wl_shm_pool},
    Connection, Dispatch, EventQueue, QueueHandle, WEnum,
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

/// One monitor, in the compositor's layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub name: String,
    /// position in the global (logical) layout
    pub x: i32,
    pub y: i32,
    /// size in physical pixels (what a capture gives)
    pub width: i32,
    pub height: i32,
    /// logical size (what pointer coordinates use)
    pub logical_width: i32,
    pub logical_height: i32,
    /// niri's nested (winit) output is Flipped180: frames come bottom-up
    pub flipped: bool,
}

#[derive(Default)]
pub(crate) struct OutputState {
    pub proxy: Option<wl_output::WlOutput>,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub logical: Option<(i32, i32, i32, i32)>,
    pub transform: Option<wl_output::Transform>,
}

#[derive(Default)]
pub(crate) struct Globals {
    pub shm: Option<wl_shm::WlShm>,
    pub seat: Option<wl_seat::WlSeat>,
    pub outputs: Vec<OutputState>,
    pub xdg_output: Option<ZxdgOutputManagerV1>,
    pub screencopy: Option<ZwlrScreencopyManagerV1>,
    pub vpointer: Option<ZwlrVirtualPointerManagerV1>,
    pub vkeyboard: Option<ZwpVirtualKeyboardManagerV1>,
}

impl Globals {
    pub fn public_outputs(&self) -> Vec<Output> {
        self.outputs
            .iter()
            .map(|o| {
                let (x, y, lw, lh) = o.logical.unwrap_or((o.x, o.y, o.width, o.height));
                Output {
                    name: o.name.clone(),
                    x,
                    y,
                    width: o.width,
                    height: o.height,
                    logical_width: lw,
                    logical_height: lh,
                    flipped: matches!(o.transform, Some(wl_output::Transform::Flipped180)),
                }
            })
            .collect()
    }
}

/// Anything that holds [`Globals`] (capture and input states do).
pub(crate) trait HasGlobals {
    fn globals(&mut self) -> &mut Globals;
}

/// Connects and binds what this crate uses; two roundtrips also deliver the outputs' modes and names.
pub(crate) fn connect<S>(state: &mut S) -> std::io::Result<(Connection, EventQueue<S>)>
where
    S: HasGlobals
        + Dispatch<wl_registry::WlRegistry, ()>
        + Dispatch<wl_output::WlOutput, usize>
        + Dispatch<zxdg_output_v1::ZxdgOutputV1, usize>
        + 'static,
{
    let conn = Connection::connect_to_env().map_err(std::io::Error::other)?;
    let mut eq = conn.new_event_queue();
    let qh = eq.handle();
    conn.display().get_registry(&qh, ());
    eq.roundtrip(state).map_err(std::io::Error::other)?;
    // logical positions and sizes come from xdg-output
    if let Some(m) = state.globals().xdg_output.clone() {
        let outs: Vec<_> = state.globals().outputs.iter().filter_map(|o| o.proxy.clone()).collect();
        for (i, o) in outs.iter().enumerate() {
            m.get_xdg_output(o, &qh, i);
        }
    }
    eq.roundtrip(state).map_err(std::io::Error::other)?;
    eq.roundtrip(state).map_err(std::io::Error::other)?;
    Ok((conn, eq))
}

pub(crate) fn registry_global<S: HasGlobals + 'static>(
    st: &mut S,
    reg: &wl_registry::WlRegistry,
    name: u32,
    interface: &str,
    version: u32,
    qh: &QueueHandle<S>,
) where
    S: Dispatch<wl_shm::WlShm, ()>
        + Dispatch<wl_seat::WlSeat, ()>
        + Dispatch<wl_output::WlOutput, usize>
        + Dispatch<ZxdgOutputManagerV1, ()>
        + Dispatch<ZwlrScreencopyManagerV1, ()>
        + Dispatch<ZwlrVirtualPointerManagerV1, ()>
        + Dispatch<ZwpVirtualKeyboardManagerV1, ()>,
{
    let g = st.globals();
    match interface {
        "wl_shm" => g.shm = Some(reg.bind(name, 1, qh, ())),
        "wl_seat" if g.seat.is_none() => g.seat = Some(reg.bind(name, 1, qh, ())),
        "wl_output" => {
            let idx = g.outputs.len();
            let proxy = reg.bind(name, version.min(4), qh, idx);
            g.outputs.push(OutputState { proxy: Some(proxy), ..Default::default() });
        }
        "zxdg_output_manager_v1" => g.xdg_output = Some(reg.bind(name, version.min(3), qh, ())),
        "zwlr_screencopy_manager_v1" => g.screencopy = Some(reg.bind(name, version.min(3), qh, ())),
        "zwlr_virtual_pointer_manager_v1" => g.vpointer = Some(reg.bind(name, version.min(2), qh, ())),
        "zwp_virtual_keyboard_manager_v1" => g.vkeyboard = Some(reg.bind(name, 1, qh, ())),
        _ => {}
    }
}

pub(crate) fn output_event(g: &mut Globals, idx: usize, ev: wl_output::Event) {
    let Some(o) = g.outputs.get_mut(idx) else { return };
    match ev {
        wl_output::Event::Mode { flags: WEnum::Value(f), width, height, .. } if f.contains(wl_output::Mode::Current) => {
            o.width = width;
            o.height = height;
        }
        wl_output::Event::Geometry { x, y, transform: WEnum::Value(t), .. } => {
            o.x = x;
            o.y = y;
            o.transform = Some(t);
        }
        wl_output::Event::Name { name } => o.name = name,
        _ => {}
    }
}

pub(crate) fn xdg_output_event(g: &mut Globals, idx: usize, ev: zxdg_output_v1::Event) {
    let Some(o) = g.outputs.get_mut(idx) else { return };
    let mut l = o.logical.unwrap_or((o.x, o.y, o.width, o.height));
    match ev {
        zxdg_output_v1::Event::LogicalPosition { x, y } => {
            l.0 = x;
            l.1 = y;
        }
        zxdg_output_v1::Event::LogicalSize { width, height } => {
            l.2 = width;
            l.3 = height;
        }
        zxdg_output_v1::Event::Name { name } if o.name.is_empty() => o.name = name,
        _ => return,
    }
    o.logical = Some(l);
}

pub(crate) fn memfd(size: usize) -> std::io::Result<OwnedFd> {
    let fd = rustix::fs::memfd_create("wlr-remote", rustix::fs::MemfdFlags::CLOEXEC)?;
    rustix::fs::ftruncate(&fd, size as u64)?;
    Ok(fd)
}


/// A probe connection: only the globals.
#[derive(Default)]
struct Probe(Globals);

impl HasGlobals for Probe {
    fn globals(&mut self) -> &mut Globals {
        &mut self.0
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for Probe {
    fn event(st: &mut Self, reg: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            registry_global(st, reg, name, &interface, version, qh);
        }
    }
}

impl Dispatch<wl_output::WlOutput, usize> for Probe {
    fn event(st: &mut Self, _: &wl_output::WlOutput, ev: wl_output::Event, idx: &usize, _: &Connection, _: &QueueHandle<Self>) {
        output_event(&mut st.0, *idx, ev);
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, usize> for Probe {
    fn event(st: &mut Self, _: &zxdg_output_v1::ZxdgOutputV1, ev: zxdg_output_v1::Event, idx: &usize, _: &Connection, _: &QueueHandle<Self>) {
        xdg_output_event(&mut st.0, *idx, ev);
    }
}

macro_rules! ignore_events {
    ($state:ty: $($t:ty),*) => {$(
        impl Dispatch<$t, ()> for $state {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
pub(crate) use ignore_events;

ignore_events!(Probe: wl_shm::WlShm, wl_seat::WlSeat, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer, ZxdgOutputManagerV1,
    ZwlrScreencopyManagerV1, ZwlrVirtualPointerManagerV1, ZwlrVirtualPointerV1, ZwpVirtualKeyboardManagerV1, ZwpVirtualKeyboardV1);

/// Whether the compositor in `WAYLAND_DISPLAY` offers all three protocols to this process.
pub fn available() -> bool {
    let mut p = Probe::default();
    match connect(&mut p) {
        Ok(_) => p.0.screencopy.is_some() && p.0.vpointer.is_some() && p.0.vkeyboard.is_some() && !p.0.outputs.is_empty(),
        Err(_) => false,
    }
}

/// The compositor's outputs, in registry order.
pub fn outputs() -> std::io::Result<Vec<Output>> {
    let mut p = Probe::default();
    connect(&mut p)?;
    Ok(p.0.public_outputs())
}
