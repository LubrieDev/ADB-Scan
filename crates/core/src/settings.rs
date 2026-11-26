//! Persistent settings in `~/.config/adb-scan/config.json`.
//!
//! Unknown keys are kept as they are, so configurations written by other
//! versions are not destroyed.

use crate::i18n::{tr, trc, Language};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ScrcpyPreset {
    pub name: String,
    /// Longest side in pixels (0 = original).
    pub max_size: u32,
    pub max_fps: u32,
    /// Mbit/s.
    pub bitrate_mbps: u32,
    /// h264 | h265 | av1
    pub codec: String,
    pub audio: bool,
    pub stay_awake: bool,
    pub turn_screen_off: bool,
    pub show_touches: bool,
    pub always_on_top: bool,
    pub fullscreen: bool,
    pub borderless: bool,
    /// 0, 90, 180, 270
    pub rotation: u32,
    pub keyboard_uhid: bool,
    pub no_control: bool,
}

impl ScrcpyPreset {
    pub fn balanced() -> Self {
        Self { name: tr("Balanced").into(), max_size: 1600, max_fps: 60, bitrate_mbps: 8, codec: "h264".into(), audio: true, stay_awake: true, ..Default::default() }
    }
    pub fn quality() -> Self {
        Self { name: tr("Quality").into(), max_size: 0, max_fps: 120, bitrate_mbps: 24, codec: "h265".into(), ..Self::balanced() }
    }
    pub fn light() -> Self {
        Self { name: trc("thermal", "Light").into(), max_size: 1024, max_fps: 30, bitrate_mbps: 3, audio: false, ..Self::balanced() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // Tools
    pub adb_path: String,
    pub fastboot_path: String,
    pub scrcpy_path: String,
    pub tshark_path: String,
    pub wireshark_path: String,
    /// The tools assistant was already shown (it only comes up again by itself
    /// if adb is missing).
    pub tools_wizard_seen: bool,
    // Notifications
    pub notifications_enabled: bool,
    pub notify_usb_detected: bool,
    pub notify_authorization: bool,
    pub notify_connected: bool,
    pub notify_disconnected: bool,
    pub notify_errors: bool,
    pub notify_battery_full: bool,
    pub notify_temperature: bool,
    /// Battery °C from which an alert is sent.
    pub temperature_alert_c: f64,
    /// Phone CPU at full load for a while, with the culprit process.
    pub notify_cpu: bool,
    /// Low battery (not charging).
    pub notify_battery_low: bool,
    /// Battery % from which the low battery alert is sent.
    pub battery_low_percent: u32,
    /// The phone keeps dropping off the USB bus (loose plug, worn cable).
    pub notify_usb_unstable: bool,
    /// The phone came back at USB 2.0 although it reached USB 3 before.
    pub notify_usb_slow: bool,
    /// Show the phone's notifications on the laptop (off by default: their text
    /// is private).
    pub mirror_notifications: bool,
    /// Laptop folder where the phone's photos are copied.
    pub photos_dir: String,
    /// Scheduled actions (see `schedule`).
    pub scheduled: Vec<crate::schedule::Scheduled>,
    /// Actions run when a phone connects (see `automation`).
    pub automations: Vec<crate::automation::Automation>,
    // Background
    pub start_hidden: bool,
    pub close_to_tray: bool,
    pub autostart: bool,
    // Telemetry
    pub live_refresh_ms: u32,
    /// Rate when no window is visible.
    pub background_refresh_ms: u32,
    // Logcat
    pub logcat_buffer_lines: u32,
    pub logcat_autoscroll: bool,
    pub logcat_save_sessions: bool,
    pub logcat_buffers: String,
    // Security
    pub confirm_destructive: bool,
    pub retain_session_days: u32,
    pub audit_log: bool,
    /// Ask GitHub once per start whether there is a newer release (see
    /// `updater`). It only shows a link, it never downloads anything.
    pub check_updates: bool,
    // Window and interface
    pub window_width: i32,
    pub window_height: i32,
    pub window_maximized: bool,
    pub last_module: String,
    pub last_device: String,
    pub panel_pinned: bool,
    pub theme: ThemeMode,
    /// Interface language (English by default). Applied at startup.
    pub language: Language,
    /// Accent colour (palette name).
    pub accent: String,
    pub animations: bool,
    /// Hides serial numbers, IPs, MACs, Wi-Fi networks and nicknames on screen.
    /// Display only: it does not change data, queries or logs.
    pub privacy_mode: bool,
    pub sidebar_visible: bool,
    /// Sidebar sections folded (by the id of their first page).
    pub sidebar_folded: Vec<String>,
    /// Pages pinned to the top of the sidebar, in order.
    pub sidebar_pinned: Vec<String>,
    pub show_hidden_files: bool,
    /// Optional invitation to turn on USB debugging (advanced analysis) when
    /// the phone does not have it on. “Continue with the basic analysis” hides
    /// it; it can be shown again in Preferences.
    pub show_advanced_invite: bool,
    // Network
    pub capture_interface: String,
    pub capture_filter: String,
    // Terminal
    pub terminal_history: Vec<String>,
    pub terminal_scrollback_lines: u32,
    pub terminal_font: String,
    // scrcpy
    pub scrcpy_presets: Vec<ScrcpyPreset>,
    pub scrcpy_active_preset: String,
    // Devices
    pub nicknames: BTreeMap<String, String>,
    /// Identifiers seen by ADB for each serial number (model, codename,
    /// product): the Flasher uses them to check that the phone in fastboot is
    /// the expected model.
    pub device_ids: BTreeMap<String, Vec<String>>,
    pub wireless_history: Vec<String>,
    /// Fastest USB link (Mb/s) each phone has reached on this PC, by serial
    /// number: tells a slow cable from a slow phone.
    pub usb_best_speed: BTreeMap<String, u32>,
    /// Latest speed tests, newest first (to compare cables and ports).
    pub speed_tests: Vec<crate::link::SpeedRecord>,
    pub first_run: bool,
    /// Unknown keys (from other versions): kept when saving.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            adb_path: String::new(),
            fastboot_path: String::new(),
            scrcpy_path: String::new(),
            tshark_path: String::new(),
            wireshark_path: String::new(),
            tools_wizard_seen: false,
            notifications_enabled: true,
            notify_usb_detected: true,
            notify_authorization: true,
            notify_connected: true,
            notify_disconnected: true,
            notify_errors: true,
            notify_battery_full: false,
            notify_temperature: true,
            temperature_alert_c: 45.0,
            notify_cpu: true,
            notify_battery_low: true,
            battery_low_percent: 15,
            notify_usb_unstable: true,
            notify_usb_slow: true,
            scheduled: Vec::new(),
            automations: Vec::new(),
            photos_dir: String::new(),
            mirror_notifications: false,
            start_hidden: false,
            close_to_tray: true,
            autostart: false,
            live_refresh_ms: 500,
            background_refresh_ms: 3000,
            logcat_buffer_lines: 20_000,
            logcat_autoscroll: true,
            logcat_save_sessions: true,
            logcat_buffers: "default".into(),
            confirm_destructive: true,
            retain_session_days: 90,
            audit_log: true,
            check_updates: true,
            window_width: 1280,
            window_height: 820,
            window_maximized: false,
            last_module: "dashboard".into(),
            last_device: String::new(),
            panel_pinned: false,
            theme: ThemeMode::System,
            accent: "orange".into(),
            animations: true,
            privacy_mode: false,
            sidebar_visible: true,
            sidebar_folded: Vec::new(),
            sidebar_pinned: Vec::new(),
            show_hidden_files: false,
            show_advanced_invite: true,
            language: Language::English,
            capture_interface: String::new(),
            capture_filter: String::new(),
            terminal_history: Vec::new(),
            terminal_scrollback_lines: 5000,
            terminal_font: "Monospace 11".into(),
            scrcpy_presets: vec![ScrcpyPreset::balanced(), ScrcpyPreset::quality(), ScrcpyPreset::light()],
            scrcpy_active_preset: tr("Balanced").into(),
            nicknames: BTreeMap::new(),
            device_ids: BTreeMap::new(),
            wireless_history: Vec::new(),
            usb_best_speed: BTreeMap::new(),
            speed_tests: Vec::new(),
            first_run: true,
            extra: Map::new(),
        }
    }
}

const HISTORY_MAX: usize = 200;

impl Settings {
    /// Fixes ranges and cleans up invalid legacy data.
    pub fn sanitize(&mut self) {
        self.live_refresh_ms = self.live_refresh_ms.clamp(250, 5000);
        self.background_refresh_ms = self.background_refresh_ms.clamp(1000, 30_000);
        self.logcat_buffer_lines = self.logcat_buffer_lines.clamp(1000, 200_000);
        self.retain_session_days = self.retain_session_days.clamp(1, 3650);
        self.window_width = self.window_width.clamp(900, 4000);
        self.window_height = self.window_height.clamp(600, 3000);
        self.terminal_scrollback_lines = self.terminal_scrollback_lines.clamp(500, 100_000);
        self.temperature_alert_c = self.temperature_alert_c.clamp(30.0, 70.0);
        self.battery_low_percent = self.battery_low_percent.clamp(5, 50);
        // Older versions sometimes saved multi-line outputs as commands.
        self.terminal_history.retain(|c| !c.contains('\n') && !c.trim().is_empty() && c.len() < 500);
        dedup_keep_last(&mut self.terminal_history);
        if self.terminal_history.len() > HISTORY_MAX {
            let cut = self.terminal_history.len() - HISTORY_MAX;
            self.terminal_history.drain(..cut);
        }
        if self.scrcpy_presets.is_empty() {
            self.scrcpy_presets = Settings::default().scrcpy_presets;
        }
        // Accent keys were Spanish words before 1.0.
        let renamed = match self.accent.as_str() {
            "naranja" => "orange",
            "azul" => "blue",
            "verde" => "green",
            "morado" => "purple",
            "rosa" => "pink",
            "rojo" => "red",
            "turquesa" => "teal",
            "grafito" => "graphite",
            _ => "",
        };
        if !renamed.is_empty() {
            self.accent = renamed.into();
        }
        // Migration of older settings: `dark_mode` → `theme`.
        if let Some(Value::Bool(dark)) = self.extra.remove("dark_mode") {
            if dark && self.theme == ThemeMode::System {
                self.theme = ThemeMode::Dark;
            }
        }
    }

    pub fn load_from(path: &Path) -> Self {
        let mut settings = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("invalid config.json ({e}); a copy is kept and default values are used");
                    let _ = std::fs::copy(path, path.with_extension("json.corrupt"));
                    Settings::default()
                }
            },
            Err(_) => Settings::default(),
        };
        settings.sanitize();
        settings
    }

    pub fn load() -> Self {
        Self::load_from(&crate::paths::config_file())
    }

    /// Atomic write: temporary file + rename.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp: PathBuf = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&crate::paths::config_file())
    }

    pub fn push_history(&mut self, command: &str) {
        let c = command.trim();
        if c.is_empty() || c.contains('\n') {
            return;
        }
        self.terminal_history.retain(|h| h != c);
        self.terminal_history.push(c.to_string());
        if self.terminal_history.len() > HISTORY_MAX {
            self.terminal_history.remove(0);
        }
    }

    /// Saves a phone's identifiers; `true` if they changed.
    pub fn remember_device_ids(&mut self, serial: &str, ids: Vec<String>) -> bool {
        let mut clean: Vec<String> = Vec::new();
        for id in ids.into_iter().map(|i| i.trim().to_string()).filter(|i| !i.is_empty()) {
            if !clean.iter().any(|c| c.eq_ignore_ascii_case(&id)) {
                clean.push(id);
            }
        }
        if clean.is_empty() || self.device_ids.get(serial) == Some(&clean) {
            return false;
        }
        self.device_ids.insert(serial.to_string(), clean);
        true
    }

    pub fn nickname<'a>(&'a self, serial: &str) -> Option<&'a str> {
        self.nicknames.get(serial).map(String::as_str).filter(|n| !n.is_empty())
    }

    pub fn active_preset(&self) -> ScrcpyPreset {
        self.scrcpy_presets.iter().find(|p| p.name == self.scrcpy_active_preset).or_else(|| self.scrcpy_presets.first()).cloned().unwrap_or_else(ScrcpyPreset::balanced)
    }
}

fn dedup_keep_last(items: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::with_capacity(items.len());
    for item in items.iter().rev() {
        if seen.insert(item.clone()) {
            out.push(item.clone());
        }
    }
    out.reverse();
    *items = out;
}

/// Extracts the command from a terminal line with Android's mksh prompt
/// (`NX809J:/sdcard $ ls -la` → `ls -la`). `None` if the line is not a prompt
/// (e.g. inside `top` or `vi`).
pub fn command_after_prompt(line: &str) -> Option<String> {
    let line = line.trim_end();
    let (head, cmd) = line.split_once(" $ ").or_else(|| line.split_once(" # "))?;
    let head = head.trim_start();
    if head.is_empty() || head.contains(' ') || !head.contains(':') {
        return None;
    }
    let cmd = cmd.trim();
    (!cmd.is_empty()).then(|| cmd.to_string())
}

/// Startup file for mksh that loads Android's usual prompt and preloads the
/// history with `print -s` (Android builds mksh without persistent history).
pub fn mksh_history_rc(history: &[String]) -> String {
    let mut out = String::from("[ -r /system/etc/mkshrc ] && . /system/etc/mkshrc\n");
    for cmd in history.iter().filter(|c| !c.contains('\n') && !c.trim().is_empty()) {
        out.push_str("print -s -- '");
        out.push_str(&cmd.replace('\'', "'\\''"));
        out.push_str("'\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("adbscan-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.json")
    }

    #[test]
    fn loads_python_config_and_preserves_unknown_keys() {
        let path = tmp("compat");
        std::fs::write(
            &path,
            r#"{"adb_path":"","dark_mode":true,"live_refresh_ms":99999,"color_scheme":"graphite",
               "refresh_dashboard":5,"terminal_history":["ls","linea1\nlinea2","ls","pwd"],"window_width":10}"#,
        )
        .unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.theme, ThemeMode::Dark);
        assert_eq!(s.live_refresh_ms, 5000);
        assert_eq!(s.window_width, 900);
        assert_eq!(s.terminal_history, vec!["ls", "pwd"]);
        assert_eq!(s.extra.get("color_scheme"), Some(&Value::String("graphite".into())));
        s.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("refresh_dashboard"));
        assert!(text.contains("\"theme\": \"dark\""));
    }

    #[test]
    fn corrupt_json_falls_back_and_keeps_copy() {
        let path = tmp("corrupt");
        std::fs::write(&path, "{not json").unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s, {
            let mut d = Settings::default();
            d.sanitize();
            d
        });
        assert!(path.with_extension("json.corrupt").exists());
    }

    #[test]
    fn history() {
        let mut s = Settings::default();
        s.push_history("ls");
        s.push_history("pwd");
        s.push_history("ls");
        s.push_history("a\nb");
        assert_eq!(s.terminal_history, vec!["pwd", "ls"]);
    }
    #[test]
    fn prompt_commands() {
        assert_eq!(command_after_prompt("NX809J:/ $ ls -la"), Some("ls -la".into()));
        assert_eq!(command_after_prompt("NX809J:/sdcard/Download $ echo a $ b   "), Some("echo a $ b".into()));
        assert_eq!(command_after_prompt("NX809J:/ # id"), Some("id".into()));
        assert_eq!(command_after_prompt("NX809J:/ $ "), None);
        assert_eq!(command_after_prompt("  PID USER  PR $ x"), None);
        let rc = mksh_history_rc(&["echo 'hola'".into(), "ls".into()]);
        assert!(rc.starts_with("[ -r /system/etc/mkshrc ]"));
        assert!(rc.contains(r"print -s -- 'echo '\''hola'\'''"));
        assert!(rc.ends_with("print -s -- 'ls'\n"));
    }
}
