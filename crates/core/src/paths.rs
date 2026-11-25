//! XDG paths. Everything that is written lives in the user's directories.

use std::path::PathBuf;

pub const APP_NAME: &str = "ADB Scan";
pub const APP_SLUG: &str = "adb-scan";
pub const APP_ID: &str = "com.adbscan.AdbScan";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn xdg(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var).map(PathBuf::from) {
        Some(p) if p.is_absolute() => p,
        _ => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp")).join(fallback),
    }
}

pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join(APP_SLUG)
}
pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join(APP_SLUG)
}
pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join(APP_SLUG)
}
pub fn cache_dir() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache").join(APP_SLUG)
}
pub fn config_file() -> PathBuf {
    config_dir().join("config.json")
}
pub fn database_file() -> PathBuf {
    data_dir().join("adb-scan.db")
}
pub fn logcat_dir() -> PathBuf {
    data_dir().join("logcat")
}
pub fn screenshots_dir() -> PathBuf {
    pictures_dir().join("ADB Scan")
}
pub fn recordings_dir() -> PathBuf {
    videos_dir().join("ADB Scan")
}
pub fn apk_dir() -> PathBuf {
    data_dir().join("apk")
}
pub fn reports_dir() -> PathBuf {
    let dir = data_dir().join("reports");
    // Before 1.0 the folder had a Spanish name.
    let old = data_dir().join("informes");
    if !dir.exists() && old.is_dir() {
        let _ = std::fs::rename(&old, &dir);
    }
    dir
}
pub fn log_file() -> PathBuf {
    state_dir().join("adb-scan.log")
}
/// Tools downloaded by the app (platform-tools, scrcpy).
pub fn managed_tools_dir() -> PathBuf {
    data_dir().join("tools")
}
pub fn autostart_file() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("autostart").join(format!("{APP_SLUG}.desktop"))
}
pub fn applications_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("applications")
}

/// User directories (`user-dirs.dirs`), falling back to `~/Pictures`.
fn user_dir(key: &str, fallback: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let file = xdg("XDG_CONFIG_HOME", ".config").join("user-dirs.dirs");
    if let Ok(text) = std::fs::read_to_string(file) {
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix(&format!("{key}=")) {
                let value = rest.trim().trim_matches('"').replace("$HOME", &home.display().to_string());
                if !value.is_empty() {
                    return PathBuf::from(value);
                }
            }
        }
    }
    home.join(fallback)
}

pub fn pictures_dir() -> PathBuf {
    user_dir("XDG_PICTURES_DIR", "Pictures")
}
pub fn videos_dir() -> PathBuf {
    user_dir("XDG_VIDEOS_DIR", "Videos")
}
pub fn downloads_dir() -> PathBuf {
    user_dir("XDG_DOWNLOAD_DIR", "Downloads")
}

pub fn ensure_dirs() -> std::io::Result<()> {
    for d in [config_dir(), data_dir(), state_dir(), cache_dir(), logcat_dir()] {
        std::fs::create_dir_all(d)?;
    }
    Ok(())
}
