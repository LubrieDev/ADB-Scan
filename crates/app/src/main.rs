//! ADB Scan — GTK4 + libadwaita interface.

mod anim;
mod app;
mod ctx;
mod help;
mod pages;
mod panel;
mod prefs;
mod search;
mod setup;
mod sidebar;
mod theme;
mod tray;
mod util;
mod widgets;
mod window;
mod wireless;

fn main() -> gtk::glib::ExitCode {
    init_logging();
    // Capture mode (development): once Mesa's shader cache is full, the window
    // stops delivering its drawing and screenshots come out empty. Only in that
    // mode; the normal app does not change.
    if std::env::var_os("ADB_SCAN_SCREENSHOT").is_some() && std::env::var_os("MESA_SHADER_CACHE_DISABLE").is_none() {
        std::env::set_var("MESA_SHADER_CACHE_DISABLE", "true");
    }
    set_language();
    theme::prefer_mint_palette();
    app::run()
}

/// The interface language comes from Preferences (English by default);
/// `ADB_SCAN_LANG=es` overrides it for a single run. It is fixed before any
/// text is built.
fn set_language() {
    use adbscan_core::i18n::{self, Language};
    let language = std::env::var("ADB_SCAN_LANG")
        .ok()
        .and_then(|code| Language::from_code(&code))
        .unwrap_or_else(|| adbscan_core::settings::Settings::load().language);
    i18n::set_language(language);
}

/// Simple logging to stderr; level with ADB_SCAN_LOG=debug|info|warn.
fn init_logging() {
    struct Logger(log::LevelFilter);
    impl log::Log for Logger {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= self.0
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                eprintln!("[{}] {}: {}", r.level(), r.target(), r.args());
            }
        }
        fn flush(&self) {}
    }
    let level = match std::env::var("ADB_SCAN_LOG").as_deref() {
        Ok("debug") => log::LevelFilter::Debug,
        Ok("info") => log::LevelFilter::Info,
        Ok("error") => log::LevelFilter::Error,
        _ => log::LevelFilter::Warn,
    };
    let logger = Box::leak(Box::new(Logger(level)));
    let _ = log::set_logger(logger);
    log::set_max_level(level);
}
