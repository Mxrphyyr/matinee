//! Matinee: a private, ad-free GTK client for following YouTube channels.

mod card;
mod feed;
mod history;
mod icons;
mod model;
mod mpv;
mod net;
mod player;
mod resolve;
mod search;
mod smooth;
mod store;
mod style;
mod thumb;
mod thumbs;
mod tools;
mod trending;
mod util;
mod watch;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};

pub const APP_ID: &str = "io.github.matinee.Matinee";

fn main() -> glib::ExitCode {
    // Video is drawn with OpenGL. GTK's OpenGL renderer composites it directly;
    // other renderers copy every frame between graphics APIs, which costs the
    // most at full screen. A GSK_RENDERER set by the user still wins.
    if std::env::var_os("GSK_RENDERER").is_none() {
        std::env::set_var("GSK_RENDERER", "ngl");
    }
    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name(APP_ID);
        style::install();
        icons::install();
    });
    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        let win = window::Win::new(app);
        win.window.present();
        win.start();
    });

    let quit = gio::ActionEntry::builder("quit")
        .activate(|app: &adw::Application, _, _| app.quit())
        .build();
    app.add_action_entries([quit]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("window.close", &["<Control>w"]);
    app.set_accels_for_action("win.refresh", &["<Control>r", "F5"]);
    app.set_accels_for_action("win.focus-search", &["<Control>f"]);
    app.set_accels_for_action("win.add-channel", &["<Control>n"]);
    app.set_accels_for_action("win.preferences", &["<Control>comma"]);
    app.set_accels_for_action("win.toggle-sidebar", &["F9"]);

    app.run()
}
