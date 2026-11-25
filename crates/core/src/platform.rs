//! Platform sheet: what sets a phone apart on the inside and is not in the
//! telemetry (SELinux, GPU, CPU clusters, thermal limits, hardware features,
//! verified boot…).
//!
//! Everything is read-only and without root. A single round trip (~0.4 s) with
//! [`PLATFORM_SCRIPT`]; what comes from `getprop` is derived from the
//! properties the snapshot already carries, without querying anything else.

use crate::{i18n::tr, t};
use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::probe::{self, Probe};

/// Fixed script (without user data). Each `cat` ends with `echo`: sysfs files
/// do not always end with a newline and the next section's marker would be
/// stuck to the value.
pub const PLATFORM_SCRIPT: &str = r#"echo @@@SELINUX@@@; getenforce 2>&1
echo @@@KERNEL@@@; cat /proc/version 2>&1; echo
echo @@@PAGESIZE@@@; getconf PAGESIZE 2>&1
echo @@@GLES@@@; dumpsys SurfaceFlinger 2>&1 | grep -m1 '^GLES:'
echo @@@VULKAN@@@; cmd gpu vkjson 2>&1
echo @@@GPUMODEL@@@; cat /sys/class/kgsl/kgsl-3d0/gpu_model 2>&1; echo
echo @@@CPUFREQ@@@; for p in /sys/devices/system/cpu/cpufreq/policy*; do echo "${p##*/}|$(cat $p/related_cpus 2>&1)|$(cat $p/scaling_governor 2>&1)|$(cat $p/cpuinfo_min_freq 2>&1)|$(cat $p/cpuinfo_max_freq 2>&1)|$(cat $p/scaling_available_frequencies 2>&1)"; done
echo @@@THERMAL@@@; dumpsys hardware_properties 2>&1
echo @@@FEATURES@@@; pm list features 2>&1
echo @@@MAINLINE@@@; dumpsys package com.google.android.modulemetadata 2>&1 | grep -m1 'versionName='
echo @@@END@@@
"#;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct KernelBuild {
    /// `6.12.38-android16-5-g665eafb62659-ab14778838-4k`
    pub release: String,
    /// `clang 19.0.1`
    pub compiler: String,
    /// Build date as the kernel writes it.
    pub built: String,
    /// Branch of Google's generic kernel (GKI), e.g. `android16-6.12`.
    pub gki: Option<String>,
    /// SMP, PREEMPT…
    pub flags: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GlesInfo {
    pub vendor: String,
    pub renderer: String,
    /// `OpenGL ES 3.2 V@0842.44`
    pub version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VulkanInfo {
    pub device: String,
    /// API version supported by the driver (`1.4.295`).
    pub api: String,
    /// Driver version in Vulkan's encoding.
    pub driver_version: String,
    pub driver_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CpuCluster {
    pub policy: String,
    /// Cores of the cluster as the kernel numbers them (`0 1 2 3 4 5`).
    pub cpus: Vec<u32>,
    pub governor: Probe<String>,
    pub min_khz: Option<i64>,
    pub max_khz: Option<i64>,
    /// Frequency steps supported by the cluster.
    pub steps: Probe<usize>,
}

impl CpuCluster {
    /// “CPU 0–5” or “CPU 6, 7”.
    pub fn cpus_label(&self) -> String {
        match self.cpus.as_slice() {
            [] => self.policy.clone(),
            [one] => format!("CPU {one}"),
            [first, .., last] if self.cpus.windows(2).all(|w| w[1] == w[0] + 1) && self.cpus.len() > 2 => {
                format!("CPU {first}–{last}")
            }
            all => format!("CPU {}", all.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")),
        }
    }
}

/// Thresholds declared by the manufacturer in the thermal HAL. They are not
/// measurements: they are the temperatures at which the system throttles
/// performance or shuts down.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThermalLimit {
    pub kind: String,
    pub throttling: Option<(f64, f64)>,
    pub shutdown: Option<(f64, f64)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HwFeature {
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlatformReport {
    /// Time of the read (seconds since 1970).
    pub read_at: f64,
    pub selinux: Probe<String>,
    pub kernel: Probe<KernelBuild>,
    pub page_size: Probe<u32>,
    pub gles: Probe<GlesInfo>,
    pub vulkan: Probe<VulkanInfo>,
    pub gpu_model: Probe<String>,
    pub clusters: Probe<Vec<CpuCluster>>,
    pub thermal_limits: Probe<Vec<ThermalLimit>>,
    pub features: Probe<Vec<HwFeature>>,
    /// Version of the “Google Play system update” modules.
    pub mainline: Probe<String>,
}

pub fn parse_platform(text: &str) -> PlatformReport {
    let s = crate::parsers::split_sections(text);
    PlatformReport {
        read_at: crate::models::now_secs(),
        selinux: probe::from_section(s.get("SELINUX"), parse_selinux),
        kernel: probe::from_section(s.get("KERNEL"), parse_kernel_version),
        page_size: probe::from_section(s.get("PAGESIZE"), |t| t.trim().parse().ok().filter(|v: &u32| *v >= 1024)),
        gles: probe::from_section(s.get("GLES"), parse_gles),
        vulkan: probe::from_section(s.get("VULKAN"), parse_vkjson),
        gpu_model: probe::from_section(s.get("GPUMODEL"), |t| Some(t.trim().to_string()).filter(|v| !v.is_empty())),
        // Each field carries its own error (a `Permission denied` on the
        // frequencies does not invalidate the governor): only if there is no
        // policy at all is the whole section looked at.
        clusters: match s.get("CPUFREQ").map(|t| parse_cpufreq_policies(t)) {
            Some(c) if !c.is_empty() => Probe::Ok(c),
            _ => probe::from_section(s.get("CPUFREQ"), |_| None),
        },
        thermal_limits: probe::from_section(s.get("THERMAL"), |t| Some(parse_thermal_limits(t)).filter(|v| !v.is_empty())),
        features: probe::from_section(s.get("FEATURES"), |t| Some(parse_features(t)).filter(|v| !v.is_empty())),
        mainline: probe::from_section(s.get("MAINLINE"), |t| {
            let v = t.trim().strip_prefix("versionName=")?.trim();
            // “2026-10-01S+” → “2026-10-01”: what follows the date is internal.
            let date = v.get(..10).filter(|d| d.len() == 10 && d.as_bytes()[4] == b'-' && d.as_bytes()[7] == b'-');
            (!v.is_empty()).then(|| date.unwrap_or(v).to_string())
        }),
    }
}

fn parse_selinux(text: &str) -> Option<String> {
    let t = text.trim();
    match t {
        "Enforcing" | "Permissive" | "Disabled" => Some(t.to_string()),
        _ => None,
    }
}

pub fn selinux_label(mode: &str) -> String {
    match mode {
        "Enforcing" => tr("Enforcing: the rules are applied").into(),
        "Permissive" => tr("Permissive: it only warns, it does not block. Common in development ROMs or with root").into(),
        "Disabled" => tr("Off").into(),
        other => other.into(),
    }
}

pub fn parse_kernel_version(text: &str) -> Option<KernelBuild> {
    static RELEASE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| Regex::new(r"Linux version (\S+)").unwrap());
    static COMPILER: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| Regex::new(r"\b(clang|gcc) version ([\d.]+)").unwrap());
    static BUILT: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| Regex::new(r"#\d+\s+((?:[A-Z_]+\s+)*)(.+)$").unwrap());
    static GKI: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| Regex::new(r"^(\d+\.\d+)\.\d+-(android\d+)-").unwrap());
    let line = text.lines().find(|l| l.contains("Linux version"))?.trim();
    let release = RELEASE.captures(line)?[1].to_string();
    let compiler = COMPILER.captures(line).map(|c| format!("{} {}", &c[1], &c[2])).unwrap_or_default();
    let (flags, built) = BUILT.captures(line).map(|c| (c[1].split_whitespace().collect::<Vec<_>>().join(" "), c[2].trim().to_string())).unwrap_or_default();
    let gki = GKI.captures(&release).map(|c| format!("{}-{}", &c[2], &c[1]));
    Some(KernelBuild { release, compiler, built, gki, flags })
}

/// `GLES: Qualcomm, Adreno (TM) 840, OpenGL ES 3.2 V@0842.44 (GIT@…) (Date:…)`
pub fn parse_gles(text: &str) -> Option<GlesInfo> {
    let line = text.lines().find_map(|l| l.trim().strip_prefix("GLES:"))?.trim();
    let mut parts = line.splitn(3, ", ");
    let vendor = parts.next()?.trim().to_string();
    let renderer = parts.next()?.trim().to_string();
    let rest = parts.next().unwrap_or("").trim();
    // What is in parentheses (build hash and date) is removed.
    let version = rest.split(" (").next().unwrap_or(rest).trim().to_string();
    Some(GlesInfo { vendor, renderer, version })
}

/// Packed Vulkan version (`VK_MAKE_API_VERSION`) → `1.4.295`.
pub fn vulkan_version(v: u64) -> String {
    format!("{}.{}.{}", (v >> 22) & 0x7f, (v >> 12) & 0x3ff, v & 0xfff)
}

/// `cmd gpu vkjson` (Android 10+): JSON with the driver's properties.
pub fn parse_vkjson(text: &str) -> Option<VulkanInfo> {
    let start = text.find('{')?;
    let json: serde_json::Value = serde_json::from_str(&text[start..]).ok()?;
    let dev = json.get("devices")?.get(0)?;
    let props = dev.get("properties")?;
    let num = |v: &serde_json::Value| v.as_f64().map(|f| f as u64);
    let device = props.get("deviceName")?.as_str()?.to_string();
    let api = props.get("apiVersion").and_then(num).map(vulkan_version).unwrap_or_default();
    // The driver version is the vendor's: the classic encoding is used without
    // the API mask (Qualcomm sets bit 31: 512.842.44).
    let driver_version = props.get("driverVersion").and_then(num).map(|v| format!("{}.{}.{}", v >> 22, (v >> 12) & 0x3ff, v & 0xfff)).unwrap_or_default();
    fn find_str<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
        match v {
            serde_json::Value::Object(m) => m.get(key).and_then(|x| x.as_str()).or_else(|| m.values().find_map(|x| find_str(x, key))),
            _ => None,
        }
    }
    let driver_name = dev.get("VK_KHR_driver_properties").and_then(|d| find_str(d, "driverName")).unwrap_or("").to_string();
    Some(VulkanInfo { device, api, driver_version, driver_name })
}

/// One line per policy: `policy0|0 1 2|walt|384000|3628800|384000 537600 …`.
pub fn parse_cpufreq_policies(text: &str) -> Vec<CpuCluster> {
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.trim().split('|').collect();
        if f.len() < 6 || !f[0].starts_with("policy") {
            continue;
        }
        let field = |s: &str| -> Probe<String> {
            match probe::classify_failure(s) {
                Some(p) => p.map_never(),
                None if s.trim().is_empty() => Probe::Missing,
                None => Probe::Ok(s.trim().to_string()),
            }
        };
        let khz = |s: &str| s.trim().parse::<i64>().ok().filter(|v| *v > 0);
        let steps = field(f[5]).map(|v| v.split_whitespace().filter(|x| x.parse::<i64>().is_ok()).count());
        out.push(CpuCluster {
            policy: f[0].to_string(),
            cpus: f[1].split_whitespace().filter_map(|c| c.parse().ok()).collect(),
            governor: field(f[2]),
            min_khz: khz(f[3]),
            max_khz: khz(f[4]),
            steps: match steps {
                Probe::Ok(0) => Probe::Missing,
                other => other,
            },
        });
    }
    out
}

/// `dumpsys hardware_properties`: throttling and shutdown thresholds.
pub fn parse_thermal_limits(text: &str) -> Vec<ThermalLimit> {
    static LINE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| Regex::new(r"(?m)^(\w+) (throttling|shutdown) temperatures: \[(.*)\]").unwrap());
    let mut map: BTreeMap<usize, ThermalLimit> = BTreeMap::new();
    let order = ["CPU", "GPU", "Battery", "Skin"];
    for c in LINE.captures_iter(text) {
        let kind = &c[1];
        // -3.4028235E38 (-FLT_MAX) means “undefined”.
        let values: Vec<f64> = c[3].split(',').filter_map(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite() && *v > -100.0 && *v < 250.0).collect();
        if values.is_empty() {
            continue;
        }
        let range = (values.iter().cloned().fold(f64::INFINITY, f64::min), values.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
        let idx = order.iter().position(|o| *o == kind).unwrap_or(order.len() + map.len());
        let entry = map.entry(idx).or_insert_with(|| ThermalLimit {
            kind: match kind {
                "Battery" => tr("Battery").into(),
                "Skin" => tr("Skin").into(),
                other => other.into(),
            },
            ..Default::default()
        });
        if &c[2] == "throttling" {
            entry.throttling = Some(range);
        } else {
            entry.shutdown = Some(range);
        }
    }
    map.into_values().collect()
}

pub fn parse_features(text: &str) -> Vec<HwFeature> {
    let mut out: Vec<HwFeature> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("feature:"))
        .map(|f| match f.split_once('=') {
            Some((n, v)) => HwFeature { name: n.to_string(), version: Some(v.to_string()) },
            None => HwFeature { name: f.to_string(), version: None },
        })
        .filter(|f| !f.name.is_empty())
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// ---------------------------------------------------------------------------
// Presentation
// ---------------------------------------------------------------------------

/// The most frequently checked hardware features, with a readable name. A
/// feature not appearing means the manufacturer does not declare it.
pub const FEATURE_HIGHLIGHTS: [(&str, &str); 22] = [
    ("android.hardware.nfc", "NFC"),
    ("android.hardware.nfc.hce", "Contactless payment (NFC HCE)"),
    ("android.hardware.telephony.euicc", "eSIM"),
    ("android.hardware.uwb", "Ultra-wideband (UWB)"),
    ("android.hardware.consumerir", "Infrared emitter"),
    ("android.hardware.fingerprint", "Fingerprint reader"),
    ("android.hardware.strongbox_keystore", "StrongBox (separate security chip)"),
    ("android.hardware.hardware_keystore", "Hardware-backed keystore"),
    ("android.hardware.se.omapi.ese", "Embedded secure element (eSE)"),
    ("android.hardware.identity_credential", "Identity credentials"),
    ("android.hardware.wifi.aware", "Wi-Fi Aware"),
    ("android.hardware.wifi.direct", "Wi-Fi Direct"),
    ("android.hardware.wifi.rtt", "Wi-Fi RTT (indoor positioning)"),
    ("android.hardware.bluetooth_le", "Bluetooth Low Energy"),
    ("android.hardware.bluetooth_le.channel_sounding", "Bluetooth Channel Sounding"),
    ("android.hardware.sensor.barometer", "Barometer"),
    ("android.hardware.sensor.stepcounter", "Step counter"),
    ("android.hardware.camera.capability.raw", "RAW photos"),
    ("android.hardware.camera.concurrent", "Concurrent cameras"),
    ("android.hardware.audio.pro", "Pro audio (low latency)"),
    ("android.hardware.usb.host", "USB host (OTG)"),
    ("android.hardware.vulkan.version", "Vulkan (guaranteed by Android)"),
];

/// (name, declared?, detail) of each highlighted feature.
pub fn feature_highlights(features: &[HwFeature]) -> Vec<(&'static str, bool, String)> {
    FEATURE_HIGHLIGHTS
        .iter()
        .map(|(key, label)| {
            let found = features.iter().find(|f| f.name == *key);
            let detail = match (found, *key) {
                (Some(f), "android.hardware.vulkan.version") => f.version.as_deref().and_then(|v| v.parse::<u64>().ok()).map(vulkan_version).unwrap_or_default(),
                (Some(f), "android.hardware.hardware_keystore") => f.version.as_deref().map(|v| format!("KeyMint {}", keymint_version(v))).unwrap_or_default(),
                _ => String::new(),
            };
            (tr(label), found.is_some(), detail)
        })
        .collect()
}

/// `300` → `3.0`; `100` → `1.0`; `41` (Keymaster 4.1) → `4.1`.
fn keymint_version(v: &str) -> String {
    match v.parse::<u32>() {
        Ok(n) if n >= 100 => format!("{}.{}", n / 100, (n % 100) / 10),
        Ok(n) => format!("{}.{}", n / 10, n % 10),
        Err(_) => v.to_string(),
    }
}

/// A sheet item with the source it comes from.
#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    pub label: &'static str,
    pub value: Probe<String>,
    /// Source command or property (for the tooltip).
    pub source: &'static str,
}

/// Android version of an API level.
pub fn android_for_api(api: i64) -> Option<&'static str> {
    Some(match api {
        21 => "5.0",
        22 => "5.1",
        23 => "6",
        24 => "7.0",
        25 => "7.1",
        26 => "8.0",
        27 => "8.1",
        28 => "9",
        29 => "10",
        30 => "11",
        31 => "12",
        32 => "12L",
        33 => "13",
        34 => "14",
        35 => "15",
        36 => "16",
        37 => "17",
        _ => return None,
    })
}

fn api_text(v: &str) -> String {
    let Ok(n) = v.parse::<i64>() else {
        return v.to_string();
    };
    // Since Android 15 the vendor level is YYYYMM (202504 = April 2025).
    if n > 200_000 {
        let (y, m) = (n / 100, (n % 100) as u32);
        return match crate::format::month_name(m) {
            Some(name) => t!("{} ({} {})", n, name, y),
            None => n.to_string(),
        };
    }
    match android_for_api(n) {
        Some(a) => format!("Android {a} (API {n})"),
        None => format!("API {n}"),
    }
}

fn bool_prop(props: &BTreeMap<String, String>, key: &str, yes: &str, no: &str) -> Probe<String> {
    probe::prop(props, key).map(|v| if v == "true" || v == "1" { yes.to_string() } else { no.to_string() })
}

/// Partitions, updates and compatibility (Treble, A/B…).
pub fn platform_facts(props: &BTreeMap<String, String>) -> Vec<Fact> {
    let ab = match probe::prop(props, "ro.build.ab_update") {
        Probe::Ok(v) if v == "true" => {
            let slot = props.get("ro.boot.slot_suffix").map(|s| s.trim_start_matches('_').to_uppercase()).filter(|s| !s.is_empty());
            Probe::Ok(match slot {
                Some(s) => t!("Yes · active slot {}", s),
                None => tr("Yes").into(),
            })
        }
        Probe::Ok(_) => Probe::Ok(tr("No: a single slot").into()),
        // Without the property, the phone does not use A/B updates.
        _ => Probe::Ok(tr("No: a single slot").into()),
    };
    vec![
        Fact { label: "Project Treble", value: bool_prop(props, "ro.treble.enabled", tr("Yes: separate system and vendor"), tr("No")), source: "ro.treble.enabled" },
        Fact { label: tr("A/B updates"), value: ab, source: "ro.build.ab_update · ro.boot.slot_suffix" },
        Fact { label: "A/B virtual", value: bool_prop(props, "ro.virtual_ab.enabled", tr("Yes (compressed snapshots)"), tr("No")), source: "ro.virtual_ab.enabled" },
        Fact { label: tr("Dynamic partitions"), value: bool_prop(props, "ro.boot.dynamic_partitions", tr("Yes (super)"), tr("No")), source: "ro.boot.dynamic_partitions" },
        Fact { label: tr("Shipped with"), value: probe::prop(props, "ro.product.first_api_level").map(|v| api_text(&v)), source: "ro.product.first_api_level" },
        Fact {
            label: tr("Vendor level"),
            value: match probe::prop(props, "ro.vendor.api_level") {
                Probe::Ok(v) => Probe::Ok(api_text(&v)),
                _ => probe::prop(props, "ro.board.api_level").map(|v| api_text(&v)),
            },
            source: "ro.vendor.api_level · ro.board.api_level",
        },
        Fact { label: tr("Vendor patch"), value: probe::prop(props, "ro.vendor.build.security_patch"), source: "ro.vendor.build.security_patch" },
    ]
}

/// Verified boot, bootloader lock, encryption and debugging.
pub fn boot_facts(props: &BTreeMap<String, String>) -> Vec<Fact> {
    let verified = probe::prop(props, "ro.boot.verifiedbootstate").map(|v| match v.as_str() {
        "green" => tr("Green: system verified with the manufacturer's keys").to_string(),
        "yellow" => tr("Yellow: verified with a custom key installed by the user").to_string(),
        "orange" => tr("Orange: bootloader unlocked, the system is not verified").to_string(),
        "red" => tr("Red: verification failed").to_string(),
        other => other.to_string(),
    });
    let locked = match probe::prop(props, "ro.boot.flash.locked") {
        Probe::Ok(v) => Probe::Ok(if v == "1" { tr("Locked").to_string() } else { tr("Unlocked").to_string() }),
        _ => probe::prop(props, "ro.boot.vbmeta.device_state").map(|v| match v.as_str() {
            "locked" => tr("Locked").to_string(),
            "unlocked" => tr("Unlocked").to_string(),
            other => other.to_string(),
        }),
    };
    let crypto = match (probe::prop(props, "ro.crypto.state"), probe::prop(props, "ro.crypto.type")) {
        (Probe::Ok(s), t) if s == "encrypted" => Probe::Ok(match t.ok().map(String::as_str) {
            Some("file") => tr("File-based encryption (FBE)").to_string(),
            Some("block") => tr("Full-disk encryption (FDE)").to_string(),
            _ => tr("Encrypted").to_string(),
        }),
        (Probe::Ok(s), _) if s == "unencrypted" => Probe::Ok(tr("Unencrypted").to_string()),
        (Probe::Ok(s), _) => Probe::Ok(s),
        (other, _) => other,
    };
    vec![
        Fact { label: tr("Verified boot"), value: verified, source: "ro.boot.verifiedbootstate" },
        Fact { label: "Bootloader", value: locked, source: "ro.boot.flash.locked · ro.boot.vbmeta.device_state" },
        Fact { label: tr("Encrypted"), value: crypto, source: "ro.crypto.state · ro.crypto.type" },
        Fact { label: tr("Debuggable system"), value: bool_prop(props, "ro.debuggable", tr("Yes: development build (userdebug/eng)"), tr("No (user build)")), source: "ro.debuggable" },
        Fact {
            label: tr("ADB requires authorisation"), value: bool_prop(props, "ro.adb.secure", tr("Yes: each PC must be accepted on the phone"), tr("No: any PC can connect")), source: "ro.adb.secure"
        },
    ]
}

/// `ro.opengles.version` (196610) → `3.2`.
pub fn opengles_from_prop(props: &BTreeMap<String, String>) -> Option<String> {
    let v: u32 = props.get("ro.opengles.version")?.trim().parse().ok()?;
    Some(format!("{}.{}", v >> 16, v & 0xffff))
}
