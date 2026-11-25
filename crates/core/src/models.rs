//! Data structures shared between the ADB engine and the interface.
//!
//! Every field Android may not expose is an `Option`: the interface shows “—”
//! instead of making up values.

use crate::{i18n::{tr, trc}, t};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::format;

pub fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Android tables
// ---------------------------------------------------------------------------

pub fn battery_status_label(code: Option<i64>) -> &'static str {
    match code.unwrap_or(0) {
        2 => tr("Charging"),
        3 => tr("Downloading"),
        4 => tr("Not charging"),
        5 => tr("Full"),
        _ => tr("Unknown"),
    }
}

pub fn battery_health_label(code: Option<i64>) -> &'static str {
    match code.unwrap_or(0) {
        2 => tr("Good"),
        3 => tr("Overheating"),
        4 => tr("Dead"),
        5 => tr("Overvoltage"),
        6 => tr("Unspecified failure"),
        7 => tr("Cold"),
        _ => tr("Unknown"),
    }
}

/// `android.os.TemperatureType`
/// Types of `android.os.Temperature` (TYPE_*), according to AOSP.
pub fn temperature_type_label(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "CPU",
        1 => "GPU",
        2 => tr("Battery"),
        3 => tr("Skin"),
        4 => tr("USB port"),
        5 => tr("Power amplifier"),
        6 => tr("Battery voltage"),
        7 => tr("Battery current"),
        8 => tr("Battery charge"),
        9 => "NPU",
        10 => "TPU",
        11 => tr("Screen"),
        12 => tr("Modem"),
        13 => "SoC",
        14 => "Wi-Fi",
        15 => tr("Camera"),
        16 => "Flash",
        17 => tr("Speaker"),
        18 => tr("Ambient"),
        19 => tr("POGO connector"),
        _ => return None,
    })
}

pub fn thermal_status_label(code: i64) -> String {
    match code {
        0 => "Normal".into(),
        1 => trc("thermal", "Light").into(),
        2 => tr("Moderate").into(),
        3 => tr("Severe").into(),
        4 => tr("Critical").into(),
        5 => tr("Shutdown").into(),
        6 => tr("Emergency").into(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Device,
    Unauthorized,
    Offline,
    NoPermissions,
    Recovery,
    Sideload,
    Bootloader,
    /// Device in fastboot mode detected with `fastboot devices`.
    Fastboot,
    /// Fastbootd (userspace fastboot, Android 10+).
    Fastbootd,
    Authorizing,
    #[default]
    Unknown,
}

impl DeviceState {
    pub fn parse(text: &str) -> Self {
        match text {
            "device" => Self::Device,
            "unauthorized" => Self::Unauthorized,
            "offline" => Self::Offline,
            "recovery" => Self::Recovery,
            "sideload" => Self::Sideload,
            "bootloader" => Self::Bootloader,
            "authorizing" => Self::Authorizing,
            "fastboot" => Self::Fastboot,
            "fastbootd" => Self::Fastbootd,
            t if t.starts_with("no") => Self::NoPermissions,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Unauthorized => "unauthorized",
            Self::Offline => "offline",
            Self::NoPermissions => "no permissions",
            Self::Recovery => "recovery",
            Self::Sideload => "sideload",
            Self::Bootloader => "bootloader",
            Self::Fastboot => "fastboot",
            Self::Fastbootd => "fastbootd",
            Self::Authorizing => "authorizing",
            Self::Unknown => "unknown",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Device => tr("Connected"),
            Self::Unauthorized => tr("Waiting for authorisation"),
            Self::Offline => tr("Offline"),
            Self::NoPermissions => tr("No permissions"),
            Self::Recovery => "Recovery",
            Self::Sideload => "Sideload",
            Self::Bootloader => "Bootloader",
            Self::Fastboot => "Fastboot",
            Self::Fastbootd => "Fastbootd",
            Self::Authorizing => tr("Authorising"),
            Self::Unknown => tr("Unknown"),
        }
    }

    /// States in which the device responds to `fastboot`.
    pub fn is_fastboot(self) -> bool {
        matches!(self, Self::Fastboot | Self::Fastbootd | Self::Bootloader)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    #[default]
    Usb,
    Tcp,
    Emulator,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AdbDevice {
    pub serial: String,
    pub state: DeviceState,
    /// Original state when ADB returns an unknown one (diagnosis).
    pub raw_state: String,
    pub product: String,
    pub model: String,
    pub device: String,
    pub transport_id: String,
    pub usb: String,
    pub transport: Transport,
}

impl AdbDevice {
    pub fn new(serial: impl Into<String>, state: DeviceState) -> Self {
        Self { serial: serial.into(), state, ..Default::default() }
    }

    pub fn connected(&self) -> bool {
        self.state == DeviceState::Device
    }

    pub fn display_name(&self) -> String {
        for value in [&self.model, &self.product] {
            if !value.is_empty() && value != "unknown" {
                return value.replace('_', " ");
            }
        }
        self.serial.clone()
    }

    pub fn state_label(&self) -> String {
        if self.state == DeviceState::Unknown && !self.raw_state.is_empty() {
            return self.raw_state.clone();
        }
        self.state.label().to_string()
    }

    pub fn is_usb(&self) -> bool {
        self.transport == Transport::Usb || !self.usb.is_empty()
    }

    pub fn is_wireless(&self) -> bool {
        self.transport == Transport::Tcp
    }
}

// ---------------------------------------------------------------------------
// Battery
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatteryInfo {
    pub level: Option<i64>,
    pub scale: i64,
    pub status_code: Option<i64>,
    pub health_code: Option<i64>,
    pub present: Option<bool>,
    pub voltage_mv: Option<i64>,
    pub temperature_c: Option<f64>,
    pub technology: String,
    pub ac_powered: bool,
    pub usb_powered: bool,
    pub wireless_powered: bool,
    pub dock_powered: bool,
    pub charge_counter_mah: Option<i64>,
    pub max_charging_current_ua: Option<i64>,
    pub max_charging_voltage_uv: Option<i64>,
    pub capacity_level: Option<i64>,
    pub charging_state: String,
    pub extra: BTreeMap<String, String>,
}

impl Default for BatteryInfo {
    fn default() -> Self {
        Self {
            level: None,
            scale: 100,
            status_code: None,
            health_code: None,
            present: None,
            voltage_mv: None,
            temperature_c: None,
            technology: String::new(),
            ac_powered: false,
            usb_powered: false,
            wireless_powered: false,
            dock_powered: false,
            charge_counter_mah: None,
            max_charging_current_ua: None,
            max_charging_voltage_uv: None,
            capacity_level: None,
            charging_state: String::new(),
            extra: BTreeMap::new(),
        }
    }
}

impl BatteryInfo {
    pub fn percentage(&self) -> Option<f64> {
        let level = self.level? as f64;
        let scale = if self.scale == 0 { 100 } else { self.scale } as f64;
        Some((level / scale * 100.0).clamp(0.0, 100.0))
    }

    pub fn status(&self) -> &'static str {
        battery_status_label(self.status_code)
    }

    pub fn health(&self) -> &'static str {
        battery_health_label(self.health_code)
    }

    pub fn is_charging(&self) -> bool {
        self.status_code == Some(2) || self.ac_powered || self.usb_powered || self.wireless_powered
    }

    pub fn power_source(&self) -> String {
        let mut sources = Vec::new();
        if self.ac_powered {
            sources.push(tr("AC"));
        }
        if self.usb_powered {
            sources.push("USB");
        }
        if self.wireless_powered {
            sources.push(tr("Wireless"));
        }
        if self.dock_powered {
            sources.push(tr("Dock"));
        }
        sources.join(", ")
    }

    /// Maximum negotiated charging power in watts, if Android exposes it.
    pub fn max_charging_watts(&self) -> Option<f64> {
        let current = self.max_charging_current_ua? as f64 / 1_000_000.0;
        let voltage = self.max_charging_voltage_uv? as f64 / 1_000_000.0;
        let watts = current * voltage;
        (watts > 0.0).then_some(watts)
    }
}

// ---------------------------------------------------------------------------
// Component health
// ---------------------------------------------------------------------------

/// Where the battery capacity comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapacitySource {
    #[default]
    None,
    /// `/sys/class/power_supply/battery` (what the charging chip measures).
    Sysfs,
    /// `dumpsys batterystats` (capacity learned by Android).
    BatteryStats,
}

/// Storage wear reading.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageWear {
    /// No descriptor (UFS/eMMC) was found in sysfs.
    #[default]
    Unknown,
    /// The descriptor exists but the `shell` user cannot read it.
    Denied { kind: String },
    /// JEDEC levels: `life` 1–10 = used in 10 % steps, 11 = exhausted; `eol` 1
    /// normal, 2 warning (80 % of reserved blocks), 3 urgent.
    Read { kind: String, life: Option<u8>, eol: Option<u8> },
}

/// Battery and storage wear. Only data the phone exposes: without it, each
/// percentage is `None` and the interface shows “—”.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ComponentHealth {
    /// At least one reading arrived (the section is static: every ~10 s).
    pub probed: bool,
    pub battery_cycles: Option<i64>,
    pub battery_full_mah: Option<i64>,
    pub battery_design_mah: Option<i64>,
    pub battery_source: CapacitySource,
    pub storage: StorageWear,
}

impl ComponentHealth {
    /// Real capacity compared to the design capacity. Discards inconsistent
    /// readings (uncalibrated sensors return 0, huge values or the design
    /// capacity).
    pub fn battery_percent(&self) -> Option<f64> {
        let full = self.battery_full_mah? as f64;
        let design = self.battery_design_mah? as f64;
        if !(500.0..=30_000.0).contains(&design) || full <= 0.0 {
            return None;
        }
        let ratio = full / design;
        (0.3..=1.3).contains(&ratio).then(|| (ratio * 100.0).min(100.0))
    }

    pub fn battery_level(&self) -> Option<HealthLevel> {
        self.battery_percent().map(|p| match p {
            p if p >= 80.0 => HealthLevel::Good,
            p if p >= 70.0 => HealthLevel::Fair,
            _ => HealthLevel::Poor,
        })
    }

    /// Remaining storage life as a range (min., max.) in %. JEDEC only gives 10
    /// % steps: an exact value is not made up.
    pub fn storage_range(&self) -> Option<(u8, u8)> {
        match &self.storage {
            StorageWear::Read { life: Some(l), .. } if (1..=10).contains(l) => Some((100 - l * 10, 110 - l * 10)),
            StorageWear::Read { life: Some(11), .. } => Some((0, 0)),
            _ => None,
        }
    }

    pub fn storage_level(&self) -> Option<HealthLevel> {
        let eol = match &self.storage {
            StorageWear::Read { eol, .. } => *eol,
            _ => None,
        };
        let by_eol = match eol {
            Some(3) => Some(HealthLevel::Poor),
            Some(2) => Some(HealthLevel::Fair),
            Some(1) => Some(HealthLevel::Good),
            _ => None,
        };
        let by_life = self.storage_range().map(|(low, _)| match low {
            l if l >= 30 => HealthLevel::Good,
            l if l >= 10 => HealthLevel::Fair,
            _ => HealthLevel::Poor,
        });
        match (by_eol, by_life) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }

    /// Text of the range: “90–100 %”, “Exhausted” or “—”.
    pub fn storage_label(&self) -> String {
        match self.storage_range() {
            Some((0, 0)) => tr("Exhausted").into(),
            Some((low, high)) => format!("{low}–{high} %"),
            None => "—".into(),
        }
    }

    /// Why there is no storage data (or the block reserve).
    pub fn storage_detail(&self) -> String {
        match &self.storage {
            StorageWear::Unknown => tr("The phone does not expose the wear").into(),
            StorageWear::Denied { kind } => t!("{}: reading restricted to root", kind),
            StorageWear::Read { kind, eol, .. } => {
                let reserve = match eol {
                    Some(1) => tr("normal reserve"),
                    Some(2) => tr("reserve at 80 %"),
                    Some(3) => tr("reserve almost exhausted"),
                    _ => tr("estimated lifetime"),
                };
                format!("{kind} · {reserve}")
            }
        }
    }
}

/// Rating of a health percentage (ordered from best to worst).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HealthLevel {
    Good,
    Fair,
    Poor,
}

impl HealthLevel {
    pub fn label(self) -> &'static str {
        match self {
            HealthLevel::Good => tr("Good condition"),
            HealthLevel::Fair => tr("Noticeable wear"),
            HealthLevel::Poor => tr("High wear"),
        }
    }
}

// ---------------------------------------------------------------------------
// Memory and storage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub total_kib: Option<i64>,
    pub free_kib: Option<i64>,
    pub available_kib: Option<i64>,
    pub buffers_kib: Option<i64>,
    pub cached_kib: Option<i64>,
    pub swap_total_kib: Option<i64>,
    pub swap_free_kib: Option<i64>,
    pub zram_total_kib: Option<i64>,
    pub zram_free_kib: Option<i64>,
    pub oom_kill_kib: Option<i64>,
}

impl MemoryInfo {
    pub fn used_kib(&self) -> Option<i64> {
        let total = self.total_kib?;
        let available = self.available_kib.or(self.free_kib)?;
        Some((total - available).max(0))
    }

    pub fn percent(&self) -> Option<f64> {
        let used = self.used_kib()?;
        let total = self.total_kib.filter(|t| *t > 0)?;
        Some(used as f64 / total as f64 * 100.0)
    }

    pub fn swap_used_kib(&self) -> Option<i64> {
        Some((self.swap_total_kib? - self.swap_free_kib?).max(0))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StorageInfo {
    pub label: String,
    pub mount: String,
    pub total_kib: Option<i64>,
    pub used_kib: Option<i64>,
    pub available_kib: Option<i64>,
    pub filesystem: String,
    pub use_percent: Option<i64>,
}

impl StorageInfo {
    pub fn percent(&self) -> Option<f64> {
        if let Some(total) = self.total_kib.filter(|t| *t > 0) {
            if let Some(used) = self.used_kib {
                return Some(used as f64 / total as f64 * 100.0);
            }
            if let Some(avail) = self.available_kib {
                return Some((total - avail) as f64 / total as f64 * 100.0);
            }
        }
        self.use_percent.map(|p| p as f64)
    }
}

// ---------------------------------------------------------------------------
// Thermals and CPU
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Temperature {
    pub name: String,
    pub value_c: f64,
    pub type_code: Option<i64>,
    pub status_code: Option<i64>,
}

impl Temperature {
    /// BCL types (6, 7, 8) are the battery's voltage, current and percentage,
    /// not temperatures.
    pub fn is_temperature(&self) -> bool {
        !matches!(self.type_code, Some(6..=8))
    }

    /// Thresholds (°C) from which it is worth watching and worrying. Chips
    /// withstand much more than the battery or the casing.
    pub fn thresholds(&self) -> (f64, f64) {
        match self.type_code {
            Some(0 | 1 | 9 | 10 | 13) => (85.0, 100.0),
            Some(2) => (40.0, 45.0),
            Some(3 | 4) => (40.0, 45.0),
            _ => match self.kind() {
                "CPU" | "GPU" => (85.0, 100.0),
                k if k == tr("Battery") || k == tr("Skin") => (40.0, 45.0),
                _ => (55.0, 70.0),
            },
        }
    }

    pub fn kind(&self) -> &'static str {
        if let Some(label) = self.type_code.and_then(temperature_type_label) {
            return label;
        }
        let lowered = self.name.to_lowercase();
        if lowered.contains("cpu") {
            "CPU"
        } else if lowered.contains("gpu") {
            "GPU"
        } else if lowered.contains("batt") {
            tr("Battery")
        } else if lowered.contains("skin") {
            tr("Skin")
        } else {
            tr("Other")
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CpuCore {
    pub index: usize,
    pub cur_khz: Option<i64>,
    pub min_khz: Option<i64>,
    pub max_khz: Option<i64>,
    pub governor: String,
}

impl CpuCore {
    /// Current frequency relative to the core's range (0–100).
    pub fn load_hint(&self) -> Option<f64> {
        let cur = self.cur_khz? as f64;
        let max = self.max_khz.filter(|m| *m > 0)? as f64;
        let min = self.min_khz.unwrap_or(0) as f64;
        if max <= min {
            return None;
        }
        Some(((cur - min) / (max - min) * 100.0).clamp(0.0, 100.0))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CpuInfo {
    pub hardware: String,
    pub model_name: String,
    pub abi: String,
    pub cores: Vec<CpuCore>,
    pub load_percent: Option<f64>,
    pub loadavg: Option<(f64, f64, f64)>,
    pub core_count: Option<usize>,
    /// (active, total) in jiffies from /proc/stat.
    pub times: Option<(i64, i64)>,
}

impl CpuInfo {
    pub fn max_freq_khz(&self) -> Option<i64> {
        self.cores.iter().filter_map(|c| c.max_khz).filter(|v| *v > 0).max()
    }

    pub fn cur_freq_khz(&self) -> Option<i64> {
        self.cores.iter().filter_map(|c| c.cur_khz).filter(|v| *v > 0).max()
    }
}

// ---------------------------------------------------------------------------
// USB, display, network
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsbInfo {
    pub connected: Option<bool>,
    pub configured: Option<bool>,
    pub functions: String,
    pub config: String,
    pub state: String,
    pub speed: String,
    pub charging: Option<bool>,
    pub host_connected: Option<bool>,
}

impl UsbInfo {
    fn all_text(&self) -> String {
        format!("{},{},{}", self.functions, self.config, self.state).to_lowercase()
    }

    pub fn adb_enabled(&self) -> bool {
        self.all_text().contains("adb")
    }

    pub fn label(&self) -> String {
        match self.connected {
            Some(false) => tr("Disconnected").into(),
            Some(true) => {
                let fc = format!("{},{}", self.functions, self.config).to_lowercase();
                let mut parts = Vec::new();
                if self.adb_enabled() {
                    parts.push("ADB");
                }
                if fc.contains("mtp") {
                    parts.push("MTP");
                }
                if fc.contains("ptp") {
                    parts.push("PTP");
                }
                if parts.is_empty() {
                    tr("Connected").into()
                } else {
                    t!("Connected ({})", parts.join(" + "))
                }
            }
            None if !self.state.is_empty() => self.state.clone(),
            None => tr("Unknown").into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub density_dpi: Option<i64>,
    pub density_bucket: String,
    pub refresh_hz: Option<f64>,
    pub supported_refresh: Vec<f64>,
    pub hdr_types: Vec<i64>,
    pub brightness: Option<i64>,
    pub size_override: String,
    pub density_override: String,
    /// `ON`, `OFF`, `DOZE`… (`mScreenState`); empty if unknown.
    #[serde(default)]
    pub screen_state: String,
    /// Rate chosen in the phone's settings (“90 Hz”, “Automatic”…).
    #[serde(default)]
    pub refresh_setting: String,
}

impl DisplayInfo {
    /// “90 Hz”, “60 Hz · set to 90 Hz”, “60 Hz · screen dozing”.
    pub fn refresh_text(&self) -> String {
        let Some(hz) = self.refresh_hz else {
            return String::new();
        };
        let mut out = format!("{hz:.0} Hz");
        let idle = match self.screen_state.as_str() {
            "OFF" => Some(tr("screen off")),
            s if s.starts_with("DOZE") => Some(tr("screen dozing")),
            _ => None,
        };
        let differs = !self.refresh_setting.is_empty() && !self.refresh_setting.starts_with(&format!("{hz:.0} "));
        if differs {
            out.push_str(&t!(" · set to {}", self.refresh_setting));
        }
        if let Some(i) = idle {
            out.push_str(&format!(" · {i}"));
        }
        out
    }
}

impl DisplayInfo {
    pub fn resolution(&self) -> String {
        match (self.width, self.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => format!("{w} × {h}"),
            _ => "—".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpAddress {
    pub family: String,
    pub address: String,
    pub prefix: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetInterface {
    pub name: String,
    pub flags: String,
    pub mtu: Option<i64>,
    pub addresses: Vec<IpAddress>,
    pub mac: String,
    pub state: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub destination: String,
    pub dev: Option<String>,
    pub via: Option<String>,
    pub src: Option<String>,
    pub raw: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub interfaces: Vec<NetInterface>,
    pub routes: Vec<Route>,
    pub timestamp: f64,
}

impl NetworkInfo {
    /// First IPv4 of wlan*, useful for wireless ADB.
    pub fn wifi_ipv4(&self) -> Option<String> {
        self.interfaces.iter().filter(|i| i.name.starts_with("wlan")).flat_map(|i| i.addresses.iter()).find(|a| a.family == "IPv4").map(|a| a.address.clone())
    }
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub serial: String,
    pub timestamp: f64,
    pub props: BTreeMap<String, String>,
    pub battery: BatteryInfo,
    pub memory: MemoryInfo,
    pub data_storage: StorageInfo,
    pub external_storage: StorageInfo,
    pub temperatures: Vec<Temperature>,
    pub thermal_status: Option<i64>,
    pub cpu: CpuInfo,
    pub usb: UsbInfo,
    pub display: DisplayInfo,
    pub network: NetworkInfo,
    /// Wi-Fi status (`cmd wifi status`, every second in the telemetry).
    #[serde(default)]
    pub wifi: Option<WifiStatus>,
    /// Battery and storage wear (static section, every ~10 s).
    #[serde(default)]
    pub health: ComponentHealth,
    pub uptime_seconds: Option<f64>,
    pub errors: Vec<String>,
}

impl Snapshot {
    pub fn new(serial: impl Into<String>) -> Self {
        Self { serial: serial.into(), timestamp: now_secs(), ..Default::default() }
    }

    pub fn prop(&self, key: &str) -> &str {
        self.props.get(key).map(String::as_str).unwrap_or("")
    }

    fn first_prop(&self, keys: &[&str]) -> &str {
        keys.iter().map(|k| self.prop(k)).find(|v| !v.is_empty()).unwrap_or("")
    }

    pub fn manufacturer(&self) -> String {
        or_dash(self.prop("ro.product.manufacturer"))
    }

    pub fn brand(&self) -> String {
        or_dash(self.prop("ro.product.brand"))
    }

    pub fn model(&self) -> String {
        or_dash(self.first_prop(&["ro.product.marketname", "ro.product.model", "ro.product.product.model"]))
    }

    pub fn commercial_name(&self) -> String {
        self.first_prop(&["ro.product.marketname", "ro.product.odm.marketname", "ro.product.vendor.marketname"]).to_string()
    }

    pub fn android_version(&self) -> String {
        let version = self.prop("ro.build.version.release");
        let sdk = self.prop("ro.build.version.sdk");
        match (version.is_empty(), sdk.is_empty()) {
            (false, false) => format!("Android {version} (API {sdk})"),
            (false, true) => version.to_string(),
            (true, false) => format!("API {sdk}"),
            (true, true) => "—".into(),
        }
    }

    pub fn sdk_level(&self) -> Option<i64> {
        self.prop("ro.build.version.sdk").parse().ok()
    }

    pub fn security_patch(&self) -> String {
        or_dash(self.prop("ro.build.version.security_patch"))
    }

    pub fn battery_temperature(&self) -> Option<f64> {
        self.battery.temperature_c.or_else(|| self.temperatures.iter().find(|t| t.type_code == Some(2) || t.name.to_lowercase().contains("batt")).map(|t| t.value_c))
    }

    pub fn cpu_temperature(&self) -> Option<f64> {
        self.temperatures.iter().filter(|t| t.type_code == Some(0) || t.name.to_lowercase().contains("cpu")).map(|t| t.value_c).fold(None, |acc: Option<f64>, v| Some(acc.map_or(v, |a| a.max(v))))
    }

    pub fn thermal_status_label(&self) -> String {
        self.thermal_status.map(thermal_status_label).unwrap_or_else(|| "—".into())
    }

    pub fn uptime_label(&self) -> String {
        format::human_duration(self.uptime_seconds)
    }

    pub fn stale_seconds(&self) -> f64 {
        (now_secs() - self.timestamp).max(0.0)
    }
}

fn or_dash(value: &str) -> String {
    if value.is_empty() {
        "—".into()
    } else {
        value.to_string()
    }
}

// ---------------------------------------------------------------------------
// Packages, processes, files, sensors, logcat
// ---------------------------------------------------------------------------

/// Space an app takes, as Android counts it (`dumpsys diskstats`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskUsage {
    pub app: i64,
    pub data: i64,
    pub cache: i64,
}

impl DiskUsage {
    pub fn total(&self) -> i64 {
        self.app + self.data + self.cache
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PackageInfo {
    pub package: String,
    pub apk_path: String,
    pub is_system: bool,
    pub version_name: String,
    pub version_code: String,
    pub min_sdk: String,
    pub target_sdk: String,
    pub uid: String,
    pub first_install: String,
    pub last_update: String,
    pub installer: String,
    pub size_bytes: Option<i64>,
    pub label: String,
    pub enabled: Option<bool>,
    pub flags: Vec<String>,
    /// Permissions granted at runtime (for audits).
    pub granted_permissions: Vec<String>,
    pub requested_permissions: Vec<String>,
    /// App + data + cache (from `dumpsys diskstats`).
    #[serde(default)]
    pub total_bytes: Option<i64>,
    /// The same split: app code, its data and its cache.
    #[serde(default)]
    pub disk: Option<DiskUsage>,
    /// Last update (APK date, seconds since 1970).
    #[serde(default)]
    pub updated_ts: Option<i64>,
}

impl PackageInfo {
    pub fn new(package: impl Into<String>) -> Self {
        Self { package: package.into(), ..Default::default() }
    }

    pub fn display_name(&self) -> String {
        if !self.label.is_empty() {
            return self.label.clone();
        }
        let last = self.package.rsplit('.').next().unwrap_or(&self.package);
        title_case(&last.replace('_', " "))
    }

    pub fn type_label(&self) -> &'static str {
        if self.is_system {
            tr("System")
        } else {
            tr("User")
        }
    }

    pub fn is_debuggable(&self) -> bool {
        self.flags.iter().any(|f| f == "DEBUGGABLE")
    }

    pub fn allows_backup(&self) -> bool {
        self.flags.iter().any(|f| f == "ALLOW_BACKUP")
    }
}

fn title_case(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Real icon of an app, read from its APK on the phone.
#[derive(Debug, Clone, PartialEq)]
pub struct AppIcon {
    /// Image (PNG, WebP or SVG, if it was an Android vector). In an adaptive
    /// icon, the foreground layer, which has a margin around it (clipped when
    /// displayed).
    pub image: Vec<u8>,
    pub adaptive: bool,
    /// Background of an adaptive icon: ARGB colour or image (like `image`).
    pub background_color: Option<u32>,
    pub background_image: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: i64,
    pub user: String,
    pub ppid: i64,
    pub vsz_kib: i64,
    pub rss_kib: i64,
    pub stat: String,
    pub name: String,
    pub args: String,
    /// Instantaneous %CPU (from `top`), if it could be measured.
    #[serde(default)]
    pub cpu: Option<f64>,
    /// State according to Android (`TOP`, `FGS`, `CEM`…) from `dumpsys activity
    /// lru`. Only app processes have it.
    #[serde(default)]
    pub android_state: Option<String>,
}

impl ProcessInfo {
    /// Kernel threads: children of kthreadd (PID 2) or names in brackets.
    pub fn is_kernel_thread(&self) -> bool {
        self.pid == 2 || self.ppid == 2 || (self.name.starts_with('[') && self.name.ends_with(']'))
    }
    /// Android package of the process (`com.app:service` → `com.app`).
    pub fn package(&self) -> &str {
        self.name.split(':').next().unwrap_or("")
    }
}

/// One-off GPU and smoothness reading.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuFps {
    /// GPU usage % (Adreno; `None` if the kernel does not expose it).
    pub gpu_busy: Option<f64>,
    /// Frames per second of the foreground app.
    pub fps: Option<f64>,
    pub newest_frame: Option<i64>,
    /// Package of the foreground app.
    pub app: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProcessOverview {
    pub processes: Vec<ProcessInfo>,
    /// Packages installed by the user (`pm list packages -3`).
    pub user_packages: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub link_target: String,
    pub size: i64,
    pub date: String,
    pub perms: String,
    pub owner: String,
    pub group: String,
}

impl FileEntry {
    pub fn is_hidden(&self) -> bool {
        self.name.starts_with('.')
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorInfo {
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub kind: String,
    pub type_code: i64,
    pub wakeup: bool,
    pub min_rate: String,
    pub max_rate: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub descriptor: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LogLevel {
    Verbose,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
    /// Continuation lines (stack traces) without a header.
    Raw,
}

impl LogLevel {
    pub fn from_char(c: &str) -> Self {
        match c.to_ascii_uppercase().as_str() {
            "V" => Self::Verbose,
            "D" => Self::Debug,
            "I" => Self::Info,
            "W" => Self::Warn,
            "E" => Self::Error,
            "F" | "A" => Self::Fatal,
            _ => Self::Raw,
        }
    }

    pub fn letter(self) -> &'static str {
        match self {
            Self::Verbose => "V",
            Self::Debug => "D",
            Self::Info => "I",
            Self::Warn => "W",
            Self::Error => "E",
            Self::Fatal => "F",
            Self::Raw => "",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Verbose => "VERBOSE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
            Self::Fatal => "FATAL",
            Self::Raw => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub raw: String,
    pub timestamp: String,
    pub pid: String,
    pub tid: String,
    pub level: LogLevel,
    pub tag: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiStatus {
    pub enabled: Option<bool>,
    pub ssid: String,
    pub ip: String,
    pub rssi: Option<i64>,
    pub link_speed: String,
    pub frequency_mhz: Option<i64>,
    pub standard: String,
    pub security: String,
    pub bssid: String,
    pub state: String,
}
