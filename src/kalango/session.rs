//! A controlling session whose UI is a channel: every `InvokeUiSession` call becomes an
//! [`Event`] (serializable, the app shows it), and decoded frames go to a one-slot mailbox the app
//! drains at its own pace (a slow window drops frames instead of piling them up).

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc::Sender,
    Arc, Condvar, Mutex, RwLock,
};

use base::message_proto::*;
use hbb_common::{log, rendezvous_proto::ConnType};
use serde_derive::Serialize;

use crate::{
    client::QualityStatus,
    ui_session_interface::{io_loop, InvokeUiSession, Session},
};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Connected { conn_type: String },
    ConnectionType { secured: bool, direct: bool, stream: String },
    PeerInfo { username: String, hostname: String, platform: String, version: String, displays: Vec<Display>, current_display: i32 },
    Display { x: i32, y: i32, width: i32, height: i32, cursor_embedded: bool },
    CursorPosition { x: i32, y: i32 },
    Quality { speed: Option<String>, fps: Option<i32>, delay: Option<i32>, codec: Option<String> },
    Permission { name: String, value: bool },
    Message { msgtype: String, title: String, text: String, retry: bool },
    CancelMessage { tag: String },
    Chat { text: String },
    Closed,
    FolderFiles { id: i32, path: String, is_local: bool, entries: Vec<FileItem> },
    JobProgress { id: i32, file_num: i32, speed: f64, finished_size: f64 },
    JobDone { id: i32, file_num: i32 },
    JobError { id: i32, file_num: i32, err: String },
    OverrideFile { id: i32, file_num: i32, to: String, is_upload: bool, is_identical: bool },
    Recording { on: bool },
    BlockInput { on: bool },
}

#[derive(Debug, Clone, Serialize)]
pub struct Display {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileItem {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
}

/// A decoded frame in libyuv "ARGB" order (bytes B, G, R, A), rows `stride` bytes apart.
pub struct Frame {
    pub display: usize,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub bgra: Vec<u8>,
}

#[derive(Default)]
pub struct FrameSlot {
    frame: Mutex<Option<Frame>>,
    ready: Condvar,
    pub dropped: AtomicUsize,
}

impl FrameSlot {
    fn put(&self, f: Frame) {
        let mut slot = self.frame.lock().unwrap();
        if slot.replace(f).is_some() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        self.ready.notify_one();
    }

    /// Waits up to `timeout` for the newest frame.
    pub fn take(&self, timeout: std::time::Duration) -> Option<Frame> {
        let slot = self.frame.lock().unwrap();
        let (mut slot, _) = self
            .ready
            .wait_timeout_while(slot, timeout, |f| f.is_none())
            .unwrap();
        slot.take()
    }
}

#[derive(Clone, Default)]
pub struct Handler {
    events: Arc<Mutex<Option<Sender<Event>>>>,
    pub frames: Arc<FrameSlot>,
    displays: Arc<RwLock<Vec<Display>>>,
}

impl Handler {
    fn emit(&self, e: Event) {
        if let Some(tx) = self.events.lock().unwrap().as_ref() {
            tx.send(e).ok();
        }
    }
}

pub type KalangoSession = Session<Handler>;

pub enum Kind {
    Control,
    ViewOnly,
    Files,
}

/// Starts a session to `peer` (a direct address such as `127.0.0.1:PORT`, or a rendezvous id when
/// a self-hosted server is configured). Events go to `events`; frames to `session.ui_handler.frames`.
pub fn start(peer: &str, password: &str, kind: Kind, events: Sender<Event>) -> KalangoSession {
    let handler = Handler::default();
    *handler.events.lock().unwrap() = Some(events);
    let session: KalangoSession = Session {
        password: password.to_owned(),
        ui_handler: handler,
        server_keyboard_enabled: Arc::new(RwLock::new(true)),
        server_file_transfer_enabled: Arc::new(RwLock::new(true)),
        server_clipboard_enabled: Arc::new(RwLock::new(true)),
        reconnect_count: Arc::new(AtomicUsize::new(0)),
        ..Default::default()
    };
    let conn_type = match kind {
        Kind::Files => ConnType::FILE_TRANSFER,
        _ => ConnType::DEFAULT_CONN,
    };
    session
        .lc
        .write()
        .unwrap()
        .initialize(peer.to_owned(), conn_type, None, false, None, None, None, "");
    if matches!(kind, Kind::ViewOnly) && !session.lc.read().unwrap().get_toggle_option("view-only") {
        // persisted in the peer's config, like the toggle in RustDesk's own toolbar
        session.lc.write().unwrap().toggle_option("view-only".to_owned());
    }
    let cloned = session.clone();
    *session.thread.lock().unwrap() = Some(std::thread::spawn(move || {
        let round = cloned.connection_round_state.lock().unwrap().new_round();
        io_loop(cloned, round);
    }));
    session
}

impl InvokeUiSession for Handler {
    fn set_cursor_data(&self, _cd: CursorData) {}
    fn set_cursor_id(&self, _id: String) {}
    fn set_cursor_position(&self, cp: CursorPosition) {
        self.emit(Event::CursorPosition { x: cp.x, y: cp.y });
    }
    fn set_display(&self, x: i32, y: i32, w: i32, h: i32, cursor_embedded: bool, _scale: f64) {
        self.emit(Event::Display { x, y, width: w, height: h, cursor_embedded });
    }
    fn switch_display(&self, _display: &SwitchDisplay) {}
    fn set_peer_info(&self, pi: &PeerInfo) {
        let displays: Vec<Display> = pi
            .displays
            .iter()
            .map(|d| Display { x: d.x, y: d.y, width: d.width, height: d.height, name: d.name.clone() })
            .collect();
        *self.displays.write().unwrap() = displays.clone();
        self.emit(Event::PeerInfo {
            username: pi.username.clone(),
            hostname: pi.hostname.clone(),
            platform: pi.platform.clone(),
            version: pi.version.clone(),
            displays,
            current_display: pi.current_display,
        });
    }
    fn set_displays(&self, _displays: &Vec<DisplayInfo>) {}
    fn set_platform_additions(&self, _data: &str) {}
    fn on_connected(&self, conn_type: ConnType) {
        self.emit(Event::Connected { conn_type: format!("{conn_type:?}") });
    }
    fn update_privacy_mode(&self) {}
    fn set_permission(&self, name: &str, value: bool) {
        self.emit(Event::Permission { name: name.to_owned(), value });
    }
    fn close_success(&self) {
        self.emit(Event::Closed);
    }
    fn update_quality_status(&self, qs: QualityStatus) {
        self.emit(Event::Quality {
            speed: qs.speed,
            fps: qs.fps.values().next().copied(),
            delay: qs.delay,
            codec: qs.codec_format.map(|c| c.to_string()),
        });
    }
    fn set_connection_type(&self, is_secured: bool, direct: bool, stream_type: &str) {
        self.emit(Event::ConnectionType { secured: is_secured, direct, stream: stream_type.to_owned() });
    }
    fn set_fingerprint(&self, _fingerprint: String) {}
    fn job_error(&self, id: i32, err: String, file_num: i32) {
        self.emit(Event::JobError { id, file_num, err });
    }
    fn job_done(&self, id: i32, file_num: i32) {
        self.emit(Event::JobDone { id, file_num });
    }
    fn clear_all_jobs(&self) {}
    fn new_message(&self, msg: String) {
        self.emit(Event::Chat { text: msg });
    }
    fn update_transfer_list(&self) {}
    fn load_last_job(&self, _cnt: i32, _job_json: &str, _auto_start: bool) {}
    fn update_folder_files(&self, id: i32, entries: &Vec<FileEntry>, path: String, is_local: bool, only_count: bool) {
        if only_count {
            return;
        }
        let entries = entries
            .iter()
            .map(|e| FileItem {
                name: e.name.clone(),
                is_dir: matches!(e.entry_type.enum_value(), Ok(FileType::Dir) | Ok(FileType::DirLink)),
                size: e.size,
                modified: e.modified_time,
            })
            .collect();
        self.emit(Event::FolderFiles { id, path, is_local, entries });
    }
    fn confirm_delete_files(&self, _id: i32, _i: i32, _name: String) {}
    fn override_file_confirm(&self, id: i32, file_num: i32, to: String, is_upload: bool, is_identical: bool) {
        self.emit(Event::OverrideFile { id, file_num, to, is_upload, is_identical });
    }
    fn update_block_input_state(&self, on: bool) {
        self.emit(Event::BlockInput { on });
    }
    fn job_progress(&self, id: i32, file_num: i32, speed: f64, finished_size: f64) {
        self.emit(Event::JobProgress { id, file_num, speed, finished_size });
    }
    fn adapt_size(&self) {}
    fn on_rgba(&self, display: usize, rgba: &mut scrap::ImageRgb) {
        if rgba.h == 0 || rgba.raw.is_empty() {
            return;
        }
        let stride = rgba.raw.len() / rgba.h;
        self.frames.put(Frame {
            display,
            width: rgba.w,
            height: rgba.h,
            stride,
            bgra: std::mem::take(&mut rgba.raw),
        });
    }
    fn msgbox(&self, msgtype: &str, title: &str, text: &str, _link: &str, retry: bool) {
        log::info!("kalango session msgbox {msgtype} {title}: {text}");
        self.emit(Event::Message { msgtype: msgtype.to_owned(), title: title.to_owned(), text: text.to_owned(), retry });
    }
    fn cancel_msgbox(&self, tag: &str) {
        self.emit(Event::CancelMessage { tag: tag.to_owned() });
    }
    fn switch_back(&self, _id: &str) {}
    fn portable_service_running(&self, _running: bool) {}
    fn on_voice_call_started(&self) {}
    fn on_voice_call_closed(&self, _reason: &str) {}
    fn on_voice_call_waiting(&self) {}
    fn on_voice_call_incoming(&self) {}
    fn get_rgba(&self, _display: usize) -> *const u8 {
        std::ptr::null()
    }
    fn next_rgba(&self, _display: usize) {}
    fn set_multiple_windows_session(&self, _sessions: Vec<WindowsSession>) {}
    fn set_current_display(&self, _disp_idx: i32) {}
    fn update_record_status(&self, start: bool) {
        self.emit(Event::Recording { on: start });
    }
    fn printer_request(&self, _id: i32, _path: String) {}
    fn handle_screenshot_resp(&self, _sid: String, _msg: String) {}
    fn handle_terminal_response(&self, _response: TerminalResponse) {}
}

/// File transfer: list a folder of the other computer (answered by `Event::FolderFiles`).
pub fn read_remote_dir(s: &KalangoSession, path: &str) {
    use crate::client::file_trait::FileManager;
    s.read_remote_dir(path.to_owned(), false);
}

/// File transfer: copy `path` to `to`; `from_remote` means `path` is on the other computer
/// (receive), otherwise it is here (send). Returns the job id that progress events carry.
pub fn transfer(s: &KalangoSession, path: &str, to: &str, from_remote: bool) -> i32 {
    use crate::client::file_trait::FileManager;
    let id = s.get_next_job_id();
    s.update_next_job_id(id + 1);
    s.send_files(id, 0, path.to_owned(), to.to_owned(), 0, false, from_remote);
    id
}

pub fn cancel_transfer(s: &KalangoSession, id: i32) {
    use crate::client::file_trait::FileManager;
    s.cancel_job(id);
}
