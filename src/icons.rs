//! Icons come from the desktop's icon theme: Breeze on Plasma, Adwaita on GNOME.
//! Any icon the theme lacks falls back to a small built-in symbolic version
//! (original drawings, recolored by GTK), so nothing shows as a broken image.

use gtk::{gdk, glib};
use std::cell::RefCell;
use std::collections::HashMap;

const BUILTIN: &[(&str, &str)] = &[
    (
        "view-refresh-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path d="M11.10 2.63 A6.2 6.2 0 1 1 4.90 2.63 L4.25 1.50 L8.37 2.76 L6.50 5.40 L5.85 4.28 A4.3 4.3 0 1 0 10.15 4.28 Z"/></g></svg>"##,
    ),
    (
        "list-add-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path d="M7 2h2v5h5v2H9v5H7V9H2V7h5z"/></g></svg>"##,
    ),
    (
        "open-menu-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path d="M2 3h12v2H2zm0 4h12v2H2zm0 4h12v2H2z"/></g></svg>"##,
    ),
    (
        "go-top-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path d="M2 1.5h12v2H2z"/><path d="M8 4.6l5.2 5.2-1.4 1.4L9 8.4V15H7V8.4l-2.8 2.8-1.4-1.4z"/></g></svg>"##,
    ),
    (
        "view-grid-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><rect x="2" y="2" width="5" height="5" rx="1.2"/><rect x="9" y="2" width="5" height="5" rx="1.2"/><rect x="2" y="9" width="5" height="5" rx="1.2"/><rect x="9" y="9" width="5" height="5" rx="1.2"/></g></svg>"##,
    ),
    (
        "media-playback-start-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path d="M4.5 2.6v10.8a.6.6 0 0 0 .9.5l8.6-5.4a.6.6 0 0 0 0-1L5.4 2.1a.6.6 0 0 0-.9.5z"/></g></svg>"##,
    ),
    (
        "system-search-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path fill-rule="evenodd" d="M1.2999999999999998 6.5 a5.2 5.2 0 1 0 10.4 0 a5.2 5.2 0 1 0 -10.4 0 Z M3.1 6.5 a3.4 3.4 0 1 1 6.8 0 a3.4 3.4 0 1 1 -6.8 0 Z"/><path d="M10.42 8.80 L14.10 12.48 L12.48 14.10 L8.80 10.42 Z"/></g></svg>"##,
    ),
    (
        "video-x-generic-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path fill-rule="evenodd" d="M3.5 1h9A1.5 1.5 0 0 1 14 2.5v11a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 13.5v-11A1.5 1.5 0 0 1 3.5 1zM5.4 3h5.2v10H5.4zM3 2.4h1.3v1.6H3zM3 5.5h1.3v1.6H3zM3 8.6h1.3v1.6H3zM3 11.7h1.3v1.6H3zM11.7 2.4H13v1.6h-1.3zM11.7 5.5H13v1.6h-1.3zM11.7 8.6H13v1.6h-1.3zM11.7 11.7H13v1.6h-1.3z"/></g></svg>"##,
    ),
    (
        "network-offline-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path fill-rule="evenodd" d="M1.2000000000000002 8 a6.8 6.8 0 1 0 13.6 0 a6.8 6.8 0 1 0 -13.6 0 Z M2.8 8 a5.2 5.2 0 1 1 10.4 0 a5.2 5.2 0 1 1 -10.4 0 Z"/><path d="M3.47 11.39 L11.39 3.47 L12.53 4.61 L4.61 12.53 Z"/></g></svg>"##,
    ),
    (
        "go-previous-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path d="M10.6 2.3 12 3.7 7.7 8l4.3 4.3-1.4 1.4L4.9 8z"/></g></svg>"##,
    ),
    (
        "media-playback-pause-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><rect x="3.5" y="2.5" width="3" height="11" rx=".8"/><rect x="9.5" y="2.5" width="3" height="11" rx=".8"/></g></svg>"##,
    ),
    (
        "view-fullscreen-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path d="M2 2h4.5v1.8H3.8v2.7H2zm7.5 0H14v4.5h-1.8V3.8H9.5zM2 9.5h1.8v2.7h2.7V14H2zm10.2 0H14V14H9.5v-1.8h2.7z"/></g></svg>"##,
    ),
    (
        "view-restore-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path d="M4.7 2h1.8v4.5H2V4.7h2.7zm4.8 0h1.8v2.7H14v1.8H9.5zM2 9.5h4.5V14H4.7v-2.7H2zm7.5 0H14v1.8h-2.7V14H9.5z"/></g></svg>"##,
    ),
    (
        "audio-volume-high-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path d="M1.5 5.8h2.6L7.6 3v10l-3.5-2.8H1.5z"/><path d="M10.48 5.47 A3.4 3.4 0 0 1 10.48 10.53 L9.54 9.49 A2.0 2.0 0 0 0 9.54 6.51 Z"/><path d="M11.83 3.35 A5.9 5.9 0 0 1 11.83 12.65 L10.97 11.55 A4.5 4.5 0 0 0 10.97 4.45 Z"/></g></svg>"##,
    ),
    (
        "audio-volume-muted-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path d="M1.5 5.8h2.6L7.6 3v10l-3.5-2.8H1.5z"/><path d="M10.61 5.35 L14.85 9.59 L13.79 10.65 L9.55 6.41 ZM9.55 9.59 L13.79 5.35 L14.85 6.41 L10.61 10.65 Z"/></g></svg>"##,
    ),
    (
        "media-playlist-repeat-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path d="M11.10 2.63 A6.2 6.2 0 1 1 4.90 2.63 L4.25 1.50 L8.37 2.76 L6.50 5.40 L5.85 4.28 A4.3 4.3 0 1 0 10.15 4.28 Z"/></g></svg>"##,
    ),
    (
        "dialog-warning-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path fill-rule="evenodd" d="M8 1.2c.4 0 .8.2 1 .6l6.3 11.3c.4.8-.1 1.7-1 1.7H1.7c-.9 0-1.4-.9-1-1.7L7 1.8c.2-.4.6-.6 1-.6zM7.1 5.2h1.8v4.9H7.1zM8 11.2a1 1 0 1 1 0 2 1 1 0 0 1 0-2z"/></g></svg>"##,
    ),
    (
        "picture-in-picture-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path fill-rule="evenodd" d="M2.5 2h11A1.5 1.5 0 0 1 15 3.5v9a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 1 12.5v-9A1.5 1.5 0 0 1 2.5 2zm0 1.5v9h11v-9z"/><rect x="8" y="7.5" width="5" height="4" rx=".6"/></g></svg>"##,
    ),
    (
        "sidebar-show-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path fill-rule="evenodd" d="M2.5 2h11A1.5 1.5 0 0 1 15 3.5v9a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 1 12.5v-9A1.5 1.5 0 0 1 2.5 2zm4 1.5v9h7v-9z"/></g></svg>"##,
    ),
    (
        "media-view-subtitles-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path fill-rule="evenodd" d="M2.5 3h11A1.5 1.5 0 0 1 15 4.5v7a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 1 11.5v-7A1.5 1.5 0 0 1 2.5 3zm0 1.5v7h11v-7z"/><path d="M3.5 6.2h5v1.4h-5zM9.7 6.2h2.8v1.4H9.7zM3.5 8.6h2.8V10H3.5zM7.5 8.6h5V10h-5z"/></g></svg>"##,
    ),
    (
        "discover-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path fill-rule="evenodd" d="M8 1.2a6.8 6.8 0 1 1 0 13.6A6.8 6.8 0 0 1 8 1.2zm0 1.6a5.2 5.2 0 1 0 0 10.4A5.2 5.2 0 0 0 8 2.8z"/><path d="M11.2 4.8 9.3 9.3 4.8 11.2l1.9-4.5zM8 7.1a.9.9 0 1 0 0 1.8.9.9 0 0 0 0-1.8z"/></g></svg>"##,
    ),
    (
        "bookmark-new-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path fill-rule="evenodd" d="M4 1.5h8A1.5 1.5 0 0 1 13.5 3v11.4a.6.6 0 0 1-.95.49L8 11.6l-4.55 3.29A.6.6 0 0 1 2.5 14.4V3A1.5 1.5 0 0 1 4 1.5zm0 1.5v10.2l4-2.9 4 2.9V3z"/></g></svg>"##,
    ),
    (
        "folder-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path fill-rule="evenodd" d="M2.5 2.5h4l1.5 1.5h5.5A1.5 1.5 0 0 1 15 5.5v7a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 1 12.5V4a1.5 1.5 0 0 1 1.5-1.5zm0 1.5v8.5h11v-7H7.4L5.9 4z"/></g></svg>"##,
    ),
    (
        "edit-delete-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><path d="M6 1.5h4a1 1 0 0 1 1 1V3h3v1.5H2V3h3v-.5a1 1 0 0 1 1-1zM3 5.5h10l-.7 8.1a1.5 1.5 0 0 1-1.5 1.4H5.2a1.5 1.5 0 0 1-1.5-1.4z"/></g></svg>"##,
    ),
    (
        "trending-symbolic",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><g fill="#2e3436"><rect x="1.5" y="9" width="3.4" height="5.5" rx=".9"/><rect x="6.3" y="5.5" width="3.4" height="9" rx=".9"/><rect x="11.1" y="1.5" width="3.4" height="13" rx=".9"/></g></svg>"##,
    ),
];

/// Writes the built-in icons to the cache and adds them to the icon search path.
/// They go inside a `hicolor` theme folder: GTK only recolors symbolic icons
/// that belong to a theme, and every theme falls back to hicolor.
pub fn install() {
    let Some(display) = gdk::Display::default() else { return };
    let dir = glib::user_cache_dir().join("matinee").join("icons");
    let themed = dir.join("hicolor").join("scalable").join("actions");
    if std::fs::create_dir_all(&themed).is_err() {
        return;
    }
    for (name, svg) in BUILTIN {
        // The loose copy is a last resort for systems without a hicolor index.
        for folder in [&themed, &dir] {
            let path = folder.join(format!("matinee-{name}.svg"));
            if std::fs::read_to_string(&path).map_or(true, |current| current != *svg) {
                let _ = std::fs::write(&path, svg);
            }
        }
    }
    gtk::IconTheme::for_display(&display).add_search_path(&dir);
}

/// The theme's icon when it has one, otherwise Matinee's built-in version.
pub fn get(name: &str) -> String {
    thread_local! {
        static CHOSEN: RefCell<HashMap<String, String>> = RefCell::default();
    }
    CHOSEN.with(|chosen| {
        if let Some(found) = chosen.borrow().get(name) {
            return found.clone();
        }
        let themed = gdk::Display::default()
            .map(|d| gtk::IconTheme::for_display(&d).has_icon(name))
            .unwrap_or(true);
        let found = if themed || !BUILTIN.iter().any(|(n, _)| *n == name) {
            name.to_string()
        } else {
            format!("matinee-{name}")
        };
        chosen.borrow_mut().insert(name.to_string(), found.clone());
        found
    })
}
