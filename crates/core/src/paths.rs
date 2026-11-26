//! Where ADB Scan writes. Everything lives in the user's directories.
//!
//! Unix: the XDG folders (`~/.config/adb-scan`, `~/.local/share/adb-scan`…).
//! Windows: `%APPDATA%\adb-scan` for the settings and the data, and
//! `%LOCALAPPDATA%\adb-scan` for the log and the cache.

use std::path::PathBuf;

pub const APP_NAME: &str = "ADB Scan";
pub const APP_SLUG: &str = "adb-scan";
pub const APP_ID: &str = "com.adbscan.AdbScan";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(unix)]
fn xdg(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var).map(PathBuf::from) {
        Some(p) if p.is_absolute() => p,
        _ => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp")).join(fallback),
    }
}

#[cfg(unix)]
pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join(APP_SLUG)
}
#[cfg(unix)]
pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join(APP_SLUG)
}
#[cfg(unix)]
pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join(APP_SLUG)
}
#[cfg(unix)]
pub fn cache_dir() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache").join(APP_SLUG)
}

/// A known folder; if Windows does not give it, the temporary folder.
#[cfg(windows)]
fn known(dir: Option<PathBuf>) -> PathBuf {
    dir.unwrap_or_else(std::env::temp_dir)
}
/// `%APPDATA%\adb-scan`.
#[cfg(windows)]
pub fn config_dir() -> PathBuf {
    known(dirs::config_dir()).join(APP_SLUG)
}
/// `%APPDATA%\adb-scan`, the same folder as the settings.
#[cfg(windows)]
pub fn data_dir() -> PathBuf {
    known(dirs::data_dir()).join(APP_SLUG)
}
/// `%LOCALAPPDATA%\adb-scan`.
#[cfg(windows)]
pub fn state_dir() -> PathBuf {
    known(dirs::data_local_dir()).join(APP_SLUG)
}
/// `%LOCALAPPDATA%\adb-scan\cache`.
#[cfg(windows)]
pub fn cache_dir() -> PathBuf {
    known(dirs::cache_dir()).join(APP_SLUG).join("cache")
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
/// Linux desktops: `~/.config/autostart/adb-scan.desktop`. On Windows,
/// starting with the session is a registry value (see the app).
#[cfg(unix)]
pub fn autostart_file() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("autostart").join(format!("{APP_SLUG}.desktop"))
}
#[cfg(unix)]
pub fn applications_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("applications")
}

/// User directories (`user-dirs.dirs`), falling back to `~/Pictures`.
#[cfg(unix)]
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

#[cfg(unix)]
pub fn pictures_dir() -> PathBuf {
    user_dir("XDG_PICTURES_DIR", "Pictures")
}
#[cfg(unix)]
pub fn videos_dir() -> PathBuf {
    user_dir("XDG_VIDEOS_DIR", "Videos")
}
#[cfg(unix)]
pub fn downloads_dir() -> PathBuf {
    user_dir("XDG_DOWNLOAD_DIR", "Downloads")
}

/// The user's known folder (it may have been moved, e.g. to OneDrive),
/// falling back to the profile's `fallback`.
#[cfg(windows)]
fn user_dir(dir: Option<PathBuf>, fallback: &str) -> PathBuf {
    dir.unwrap_or_else(|| dirs::home_dir().unwrap_or_else(std::env::temp_dir).join(fallback))
}
#[cfg(windows)]
pub fn pictures_dir() -> PathBuf {
    user_dir(dirs::picture_dir(), "Pictures")
}
#[cfg(windows)]
pub fn videos_dir() -> PathBuf {
    user_dir(dirs::video_dir(), "Videos")
}
#[cfg(windows)]
pub fn downloads_dir() -> PathBuf {
    user_dir(dirs::download_dir(), "Downloads")
}

/// The user's home folder (`$HOME`).
#[cfg(unix)]
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
/// The user's profile folder (`%USERPROFILE%`).
#[cfg(windows)]
pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

pub fn ensure_dirs() -> std::io::Result<()> {
    for d in [config_dir(), data_dir(), state_dir(), cache_dir(), logcat_dir()] {
        std::fs::create_dir_all(d)?;
    }
    Ok(())
}
