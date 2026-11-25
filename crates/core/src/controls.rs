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
echo @@@MEDIAVOL@@@; cmd media_session volume --stream 3 --get 2>&1
echo @@@RINGVOL@@@; cmd media_session volume --stream 2 --get 2>&1
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
    format!(
        "last={from}; same=0; i=0
while [ $i -lt 40 ]; do
  input keyevent {key}; sleep 0.35
  v=$(cmd media_session volume --stream {stream} --get 2>/dev/null | grep -o 'volume is [0-9]*' | grep -o '[0-9]*$')
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
    let line = text.lines().find(|l| l.contains("volume is "))?;
    let rest = line.split("volume is ").nth(1)?;
    let mut it = rest.split_whitespace();
    let value = it.next()?.parse().ok()?;
    let range = rest.split('[').nth(1)?.trim_end_matches(']');
    let (lo, hi) = range.split_once("..")?;
    Some(Volume { value, min: lo.trim().parse().ok()?, max: hi.trim().trim_end_matches(']').parse().ok()? })
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
