//! Colors. The palette is a cinema at night: aubergine velvet seats, warm
//! projector-light text, and marquee gold reserved for what needs attention.

use gtk::gdk;
use std::fmt::Write;

const BASE_CSS: &str = include_str!("../data/style.css");

struct Palette {
    // Matinee's own tokens, used by style.css.
    gold: &'static str,
    gold_ink: &'static str,
    /// Gold dark enough to read as text on the background.
    gold_text: &'static str,
    dim: &'static str,
    seat: &'static str,
    velvet: &'static str,
    // libadwaita colors.
    accent: &'static str,
    window_bg: &'static str,
    view_bg: &'static str,
    sidebar_bg: &'static str,
    raised_bg: &'static str,
    fg: &'static str,
}

const DARK: Palette = Palette {
    gold: "#f2b544",
    gold_ink: "#2a1b00",
    gold_text: "#f2b544",
    dim: "#ab9db7",
    seat: "#2e2338",
    velvet: "#1c1421",
    accent: "#f6c869",
    window_bg: "#1c1421",
    view_bg: "#1c1421",
    sidebar_bg: "#231a2b",
    raised_bg: "#2c2135",
    fg: "#f3ebdd",
};

const LIGHT: Palette = Palette {
    gold: "#e9a52b",
    gold_ink: "#2a1b00",
    gold_text: "#8f5b00",
    dim: "#6c5d78",
    seat: "#e6dcea",
    velvet: "#2b1f33",
    accent: "#8a5a00",
    window_bg: "#f6f2f7",
    view_bg: "#fbf9fc",
    sidebar_bg: "#eee7f1",
    raised_bg: "#ffffff",
    fg: "#2b1f33",
};

pub fn install() {
    let Some(display) = gdk::Display::default() else { return };

    let base = gtk::CssProvider::new();
    base.load_from_string(BASE_CSS);
    gtk::style_context_add_provider_for_display(&display, &base, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

    let palette = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(&display, &palette, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

    // A video app is happiest in the dark; a system that asks for light still gets light.
    let manager = adw::StyleManager::default();
    manager.set_color_scheme(adw::ColorScheme::PreferDark);
    palette.load_from_string(&palette_css(manager.is_dark()));
    manager.connect_dark_notify(move |m| palette.load_from_string(&palette_css(m.is_dark())));
}

/// Dark whatever the system says, or follow the system (dark when it has no preference).
pub fn set_always_dark(on: bool) {
    adw::StyleManager::default().set_color_scheme(if on {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::PreferDark
    });
}

fn palette_css(dark: bool) -> String {
    let p = if dark { &DARK } else { &LIGHT };
    let mut css = String::new();
    for (name, value) in [
        ("mt_gold", p.gold),
        ("mt_gold_ink", p.gold_ink),
        ("mt_gold_text", p.gold_text),
        ("mt_dim", p.dim),
        ("mt_seat", p.seat),
        ("mt_velvet", p.velvet),
    ] {
        let _ = writeln!(css, "@define-color {name} {value};");
    }

    let mut adw_colors = vec![
        ("accent_color", p.accent),
        ("accent_bg_color", p.gold),
        ("accent_fg_color", p.gold_ink),
        ("window_bg_color", p.window_bg),
        ("window_fg_color", p.fg),
        ("view_bg_color", p.view_bg),
        ("view_fg_color", p.fg),
        ("headerbar_bg_color", p.window_bg),
        ("headerbar_fg_color", p.fg),
        ("sidebar_bg_color", p.sidebar_bg),
        ("sidebar_fg_color", p.fg),
        // Same colors when the window is in the background, so nothing turns gray.
        ("sidebar_backdrop_color", p.sidebar_bg),
        ("headerbar_backdrop_color", p.window_bg),
        ("popover_bg_color", p.raised_bg),
        ("dialog_bg_color", p.raised_bg),
    ];
    if dark {
        adw_colors.push(("card_bg_color", "rgba(255, 255, 255, 0.06)"));
    }

    // libadwaita 1.6 moved its colors to CSS variables; earlier versions use named colors.
    if adw::minor_version() >= 6 {
        css.push_str(":root {\n");
        for (name, value) in adw_colors {
            let _ = writeln!(css, "  --{}: {value};", name.replace('_', "-"));
        }
        css.push_str("}\n");
    } else {
        for (name, value) in adw_colors {
            let _ = writeln!(css, "@define-color {name} {value};");
        }
    }
    css
}
