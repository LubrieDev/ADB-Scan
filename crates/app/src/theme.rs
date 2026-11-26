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

/// `#rrggbb` → red, green and blue in 0..=1.
fn rgb(hex: &str) -> (f64, f64, f64) {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).unwrap_or(0) as f64 / 255.0;
    (p(0), p(2), p(4))
}

fn to_hex((r, g, b): (f64, f64, f64)) -> String {
    let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

/// The colour with its saturation scaled to `percent` (100: unchanged), in
/// HSL so the hue and lightness stay.
pub fn with_intensity(hex: &str, percent: u32) -> String {
    let (r, g, b) = rgb(hex);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 || percent == 100 {
        return to_hex((r, g, b));
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } * 60.0;
    let s = (s * percent as f64 / 100.0).clamp(0.0, 1.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r1, g1, b1) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    to_hex((r1 + m, g1 + m, b1 + m))
}

/// Relative luminance (WCAG).
fn luminance(hex: &str) -> f64 {
    let (r, g, b) = rgb(hex);
    let lin = |c: f64| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// Text on `bg`: white, as on every palette colour (like GNOME's accents),
/// unless the colour is so light that white barely shows (yellow, mint…).
pub fn text_on(bg: &str) -> &'static str {
    // Contrast ratio of white on the colour.
    if 1.05 / (luminance(bg) + 0.05) >= 2.2 {
        "#ffffff"
    } else {
        "#1e1e1e"
    }
}

/// The accent as the settings ask for it: the palette colour or the custom
/// one, with its intensity.
pub fn accent_color(s: &Settings) -> String {
    let base = if s.accent == "custom" { s.accent_custom.as_str() } else { accent_hex(&s.accent) };
    with_intensity(base, s.accent_intensity)
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
    apply_accent(settings);
    if let Some(gs) = gtk::Settings::default() {
        // Also respects the system preference if they are turned off.
        if !settings.animations {
            gs.set_gtk_enable_animations(false);
        }
    }
}

thread_local! {
    /// Active accent colour, to recompute its shade when switching light/dark.
    static CURRENT_ACCENT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// Applies the accent of `settings` (palette or custom colour, intensity).
pub fn apply_accent(settings: &Settings) {
    apply_accent_color(&accent_color(settings));
}

fn apply_accent_color(bg: &str) {
    CURRENT_ACCENT.with(|c| *c.borrow_mut() = bg.to_string());
    let fg = text_on(bg);
    let dark = adw::StyleManager::default().is_dark();
    // In dark mode the accent text must be lighter to contrast.
    // A light custom colour (yellow, mint…) also needs to be darker as text
    // on a light background.
    let accent_text = if dark {
        mix(bg, (255, 255, 255), 0.25)
    } else if luminance(bg) > 0.4 {
        mix(bg, (0, 0, 0), 0.45)
    } else {
        mix(bg, (0, 0, 0), 0.12)
    };
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
        let bg = CURRENT_ACCENT.with(|c| c.borrow().clone());
        if !bg.is_empty() {
            glib::idle_add_local_once(move || apply_accent_color(&bg));
        }
    });
}


pub fn animations_enabled() -> bool {
    gtk::Settings::default().map(|s| s.is_gtk_enable_animations()).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intensity_keeps_the_hue_and_scales_the_saturation() {
        assert_eq!(with_intensity("#e66100", 100), "#e66100");
        // No saturation left: a grey of the same lightness.
        let grey = with_intensity("#e66100", 0);
        let (r, g, b) = rgb(&grey);
        assert!((r - g).abs() < 0.01 && (g - b).abs() < 0.01, "{grey}");
        // Softer and more vivid stay orange (red > green > blue).
        for pct in [40, 160] {
            let (r, g, b) = rgb(&with_intensity("#e66100", pct));
            assert!(r > g && g > b, "{pct}");
        }
        // Greys have no hue to scale.
        assert_eq!(with_intensity("#6f6f6f", 160), "#6f6f6f");
    }

    #[test]
    fn text_colour_reads_on_light_and_dark_accents() {
        for (_, _, bg, fg) in ACCENTS {
            assert_eq!(text_on(bg), fg, "{bg}");
        }
        assert_eq!(text_on("#f6d32d"), "#1e1e1e");
        assert_eq!(text_on("#ffffff"), "#1e1e1e");
    }
}
