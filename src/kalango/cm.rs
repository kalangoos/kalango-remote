//! The connection manager (the "_cm" IPC endpoint RustDesk's server talks to) with a channel for
//! a UI: incoming requests, who is connected, chat. Answers go back through the `ui_cm_interface`
//! functions re-exported here.

use std::sync::{mpsc::Sender, Arc, Mutex};

use serde_derive::Serialize;

use crate::ui_cm_interface::{self, Client, ConnectionManager, InvokeUiCM};

pub use crate::ui_cm_interface::{authorize, close, send_chat, switch_permission};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CmEvent {
    Connection {
        id: i32,
        name: String,
        peer_id: String,
        authorized: bool,
        file_transfer: bool,
        port_forward: String,
        keyboard: bool,
        clipboard: bool,
        audio: bool,
        file: bool,
    },
    Removed { id: i32, close: bool },
    Chat { id: i32, text: String },
    FileLog { action: String, log: String },
}

#[derive(Clone)]
struct Handler(Arc<Mutex<Sender<CmEvent>>>);

impl Handler {
    fn emit(&self, e: CmEvent) {
        self.0.lock().unwrap().send(e).ok();
    }
}

impl InvokeUiCM for Handler {
    fn add_connection(&self, c: &Client) {
        self.emit(CmEvent::Connection {
            id: c.id,
            name: c.name.clone(),
            peer_id: c.peer_id.clone(),
            authorized: c.authorized,
            file_transfer: c.is_file_transfer,
            port_forward: c.port_forward.clone(),
            keyboard: c.keyboard,
            clipboard: c.clipboard,
            audio: c.audio,
            file: c.file,
        });
    }
    fn remove_connection(&self, id: i32, close: bool) {
        self.emit(CmEvent::Removed { id, close });
    }
    fn new_message(&self, id: i32, text: String) {
        self.emit(CmEvent::Chat { id, text });
    }
    fn change_theme(&self, _dark: String) {}
    fn change_language(&self) {}
    fn show_elevation(&self, _show: bool) {}
    fn update_voice_call_state(&self, _client: &Client) {}
    fn file_transfer_log(&self, action: &str, log: &str) {
        self.emit(CmEvent::FileLog { action: action.to_owned(), log: log.to_owned() });
    }
}

/// Listens as the connection manager on its own thread (the server process connects to it).
pub fn start(events: Sender<CmEvent>) {
    let cm = ConnectionManager { ui_handler: Handler(Arc::new(Mutex::new(events))) };
    std::thread::spawn(move || ui_cm_interface::start_ipc(cm));
}
