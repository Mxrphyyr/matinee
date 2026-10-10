//! Matinee: a private, ad-free GTK client for following YouTube channels.

mod card;
mod feed;
mod history;
mod icons;
mod model;
mod mpv;
mod net;
mod perf;
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
mod ytdlp;

use adw::prelude::*;
use gtk::{gio, glib};

pub const APP_ID: &str = "io.github.matinee.Matinee";

/// Where a crash's details are written, for the note Matinee shows next time.
pub fn crash_log() -> std::path::PathBuf {
    glib::user_cache_dir().join("matinee").join("crash.log")
}

/// Writes what went wrong, and where in the code, before the program stops.
fn install_crash_log() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let path = crash_log();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text = format!(
            "Matinee {} stopped unexpectedly at {} (Unix seconds)\n\n{info}\n\n{}",
            env!("CARGO_PKG_VERSION"),
            util::now(),
            std::backtrace::Backtrace::force_capture()
        );
        let _ = std::fs::write(&path, text);
        previous(info);
    }));
}

fn main() -> glib::ExitCode {
    perf::init();
    install_crash_log();
    // Video is drawn with OpenGL. GTK's OpenGL renderer composites it directly;
    // other renderers copy every frame between graphics APIs, which costs the
    // most at full screen. A GSK_RENDERER set by the user still wins. GTK 4.18
    // renamed that renderer from "ngl" to "gl" (before, "gl" was an older one).
    if std::env::var_os("GSK_RENDERER").is_none() {
        std::env::set_var("GSK_RENDERER", if gtk::minor_version() >= 18 { "gl" } else { "ngl" });
    }
    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|_| {
        perf::mark("GTK ready");
        gtk::Window::set_default_icon_name(APP_ID);
        style::install();
        icons::install();
        perf::mark("styles and icons installed");
    });
    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        let win = window::Win::new(app);
        perf::mark("window built");
        win.window.present();
        win.start();
    });

    let quit = gio::ActionEntry::builder("quit")
        // Closing the main window (rather than quitting outright) notes where
        // you were in a video and the window's size; the app ends with it.
        .activate(|app: &adw::Application, _, _| {
            match app.windows().into_iter().find(|window| window.is::<adw::ApplicationWindow>()) {
                Some(main) => main.close(),
                None => app.quit(),
            }
        })
        .build();
    app.add_action_entries([quit]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("window.close", &["<Control>w"]);
    app.set_accels_for_action("win.refresh", &["<Control>r", "F5"]);
    app.set_accels_for_action("win.focus-search", &["<Control>f"]);
    app.set_accels_for_action("win.add-channel", &["<Control>n"]);
    app.set_accels_for_action("win.preferences", &["<Control>comma"]);
    app.set_accels_for_action("win.toggle-sidebar", &["F9"]);

    let code = app.run();
    // A second launch only brings the running Matinee forward and returns
    // here at once; say so, or a terminal run looks like it did nothing.
    if app.is_remote() {
        eprintln!("Matinee is already running; its window was brought forward. Quit it (Ctrl+Q) to start afresh.");
    }
    code
}
