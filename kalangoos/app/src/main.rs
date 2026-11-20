//! Kalango Remote · KalangoOS's remote-access app (GTK 4). RustDesk's core runs in
//! `kalango-remote-core` (see kalangoos/core and docs/POC.md for why it is a separate process).
//!
//!   kalango-remote                      the window
//!   kalango-remote --page ssh|settings  opening on a page (also for tests and captures)

mod core;
mod home;
mod i18n;
mod incoming;
mod secrets;
mod session;
mod settings;
mod store;
mod ui;
mod window;

use gtk::{gio, glib, prelude::*};

const APP_ID: &str = "os.kalango.Remote";

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id(std::env::var("KALANGO_REMOTE_APP_ID").unwrap_or_else(|_| APP_ID.into()))
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(|_| ui::load_css());
    app.connect_command_line(|app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().skip(1).filter_map(|a| a.to_str().map(str::to_owned)).collect();
        let page = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)).cloned();
        let w = window::Window::get_or_create(app);
        if let Some(p) = page {
            w.show_page(&p);
        }
        w.present();
        0
    });
    app.run()
}
