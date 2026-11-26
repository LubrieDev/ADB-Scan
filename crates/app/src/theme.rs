//! Theme: embedded icons, CSS, light/dark scheme and accent colour.

use adbscan_core::paths;
use adbscan_core::settings::{Settings, ThemeMode};
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::path::PathBuf;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/icons.rs"));
}

/// (key, display name, background colour, text colour on the background)
pub const ACCENTS: [(&str, &str, &str, &str); 8] = [
    ("orange", "Orange", "#e66100", "#ffffff"),
    ("blue", "Blue", "#3584e4", "#ffffff"),
    ("green", "Green", "#2ec27e", "#ffffff"),
    ("purple", "Purple", "#9141ac", "#ffffff"),
    ("pink", "Pink", "#d56199", "#ffffff"),
    ("red", "Red", "#e01b24", "#ffffff"),
    ("teal", "Teal", "#2190a4", "#ffffff"),
    ("graphite", "Graphite", "#6f8396", "#ffffff"),
];

pub fn icons_dir() -> PathBuf {
    paths::cache_dir().join("icons")
}

/// Writes the embedded SVGs to the cache (only if they change) and adds them to
/// the icon theme. They also serve the tray icon (SNI).
pub fn install_icons() {
    let root = icons_dir();
    for (rel, bytes) in embedded::ICONS {
        let path = root.join(rel);
        if std::fs::read(&path).map(|b| b == *bytes).unwrap_or(false) {
            continue;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, bytes);
    }
    if let Some(display) = gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_search_path(&root);
    }
}

thread_local! {
    static ACCENT_PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
}

fn mix(hex: &str, other: (u8, u8, u8), t: f64) -> String {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f64;
    let (r, g, b) = (p(0), p(2), p(4));
    let m = |a: f64, b: u8| (a + (b as f64 - a) * t).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", m(r, other.0), m(g, other.1), m(b, other.2))
}

pub fn accent_hex(key: &str) -> &'static str {
    ACCENTS.iter().find(|a| a.0 == key).map(|a| a.2).unwrap_or(ACCENTS[0].2)
}

/// Linux Mint theme whose dark palette ADB Scan uses.
#[cfg(unix)]
const MINT_THEME: &str = "Mint-Y-Aqua";

/// Before starting GTK: if ADB Scan is going to open in dark mode, make Mint's
/// libadwaita use the Mint-Y palette (greys with a bluish tint) instead of the
/// desktop's GTK theme (e.g. Orchis, with neutral greys). Only with that
/// libadwaita (the variable does not exist in GNOME's), if Mint-Y is installed
/// and if the user has not set another theme. It is decided at startup:
/// switching to light without restarting still shows Mint-Y.
#[cfg(unix)]
pub fn prefer_mint_palette() {
    const VAR: &str = "ADW_DEBUG_THEME_NAME";
    if std::env::var_os(VAR).is_some() || std::env::var_os("GTK_THEME").is_some() {
        return;
    }
    if !std::path::Path::new("/usr/share/themes").join(MINT_THEME).join("gtk-4.0").is_dir() {
        return;
    }
    let dark = match Settings::load().theme {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::System => system_prefers_dark(),
    };
    log::debug!("dark palette of {MINT_THEME}: {}", if dark { "yes" } else { "no (opens in light mode)" });
    if dark {
        // There are no other threads yet: this is the start of `main`.
        std::env::set_var(VAR, MINT_THEME);
    }
}

/// The desktop's dark preference (before GTK reads it).
#[cfg(unix)]
fn system_prefers_dark() -> bool {
    use gtk::gio::{self, prelude::SettingsExt};
    let Some(source) = gio::SettingsSchemaSource::default() else { return false };
    [("org.x.apps.portal", "color-scheme"), ("org.gnome.desktop.interface", "color-scheme")]
        .iter()
        .filter(|(schema, key)| source.lookup(schema, true).is_some_and(|s| s.has_key(key)))
        .any(|(schema, key)| gio::Settings::new(schema).string(key) == "prefer-dark")
}

pub fn apply(settings: &Settings) {
    let manager = adw::StyleManager::default();
    manager.set_color_scheme(match settings.theme {
        ThemeMode::System => adw::ColorScheme::Default,
        ThemeMode::Light => adw::ColorScheme::ForceLight,
        ThemeMode::Dark => adw::ColorScheme::ForceDark,
    });
    apply_accent(&settings.accent);
    if let Some(gs) = gtk::Settings::default() {
        // Also respects the system preference if they are turned off.
        if !settings.animations {
            gs.set_gtk_enable_animations(false);
        }
    }
}

thread_local! {
    /// Active accent, to recompute its shade when switching light/dark.
    static CURRENT_ACCENT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

pub fn apply_accent(key: &str) {
    CURRENT_ACCENT.with(|c| *c.borrow_mut() = key.to_string());
    let (_, _, bg, fg) = ACCENTS.iter().find(|a| a.0 == key).copied().unwrap_or(ACCENTS[0]);
    let dark = adw::StyleManager::default().is_dark();
    // In dark mode the accent text must be lighter to contrast.
    let accent_text = if dark { mix(bg, (255, 255, 255), 0.25) } else { mix(bg, (0, 0, 0), 0.12) };
    let mut css = format!("@define-color accent_bg_color {bg};\n@define-color accent_fg_color {fg};\n@define-color accent_color {accent_text};\n");
    if dark {
        // Sidebar slightly lighter than the content, so it stands out.
        css.push_str("@define-color sidebar_bg_color #2b2b30;\n@define-color sidebar_backdrop_color #28282c;\n");
    }
    ACCENT_PROVIDER.with(|cell| {
        let mut slot = cell.borrow_mut();
        let provider = slot.get_or_insert_with(|| {
            let p = gtk::CssProvider::new();
            if let Some(display) = gdk::Display::default() {
                gtk::style_context_add_provider_for_display(&display, &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
            }
            p
        });
        provider.load_from_string(&css);
    });
}

pub fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("style.css"));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    // When switching light/dark the accent shade is recomputed.
    let manager = adw::StyleManager::default();
    manager.connect_dark_notify(|_| {
        let key = CURRENT_ACCENT.with(|c| c.borrow().clone());
        glib::idle_add_local_once(move || apply_accent(&key));
    });
}


pub fn animations_enabled() -> bool {
    gtk::Settings::default().map(|s| s.is_gtk_enable_animations()).unwrap_or(true)
}
