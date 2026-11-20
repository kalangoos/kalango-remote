//! Frames of one output through wlr-screencopy. A thread asks for the next frame *with damage*
//! (the compositor answers only when something changed) and keeps the newest one; readers take it
//! when they want, waiting up to a timeout.

use std::{
    os::fd::AsFd,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::Duration,
};

use wayland_client::{
    protocol::{wl_buffer, wl_output, wl_registry, wl_seat, wl_shm, wl_shm_pool},
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1::ZxdgOutputManagerV1, zxdg_output_v1};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::{
    screencopy::v1::client::{
        zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
        zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
    },
    virtual_pointer::v1::client::{
        zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1, zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
    },
};

use crate::globals::{self, ignore_events, Globals, HasGlobals, Output};

/// A captured frame: BGRA rows (`stride` bytes apart), top row first.
#[derive(Clone)]
pub struct CapturedFrame {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub bgra: Vec<u8>,
    /// counts frames since the capture started
    pub seq: u64,
}

#[derive(Default)]
struct Shared {
    frame: Option<CapturedFrame>,
    seq: u64,
    error: Option<String>,
}

pub struct Capture {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    stop: Arc<AtomicBool>,
    pub output: Output,
}

impl Capture {
    /// Starts capturing the output at `index` (registry order), cursor included.
    pub fn start(index: usize) -> std::io::Result<Capture> {
        let outputs = globals::outputs()?;
        let output = outputs
            .get(index)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no such output"))?;
        let shared: Arc<(Mutex<Shared>, Condvar)> = Default::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (shared.clone(), stop.clone());
        std::thread::Builder::new().name("wlr-capture".into()).spawn(move || {
            if let Err(e) = run(index, &s, &st) {
                log::warn!("wlr capture: {e}");
                s.0.lock().unwrap().error = Some(e.to_string());
                s.1.notify_all();
            }
        })?;
        Ok(Capture { shared, stop, output })
    }

    /// The newest frame after `after` (a `seq`), waiting up to `timeout`; `Ok(None)` when nothing
    /// changed in that time.
    pub fn next(&self, after: u64, timeout: Duration) -> std::io::Result<Option<CapturedFrame>> {
        let (lock, cv) = &*self.shared;
        let guard = lock.lock().unwrap();
        let (mut g, _) = cv
            .wait_timeout_while(guard, timeout, |s| s.seq <= after && s.error.is_none())
            .unwrap();
        if let Some(e) = &g.error {
            return Err(std::io::Error::other(e.clone()));
        }
        if g.seq <= after {
            return Ok(None);
        }
        Ok(g.frame.take())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct FrameInfo {
    format: Option<wl_shm::Format>,
    width: u32,
    height: u32,
    stride: u32,
    buffer_done: bool,
    ready: bool,
    failed: bool,
    y_invert: bool,
}

#[derive(Default)]
struct State {
    g: Globals,
    frame: FrameInfo,
}

impl HasGlobals for State {
    fn globals(&mut self) -> &mut Globals {
        &mut self.g
    }
}

fn run(index: usize, shared: &Arc<(Mutex<Shared>, Condvar)>, stop: &AtomicBool) -> std::io::Result<()> {
    // KalangoOS's niri offers screencopy only after kalangod allowed this process (niri patch 0008), which the
    // app asks for right when the person approves: wait for it a little
    let started = std::time::Instant::now();
    let (mut st, conn, mut eq) = loop {
        let mut st = State::default();
        let (conn, eq) = globals::connect(&mut st)?;
        if st.g.screencopy.is_some() || started.elapsed() > Duration::from_secs(10) || stop.load(Ordering::Relaxed) {
            break (st, conn, eq);
        }
        std::thread::sleep(Duration::from_millis(300));
    };
    let qh = eq.handle();
    let mgr = st.g.screencopy.clone().ok_or_else(|| std::io::Error::other("no zwlr_screencopy_manager_v1 (not allowed by the compositor?)"))?;
    let out = st.g.outputs.get(index).and_then(|o| o.proxy.clone()).ok_or_else(|| std::io::Error::other("output gone"))?;
    let flipped = st.g.public_outputs().get(index).map(|o| o.flipped).unwrap_or(false);
    let shm = st.g.shm.clone().ok_or_else(|| std::io::Error::other("no wl_shm"))?;
    // the pool and buffer are reused while the size stays
    let mut pool: Option<(wl_shm_pool::WlShmPool, wl_buffer::WlBuffer, std::os::fd::OwnedFd, usize, (u32, u32, u32))> = None;
    let mut first = true;
    let mut seq = 0u64;
    while !stop.load(Ordering::Relaxed) {
        st.frame = FrameInfo::default();
        let frame = mgr.capture_output(1, &out, &qh, ());
        while !(st.frame.buffer_done || st.frame.failed) {
            eq.blocking_dispatch(&mut st).map_err(std::io::Error::other)?;
        }
        if st.frame.failed {
            return Err(std::io::Error::other("screencopy refused the frame"));
        }
        let (w, h, stride) = (st.frame.width, st.frame.height, st.frame.stride);
        let size = (stride * h) as usize;
        if pool.as_ref().map(|p| p.4) != Some((w, h, stride)) {
            if let Some((p, b, _, _, _)) = pool.take() {
                b.destroy();
                p.destroy();
            }
            let fd = globals::memfd(size)?;
            let p = shm.create_pool(fd.as_fd(), size as i32, &qh, ());
            let format = st.frame.format.ok_or_else(|| std::io::Error::other("no shm format offered"))?;
            let b = p.create_buffer(0, w as i32, h as i32, stride as i32, format, &qh, ());
            pool = Some((p, b, fd, size, (w, h, stride)));
        }
        let (_, buf, fd, _, _) = pool.as_ref().unwrap();
        // the first frame comes at once; the next ones only when the screen changes
        if first {
            frame.copy(buf);
            first = false;
        } else {
            frame.copy_with_damage(buf);
        }
        while !(st.frame.ready || st.frame.failed) {
            // wake up now and then to notice `stop` on an idle screen
            if !dispatch_with_timeout(&conn, &mut eq, &mut st, Duration::from_millis(500))? && stop.load(Ordering::Relaxed) {
                frame.destroy();
                return Ok(());
            }
        }
        if st.frame.failed {
            frame.destroy();
            first = true;
            continue;
        }
        let format = st.frame.format.unwrap_or(wl_shm::Format::Xrgb8888);
        let bgra = read_bgra(fd, size, w as usize, h as usize, stride as usize, format, st.frame.y_invert != flipped)?;
        frame.destroy();
        seq += 1;
        let (lock, cv) = &**shared;
        let mut g = lock.lock().unwrap();
        g.frame = Some(CapturedFrame { width: w as usize, height: h as usize, stride: w as usize * 4, bgra, seq });
        g.seq = seq;
        cv.notify_all();
    }
    Ok(())
}

/// One dispatch round waiting at most `timeout` for the socket; true when events came.
fn dispatch_with_timeout(conn: &Connection, eq: &mut wayland_client::EventQueue<State>, st: &mut State, timeout: Duration) -> std::io::Result<bool> {
    eq.dispatch_pending(st).map_err(std::io::Error::other)?;
    conn.flush().map_err(std::io::Error::other)?;
    let Some(guard) = conn.prepare_read() else {
        eq.dispatch_pending(st).map_err(std::io::Error::other)?;
        return Ok(true);
    };
    let fd = guard.connection_fd();
    let mut fds = [rustix::event::PollFd::new(&fd, rustix::event::PollFlags::IN)];
    let ts = rustix::event::Timespec { tv_sec: timeout.as_secs() as _, tv_nsec: timeout.subsec_nanos() as _ };
    let n = rustix::event::poll(&mut fds, Some(&ts)).map_err(std::io::Error::from)?;
    if n == 0 {
        drop(guard);
        return Ok(false);
    }
    guard.read().map_err(std::io::Error::other)?;
    eq.dispatch_pending(st).map_err(std::io::Error::other)?;
    Ok(true)
}

/// Copies the shm buffer into tightly packed BGRA, top row first.
fn read_bgra(fd: &std::os::fd::OwnedFd, size: usize, w: usize, h: usize, stride: usize, format: wl_shm::Format, flip: bool) -> std::io::Result<Vec<u8>> {
    let map = unsafe { rustix::mm::mmap(std::ptr::null_mut(), size, rustix::mm::ProtFlags::READ, rustix::mm::MapFlags::SHARED, fd, 0) }?;
    let src = unsafe { std::slice::from_raw_parts(map as *const u8, size) };
    let swap_rb = matches!(format, wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888);
    let out = to_bgra(src, w, h, stride, swap_rb, flip);
    unsafe { rustix::mm::munmap(map, size).ok() };
    Ok(out)
}

/// wl_shm Xrgb/Argb8888 are B,G,R,X in memory (little endian); Xbgr/Abgr are R,G,B,X.
pub(crate) fn to_bgra(src: &[u8], w: usize, h: usize, stride: usize, swap_rb: bool, flip: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = if flip { h - 1 - y } else { y };
        let line = &src[row * stride..row * stride + w * 4];
        if swap_rb {
            for px in line.chunks_exact(4) {
                out.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        } else {
            out.extend_from_slice(line);
        }
    }
    out
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

impl Dispatch<ZwlrScreencopyFrameV1, ()> for State {
    fn event(st: &mut Self, _: &ZwlrScreencopyFrameV1, ev: zwlr_screencopy_frame_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use zwlr_screencopy_frame_v1::Event as E;
        let f = &mut st.frame;
        match ev {
            E::Buffer { format: WEnum::Value(format), width, height, stride } if f.format.is_none() => {
                f.format = Some(format);
                f.width = width;
                f.height = height;
                f.stride = stride;
            }
            E::BufferDone => f.buffer_done = true,
            E::Flags { flags: WEnum::Value(fl) } => f.y_invert = fl.contains(zwlr_screencopy_frame_v1::Flags::YInvert),
            E::Ready { .. } => f.ready = true,
            E::Failed => f.failed = true,
            _ => {}
        }
    }
}

ignore_events!(State: wl_shm::WlShm, wl_seat::WlSeat, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer, ZxdgOutputManagerV1,
    ZwlrScreencopyManagerV1, ZwlrVirtualPointerManagerV1, ZwlrVirtualPointerV1, ZwpVirtualKeyboardManagerV1, ZwpVirtualKeyboardV1);

#[cfg(test)]
mod tests {
    use super::to_bgra;

    #[test]
    fn converts_and_flips_rows() {
        // 2×2, stride 12 (4 bytes of padding per row); R,G,B,X order (Xbgr8888)
        let src = [
            1, 2, 3, 0, 4, 5, 6, 0, 9, 9, 9, 9, //
            7, 8, 9, 0, 10, 11, 12, 0, 9, 9, 9, 9,
        ];
        assert_eq!(to_bgra(&src, 2, 2, 12, true, false), vec![3, 2, 1, 255, 6, 5, 4, 255, 9, 8, 7, 255, 12, 11, 10, 255]);
        assert_eq!(&to_bgra(&src, 2, 2, 12, false, true)[..8], &[7, 8, 9, 0, 10, 11, 12, 0]);
    }
}
