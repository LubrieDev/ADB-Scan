//! Pure parsers of ADB/Android output.
//!
//! Free of side effects and covered by tests with real captured outputs. They
//! are the only interpretation of the device's state.

use crate::{i18n::{tr, trc}, t};
use regex::Regex;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

use crate::models::*;

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).expect("valid regex"));
    };
}

pub(crate) fn safe_int(value: Option<&str>) -> Option<i64> {
    value?.trim().parse().ok()
}

fn safe_float(value: &str) -> Option<f64> {
    value.trim().parse().ok()
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// adb devices / fastboot devices
// ---------------------------------------------------------------------------

re!(DEVICE_LINE, r"^(?P<serial>\S+)\s+(?P<rest>.+)$");
re!(KEY_VALUE, r"(?P<key>[a-zA-Z_]+):(?P<value>\S+)");

/// Parses `adb devices -l` (authorised, unauthorised, offline, no permissions…).
pub fn parse_devices(text: &str) -> Vec<AdbDevice> {
    let mut devices = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("List of devices") || line.starts_with('*') {
            continue;
        }
        let Some(caps) = DEVICE_LINE.captures(line) else {
            continue;
        };
        let serial = caps["serial"].to_string();
        let rest = caps["rest"].trim();
        if rest.is_empty() {
            continue;
        }
        let raw_state = if rest.starts_with("no permissions") { "no permissions".to_string() } else { rest.split_whitespace().next().unwrap_or("").trim_end_matches(':').to_string() };
        let fields: HashMap<&str, &str> = KEY_VALUE.captures_iter(rest).map(|c| (c.name("key").unwrap().as_str(), c.name("value").unwrap().as_str())).collect();
        let transport = if serial.starts_with("emulator-") {
            Transport::Emulator
        } else if serial.contains(':') || serial.contains("._adb-tls-connect.") {
            Transport::Tcp
        } else {
            Transport::Usb
        };
        let state = DeviceState::parse(&raw_state);
        let get = |k: &str| fields.get(k).map(|s| s.to_string()).unwrap_or_default();
        devices.push(AdbDevice {
            raw_state: if state == DeviceState::Unknown { raw_state } else { String::new() },
            state,
            product: get("product"),
            model: get("model"),
            device: get("device"),
            transport_id: get("transport_id"),
            usb: get("usb"),
            transport,
            serial,
        });
    }
    devices
}

/// Parses `fastboot devices -l`: `SERIAL\tfastboot` or `SERIAL fastbootd`.
pub fn parse_fastboot_devices(text: &str) -> Vec<AdbDevice> {
    let mut devices = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(serial), Some(mode)) = (parts.next(), parts.next()) else {
            continue;
        };
        if serial.starts_with('<') || serial == "???????????" {
            continue;
        }
        let state = if mode == "fastbootd" { DeviceState::Fastbootd } else { DeviceState::Fastboot };
        let fields: HashMap<&str, &str> = KEY_VALUE.captures_iter(line).map(|c| (c.name("key").unwrap().as_str(), c.name("value").unwrap().as_str())).collect();
        devices.push(AdbDevice {
            serial: serial.to_string(),
            state,
            usb: fields.get("usb").map(|s| s.to_string()).unwrap_or_default(),
            transport: if serial.contains(':') { Transport::Tcp } else { Transport::Usb },
            ..Default::default()
        });
    }
    devices
}

/// `fastboot getvar all` prints `(bootloader) key: value` lines on stderr.
pub fn parse_fastboot_vars(text: &str) -> BTreeMap<String, String> {
    let mut vars = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("(bootloader)").unwrap_or(line).trim();
        if line.starts_with("Finished") || line.starts_with("all:") || line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            if !key.is_empty() && !key.contains(' ') {
                vars.insert(key.to_string(), v.trim().to_string());
            }
        }
    }
    vars
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

re!(PROP_LINE, r"^\[(?P<key>[^\]]+)\]:\s*\[(?P<value>.*)\]\s*$");

pub fn parse_getprop(text: &str) -> BTreeMap<String, String> {
    text.lines().filter_map(|line| PROP_LINE.captures(line.trim())).map(|c| (c["key"].to_string(), c["value"].to_string())).collect()
}

// ---------------------------------------------------------------------------
// Battery
// ---------------------------------------------------------------------------

fn battery_key(normalized: &str) -> Option<&'static str> {
    Some(match normalized {
        "ac powered" => "ac_powered",
        "usb powered" => "usb_powered",
        "wireless powered" => "wireless_powered",
        "dock powered" => "dock_powered",
        "max charging current" => "max_charging_current",
        "max charging voltage" => "max_charging_voltage",
        "charge counter" => "charge_counter",
        "status" => "status",
        "health" => "health",
        "present" => "present",
        "level" => "level",
        "scale" => "scale",
        "voltage" => "voltage",
        "temperature" => "temperature",
        "technology" => "technology",
        "capacity level" => "capacity_level",
        "charging state" => "charging_state",
        "charging policy" => "charging_policy",
        _ => return None,
    })
}

pub fn parse_battery(text: &str) -> BatteryInfo {
    let mut raw: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if let Some(mapped) = battery_key(&key.trim().to_lowercase()) {
            raw.insert(mapped.to_string(), value.trim().to_string());
        }
    }
    let get = |k: &str| raw.get(k).map(String::as_str);
    let is_true = |k: &str| get(k).is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let mut info = BatteryInfo {
        ac_powered: is_true("ac_powered"),
        usb_powered: is_true("usb_powered"),
        wireless_powered: is_true("wireless_powered"),
        dock_powered: is_true("dock_powered"),
        level: safe_int(get("level")),
        scale: safe_int(get("scale")).filter(|s| *s != 0).unwrap_or(100),
        status_code: safe_int(get("status")),
        health_code: safe_int(get("health")),
        present: get("present").filter(|v| !v.is_empty()).map(|v| v.eq_ignore_ascii_case("true")),
        voltage_mv: safe_int(get("voltage")),
        temperature_c: safe_int(get("temperature")).map(|t| t as f64 / 10.0),
        technology: get("technology").unwrap_or("").to_string(),
        charge_counter_mah: safe_int(get("charge_counter")).map(|c| (c as f64 / 1000.0).round() as i64),
        max_charging_current_ua: safe_int(get("max_charging_current")),
        max_charging_voltage_uv: safe_int(get("max_charging_voltage")),
        capacity_level: safe_int(get("capacity_level")),
        charging_state: get("charging_state").unwrap_or("").to_string(),
        ..Default::default()
    };
    info.extra = raw;
    info
}

// ---------------------------------------------------------------------------
// Component health
// ---------------------------------------------------------------------------

/// Capacity from sysfs: charging chips give it in µAh; some, in mAh.
fn capacity_mah(value: &str) -> Option<i64> {
    let v: i64 = value.trim().parse().ok()?;
    match v {
        v if v >= 100_000 => Some(v / 1000),
        v if v > 0 => Some(v),
        _ => None,
    }
}

/// JEDEC level in hexadecimal (“0x01”). 0 means “not defined”.
fn jedec_level(value: &str) -> Option<u8> {
    let v = value.trim();
    let n = u8::from_str_radix(v.strip_prefix("0x").or_else(|| v.strip_prefix("0X"))?, 16).ok()?;
    (n > 0).then_some(n)
}

/// `WEAR` section of the live script (`key=value`; empty if it could not be
/// read).
pub fn parse_wear(text: &str) -> ComponentHealth {
    let mut h = ComponentHealth { probed: true, ..Default::default() };
    let mut kind = String::new();
    let mut life = None;
    let mut eol = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "batt_cycle_count" => h.battery_cycles = value.trim().parse().ok().filter(|c| (0..=10_000).contains(c)),
            "batt_charge_full" => h.battery_full_mah = capacity_mah(value),
            "batt_charge_full_design" => h.battery_design_mah = capacity_mah(value),
            "kind" => kind = value.trim().to_string(),
            // UFS gives A and B separately (eMMC, “0x01 0x02”): the worse one
            // counts.
            "life" => life = value.split_whitespace().filter_map(jedec_level).max(),
            "eol" => eol = jedec_level(value),
            _ => {}
        }
    }
    if h.battery_full_mah.is_some() && h.battery_design_mah.is_some() {
        h.battery_source = CapacitySource::Sysfs;
    }
    h.storage = match (kind.is_empty(), life.is_some() || eol.is_some()) {
        (true, _) => StorageWear::Unknown,
        (false, false) => StorageWear::Denied { kind },
        (false, true) => StorageWear::Read { kind, life, eol },
    };
    h
}

/// Samsung's lines in `dumpsys battery` (One UI): `mSavedBatteryAsoc: [89]`
/// (capacity left, %), `mSavedBatteryUsage: [20179]` (charge used over its
/// life in %, so 100 is one full cycle) and `battery FirstUseDate:
/// [20240813]`. Only those three fields are filled.
pub fn parse_samsung_battery(text: &str) -> ComponentHealth {
    let field = |key: &str| {
        text.lines().find_map(|l| l.trim().strip_prefix(key)).map(|v| v.trim().trim_start_matches('[').trim_end_matches(']').trim().to_string()).filter(|v| !v.is_empty())
    };
    ComponentHealth {
        battery_asoc: field("mSavedBatteryAsoc:").and_then(|v| v.parse::<u8>().ok()).filter(|p| (1..=100).contains(p)),
        battery_cycles: field("mSavedBatteryUsage:").and_then(|v| v.parse::<i64>().ok()).filter(|u| (1..=1_000_000).contains(u)).map(|u| (u + 50) / 100),
        battery_first_use: field("battery FirstUseDate:")
            .filter(|d| d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit()) && d.as_str() > "2000")
            .map(|d| format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..])),
        ..Default::default()
    }
}

/// `dumpsys batterystats`: (design capacity, last learned capacity) in mAh.
/// Fallback when sysfs is not readable (it takes ~10 s: never live).
pub fn parse_batterystats_capacity(text: &str) -> (Option<i64>, Option<i64>) {
    let value = |label: &str| {
        text.lines().find_map(|l| l.trim().strip_prefix(label)).and_then(|rest| rest.trim().trim_end_matches("mAh").trim().parse::<f64>().ok()).map(|v| v.round() as i64).filter(|v| *v > 0)
    };
    (value("Estimated battery capacity:"), value("Last learned battery capacity:"))
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

re!(MEMINFO_LINE, r"(?i)^(?P<key>[A-Za-z0-9_()]+):\s*(?P<value>\d+)\s*(?P<unit>kB)?");

pub fn parse_meminfo(text: &str) -> MemoryInfo {
    let mut values: HashMap<String, i64> = HashMap::new();
    for line in text.lines() {
        if let Some(c) = MEMINFO_LINE.captures(line.trim()) {
            if let Ok(v) = c["value"].parse() {
                values.insert(c["key"].to_string(), v);
            }
        }
    }
    let get = |k: &str| values.get(k).copied();
    // `x or y` semantics: a 0 counts as missing and falls back to the next
    // value.
    let or_nonzero = |a: Option<i64>, b: Option<i64>| a.filter(|v| *v != 0).or(b);
    let mut info = MemoryInfo {
        total_kib: get("MemTotal"),
        free_kib: get("MemFree"),
        available_kib: get("MemAvailable"),
        buffers_kib: get("Buffers"),
        cached_kib: get("Cached"),
        swap_total_kib: get("SwapTotal"),
        swap_free_kib: get("SwapFree"),
        zram_total_kib: or_nonzero(get("ZramTotal"), get("SwapTotal")),
        zram_free_kib: or_nonzero(get("ZramFree"), get("SwapFree")),
        oom_kill_kib: get("OOMKill"),
    };
    if info.available_kib.is_none() {
        if let Some(free) = info.free_kib {
            info.available_kib = Some(free + info.cached_kib.unwrap_or(0));
        }
    }
    info
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

pub fn parse_df(text: &str) -> Vec<StorageInfo> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.to_lowercase().starts_with("filesystem") {
            continue;
        }
        let parts = split_max(line, 6);
        if parts.len() < 6 {
            continue;
        }
        let (fs, total, used, avail, pct, mount) = (parts[0], parts[1], parts[2], parts[3], parts[4], parts[5]);
        if !is_digits(total) {
            continue;
        }
        entries.push(StorageInfo {
            filesystem: fs.to_string(),
            total_kib: total.parse().ok(),
            used_kib: if is_digits(used) { used.parse().ok() } else { None },
            available_kib: if is_digits(avail) { avail.parse().ok() } else { None },
            use_percent: safe_int(Some(pct.trim_end_matches('%'))),
            mount: mount.to_string(),
            label: mount.to_string(),
        });
    }
    entries
}

/// Equivalent to `str.split(maxsplit=n-1)`.
pub(crate) fn split_max(text: &str, n: usize) -> Vec<&str> {
    let mut out = Vec::with_capacity(n);
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        if out.len() == n - 1 {
            out.push(rest.trim_end());
            break;
        }
        match rest.find(char::is_whitespace) {
            Some(idx) => {
                out.push(&rest[..idx]);
                rest = rest[idx..].trim_start();
            }
            None => {
                out.push(rest);
                break;
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageKind {
    Data,
    Shared,
}

pub fn select_storage(entries: &[StorageInfo], kind: StorageKind) -> StorageInfo {
    let (preferred, label): (&[&str], &str) = match kind {
        StorageKind::Data => (&["/data/user/0", "/data", "/"], tr("Internal storage")),
        StorageKind::Shared => (&["/storage/emulated/0", "/storage/self/primary", "/sdcard"], tr("Shared storage")),
    };
    let with_label = |e: &StorageInfo| StorageInfo { label: label.to_string(), ..e.clone() };
    for mount in preferred {
        if let Some(e) = entries.iter().find(|e| e.mount == *mount) {
            return with_label(e);
        }
    }
    for e in entries {
        let hit = match kind {
            StorageKind::Data => e.mount == "/data" || e.mount == "/",
            StorageKind::Shared => e.mount.starts_with("/storage/emulated"),
        };
        if hit {
            return with_label(e);
        }
    }
    StorageInfo { label: label.to_string(), ..Default::default() }
}

/// Mounts of external SD cards (`/storage/XXXX-XXXX`).
pub fn external_sd_mounts(entries: &[StorageInfo]) -> Vec<StorageInfo> {
    static SD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^/storage/[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}$").unwrap());
    entries.iter().filter(|e| SD.is_match(&e.mount)).cloned().collect()
}

// ---------------------------------------------------------------------------
// Thermals
// ---------------------------------------------------------------------------

re!(TEMP_RE, r"Temperature\{mValue=(?P<value>-?\d+(?:\.\d+)?),\s*mType=(?P<type>-?\d+),\s*mName=(?P<name>[^,}]+),\s*mStatus=(?P<status>\d+)\}");
re!(THERMAL_STATUS_RE, r"Thermal Status:\s*(\d+)");

fn find_from(text: &str, needle: &str, from: usize) -> Option<usize> {
    text.get(from..)?.find(needle).map(|i| i + from)
}

pub fn parse_thermalservice(text: &str) -> (Vec<Temperature>, Option<i64>) {
    let mut current_block = "";
    if let Some(start) = text.find("Current temperatures from HAL:") {
        current_block = &text[start..];
        if let Some(end) = find_from(current_block, "\nHAL Ready:", 20) {
            current_block = &current_block[..end];
        }
    }
    let mut cached_block = "";
    if let Some(start) = text.find("Cached temperatures:") {
        cached_block = &text[start..];
        for marker in ["\nHAL Ready:", "\nCurrent temperatures from HAL:"] {
            if let Some(end) = find_from(cached_block, marker, 20) {
                cached_block = &cached_block[..end];
            }
        }
    }
    let block = if current_block.is_empty() { cached_block } else { current_block };
    let mut temperatures = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for c in TEMP_RE.captures_iter(block) {
        let name = c["name"].trim().to_string();
        let type_code: i64 = c["type"].parse().unwrap_or(-1);
        let Ok(value) = c["value"].parse::<f64>() else {
            continue;
        };
        // Types 6 and 7 are current and voltage, not temperatures.
        let lname = name.to_lowercase();
        if type_code == 6 || type_code == 7 || lname == "ibat" || lname == "vbat" {
            continue;
        }
        if !seen.insert((name.clone(), c["value"].to_string())) {
            continue;
        }
        temperatures.push(Temperature { name, value_c: value, type_code: Some(type_code), status_code: c["status"].parse().ok() });
    }
    let status = THERMAL_STATUS_RE.captures(text).and_then(|c| c[1].parse().ok());
    (temperatures, status)
}

// ---------------------------------------------------------------------------
// CPU
// ---------------------------------------------------------------------------

pub fn parse_cpuinfo(text: &str) -> CpuInfo {
    let mut processors: HashSet<i64> = HashSet::new();
    let mut hardware = String::new();
    let mut model_name = String::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "processor" if is_digits(value) => {
                processors.insert(value.parse().unwrap_or(0));
            }
            "Hardware" | "hardware" => hardware = value.to_string(),
            "model name" | "Model" => model_name = value.to_string(),
            _ => {}
        }
    }
    CpuInfo { model_name: if model_name.is_empty() { hardware.clone() } else { model_name }, hardware, core_count: (!processors.is_empty()).then_some(processors.len()), ..Default::default() }
}

/// `###COREn` blocks with `cur=`, `max=`, `min=`, `gov=`.
pub fn parse_cpu_freq_blocks(text: &str) -> BTreeMap<usize, HashMap<String, String>> {
    static CORE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^###CORE(\d+)").unwrap());
    let mut cores: BTreeMap<usize, HashMap<String, String>> = BTreeMap::new();
    let mut current: Option<usize> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(c) = CORE.captures(line) {
            let idx = c[1].parse().unwrap_or(0);
            cores.insert(idx, HashMap::new());
            current = Some(idx);
            continue;
        }
        let Some(idx) = current else { continue };
        if let Some((k, v)) = line.split_once('=') {
            cores.get_mut(&idx).unwrap().insert(k.trim().into(), v.trim().into());
        }
    }
    cores
}

pub fn apply_cpu_freqs(cpu: &mut CpuInfo, freq_text: &str) {
    let blocks = parse_cpu_freq_blocks(freq_text);
    let cores: Vec<CpuCore> = blocks
        .iter()
        .map(|(index, data)| CpuCore {
            index: *index,
            cur_khz: safe_int(data.get("cur").map(String::as_str)),
            min_khz: safe_int(data.get("min").map(String::as_str)),
            max_khz: safe_int(data.get("max").map(String::as_str)),
            governor: data.get("gov").cloned().unwrap_or_default(),
        })
        .collect();
    if !cores.is_empty() {
        if cpu.core_count.is_none() {
            cpu.core_count = Some(cores.len());
        }
        cpu.cores = cores;
    }
}

pub fn parse_proc_stat(text: &str) -> Option<(i64, i64)> {
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 5 {
        return None;
    }
    let values: Vec<i64> = fields[1..].iter().filter(|f| is_digits(f)).filter_map(|f| f.parse().ok()).collect();
    if values.len() < 4 {
        return None;
    }
    let total: i64 = values.iter().sum();
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    Some(((total - idle).max(0), total))
}

pub fn parse_loadavg(text: &str) -> Option<(f64, f64, f64)> {
    let f: Vec<&str> = text.split_whitespace().collect();
    if f.len() < 3 {
        return None;
    }
    Some((f[0].parse().ok()?, f[1].parse().ok()?, f[2].parse().ok()?))
}

pub fn parse_uptime(text: &str) -> Option<f64> {
    text.split_whitespace().next().and_then(safe_float)
}

pub fn compute_cpu_load(previous: Option<(i64, i64)>, current: Option<(i64, i64)>) -> Option<f64> {
    let (pa, pt) = previous?;
    let (a, t) = current?;
    let td = t - pt;
    if td <= 0 {
        return None;
    }
    Some(((a - pa) as f64 / td as f64 * 100.0).clamp(0.0, 100.0))
}

// ---------------------------------------------------------------------------
// Processes
// ---------------------------------------------------------------------------

/// `ps -A -o PID,USER,PPID,VSZ,RSS,STAT,NAME,ARGS`
/// `top -b -n 1 -o PID,%CPU,NAME`: PID → %CPU.
pub fn parse_top_cpu(text: &str) -> std::collections::HashMap<i64, f64> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid: i64 = it.next()?.parse().ok()?;
            let cpu: f64 = it.next()?.parse().ok()?;
            Some((pid, cpu))
        })
        .collect()
}

/// Last sample of `top -b -n 2 -o %CPU,NAME`: (name, %CPU) in `top`'s order.
/// The first sample is useless: `top` has nothing to compare with yet and gives
/// 0 % to everything. `top` itself is skipped.
pub fn parse_top_last(text: &str) -> Vec<(String, f64)> {
    let last = text.rfind("Tasks:").map_or(text, |i| &text[i..]);
    last.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let cpu: f64 = it.next()?.parse().ok()?;
            let name = it.next()?.to_string();
            (name != "top").then_some((name, cpu))
        })
        .collect()
}

pub fn parse_ps(text: &str) -> Vec<ProcessInfo> {
    let mut processes = Vec::new();
    let mut header_seen = false;
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if !header_seen && line.contains("PID") && line.contains("NAME") {
            header_seen = true;
            continue;
        }
        let parts = split_max(line, 8);
        if parts.len() < 8 || !is_digits(parts[0]) {
            continue;
        }
        processes.push(ProcessInfo {
            pid: parts[0].parse().unwrap_or(0),
            user: parts[1].into(),
            ppid: parts[2].parse().unwrap_or(0),
            vsz_kib: parts[3].parse().unwrap_or(0),
            rss_kib: parts[4].parse().unwrap_or(0),
            stat: parts[5].into(),
            name: parts[6].into(),
            args: parts[7].trim().into(),
            cpu: None,
            android_state: None,
        });
    }
    processes
}

// ---------------------------------------------------------------------------
// Packages
// ---------------------------------------------------------------------------

re!(PACKAGE_LINE_VERSION, r"^package:(?P<path>[^=]+)=(?P<package>[\w.]+)(?:\s+versionCode:(?P<version>\d+))?\s*$");
// `pm list packages -f` may contain '=' in the path (split APKs); the last
// occurrence is used as the separator.
re!(PACKAGE_LINE_GREEDY, r"^package:(?P<path>.+)=(?P<package>[\w.]+)(?:\s+versionCode:(?P<version>\d+))?\s*$");

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageRecord {
    pub path: String,
    pub version_code: String,
}

pub fn parse_package_records(text: &str) -> BTreeMap<String, PackageRecord> {
    let mut records = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(c) = PACKAGE_LINE_VERSION.captures(line).or_else(|| PACKAGE_LINE_GREEDY.captures(line)) else {
            continue;
        };
        records.insert(c["package"].to_string(), PackageRecord { path: c["path"].to_string(), version_code: c.name("version").map(|m| m.as_str().to_string()).unwrap_or_default() });
    }
    records
}

pub fn parse_package_list(text: &str) -> BTreeMap<String, String> {
    parse_package_records(text).into_iter().map(|(k, v)| (k, v.path)).collect()
}

/// `pm list packages -d` / `-e` / `-3`: names only.
pub fn parse_package_names(text: &str) -> HashSet<String> {
    text.lines().filter_map(|l| l.trim().strip_prefix("package:")).map(|p| p.rsplit('=').next().unwrap_or(p).split_whitespace().next().unwrap_or("").to_string()).filter(|p| !p.is_empty()).collect()
}

const SYSTEM_PREFIXES: [&str; 8] = ["/system/", "/system_ext/", "/product/", "/vendor/", "/odm/", "/apex/", "/vendor_dlkm/", "/odm_dlkm/"];

fn capture_line(text: &str, key: &str) -> Option<String> {
    let re = Regex::new(&format!(r"(?m)^\s*{}=(.+)$", regex::escape(key))).ok()?;
    re.captures(text).map(|c| c[1].trim().to_string())
}

pub fn parse_package_dump(text: &str, package: &str, apk_path: &str) -> PackageInfo {
    re!(VC_LINE, r"versionCode=(\d+)\s+minSdk=(\d+)\s+targetSdk=(\d+)");
    re!(VC_ONLY, r"(?m)^\s*versionCode=(\d+)");
    re!(MIN_SDK, r"\bminSdk=(\d+)");
    re!(TARGET_SDK, r"\btargetSdk=(\d+)");
    re!(VN, r"(?m)^\s*versionName=(\S+)");
    re!(FLAGS, r"(?m)^\s*flags=\[(.*?)\]");
    re!(PKG_FLAGS, r"(?m)^\s*pkgFlags=\[(.*?)\]");
    re!(ENABLED, r"(?m)^\s*enabled=(\w+)");
    // `userId=` up to Android 15; `appId=` since Android 16.
    re!(USER_ID, r"(?m)^\s*(?:userId|appId)=(\d+)");

    let mut info = PackageInfo { package: package.into(), apk_path: apk_path.into(), ..Default::default() };
    if let Some(c) = VC_LINE.captures(text) {
        info.version_code = c[1].into();
        info.min_sdk = c[2].into();
        info.target_sdk = c[3].into();
    } else {
        if let Some(c) = VC_ONLY.captures(text) {
            info.version_code = c[1].into();
        }
        if let Some(c) = MIN_SDK.captures(text) {
            info.min_sdk = c[1].into();
        }
        if let Some(c) = TARGET_SDK.captures(text) {
            info.target_sdk = c[1].into();
        }
    }
    if let Some(c) = VN.captures(text) {
        info.version_name = c[1].into();
    }
    if let Some(v) = capture_line(text, "firstInstallTime") {
        info.first_install = v;
    }
    if let Some(v) = capture_line(text, "lastUpdateTime") {
        info.last_update = v;
    }
    if let Some(v) = capture_line(text, "installerPackageName") {
        info.installer = v;
    }
    // Only a number: on Android 16 the first “uid=” line belongs to a declared
    // permission (“uid=10441 gids=[] type=0 prot=signature”).
    if let Some(v) = capture_line(text, "uid").filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())) {
        info.uid = v;
    }
    if let Some(v) = capture_line(text, "codePath") {
        info.apk_path = v;
    }
    if let Some(c) = FLAGS.captures(text).or_else(|| PKG_FLAGS.captures(text)) {
        info.flags = c[1].split_whitespace().map(String::from).collect();
        info.is_system = info.flags.iter().any(|f| f == "SYSTEM");
    }
    if !info.is_system && !info.apk_path.is_empty() {
        info.is_system = SYSTEM_PREFIXES.iter().any(|p| info.apk_path.starts_with(p));
    }
    if let Some(c) = ENABLED.captures(text) {
        info.enabled = Some(c[1].eq_ignore_ascii_case("true"));
    }
    if let Some(c) = USER_ID.captures(text) {
        info.uid = c[1].into();
    }
    let (requested, granted) = parse_package_permissions(text);
    info.requested_permissions = requested;
    info.granted_permissions = granted;
    info
}

/// Extracts requested and granted permissions from `dumpsys package <pkg>`.
pub fn parse_package_permissions(text: &str) -> (Vec<String>, Vec<String>) {
    re!(PERM_GRANTED, r"^\s*([\w.]+): granted=true");
    let mut requested = Vec::new();
    let mut granted = Vec::new();
    let mut section = "";
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("requested permissions:") {
            section = "requested";
            continue;
        }
        if trimmed.starts_with("install permissions:") || trimmed.starts_with("runtime permissions:") {
            section = "granted";
            continue;
        }
        if trimmed.is_empty() || (trimmed.ends_with(':') && !trimmed.contains('.')) {
            section = "";
            continue;
        }
        match section {
            "requested" => {
                let name = trimmed.split(':').next().unwrap_or("").split_whitespace().next().unwrap_or("");
                if name.contains('.') {
                    requested.push(name.to_string());
                } else {
                    section = "";
                }
            }
            "granted" => {
                if let Some(c) = PERM_GRANTED.captures(line) {
                    granted.push(c[1].to_string());
                } else if !trimmed.contains("granted=") {
                    section = "";
                }
            }
            _ => {}
        }
    }
    requested.sort();
    requested.dedup();
    granted.sort();
    granted.dedup();
    (requested, granted)
}

pub fn package_uid_map(text: &str) -> HashMap<String, i64> {
    re!(PKG_HDR, r"^\s*Package \[([\w.]+)\]");
    re!(UID, r"^\s*userId=(\d+)");
    let mut result = HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(c) = PKG_HDR.captures(line) {
            current = Some(c[1].to_string());
            continue;
        }
        if let Some(pkg) = &current {
            if let Some(c) = UID.captures(line) {
                result.insert(pkg.clone(), c[1].parse().unwrap_or(0));
                current = None;
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// File listings
// ---------------------------------------------------------------------------

re!(LS_LINE, r"^(?P<perms>[bcdlps-][rwxsStT-]{9}[.+@]?)\s+(?P<links>\d+)\s+(?P<owner>\S+)\s+(?P<group>\S+)\s+(?P<size>\d+)\s+(?P<date>\d{4}-\d{2}-\d{2})\s+(?P<time>\d{2}:\d{2})\s+(?P<name>.*)$");
re!(LS_ALT, r"^(?P<perms>[bcdlps-][rwxsStT-]{9}[.+@]?)\s+(?P<links>\d+)\s+(?P<owner>\S+)\s+(?P<group>\S+)\s+(?P<size>\d+)\s+(?P<date>[A-Z][a-z]{2}\s+\d{1,2}\s+(?:\d{2}:\d{2}|\d{4}))\s+(?P<name>.*)$");

pub fn parse_ls(text: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with("total ") {
            continue;
        }
        let Some(c) = LS_LINE.captures(line).or_else(|| LS_ALT.captures(line)) else {
            continue;
        };
        let mut name = c["name"].trim().to_string();
        if name == "." || name == ".." {
            continue;
        }
        let perms = c["perms"].to_string();
        let is_link = perms.starts_with('l');
        let mut link_target = String::new();
        if is_link {
            if let Some((n, t)) = name.split_once(" -> ") {
                link_target = t.to_string();
                name = n.to_string();
            }
        }
        let date = match c.name("time") {
            Some(t) => format!("{} {}", &c["date"], t.as_str()),
            None => c["date"].to_string(),
        };
        entries.push(FileEntry { name, is_dir: perms.starts_with('d'), is_link, link_target, size: c["size"].parse().unwrap_or(0), date, perms, owner: c["owner"].into(), group: c["group"].into() });
    }
    entries.sort_by_key(|a| (!a.is_dir, a.name.to_lowercase()));
    entries
}

// ---------------------------------------------------------------------------
// Sensors
// ---------------------------------------------------------------------------

re!(
    SENSOR_RE,
    r"^(?P<handle>0x[0-9a-fA-F]+)\)\s+(?P<name>.*?)\s*\|\s*(?P<vendor>.*?)\s*\|\s*ver:\s*(?P<version>\d+)\s*\|\s*type:\s*(?P<type>[^\s(]+)\((?P<code>\d+)\)\s*\|\s*perm:\s*(?P<perm>[^|]+)\|\s*flags:\s*(?P<flags>0x[0-9a-fA-F]+)"
);

pub fn parse_sensors(text: &str) -> Vec<SensorInfo> {
    re!(MIN_RATE, r"minRate=(?P<min>[\d.]+)Hz");
    re!(MAX_RATE, r"maxRate=(?P<max>[\d.]+)Hz");
    let lines: Vec<&str> = text.lines().collect();
    let mut sensors = Vec::new();
    let mut seen: HashSet<(String, String, bool)> = HashSet::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(c) = SENSOR_RE.captures(line.trim()) else {
            continue;
        };
        let detail = lines.get(i + 1).map(|l| l.trim()).unwrap_or("").to_string();
        let min_rate = MIN_RATE.captures(&detail).map(|m| m["min"].to_string()).unwrap_or_else(|| if detail.contains("on-change") { "on-change".into() } else { String::new() });
        let sensor = SensorInfo {
            name: c["name"].trim().into(),
            vendor: c["vendor"].trim().into(),
            version: c["version"].into(),
            kind: c["type"].trim().into(),
            type_code: c["code"].parse().unwrap_or(0),
            wakeup: detail.contains("wakeUp"),
            min_rate,
            max_rate: MAX_RATE.captures(&detail).map(|m| m["max"].to_string()).unwrap_or_default(),
            detail,
        };
        if seen.insert((sensor.name.clone(), sensor.kind.clone(), sensor.wakeup)) {
            sensors.push(sensor);
        }
    }
    sensors
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

re!(IP_HEADER, r"^\d+:\s+(?P<name>[^:@\s]+)(?:@\S+)?:\s+<(?P<flags>[^>]*)>(?P<rest>.*)$");
re!(IP_COMPACT, r"^\d+:\s+(?P<name>[^:@\s]+)(?:@\S+)?\s+(?P<rest>.+)$");
re!(IP_INET, r"^inet\s+(?P<addr>[\d.]+)(?:/(?P<prefix>\d+))?");
re!(IP_INET6, r"^inet6\s+(?P<addr>[0-9a-fA-F:]+)(?:/(?P<prefix>\d+))?");
re!(IP_LINK, r"\blink/\w+\s+(?P<mac>[0-9a-fA-F:][0-9a-fA-F:]{16})");
re!(IP_MTU, r"\bmtu\s+(\d+)");
re!(IP_FLAGS, r"<([^>]*)>");

fn flags_up(flags: &str) -> bool {
    flags.split(',').any(|f| f == "UP") || flags.contains("UP")
}

fn apply_ip_tokens(iface: &mut NetInterface, stripped: &str) {
    if let Some(c) = IP_INET.captures(stripped) {
        iface.addresses.push(IpAddress { family: "IPv4".into(), address: c["addr"].into(), prefix: c.name("prefix").map_or("32", |m| m.as_str()).into() });
        if iface.state.is_empty() {
            iface.state = "UP".into();
        }
    } else if let Some(c) = IP_INET6.captures(stripped) {
        let address = c["addr"].to_string();
        if !address.starts_with("fe80") {
            iface.addresses.push(IpAddress { family: "IPv6".into(), address, prefix: c.name("prefix").map_or("128", |m| m.as_str()).into() });
        }
        if iface.state.is_empty() {
            iface.state = "UP".into();
        }
    } else if let Some(c) = IP_LINK.captures(stripped) {
        iface.mac = c["mac"].into();
    }
    if let Some(c) = IP_MTU.captures(stripped) {
        iface.mtu = c[1].parse().ok();
    }
    if let Some(c) = IP_FLAGS.captures(stripped) {
        iface.flags = c[1].into();
        iface.state = if flags_up(&c[1]) { "UP" } else { "DOWN" }.into();
    }
}

/// Accepts classic `ip addr` and the compact `ip -o addr` / `ip -o link`.
pub fn parse_ip_addr(text: &str) -> Vec<NetInterface> {
    let mut interfaces: Vec<NetInterface> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut current: Option<usize> = None;
    let mut ensure = |name: &str, interfaces: &mut Vec<NetInterface>| -> usize {
        if let Some(i) = index.get(name) {
            return *i;
        }
        interfaces.push(NetInterface { name: name.into(), ..Default::default() });
        index.insert(name.into(), interfaces.len() - 1);
        interfaces.len() - 1
    };
    for raw in text.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        if let Some(c) = IP_HEADER.captures(raw) {
            let i = ensure(&c["name"], &mut interfaces);
            apply_ip_tokens(&mut interfaces[i], &c["rest"]);
            let flags = &c["flags"];
            if !flags.is_empty() {
                interfaces[i].flags = flags.into();
                interfaces[i].state = if flags_up(flags) { "UP" } else { "DOWN" }.into();
            }
            current = Some(i);
            continue;
        }
        if let Some(c) = IP_COMPACT.captures(raw) {
            let rest = c["rest"].trim_start();
            if rest.starts_with("inet") || rest.starts_with("link/") || rest.starts_with('<') {
                let i = ensure(&c["name"], &mut interfaces);
                apply_ip_tokens(&mut interfaces[i], rest.trim());
                current = Some(i);
                continue;
            }
        }
        if let Some(i) = current {
            apply_ip_tokens(&mut interfaces[i], raw.trim());
        }
    }
    interfaces
}

pub fn parse_ip_route(text: &str) -> Vec<Route> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let after = |tok: &str| parts.iter().position(|p| *p == tok).and_then(|i| parts.get(i + 1)).map(|s| s.to_string());
            Route { destination: parts[0].into(), dev: after("dev"), via: after("via"), src: after("src"), raw: line.into() }
        })
        .collect()
}

pub fn parse_wifi_status(text: &str) -> WifiStatus {
    re!(SSID_CONNECTED, r#"Wifi is connected to "([^"]*)""#);
    re!(SSID_ANY, r#"SSID:\s*"([^"]*)""#);
    re!(WIFI_INFO, r"(?m)WifiInfo:\s*(.+)$");
    let mut status = WifiStatus::default();
    if text.contains("Wifi is enabled") || text.contains("Wi-Fi is enabled") {
        status.enabled = Some(true);
    } else if text.contains("Wifi is disabled") || text.contains("Wi-Fi is disabled") {
        status.enabled = Some(false);
    }
    if let Some(c) = SSID_CONNECTED.captures(text).or_else(|| SSID_ANY.captures(text)) {
        status.ssid = c[1].into();
    }
    let info_line = WIFI_INFO.captures(text).map(|c| c[1].to_string()).unwrap_or_else(|| text.to_string());
    let grab = |pat: &str| Regex::new(pat).ok()?.captures(&info_line).map(|c| c[1].trim().to_string());
    status.ip = grab(r"IP:\s*/([0-9.]+)").unwrap_or_default();
    status.rssi = grab(r"RSSI:\s*(-?\d+)").and_then(|v| v.parse().ok());
    status.link_speed =
        grab(r"Link speed:\s*(\d+)\s*Mbps").filter(|v| !v.starts_with('-')).map(|v| format!("{v} Mb/s")).or_else(|| grab(r"Link speed:\s*([^,]+)").filter(|v| !v.starts_with('-'))).unwrap_or_default();
    status.frequency_mhz = grab(r"Frequency:\s*(\d+)MHz").and_then(|v| v.parse().ok());
    status.standard = grab(r"Wi-Fi standard:\s*([^,]+)").map(|v| wifi_standard_name(&v)).unwrap_or_default();
    status.security = grab(r"Security type:\s*([^,]+)").map(|v| wifi_security_name(&v)).unwrap_or_default();
    status.bssid = grab(r"BSSID:\s*([0-9a-fA-F:]+)").unwrap_or_default();
    status.state = grab(r"Supplicant state:\s*([^,]+)").unwrap_or_default();
    status
}

/// `ip -o -4 addr show wlan0` → its IPv4 address.
pub fn interface_ipv4(text: &str) -> Option<String> {
    let addr = text.split_whitespace().skip_while(|t| *t != "inet").nth(1)?;
    let ip = addr.split('/').next()?;
    ip.parse::<std::net::Ipv4Addr>().ok().map(|_| ip.to_string())
}

/// `Wi-Fi standard: 11ac` → “Wi-Fi 5 (802.11ac)”. Android 11 gives the
/// number of `ScanResult.WIFI_STANDARD_*` instead (4 → 11n).
pub fn wifi_standard_name(raw: &str) -> String {
    let raw = match raw.trim() {
        "0" => "unknown",
        "1" => "legacy",
        "4" => "11n",
        "5" => "11ac",
        "6" => "11ax",
        "7" => "11ad",
        "8" => "11be",
        other => other,
    };
    let gen = match raw {
        "11n" => "Wi-Fi 4",
        "11ac" => "Wi-Fi 5",
        "11ax" => "Wi-Fi 6",
        "11ad" => "WiGig",
        "11be" => "Wi-Fi 7",
        "legacy" => "802.11a/b/g",
        "unknown" | "" => return String::new(),
        _ => return raw.into(),
    };
    if raw == "legacy" {
        gen.into()
    } else {
        format!("{gen} (802.{raw})")
    }
}

/// `Security type: 2` (`WifiInfo.SECURITY_TYPE_*` constants) → name.
pub fn wifi_security_name(raw: &str) -> String {
    let raw = raw.trim();
    let name = match raw {
        "0" => trc("wifi", "Open"),
        "1" => "WEP",
        "2" => "WPA/WPA2-Personal",
        "3" => "WPA/WPA2-Enterprise",
        "4" => "WPA3-Personal (SAE)",
        "5" => "WPA3-Enterprise 192 bits",
        "6" => tr("Enhanced open (OWE)"),
        "7" => "WAPI-PSK",
        "8" => "WAPI-CERT",
        "9" => "WPA3-Enterprise",
        "10" => "OSEN",
        "11" | "12" => "Passpoint",
        "13" => "DPP",
        "-1" | "" => return String::new(),
        _ => return raw.into(),
    };
    name.into()
}

pub fn parse_services(text: &str) -> Vec<ServiceInfo> {
    re!(SERVICE, r"^\s*\d+\s+(?P<name>[^:]+):\s*\[(?P<descriptor>.*)\]\s*$");
    text.lines()
        .filter_map(|l| SERVICE.captures(l))
        .map(|c| {
            let d = c["descriptor"].trim();
            ServiceInfo { name: c["name"].trim().into(), descriptor: if d.is_empty() { "—".into() } else { d.into() } }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

pub fn parse_wm_size(text: &str) -> (Option<i64>, Option<i64>, String) {
    re!(OVERRIDE, r"Override size:\s*(\d+)x(\d+)");
    re!(PHYSICAL, r"Physical size:\s*(\d+)x(\d+)");
    let ov = OVERRIDE.captures(text);
    let is_override = ov.is_some();
    match ov.or_else(|| PHYSICAL.captures(text)) {
        Some(c) => (c[1].parse().ok(), c[2].parse().ok(), if is_override { "Override".into() } else { String::new() }),
        None => (None, None, String::new()),
    }
}

pub fn parse_wm_density(text: &str) -> (Option<i64>, String) {
    re!(OVERRIDE, r"Override density:\s*(\d+)");
    re!(PHYSICAL, r"Physical density:\s*(\d+)");
    let ov = OVERRIDE.captures(text);
    let is_override = ov.is_some();
    match ov.or_else(|| PHYSICAL.captures(text)) {
        Some(c) => (c[1].parse().ok(), if is_override { "Override".into() } else { String::new() }),
        None => (None, String::new()),
    }
}

pub fn density_bucket(dpi: Option<i64>) -> String {
    let Some(dpi) = dpi.filter(|d| *d > 0) else {
        return String::new();
    };
    const BUCKETS: [(i64, &str); 7] = [(120, "ldpi"), (160, "mdpi"), (213, "tvdpi"), (240, "hdpi"), (320, "xhdpi"), (480, "xxhdpi"), (640, "xxxhdpi")];
    let mut bucket = "mdpi";
    for (threshold, name) in BUCKETS {
        if dpi >= threshold {
            bucket = name;
        }
    }
    bucket.into()
}

pub fn parse_display_info(size_text: &str, density_text: &str, display_dump: &str) -> DisplayInfo {
    re!(BASE, r"(?m)mBaseDisplayInfo=(.+)$");
    re!(FPS, r"(?i)\b(?:fps|refreshRate|mRefreshRate|reportedRefreshRate)=([\d.]+)");
    re!(RATES, r"supportedRefreshRates \[([^\]]+)\]");
    re!(HDR, r"mSupportedHdrTypes=\[([^\]]*)\]");
    re!(BRIGHT, r"brightnessDefault=([\d.]+)");
    let mut info = DisplayInfo::default();
    (info.width, info.height, info.size_override) = parse_wm_size(size_text);
    (info.density_dpi, info.density_override) = parse_wm_density(density_text);
    info.density_bucket = density_bucket(info.density_dpi);
    if !display_dump.is_empty() {
        let line = BASE.captures(display_dump).map(|c| c[1].to_string()).unwrap_or_else(|| display_dump.to_string());
        info.refresh_hz = active_refresh_in_info(&line).or_else(|| FPS.captures(&line).and_then(|c| safe_float(&c[1])));
        if let Some(c) = RATES.captures(&line) {
            let mut rates: Vec<f64> = c[1].split(',').filter_map(|s| s.trim().parse::<f64>().ok()).map(|v| (v * 100.0).round() / 100.0).collect();
            rates.sort_by(|a, b| a.partial_cmp(b).unwrap());
            rates.dedup();
            info.supported_refresh = rates;
        }
        if let Some(c) = HDR.captures(&line) {
            info.hdr_types = c[1].split(',').filter_map(|s| s.trim().parse().ok()).collect();
        }
        if let Some(c) = BRIGHT.captures(&line) {
            if let Ok(v) = c[1].parse::<f64>() {
                info.brightness = Some((v * 255.0).round() as i64);
            }
        }
    }
    info
}

pub fn parse_brightness(text: &str) -> Option<i64> {
    let v = text.trim();
    if is_digits(v) {
        v.parse().ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// USB
// ---------------------------------------------------------------------------

pub fn parse_usb_dump(text: &str) -> UsbInfo {
    let get_bool = |key: &str| -> Option<bool> {
        let re = Regex::new(&format!(r"(?m)^\s*{key}=(\w+)")).ok()?;
        re.captures(text).map(|c| c[1].eq_ignore_ascii_case("true"))
    };
    re!(KERNEL, r"(?m)^\s*kernel_state=(\w+)");
    UsbInfo {
        connected: get_bool("connected"),
        configured: get_bool("configured"),
        charging: get_bool("usb_charging"),
        host_connected: get_bool("host_connected"),
        state: KERNEL.captures(text).map(|c| c[1].to_string()).unwrap_or_default(),
        ..Default::default()
    }
}

pub fn parse_usb_props(functions: &str, config: &str, state: &str) -> UsbInfo {
    let mut info = UsbInfo { functions: functions.trim().into(), config: config.trim().into(), state: state.trim().into(), ..Default::default() };
    if !info.functions.is_empty() || !info.config.is_empty() {
        info.connected = Some(true);
    }
    info
}

// ---------------------------------------------------------------------------
// Logcat
// ---------------------------------------------------------------------------

re!(THREADTIME_RE, r"^(?P<ts>\d{2}-\d{2}\s+\d{2}:\d{2}:\d{2}\.\d{3})\s+(?P<pid>\d+)\s+(?P<tid>\d+)\s+(?P<level>[VDIWEFA])\s+(?P<tag>.*?)\s*:\s?(?P<message>.*)$");

pub fn parse_logcat_line(line: &str) -> LogEntry {
    let line = line.trim_end_matches(['\n', '\r']);
    if let Some(c) = THREADTIME_RE.captures(line) {
        return LogEntry {
            raw: line.into(),
            timestamp: c["ts"].into(),
            pid: c["pid"].into(),
            tid: c["tid"].into(),
            level: LogLevel::from_char(&c["level"]),
            tag: c["tag"].trim().into(),
            message: c["message"].into(),
        };
    }
    LogEntry { raw: line.into(), timestamp: String::new(), pid: String::new(), tid: String::new(), level: LogLevel::Raw, tag: String::new(), message: line.into() }
}

pub fn parse_logcat_text(text: &str) -> Vec<LogEntry> {
    text.lines().filter(|l| !l.trim().is_empty()).map(parse_logcat_line).collect()
}

// ---------------------------------------------------------------------------
// @@@NAME@@@ sections
// ---------------------------------------------------------------------------

re!(SECTION_RE, r"(?m)^@@@(?P<name>[A-Z0-9_]+)@@@\s*$");

/// Splits a remote script's output by `@@@NAME@@@` markers.
pub fn split_sections(text: &str) -> HashMap<String, String> {
    let matches: Vec<_> = SECTION_RE.captures_iter(text).collect();
    let mut sections = HashMap::new();
    for (i, c) in matches.iter().enumerate() {
        let m = c.get(0).unwrap();
        let end = matches.get(i + 1).map(|n| n.get(0).unwrap().start()).unwrap_or(text.len());
        sections.insert(c["name"].to_string(), text[m.end()..end].trim_matches('\n').to_string());
    }
    sections
}

pub fn humanize_adb_device(device: &AdbDevice) -> String {
    format!("{} ({}) · {}", device.display_name(), device.serial, device.state_label())
}

// ---------------------------------------------------------------------------
// GPU and FPS (without root)
// ---------------------------------------------------------------------------

/// `/sys/class/kgsl/kgsl-3d0/gpubusy` (Adreno): “busy total” in µs of the last
/// period → usage %.
pub fn parse_gpubusy(text: &str) -> Option<f64> {
    let mut it = text.split_whitespace();
    let busy: f64 = it.next()?.parse().ok()?;
    let total: f64 = it.next()?.parse().ok()?;
    (total > 0.0).then(|| (busy / total * 100.0).clamp(0.0, 100.0))
}

/// `mCurrentFocus=Window{… u0 com.app/com.app.Activity}` → “com.app/com.app.Activity”.
pub fn parse_focus(text: &str) -> Option<String> {
    let line = text.lines().find(|l| l.contains("mCurrentFocus"))?;
    let inner = line.split_once("Window{")?.1.trim_end().trim_end_matches('}');
    let comp = inner.split_whitespace().last()?;
    comp.contains('/').then(|| comp.to_string())
}

/// `dumpsys SurfaceFlinger --list`: layer names (Android 14+ wraps them in
/// `RequestedLayerState{… parentId=…}`).
pub fn parse_layer_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let l = l.strip_prefix("RequestedLayerState{").unwrap_or(l);
            let l = l.split(" parentId=").next().unwrap_or(l);
            l.trim_end_matches('}').to_string()
        })
        .collect()
}

/// Layers drawing the focused app: its `SurfaceView(BLAST)` (video, games) and
/// the activity's layer. Backgrounds and containers are skipped.
pub fn fps_layers(layers: &[String], focus: &str) -> Vec<String> {
    let pkg = focus.split('/').next().unwrap_or(focus);
    let mut out: Vec<String> = layers.iter().filter(|l| l.contains("(BLAST)") && l.contains(&format!("SurfaceView[{pkg}"))).cloned().collect();
    out.extend(layers.iter().filter(|l| l.starts_with(&format!("{focus}#"))).cloned());
    out.truncate(3);
    out
}

/// `dumpsys SurfaceFlinger --latency LAYER`: real presentation times (second
/// column, ns). Pending ones (0 or i64::MAX) are discarded.
pub fn parse_latency(text: &str) -> Vec<i64> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let cols: Vec<i64> = l.split_whitespace().filter_map(|c| c.parse().ok()).collect();
            (cols.len() == 3).then(|| cols[1])
        })
        .filter(|&t| t > 0 && t != i64::MAX)
        .collect()
}

/// Frames per second in the last second before the most recent one. Returns
/// (fps, timestamp of the most recent) to detect idle screens.
pub fn fps_from_presents(times: &[i64]) -> Option<(f64, i64)> {
    let newest = *times.iter().max()?;
    let window = 1_000_000_000;
    let n = times.iter().filter(|&&t| t > newest - window).count();
    Some((n as f64, newest))
}

// ---------------------------------------------------------------------------
// Bulk app data (installer, sizes, date)
// ---------------------------------------------------------------------------

/// `pm list packages -i`: `package:x  installer=y` → x → y (“” if null).
pub fn parse_installers(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("package:")?;
            let (pkg, inst) = rest.split_once("installer=").map(|(p, i)| (p.trim(), i.trim())).unwrap_or((rest.trim(), ""));
            Some((pkg.to_string(), if inst == "null" { String::new() } else { inst.to_string() }))
        })
        .collect()
}

/// `dumpsys diskstats`: package → bytes of app, data and cache.
pub fn parse_diskstats(text: &str) -> HashMap<String, crate::models::DiskUsage> {
    let array = |key: &str| -> Vec<String> {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key))
            .map(|v| v.trim().trim_start_matches(':').trim().trim_start_matches('[').trim_end_matches(']').split(',').map(|x| x.trim().trim_matches('"').to_string()).collect())
            .unwrap_or_default()
    };
    let names = array("Package Names");
    let nums = |key: &str| array(key).iter().map(|x| x.parse::<i64>().unwrap_or(0)).collect::<Vec<_>>();
    let (apps, data, cache) = (nums("App Sizes"), nums("App Data Sizes"), nums("Cache Sizes"));
    let at = |v: &Vec<i64>, i: usize| v.get(i).copied().unwrap_or(0).max(0);
    names.into_iter().enumerate().filter(|(_, n)| !n.is_empty()).map(|(i, n)| (n, crate::models::DiskUsage { app: at(&apps, i), data: at(&data, i), cache: at(&cache, i) })).collect()
}

/// `stat -c '%Y %n' …`: path → seconds since 1970.
pub fn parse_stat_mtimes(text: &str) -> HashMap<String, i64> {
    text.lines()
        .filter_map(|l| {
            let (t, path) = l.trim().split_once(' ')?;
            Some((path.to_string(), t.parse().ok()?))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Refresh rate
// ---------------------------------------------------------------------------

/// Active mode within `mBaseDisplayInfo`: `renderFrameRate 90.0` or, if
/// missing, the `fps` of the mode whose id is `mode N` (the first `fps=` on the
/// line belongs to the first mode in the list, not the active one).
pub fn active_refresh_in_info(line: &str) -> Option<f64> {
    re!(RENDER, r"renderFrameRate ([\d.]+)");
    re!(MODE, r"(?:^|[ ,])mode (\d+)");
    re!(MODE_FPS, r"\{id=(\d+), [^}]*?fps=([\d.]+)");
    if let Some(c) = RENDER.captures(line) {
        return safe_float(&c[1]);
    }
    let id = MODE.captures(line)?[1].to_string();
    MODE_FPS.captures_iter(line).find(|c| c[1] == id).and_then(|c| safe_float(&c[2]))
}

/// `@@@REFRESH@@@` block: `mActiveRenderFrameRate=…` and `mScreenState=…`.
pub fn parse_refresh_live(text: &str) -> (Option<f64>, String) {
    re!(RATE, r"mActiveRenderFrameRate=([\d.]+)");
    re!(STATE, r"mScreenState=([A-Z_]+)");
    (RATE.captures(text).and_then(|c| safe_float(&c[1])), STATE.captures(text).map(|c| c[1].to_string()).unwrap_or_default())
}

/// `@@@REFRESHCFG@@@` block: `peak_refresh_rate`, `min_refresh_rate` and, on
/// ZTE/nubia/REDMAGIC phones, `refresh_rate_mode` as an index into
/// `all_refresh_rate` (“auto,60,90,120,144”). Returns readable text.
pub fn parse_refresh_setting(text: &str) -> String {
    let vals: Vec<&str> = text.lines().map(str::trim).collect();
    let get = |i: usize| vals.get(i).copied().filter(|v| !v.is_empty() && *v != "null");
    if let (Some(mode), Some(all)) = (get(2), get(3)) {
        let options: Vec<&str> = all.split(',').map(str::trim).collect();
        if let Some(opt) = mode.parse::<usize>().ok().and_then(|i| options.get(i)) {
            return if opt.eq_ignore_ascii_case("auto") { tr("Automatic").into() } else { format!("{opt} Hz") };
        }
    }
    let hz = |v: &str| v.parse::<f64>().ok().filter(|f| f.is_finite() && *f > 0.0);
    match (get(0).and_then(hz), get(1).and_then(hz)) {
        (Some(p), Some(m)) if (p - m).abs() < 0.5 => format!("{p:.0} Hz"),
        (Some(p), Some(m)) => format!("{m:.0}–{p:.0} Hz"),
        (Some(p), None) => t!("up to {} Hz", format!("{:.0}", p)),
        _ => String::new(),
    }
}
