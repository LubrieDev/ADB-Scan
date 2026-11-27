//! Phone controls (those in the quick settings): Wi-Fi, mobile data, Bluetooth,
//! airplane mode, location, Do Not Disturb, brightness, volume…
//!
//! **These commands change the phone.** They are kept apart from everything
//! else and only run through an explicit user action. Reading the state
//! ([`STATE_SCRIPT`]) is read-only (~0.2 s).
//!
//! The commands are fixed strings: they never carry user text, only bounded
//! numbers. They change with the Android version because `svc wifi`, `svc data`
//! and `svc bluetooth` went away (Android 16 no longer has them) and their `cmd
//! …` replacements arrived in different versions.
//!
//! Verified on a REDMAGIC 11S Pro (Android 16, without root). Without root
//! these cannot be controlled: flashlight (there is no shell command), Wi-Fi
//! hotspot (`cmd wifi start-softap` only exists in debug builds) and NFC (`svc
//! nfc enable` is killed by the system). For those the state is read and their
//! screen is opened on the phone.
//!
//! Volume cannot be set from the shell on that phone either (`cmd media_session
//! volume --set` is accepted and ignored): media volume is changed with the
//! volume keys, as the user would.

use crate::{i18n::tr, t};
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::probe::{self, Probe};

/// Reading of the state, read-only. `settings list` reads each settings
/// namespace at once (three calls instead of one per item).
pub const STATE_SCRIPT: &str = r#"echo @@@SDK@@@; getprop ro.build.version.sdk
echo @@@GLOBAL@@@; settings list global 2>&1
echo @@@SYSTEM@@@; settings list system 2>&1
echo @@@SECURE@@@; settings list secure 2>&1
echo @@@WIFI@@@; cmd wifi status 2>&1 | head -n 1
echo @@@AIRPLANE@@@; cmd connectivity airplane-mode 2>&1
echo @@@LOCATION@@@; cmd location is-location-enabled 2>&1
echo @@@NIGHT@@@; cmd uimode night 2>&1
echo @@@DATASAVER@@@; cmd netpolicy get restrict-background 2>&1
a=
echo @@@MEDIAVOL@@@; v=$(cmd media_session volume --stream 3 --get 2>&1); if [ -n "$v" ]; then echo "$v"; else a=$(dumpsys audio 2>&1 | grep -A6 -E '^- STREAM_(MUSIC|RING):'); echo "$a" | grep -A6 '^- STREAM_MUSIC:'; fi
echo @@@RINGVOL@@@; v=$(cmd media_session volume --stream 2 --get 2>&1); if [ -n "$v" ]; then echo "$v"; else [ -z "$a" ] && a=$(dumpsys audio 2>&1 | grep -A6 -E '^- STREAM_(MUSIC|RING):'); echo "$a" | grep -A6 '^- STREAM_RING:'; fi
echo @@@NFC@@@; dumpsys nfc 2>&1 | grep -m1 'mState='
echo @@@TETHER@@@; dumpsys tethering 2>&1 | sed -n '/Tether state:/,/^  [A-Z]/p'
echo @@@BATTERY@@@; dumpsys battery 2>&1 | grep -E 'powered:'
echo @@@END@@@
"#;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Volume {
    pub value: i64,
    pub min: i64,
    pub max: i64,
}

/// Do Not Disturb (`zen_mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dnd {
    Off,
    Priority,
    Alarms,
    Silence,
}

impl Dnd {
    pub const ALL: [Dnd; 4] = [Dnd::Off, Dnd::Priority, Dnd::Alarms, Dnd::Silence];

    pub fn label(self) -> &'static str {
        match self {
            Dnd::Off => tr("Off"),
            Dnd::Priority => tr("Priority only"),
            Dnd::Alarms => tr("Alarms only"),
            Dnd::Silence => tr("Total silence"),
        }
    }

    fn from_zen(v: &str) -> Option<Dnd> {
        Some(match v.trim() {
            "0" => Dnd::Off,
            "1" => Dnd::Priority,
            "2" => Dnd::Silence,
            "3" => Dnd::Alarms,
            _ => return None,
        })
    }

    fn arg(self) -> &'static str {
        match self {
            Dnd::Off => "off",
            Dnd::Priority => "priority",
            Dnd::Alarms => "alarms",
            Dnd::Silence => "none",
        }
    }
}

/// Switches with an on/off state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Toggle {
    Wifi,
    MobileData,
    Bluetooth,
    Airplane,
    Location,
    DarkMode,
    AutoRotate,
    AutoBrightness,
    BatterySaver,
    DataSaver,
    NightLight,
}

impl Toggle {
    pub fn label(self) -> &'static str {
        match self {
            Toggle::Wifi => "Wi-Fi",
            Toggle::MobileData => tr("Mobile data"),
            Toggle::Bluetooth => "Bluetooth",
            Toggle::Airplane => tr("Airplane mode"),
            Toggle::Location => tr("Location"),
            Toggle::DarkMode => tr("Dark theme"),
            Toggle::AutoRotate => tr("Auto-rotate"),
            Toggle::AutoBrightness => tr("Auto brightness"),
            Toggle::BatterySaver => tr("Battery saver"),
            Toggle::DataSaver => tr("Data saver"),
            Toggle::NightLight => tr("Night Light"),
        }
    }

    /// Command that turns it on or off on this Android version; `None` if there
    /// is no way without root.
    pub fn command(self, on: bool, sdk: Option<i64>) -> Option<String> {
        let sdk = sdk.unwrap_or(0);
        let (en, ed) = if on { ("enable", "enabled") } else { ("disable", "disabled") };
        let (tf, one) = if on { ("true", "1") } else { ("false", "0") };
        Some(match self {
            // `cmd wifi` since Android 11; before that, `svc wifi`.
            Toggle::Wifi if sdk >= 30 => format!("cmd wifi set-wifi-enabled {ed}"),
            Toggle::Wifi => format!("svc wifi {en}"),
            // `cmd phone data` (Android 13+) acts on the data SIM.
            Toggle::MobileData if sdk >= 33 => format!("cmd phone data {en}"),
            Toggle::MobileData => format!("svc data {en}"),
            Toggle::Bluetooth if sdk >= 33 => format!("cmd bluetooth_manager {en}"),
            Toggle::Bluetooth => format!("svc bluetooth {en}"),
            // Before Android 11 a system broadcast was needed that the shell
            // user cannot send.
            Toggle::Airplane if sdk >= 30 => format!("cmd connectivity airplane-mode {en}"),
            Toggle::Airplane => return None,
            Toggle::Location if sdk >= 30 => format!("cmd location set-location-enabled {tf}"),
            Toggle::Location => format!("settings put secure location_mode {}", if on { 3 } else { 0 }),
            Toggle::DarkMode => format!("cmd uimode night {}", if on { "yes" } else { "no" }),
            Toggle::AutoRotate => format!("settings put system accelerometer_rotation {one}"),
            Toggle::AutoBrightness => format!("settings put system screen_brightness_mode {one}"),
            Toggle::BatterySaver if sdk >= 30 => format!("cmd power set-mode {one}"),
            Toggle::BatterySaver => format!("settings put global low_power {one}"),
            Toggle::DataSaver => format!("cmd netpolicy set restrict-background {tf}"),
            // Android's Night Light (warmer colours); its schedule, if any, is
            // kept.
            Toggle::NightLight => format!("settings put secure night_display_activated {one}"),
        })
    }

    /// State read for this switch.
    pub fn state(self, s: &ControlState) -> &Probe<bool> {
        match self {
            Toggle::Wifi => &s.wifi,
            Toggle::MobileData => &s.mobile_data,
            Toggle::Bluetooth => &s.bluetooth,
            Toggle::Airplane => &s.airplane,
            Toggle::Location => &s.location,
            Toggle::DarkMode => &s.dark_mode,
            Toggle::AutoRotate => &s.auto_rotate,
            Toggle::AutoBrightness => &s.auto_brightness,
            Toggle::BatterySaver => &s.battery_saver,
            Toggle::DataSaver => &s.data_saver,
            Toggle::NightLight => &s.night_light,
        }
    }

    /// Radio changes take a few seconds to show.
    pub fn settle_ms(self) -> u64 {
        match self {
            Toggle::Wifi | Toggle::MobileData | Toggle::Bluetooth | Toggle::Airplane => 4000,
            _ => 1500,
        }
    }
}

/// Values with a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Level {
    /// Manual brightness, 1–255.
    Brightness(i64),
    /// Time until the screen turns off, in ms.
    ScreenTimeout(i64),
    MediaVolume(i64),
    RingVolume(i64),
    Dnd(Dnd),
}

impl Level {
    /// Command with the value clamped to the range Android accepts.
    pub fn command(self, state: &ControlState) -> String {
        let clamp = |v: i64, vol: &Probe<Volume>, fallback_max: i64| {
            let (lo, hi) = vol.ok().map(|v| (v.min, v.max)).unwrap_or((0, fallback_max));
            v.clamp(lo, hi)
        };
        match self {
            Level::Brightness(v) => {
                format!("settings put system screen_brightness {}", v.clamp(1, 255))
            }
            Level::ScreenTimeout(ms) => format!("settings put system screen_off_timeout {}", ms.clamp(10_000, 86_400_000)),
            Level::MediaVolume(v) => format!("cmd media_session volume --stream 3 --set {}", clamp(v, &state.media_volume, 15)),
            Level::RingVolume(v) => format!("cmd media_session volume --stream 2 --set {}", clamp(v, &state.ring_volume, 7)),
            Level::Dnd(d) => format!("cmd notification set_dnd {}", d.arg()),
        }
    }
}

/// Changes a volume with the volume keys, as the user would, in a loop that
/// runs on the phone itself: press, wait and read until reaching `target`. It
/// stops if three presses in a row change nothing (limit, another sound
/// active…) and never goes beyond 40 presses.
///
/// On some ROMs (REDMAGIC) the first press only opens the volume panel and the
/// following ones need ~0.3 s between them: that is why it reads after each one
/// instead of computing how many are needed. It only carries numbers.
pub fn volume_keys_script(stream: u8, from: i64, target: i64) -> String {
    let (key, cmp) = if target > from { ("KEYCODE_VOLUME_UP", "-ge") } else { ("KEYCODE_VOLUME_DOWN", "-le") };
    let name = if stream == 2 { "RING" } else { "MUSIC" };
    format!(
        "last={from}; same=0; i=0
while [ $i -lt 40 ]; do
  input keyevent {key}; sleep 0.35
  v=$(cmd media_session volume --stream {stream} --get 2>/dev/null | grep -o 'volume is [0-9]*' | grep -o '[0-9]*$')
  [ -z \"$v\" ] && v=$(dumpsys audio 2>/dev/null | grep -A6 '^- STREAM_{name}:' | grep -o 'streamVolume:[0-9]*' | grep -o '[0-9]*$')
  [ -z \"$v\" ] && break
  if [ \"$v\" = \"$last\" ]; then same=$((same + 1)); [ $same -ge 3 ] && break; else same=0; fi
  last=$v
  [ $v {cmp} {target} ] && break
  i=$((i + 1))
done
echo \"@@@VOLUME@@@ $last\""
    )
}

/// Screen timeouts offered by Android.
pub const SCREEN_TIMEOUTS: [(i64, &str); 7] =
    [(15_000, "15 seconds"), (30_000, "30 seconds"), (60_000, "1 minute"), (120_000, "2 minutes"), (300_000, "5 minutes"), (600_000, "10 minutes"), (1_800_000, "30 minutes")];

/// Stateless actions: they open something on the phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    QuickSettings,
    Notifications,
    ClosePanels,
    ScreenOn,
    ScreenOff,
    /// Opens a Settings screen (public `android.settings.*` action).
    OpenSettings(&'static str),
}

impl Action {
    pub fn command(self) -> String {
        match self {
            Action::QuickSettings => "cmd statusbar expand-settings".into(),
            Action::Notifications => "cmd statusbar expand-notifications".into(),
            Action::ClosePanels => "cmd statusbar collapse".into(),
            Action::ScreenOn => "input keyevent KEYCODE_WAKEUP".into(),
            Action::ScreenOff => "input keyevent KEYCODE_SLEEP".into(),
            Action::OpenSettings(a) => format!("am start -a {a}"),
        }
    }
}

/// Settings screens that can be opened on the phone (name, action).
pub const SETTINGS_SCREENS: [(&str, &str); 18] = [
    ("Preferences", "android.settings.SETTINGS"),
    ("Wi-Fi", "android.settings.WIFI_SETTINGS"),
    ("Bluetooth", "android.settings.BLUETOOTH_SETTINGS"),
    ("Mobile network", "android.settings.NETWORK_OPERATOR_SETTINGS"),
    ("Hotspot and tethering", "android.settings.TETHER_SETTINGS"),
    ("NFC and connected devices", "android.settings.NFC_SETTINGS"),
    ("Data usage", "android.settings.DATA_USAGE_SETTINGS"),
    ("Location", "android.settings.LOCATION_SOURCE_SETTINGS"),
    ("Screen", "android.settings.DISPLAY_SETTINGS"),
    ("Sound", "android.settings.SOUND_SETTINGS"),
    ("Do Not Disturb", "android.settings.ZEN_MODE_SETTINGS"),
    ("Battery", "android.intent.action.POWER_USAGE_SUMMARY"),
    ("Storage", "android.settings.INTERNAL_STORAGE_SETTINGS"),
    ("Apps", "android.settings.APPLICATION_SETTINGS"),
    ("Security", "android.settings.SECURITY_SETTINGS"),
    ("Privacy", "android.settings.PRIVACY_SETTINGS"),
    ("Developer options", "android.settings.APPLICATION_DEVELOPMENT_SETTINGS"),
    ("About phone", "android.settings.DEVICE_INFO_SETTINGS"),
];

/// Private DNS (DNS over TLS, Android 9+): where the phone sends its name
/// lookups. A filtering server blocks ads and trackers in every app, without
/// root and without a VPN. Stored in `private_dns_mode` (`off`,
/// `opportunistic` or `hostname`) and `private_dns_specifier`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivateDns {
    Off,
    /// Android's default: encrypted when the network's DNS supports it.
    Automatic,
    /// A specific server.
    Host(String),
}

/// A well-known private DNS server.
pub struct DnsPreset {
    pub name: &'static str,
    pub host: &'static str,
    /// What it blocks.
    pub blurb: &'static str,
}

/// Servers offered (their DNS-over-TLS host names).
pub fn dns_presets() -> [DnsPreset; 6] {
    [
        DnsPreset { name: "AdGuard DNS", host: "dns.adguard-dns.com", blurb: tr("Blocks ads and trackers") },
        DnsPreset { name: "AdGuard Family", host: "family.adguard-dns.com", blurb: tr("Blocks ads, trackers and adult content") },
        DnsPreset { name: "Mullvad", host: "adblock.dns.mullvad.net", blurb: tr("Blocks ads and trackers") },
        DnsPreset { name: "Quad9", host: "dns.quad9.net", blurb: tr("Blocks dangerous sites, not ads") },
        DnsPreset { name: "Cloudflare", host: "one.one.one.one", blurb: tr("Fast, blocks nothing") },
        DnsPreset { name: "Cloudflare for Families", host: "family.cloudflare-dns.com", blurb: tr("Blocks dangerous sites and adult content") },
    ]
}

/// A host name Android accepts as private DNS: letters, digits, hyphens and
/// dots, at least one dot, no empty labels (also keeps anything else out of
/// the command).
pub fn is_valid_dns_host(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|label| !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-') && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
}

impl PrivateDns {
    /// From the two settings. Without a mode Android uses `opportunistic`.
    fn from_settings(mode: Option<&str>, host: Option<&str>) -> Self {
        match mode.map(str::trim) {
            Some("off") => PrivateDns::Off,
            Some("hostname") => match host.map(str::trim).filter(|h| !h.is_empty() && *h != "null") {
                Some(h) => PrivateDns::Host(h.to_string()),
                None => PrivateDns::Automatic,
            },
            _ => PrivateDns::Automatic,
        }
    }

    /// Commands (arguments for `adb shell`) that set it. The host goes first
    /// so the mode never points at an old one. `None` for an invalid host.
    pub fn commands(&self) -> Option<Vec<Vec<String>>> {
        let put = |key: &str, value: &str| vec!["settings".to_string(), "put".into(), "global".into(), key.into(), value.into()];
        Some(match self {
            PrivateDns::Off => vec![put("private_dns_mode", "off")],
            PrivateDns::Automatic => vec![put("private_dns_mode", "opportunistic")],
            PrivateDns::Host(h) if is_valid_dns_host(h) => vec![put("private_dns_specifier", h.trim_end_matches('.')), put("private_dns_mode", "hostname")],
            PrivateDns::Host(_) => return None,
        })
    }

    pub fn label(&self) -> String {
        match self {
            PrivateDns::Off => tr("Off").into(),
            PrivateDns::Automatic => tr("Automatic").into(),
            PrivateDns::Host(h) => dns_presets().iter().find(|p| p.host == h).map(|p| p.name.to_string()).unwrap_or_else(|| h.clone()),
        }
    }
}

/// A saved set of controls applied with one click (“Night”: Do Not Disturb,
/// dark theme, low brightness…). Taken from the phone's state when saved.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ControlProfile {
    pub name: String,
    pub toggles: Vec<(Toggle, bool)>,
    pub levels: Vec<Level>,
}

/// Switches a profile keeps. Airplane mode is left out: it would cut wireless
/// ADB with no way back.
pub const PROFILE_TOGGLES: [Toggle; 8] = [Toggle::Wifi, Toggle::MobileData, Toggle::Bluetooth, Toggle::Location, Toggle::DarkMode, Toggle::AutoRotate, Toggle::BatterySaver, Toggle::DataSaver];

impl ControlProfile {
    /// The phone's current switches, Do Not Disturb, brightness and screen
    /// timeout (what could be read).
    pub fn from_state(name: &str, s: &ControlState) -> Self {
        let mut toggles: Vec<(Toggle, bool)> = PROFILE_TOGGLES.iter().filter_map(|t| t.state(s).ok().map(|on| (*t, *on))).collect();
        let mut levels = Vec::new();
        if let Some(d) = s.dnd.ok() {
            levels.push(Level::Dnd(*d));
        }
        // Manual brightness only means something with automatic brightness off.
        match s.auto_brightness.ok() {
            Some(true) => toggles.push((Toggle::AutoBrightness, true)),
            Some(false) => {
                toggles.push((Toggle::AutoBrightness, false));
                if let Some(b) = s.brightness.ok() {
                    levels.push(Level::Brightness(*b));
                }
            }
            None => {}
        }
        if let Some(ms) = s.screen_timeout_ms.ok() {
            levels.push(Level::ScreenTimeout(*ms));
        }
        Self { name: name.trim().chars().take(40).collect(), toggles, levels }
    }

    /// One line: “Wi-Fi on · Bluetooth off · Do Not Disturb: priority only…”.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = self.toggles.iter().map(|(t, on)| format!("{} {}", t.label(), if *on { tr("on") } else { tr("off") })).collect();
        for l in &self.levels {
            parts.push(match l {
                Level::Dnd(d) => t!("Do Not Disturb: {}", d.label().to_lowercase()),
                Level::Brightness(b) => t!("Brightness: {}", b),
                Level::ScreenTimeout(ms) => t!("screen off after {}", crate::format::human_duration(Some(*ms as f64 / 1000.0))),
                Level::MediaVolume(v) => t!("Media volume: {}", v),
                Level::RingVolume(v) => t!("Ring volume: {}", v),
            });
        }
        parts.join(" · ")
    }

    /// What differs from the phone's state: the switches to change and the
    /// values to set.
    pub fn pending(&self, s: &ControlState) -> (Vec<(Toggle, bool)>, Vec<Level>) {
        let toggles = self.toggles.iter().filter(|(t, on)| t.state(s).ok() != Some(on)).copied().collect();
        let levels = self
            .levels
            .iter()
            .filter(|l| match l {
                Level::Dnd(d) => s.dnd.ok() != Some(d),
                Level::Brightness(b) => s.brightness.ok() != Some(b),
                Level::ScreenTimeout(ms) => s.screen_timeout_ms.ok() != Some(ms),
                Level::MediaVolume(v) => s.media_volume.ok().map(|x| x.value) != Some(*v),
                Level::RingVolume(v) => s.ring_volume.ok().map(|x| x.value) != Some(*v),
            })
            .copied()
            .collect();
        (toggles, levels)
    }
}

/// State of the controls. Each item with its availability.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ControlState {
    pub read_at: f64,
    pub sdk: Option<i64>,
    pub wifi: Probe<bool>,
    pub mobile_data: Probe<bool>,
    pub bluetooth: Probe<bool>,
    pub airplane: Probe<bool>,
    pub location: Probe<bool>,
    pub dark_mode: Probe<bool>,
    pub auto_rotate: Probe<bool>,
    pub auto_brightness: Probe<bool>,
    pub battery_saver: Probe<bool>,
    pub data_saver: Probe<bool>,
    /// Night Light (warmer colours at night).
    #[serde(default)]
    pub night_light: Probe<bool>,
    pub dnd: Probe<Dnd>,
    pub brightness: Probe<i64>,
    pub screen_timeout_ms: Probe<i64>,
    pub media_volume: Probe<Volume>,
    pub ring_volume: Probe<Volume>,
    /// Read-only: it cannot be changed without root.
    pub nfc: Probe<bool>,
    /// Read-only: some Wi-Fi interface is tethering.
    pub hotspot: Probe<bool>,
    /// Plugged in (battery saver does not turn on like this).
    pub charging: Option<bool>,
    /// Private DNS (Android 9+).
    pub private_dns: Probe<PrivateDns>,
    /// Readability and accessibility settings.
    pub access: Accessibility,
}

/// Settings that make the phone easier to read and use (for an older relative,
/// for low vision): all of them are in Settings → Accessibility and are
/// changed with `settings put`, so they can always be undone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Accessibility {
    /// Text size (`font_scale`, 1.0 = normal).
    pub font_scale: Probe<f64>,
    /// Bold text (`font_weight_adjustment`, 300 = bold, Android 12+).
    pub bold_text: Probe<bool>,
    pub high_contrast_text: Probe<bool>,
    pub color_inversion: Probe<bool>,
    /// Grayscale (colour correction set to monochromacy).
    pub grayscale: Probe<bool>,
    /// Touch and hold delay (`long_press_timeout`, ms; 400 by default).
    pub long_press_ms: Probe<i64>,
    /// Remove animations (`accessibility_disable_animations`, Android 13+
    /// setting read by the system UI).
    pub remove_animations: Probe<bool>,
}

/// An accessibility switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccessToggle {
    BoldText,
    HighContrastText,
    ColorInversion,
    Grayscale,
}

impl AccessToggle {
    pub const ALL: [AccessToggle; 4] = [AccessToggle::BoldText, AccessToggle::HighContrastText, AccessToggle::ColorInversion, AccessToggle::Grayscale];

    pub fn label(self) -> &'static str {
        match self {
            AccessToggle::BoldText => tr("Bold text"),
            AccessToggle::HighContrastText => tr("High contrast text"),
            AccessToggle::ColorInversion => tr("Colour inversion"),
            AccessToggle::Grayscale => tr("Grayscale"),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            AccessToggle::BoldText => tr("Thicker letters everywhere"),
            AccessToggle::HighContrastText => tr("Black or white text with an outline"),
            AccessToggle::ColorInversion => tr("Light backgrounds become dark"),
            AccessToggle::Grayscale => tr("No colours: fewer distractions, easier on the eyes at night"),
        }
    }

    /// `settings put` arguments that turn it on or off.
    pub fn commands(self, on: bool) -> Vec<Vec<String>> {
        let put = |ns: &str, key: &str, value: &str| vec!["settings".to_string(), "put".into(), ns.into(), key.into(), value.into()];
        let one = if on { "1" } else { "0" };
        match self {
            AccessToggle::BoldText => vec![put("secure", "font_weight_adjustment", if on { "300" } else { "0" })],
            AccessToggle::HighContrastText => vec![put("secure", "high_text_contrast_enabled", one)],
            AccessToggle::ColorInversion => vec![put("secure", "accessibility_display_inversion_enabled", one)],
            // Monochromacy is daltonizer mode 0.
            AccessToggle::Grayscale if on => vec![put("secure", "accessibility_display_daltonizer", "0"), put("secure", "accessibility_display_daltonizer_enabled", "1")],
            AccessToggle::Grayscale => vec![put("secure", "accessibility_display_daltonizer_enabled", "0")],
        }
    }

    pub fn state(self, a: &Accessibility) -> &Probe<bool> {
        match self {
            AccessToggle::BoldText => &a.bold_text,
            AccessToggle::HighContrastText => &a.high_contrast_text,
            AccessToggle::ColorInversion => &a.color_inversion,
            AccessToggle::Grayscale => &a.grayscale,
        }
    }
}

/// Text sizes offered (as in Settings: small to largest).
pub const FONT_SCALES: [(f64, &str); 6] = [(0.85, "Small"), (1.0, "Default"), (1.15, "Large"), (1.3, "Larger"), (1.5, "Largest"), (1.8, "Huge")];

/// Touch and hold delays offered (Settings: short, medium, long).
pub const LONG_PRESS: [(i64, &str); 3] = [(400, "Short"), (1000, "Medium"), (1500, "Long")];

/// `settings put` for the text size, clamped to what Android allows.
pub fn font_scale_command(scale: f64) -> Vec<String> {
    let v = scale.clamp(0.5, 2.0);
    vec!["settings".into(), "put".into(), "system".into(), "font_scale".into(), format!("{v:.2}")]
}

pub fn long_press_command(ms: i64) -> Vec<String> {
    vec!["settings".into(), "put".into(), "secure".into(), "long_press_timeout".into(), ms.clamp(300, 3000).to_string()]
}

/// `settings list …`: `key=value` per line. Some values span several lines
/// (JSON from some manufacturers): only lines with a valid key count.
pub fn parse_settings_list(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            let valid = !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-');
            valid.then(|| (k.to_string(), v.trim().to_string()))
        })
        .collect()
}

/// `[V] volume is 40 in range [0..150]`
pub fn parse_volume(text: &str) -> Option<Volume> {
    let Some(line) = text.lines().find(|l| l.contains("volume is ")) else {
        return parse_audio_stream(text);
    };
    let rest = line.split("volume is ").nth(1)?;
    let mut it = rest.split_whitespace();
    let value = it.next()?.parse().ok()?;
    let range = rest.split('[').nth(1)?.trim_end_matches(']');
    let (lo, hi) = range.split_once("..")?;
    Some(Volume { value, min: lo.trim().parse().ok()?, max: hi.trim().trim_end_matches(']').parse().ok()? })
}

/// Android 11 answers nothing to `volume --get`: the stream's block of
/// `dumpsys audio` (`Min: 0`, `Max: 15`, `streamVolume:5`).
fn parse_audio_stream(text: &str) -> Option<Volume> {
    let field = |key: &str| text.lines().find_map(|l| l.trim().strip_prefix(key)).and_then(|v| v.trim().parse::<i64>().ok());
    Some(Volume { value: field("streamVolume:")?, min: field("Min:")?, max: field("Max:")? })
}

pub fn parse_state(text: &str) -> ControlState {
    let s = crate::parsers::split_sections(text);
    let empty = String::new();
    let global = parse_settings_list(s.get("GLOBAL").unwrap_or(&empty));
    let system = parse_settings_list(s.get("SYSTEM").unwrap_or(&empty));
    let secure = parse_settings_list(s.get("SECURE").unwrap_or(&empty));
    let setting = |map: &HashMap<String, String>, key: &str| -> Probe<String> {
        match map.get(key).filter(|v| !v.is_empty() && *v != "null") {
            Some(v) => Probe::Ok(v.clone()),
            None if map.is_empty() => Probe::Failed(tr("could not read the settings").into()),
            None => Probe::Missing,
        }
    };
    let flag = |p: Probe<String>| p.map(|v| v == "1");

    // Mobile data: the setting is per SIM (`mobile_data<subId>`); the global
    // `mobile_data` does not always reflect the data SIM.
    let mobile_data = match global.get("multi_sim_data_call").and_then(|sub| global.get(&format!("mobile_data{sub}"))) {
        Some(v) => Probe::Ok(v == "1"),
        None => flag(setting(&global, "mobile_data")),
    };
    let zen = setting(&global, "zen_mode");
    ControlState {
        read_at: crate::models::now_secs(),
        sdk: s.get("SDK").and_then(|v| v.trim().parse().ok()),
        wifi: match probe::from_section(s.get("WIFI"), |t| {
            let t = t.to_lowercase();
            if t.contains("is enabled") {
                Some(true)
            } else if t.contains("is disabled") {
                Some(false)
            } else {
                None
            }
        }) {
            Probe::Ok(v) => Probe::Ok(v),
            // Android < 11: without `cmd wifi status`, the setting (1 and 2 =
            // on).
            _ => setting(&global, "wifi_on").map(|v| v == "1" || v == "2"),
        },
        mobile_data,
        bluetooth: flag(setting(&global, "bluetooth_on")),
        airplane: match probe::from_section(s.get("AIRPLANE"), |t| match t.trim() {
            "enabled" => Some(true),
            "disabled" => Some(false),
            _ => None,
        }) {
            Probe::Ok(v) => Probe::Ok(v),
            _ => flag(setting(&global, "airplane_mode_on")),
        },
        location: match probe::from_section(s.get("LOCATION"), |t| t.trim().parse::<bool>().ok()) {
            Probe::Ok(v) => Probe::Ok(v),
            _ => setting(&secure, "location_mode").map(|v| v != "0"),
        },
        dark_mode: probe::from_section(s.get("NIGHT"), |t| {
            let v = t.trim().strip_prefix("Night mode:")?.trim();
            match v {
                "yes" => Some(true),
                "no" => Some(false),
                // auto / custom: depends on the time; it is neither yes nor no.
                _ => None,
            }
        }),
        auto_rotate: flag(setting(&system, "accelerometer_rotation")),
        // Not written until it is first used: off.
        night_light: match setting(&secure, "night_display_activated") {
            Probe::Missing => Probe::Ok(false),
            other => flag(other),
        },
        auto_brightness: flag(setting(&system, "screen_brightness_mode")),
        battery_saver: flag(setting(&global, "low_power")),
        data_saver: probe::from_section(s.get("DATASAVER"), |t| {
            if t.contains("enabled") {
                Some(true)
            } else if t.contains("disabled") {
                Some(false)
            } else {
                None
            }
        }),
        dnd: match zen {
            Probe::Ok(v) => Dnd::from_zen(&v).map(Probe::Ok).unwrap_or(Probe::Missing),
            other => other.map(|_| Dnd::Off),
        },
        brightness: setting(&system, "screen_brightness").and_then_parse(),
        screen_timeout_ms: setting(&system, "screen_off_timeout").and_then_parse(),
        media_volume: probe::from_section(s.get("MEDIAVOL"), parse_volume),
        ring_volume: probe::from_section(s.get("RINGVOL"), parse_volume),
        nfc: probe::from_section(s.get("NFC"), |t| {
            let v = t.trim().strip_prefix("mState=")?.trim().to_lowercase();
            match v.as_str() {
                "on" => Some(true),
                "off" => Some(false),
                // turning_on / turning_off: in transition.
                _ => None,
            }
        }),
        hotspot: probe::from_section(s.get("TETHER"), |t| {
            t.contains("Tether state:").then(|| {
                t.lines().any(|l| {
                    let l = l.trim();
                    (l.starts_with("wlan") || l.starts_with("softap") || l.starts_with("ap_br")) && l.contains(" - ") && l.contains("Tethered")
                })
            })
        }),
        charging: s.get("BATTERY").map(|t| t.lines().any(|l| l.contains("powered: true"))),
        private_dns: if global.is_empty() {
            Probe::Failed(tr("could not read the settings").into())
        } else if s.get("SDK").and_then(|v| v.trim().parse::<i64>().ok()).is_some_and(|sdk| sdk < 28) {
            Probe::Unsupported(tr("Android 9 or later").into())
        } else {
            Probe::Ok(PrivateDns::from_settings(global.get("private_dns_mode").map(String::as_str), global.get("private_dns_specifier").map(String::as_str)))
        },
        access: parse_accessibility(&system, &secure),
    }
}

/// Readability settings. A missing key is the default (off, normal size).
fn parse_accessibility(system: &HashMap<String, String>, secure: &HashMap<String, String>) -> Accessibility {
    if system.is_empty() || secure.is_empty() {
        fn failed<T>() -> Probe<T> {
            Probe::Failed(tr("could not read the settings").into())
        }
        return Accessibility { font_scale: failed(), bold_text: failed(), high_contrast_text: failed(), color_inversion: failed(), grayscale: failed(), long_press_ms: failed(), remove_animations: failed() };
    }
    let get = |map: &HashMap<String, String>, key: &str| map.get(key).map(|v| v.trim().to_string()).filter(|v| !v.is_empty() && v != "null");
    let on = |map: &HashMap<String, String>, key: &str| Probe::Ok(get(map, key).is_some_and(|v| v == "1"));
    Accessibility {
        font_scale: Probe::Ok(get(system, "font_scale").and_then(|v| v.parse().ok()).unwrap_or(1.0)),
        bold_text: Probe::Ok(get(secure, "font_weight_adjustment").and_then(|v| v.parse::<i64>().ok()).is_some_and(|w| w > 0)),
        high_contrast_text: on(secure, "high_text_contrast_enabled"),
        color_inversion: on(secure, "accessibility_display_inversion_enabled"),
        grayscale: Probe::Ok(get(secure, "accessibility_display_daltonizer_enabled").is_some_and(|v| v == "1") && get(secure, "accessibility_display_daltonizer").map_or(true, |m| m == "0")),
        long_press_ms: Probe::Ok(get(secure, "long_press_timeout").and_then(|v| v.parse().ok()).unwrap_or(400)),
        remove_animations: on(secure, "accessibility_disable_animations"),
    }
}

trait ParseProbe {
    fn and_then_parse(self) -> Probe<i64>;
}

impl ParseProbe for Probe<String> {
    fn and_then_parse(self) -> Probe<i64> {
        match self {
            Probe::Ok(v) => v.trim().parse().map(Probe::Ok).unwrap_or(Probe::Missing),
            other => other.map(|_| 0),
        }
    }
}

/// Message when Android did not apply a requested change.
pub fn not_applied_hint(toggle: Toggle, on: bool, state: &ControlState) -> String {
    match toggle {
        Toggle::BatterySaver if on && state.charging == Some(true) => tr("Android does not turn on battery saver while the phone is charging").into(),
        Toggle::MobileData if state.airplane.ok() == Some(&true) => tr("With airplane mode on there is no mobile data").into(),
        _ => t!("The phone did not apply the change to “{}”", toggle.label()),
    }
}
