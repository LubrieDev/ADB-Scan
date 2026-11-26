//! ADB Scan — GTK4 + libadwaita interface.

// Windows: a window program, without a console behind it (debug builds keep
// it, to see the log).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
#[cfg(windows)]
mod win;
mod window;
mod wireless;

fn main() -> gtk::glib::ExitCode {
    #[cfg(windows)]
    win::wait_for_previous_instance();
    init_logging();
    // Capture mode (development): once Mesa's shader cache is full, the window
    // stops delivering its drawing and screenshots come out empty. Only in that
    // mode; the normal app does not change.
    if std::env::var_os("ADB_SCAN_SCREENSHOT").is_some() && std::env::var_os("MESA_SHADER_CACHE_DISABLE").is_none() {
        std::env::set_var("MESA_SHADER_CACHE_DISABLE", "true");
    }
    set_language();
    #[cfg(unix)]
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
///
/// On Windows the program has no console: the log also goes to
/// `%LOCALAPPDATA%\adb-scan\adb-scan.log` (from `info` up, and panics too),
/// so a problem can be sent without a terminal.
fn init_logging() {
    struct Logger(log::LevelFilter, Option<std::sync::Mutex<std::fs::File>>);
    impl log::Log for Logger {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= self.0
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                eprintln!("[{}] {}: {}", r.level(), r.target(), r.args());
                if let Some(file) = &self.1 {
                    use std::io::Write;
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                    let mut f = file.lock().unwrap_or_else(|e| e.into_inner());
                    let _ = writeln!(f, "{} [{}] {}: {}", adbscan_core::format::file_stamp(now), r.level(), r.target(), r.args());
                }
            }
        }
        fn flush(&self) {}
    }
    let default = if cfg!(windows) { log::LevelFilter::Info } else { log::LevelFilter::Warn };
    let level = match std::env::var("ADB_SCAN_LOG").as_deref() {
        Ok("debug") => log::LevelFilter::Debug,
        Ok("info") => log::LevelFilter::Info,
        Ok("warn") => log::LevelFilter::Warn,
        Ok("error") => log::LevelFilter::Error,
        _ => default,
    };
    #[cfg(windows)]
    let file = win::open_log_file().map(std::sync::Mutex::new);
    #[cfg(not(windows))]
    let file = None;
    let logger = Box::leak(Box::new(Logger(level, file)));
    let _ = log::set_logger(logger);
    log::set_max_level(level);
    #[cfg(windows)]
    {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log::error!("panic: {info}");
            default_hook(info);
        }));
        log::info!("ADB Scan {} on Windows", adbscan_core::paths::VERSION);
    }
}
