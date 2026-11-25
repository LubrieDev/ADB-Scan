//! Technical inspection of an installed app, without root and read-only.
//!
//! A single `adb shell` (~0.3 s) gathers what Android knows about the app that
//! does not appear in `pm list`: ABI, APK splits, signature, ART compilation,
//! per-user state, standby bucket, accesses recorded by App Ops, recent exits
//! (`ApplicationExitInfo`) and the state of its processes. The components and
//! their exposure are read from the manifest itself with `unzip -p`.
//!
//! None of the commands changes anything: `dumpsys`, `pm get-app-links`, `am
//! get-standby-bucket` and `cmd appops get` only query.

use crate::i18n::tr;
use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::apk::ManifestSummary;
use crate::exec::shell_quote;
use crate::probe::{self, Probe};

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| Regex::new($pat).expect("valid regex"));
    };
}

/// Inspection script. The package is already validated and is also quoted.
pub fn inspect_script(package: &str) -> String {
    let p = shell_quote(package);
    format!(
        "p={p}
echo @@@DUMP@@@; dumpsys package \"$p\" 2>&1
echo @@@PATH@@@; pm path \"$p\" 2>&1
echo @@@LINKS@@@; pm get-app-links \"$p\" 2>&1
echo @@@BUCKET@@@; am get-standby-bucket \"$p\" 2>&1
echo @@@APPOPS@@@; cmd appops get \"$p\" 2>&1
echo @@@IDLE@@@; dumpsys deviceidle whitelist 2>&1
echo @@@EXIT@@@; dumpsys activity exit-info \"$p\" 2>&1 | head -n 200
echo @@@LRU@@@; dumpsys activity lru 2>&1
echo @@@END@@@
"
    )
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

/// State of the app for user 0 (`User 0:` from `dumpsys package`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UserState {
    pub installed: Option<bool>,
    pub hidden: Option<bool>,
    pub suspended: Option<bool>,
    /// Stopped: it receives no notifications or alarms until it is opened.
    pub stopped: Option<bool>,
    /// Never opened since it was installed.
    pub not_launched: Option<bool>,
    /// 0 default · 1 enabled · 2 disabled · 3 by the user · 4 until used.
    pub enabled: Option<i64>,
    pub quarantined: Option<bool>,
}

/// Data from `dumpsys package <package>` that [`crate::models::PackageInfo`] does not
/// collect.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PackageExtras {
    pub code_path: String,
    pub primary_abi: Option<String>,
    pub secondary_abi: Option<String>,
    /// APK splits (`base`, `config.arm64_v8a`, `config.es`…).
    pub splits: Vec<String>,
    /// APK signature scheme (1, 2, 3, 4); `None` if Android does not say.
    pub signing_scheme: Option<u32>,
    pub installer: Option<String>,
    pub initiating_installer: Option<String>,
    pub data_dir: Option<String>,
    pub shared_user: Option<String>,
    pub user: UserState,
    /// Permissions declared by the app itself, with their protection level.
    pub declared_permissions: Vec<(String, String)>,
    /// Components disabled at runtime.
    pub disabled_components: Vec<String>,
    /// Factory version when the installed one is an update (name, path).
    pub factory_version: Option<(String, String)>,
    /// ART compilation of the base APK (status, reason).
    pub dexopt: Option<(String, String)>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppLinks {
    /// SHA-256 of the signing certificates.
    pub signatures: Vec<String>,
    /// (domain, verification state).
    pub domains: Vec<(String, String)>,
}

/// Use of a protected operation recorded by App Ops.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppOpUse {
    pub op: String,
    pub mode: String,
    /// Last allowed access, in seconds before the read.
    pub last_access_ago: Option<f64>,
    /// Last denied attempt, in seconds before the read.
    pub last_reject_ago: Option<f64>,
    pub duration: Option<f64>,
}

/// A process exit (`ApplicationExitInfo`, Android 11+).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExitRecord {
    pub timestamp: String,
    pub process: String,
    pub reason: i64,
    pub subreason: String,
    pub status: Option<i64>,
    pub importance: Option<i64>,
    pub rss: String,
    pub description: String,
}

impl ExitRecord {
    /// Crash, native crash, ANR or startup error.
    pub fn is_failure(&self) -> bool {
        matches!(self.reason, 4..=7)
    }

    pub fn reason_label(&self) -> String {
        let base = exit_reason_label(self.reason);
        // SIGKILL (9) is usually the system freeing memory or the user closing it.
        match (self.reason, self.status) {
            (2, Some(9)) => format!("{base} (SIGKILL)"),
            (2, Some(s)) => format!("{base} ({s})"),
            _ => base.to_string(),
        }
    }
}

pub fn exit_reason_label(code: i64) -> &'static str {
    match code {
        1 => tr("Exited by itself"),
        2 => tr("Stopped by a signal"),
        3 => tr("Out of memory"),
        4 => tr("Crash (exception)"),
        5 => tr("Native crash"),
        6 => tr("Not responding (ANR)"),
        7 => tr("Failed to start"),
        8 => tr("Permission change"),
        9 => tr("Excessive resource use"),
        10 => tr("Closed by the user"),
        11 => tr("Stopped by the user"),
        12 => tr("A process it depended on died"),
        13 => tr("Other system reason"),
        14 => tr("Frozen by the system"),
        15 => tr("Package state change"),
        16 => tr("The app was updated"),
        _ => tr("Unknown reason"),
    }
}

/// Exit sub-reason in words (the most common ones); the rest in lower case.
pub fn exit_subreason_label(sub: &str) -> String {
    match sub {
        "REMOVE TASK" => tr("swiped away from Recents").into(),
        "FORCE STOP" => tr("force stop").into(),
        "KILL BACKGROUND" => tr("closed to free memory").into(),
        "TOO MANY CACHED" | "TOO MANY EMPTY" => tr("too many cached apps").into(),
        "TRIM EMPTY" => tr("cleanup of empty processes").into(),
        "LARGER THAN QUOTA" | "EXCESSIVE CPU" => tr("excessive usage").into(),
        "KILL ALL BG EXCEPT" | "KILL ALL FG" => tr("system-wide shutdown of apps").into(),
        "STOP APP" => tr("stopped").into(),
        "UNKNOWN" | "" => String::new(),
        other => other.to_lowercase(),
    }
}

/// Importance of a process (`RunningAppProcessInfo.importance`).
pub fn importance_label(code: i64) -> &'static str {
    match code {
        100 => tr("on screen"),
        125 => tr("foreground service"),
        200 => "visible",
        230 => "perceptible",
        300 => tr("service"),
        325 => tr("on screen with the screen off"),
        350 => tr("cannot save state"),
        400 => tr("cached"),
        1000 => tr("no process"),
        _ => "",
    }
}

/// A process in the activity manager's LRU list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LruEntry {
    pub pid: i64,
    pub process: String,
    /// Abbreviated priority adjustment (`fg`, `vis`, `cch+ 5`…).
    pub adj: String,
    /// Abbreviated process state (`TOP`, `FGS`, `CEM`…).
    pub state: String,
}

impl LruEntry {
    pub fn package(&self) -> &str {
        self.process.split(':').next().unwrap_or("")
    }
}

/// Process state according to Android (`ProcessList.makeProcStateString`).
pub fn proc_state_label(code: &str) -> &str {
    match code {
        "PER" | "PERU" => tr("Persistent system process"),
        "TOP" => tr("On screen"),
        "BTOP" => tr("Bound to the app on screen"),
        "FGS" => tr("Foreground service"),
        "BFGS" => tr("Bound to a foreground service"),
        "IMPF" => tr("Perceptible to the user"),
        "IMPB" => tr("Important in the background"),
        "TRNB" => tr("Transient background"),
        "BKUP" => tr("Backup"),
        "SVC" => tr("Background service"),
        "RCVR" => tr("Handling a broadcast"),
        "TPSL" => tr("On screen (screen off)"),
        "HVY" => tr("Heavy-weight process"),
        "HOME" => tr("Home screen"),
        "LAST" => tr("Last app used"),
        "CAC" | "CACC" | "CRE" | "CEM" => tr("Cached (paused)"),
        "NONE" => tr("No state"),
        other => other,
    }
}

/// The app's standby bucket (App Standby Bucket, Android 9+).
pub fn standby_bucket_label(bucket: u32) -> (&'static str, &'static str) {
    match bucket {
        5 => (tr("Exempted"), tr("The system does not limit it")),
        10 => (tr("Active"), tr("In use: no limits")),
        20 => (tr("Working set"), tr("Mild limits on jobs and alarms")),
        30 => (tr("Frequent"), tr("Jobs and alarms limited")),
        40 => (tr("Rare"), tr("Jobs, alarms and network heavily limited")),
        45 => (tr("Restricted"), tr("Can hardly run in the background")),
        50 => (tr("Never used"), tr("Has not been opened")),
        _ => (tr("Unknown"), ""),
    }
}

/// Everything known about an app. Each item carries its own status.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppInspection {
    pub package: String,
    pub read_at: f64,
    pub extras: Probe<PackageExtras>,
    pub links: Probe<AppLinks>,
    pub standby_bucket: Probe<u32>,
    pub appops: Probe<Vec<AppOpUse>>,
    /// `Ok(Some(type))` if it is exempt from battery optimisation.
    pub battery_exemption: Probe<Option<String>>,
    pub exits: Probe<Vec<ExitRecord>>,
    /// Live processes of the app; empty if it is not running.
    pub processes: Probe<Vec<LruEntry>>,
    pub manifest: Probe<ManifestSummary>,
}

impl AppInspection {
    /// Path of the base APK according to `pm path` or, failing that, `dumpsys`.
    pub fn base_apk(path_text: &str, extras: &Probe<PackageExtras>) -> Option<String> {
        let paths: Vec<&str> = path_text.lines().filter_map(|l| l.trim().strip_prefix("package:")).collect();
        paths.iter().find(|p| p.ends_with("/base.apk")).or_else(|| paths.first()).map(|p| p.to_string()).or_else(|| extras.ok().map(|e| e.code_path.clone()).filter(|c| c.ends_with(".apk")))
    }

    pub fn component_disabled_at_runtime(&self, name: &str) -> bool {
        self.extras.ok().is_some_and(|e| e.disabled_components.iter().any(|c| c == name))
    }
}

/// Parses the output of [`inspect_script`] (without the manifest, which arrives
/// separately).
pub fn parse_inspection(package: &str, text: &str) -> AppInspection {
    let s = crate::parsers::split_sections(text);
    let extras = probe::from_section(s.get("DUMP"), |t| parse_package_extras(t, package));
    AppInspection {
        package: package.to_string(),
        read_at: crate::models::now_secs(),
        links: probe::from_section(s.get("LINKS"), |t| parse_app_links(t).filter(|l| !l.domains.is_empty() || !l.signatures.is_empty())),
        standby_bucket: probe::from_section(s.get("BUCKET"), |t| t.trim().parse().ok()),
        appops: probe::from_section(s.get("APPOPS"), |t| t.contains(':').then(|| parse_appops(t))),
        battery_exemption: probe::from_section(s.get("IDLE"), |t| Some(deviceidle_exemption(t, package))),
        exits: exit_section(s.get("EXIT")),
        processes: probe::from_section(s.get("LRU"), |t| t.contains("LRU").then(|| parse_lru(t).into_iter().filter(|e| e.package() == package).collect())),
        extras,
        manifest: Probe::Missing,
    }
}

fn exit_section(section: Option<&String>) -> Probe<Vec<ExitRecord>> {
    let Some(text) = section else {
        return Probe::Failed(tr("no response").into());
    };
    if let Some(p) = probe::classify_failure(text) {
        return p.map_never();
    }
    // Before Android 11 `dumpsys activity` does not know “exit-info”.
    if !text.contains("EXIT INFO") {
        return Probe::Unsupported(tr("Android 11 or later").into());
    }
    Probe::Ok(parse_exit_info(text))
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

/// `  Package [package]` block within a top-level section (`Packages:`, `Hidden
/// system packages:`), up to the next package or section.
fn package_block<'a>(text: &'a str, header: &str, package: &str) -> Option<&'a str> {
    let marker = format!("Package [{package}]");
    let (mut offset, mut in_section, mut start) = (0usize, false, None);
    for line in text.split_inclusive('\n') {
        let l = line.trim_end_matches(['\n', '\r']);
        let top_level = !l.is_empty() && !l.starts_with(' ');
        if top_level || (in_section && l.starts_with("  Package [")) {
            if let Some(s) = start {
                return Some(&text[s..offset]);
            }
        }
        if top_level {
            in_section = l == header;
        } else if in_section && l.starts_with("  Package [") && l.trim_start().starts_with(&marker) {
            start = Some(offset);
        }
        offset += line.len();
    }
    start.map(|s| &text[s..])
}

pub fn parse_package_extras(text: &str, package: &str) -> Option<PackageExtras> {
    re!(KV, r"(?m)^\s*(\w+)=(.*)$");
    re!(USER0, r"(?m)^\s*User 0:(.*)$");
    re!(FLAG, r"(\w+)=(\S+)");
    re!(DEXOPT, r"\[status=([^\]]+)\]\s*\[reason=([^\]]+)\]");
    re!(SHARED, r"sharedUser=\S*?\{\S+\s+([^/\s]+)/(\d+)\}");

    let block = package_block(text, "Packages:", package)?;
    let mut kv: HashMap<&str, &str> = HashMap::new();
    for c in KV.captures_iter(block) {
        let (k, v) = (c.get(1).unwrap().as_str(), c.get(2).unwrap().as_str().trim());
        kv.entry(k).or_insert(v);
    }
    let some = |k: &str| kv.get(k).map(|v| v.to_string()).filter(|v| !v.is_empty() && v != "null");
    let mut out = PackageExtras {
        code_path: some("codePath").unwrap_or_default(),
        primary_abi: some("primaryCpuAbi"),
        secondary_abi: some("secondaryCpuAbi"),
        splits: kv.get("splits").map(|v| v.trim_matches(|c| c == '[' || c == ']').split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default(),
        signing_scheme: kv.get("apkSigningVersion").and_then(|v| v.parse().ok()).filter(|v: &u32| *v > 0),
        installer: some("installerPackageName"),
        initiating_installer: some("initiatingPackageName"),
        data_dir: some("dataDir"),
        shared_user: SHARED.captures(block).map(|c| format!("{} (UID {})", &c[1], &c[2])),
        ..Default::default()
    };
    if let Some(c) = USER0.captures(block) {
        let flags: HashMap<&str, &str> = FLAG.captures_iter(c.get(1).unwrap().as_str()).map(|f| (f.get(1).unwrap().as_str(), f.get(2).unwrap().as_str())).collect();
        let b = |k: &str| flags.get(k).map(|v| *v == "true");
        out.user = UserState {
            installed: b("installed"),
            hidden: b("hidden"),
            suspended: b("suspended"),
            stopped: b("stopped"),
            not_launched: b("notLaunched"),
            enabled: flags.get("enabled").and_then(|v| v.parse().ok()),
            quarantined: b("quarantined"),
        };
    }
    // Indented lists: “declared permissions:” and “disabledComponents:”.
    let list_after = |title: &str| -> Vec<String> {
        let mut lines = block.lines().skip_while(|l| l.trim() != title);
        let Some(head) = lines.next() else {
            return Vec::new();
        };
        let indent = head.len() - head.trim_start().len();
        lines.take_while(|l| !l.trim().is_empty() && l.len() - l.trim_start().len() > indent).map(|l| l.trim().to_string()).collect()
    };
    out.declared_permissions = list_after("declared permissions:")
        .into_iter()
        .map(|l| match l.split_once(": prot=") {
            Some((n, p)) => (n.to_string(), p.split(',').next().unwrap_or(p).to_string()),
            None => (l, String::new()),
        })
        .collect();
    out.disabled_components = list_after("disabledComponents:");
    if let Some(hidden) = package_block(text, "Hidden system packages:", package) {
        let get = |k: &str| hidden.lines().find_map(|l| l.trim().strip_prefix(&format!("{k}="))).map(|v| v.trim().to_string());
        if let (Some(v), Some(p)) = (get("versionName"), get("codePath")) {
            out.factory_version = Some((v, p));
        }
    }
    if let Some(start) = text.find("\nDexopt state:") {
        let dex = &text[start..];
        let marker = format!("[{package}]");
        if let Some(pos) = dex.find(&marker) {
            let after = &dex[pos..];
            let end = after[1..].find("\n  [").map(|e| e + 1).unwrap_or(after.len());
            if let Some(c) = DEXOPT.captures(&after[..end]) {
                out.dexopt = Some((c[1].to_string(), c[2].to_string()));
            }
        }
    }
    Some(out)
}

/// ART compilation state in words.
pub fn dexopt_label(status: &str) -> &'static str {
    match status {
        "speed-profile" => tr("Compiled according to use (profile)"),
        "speed" => tr("Fully compiled"),
        "everything" => tr("Fully compiled, including rarely used code"),
        "verify" => tr("Verified only: interpreted and compiled on the fly (JIT)"),
        "quicken" => tr("Verified and optimised (Android 8–11)"),
        "space-profile" | "space" => tr("Compiled to take little space"),
        "run-from-apk" | "run-from-apk-fallback" => tr("Runs from the APK without compiling"),
        "extract" => tr("Extracted, not compiled"),
        "error" => tr("Compilation error"),
        _ => tr("Unrecognised state"),
    }
}

pub fn parse_app_links(text: &str) -> Option<AppLinks> {
    re!(SIG, r"Signatures:\s*\[([0-9A-Fa-f:,\s]+)\]");
    re!(DOMAIN, r"^\s+([A-Za-z0-9*][A-Za-z0-9.*-]*\.[A-Za-z]{2,}):\s+(\S+)\s*$");
    let mut out = AppLinks::default();
    if let Some(c) = SIG.captures(text) {
        out.signatures = c[1].split(',').map(|s| s.trim().to_uppercase()).filter(|s| s.len() >= 32).collect();
    }
    let mut in_state = false;
    for line in text.lines() {
        let t = line.trim();
        if t == "Domain verification state:" {
            in_state = true;
            continue;
        }
        if in_state {
            match DOMAIN.captures(line) {
                Some(c) => {
                    let d = (c[1].to_string(), c[2].to_string());
                    if !out.domains.contains(&d) {
                        out.domains.push(d);
                    }
                }
                None => in_state = false,
            }
        }
    }
    Some(out)
}

pub fn domain_state_label(state: &str) -> &'static str {
    match state {
        "verified" => tr("Verified by Android"),
        "approved" => tr("Approved"),
        "denied" => tr("Denied"),
        "migrated" => tr("Migrated from a previous version"),
        "restored" => tr("Restored"),
        "legacy_failure" => tr("Verification failed"),
        "system_configured" => tr("Configured by the system"),
        "none" => tr("Not verified"),
        _ => tr("Other state"),
    }
}

/// `+2d6h11m6s649ms` → seconds.
pub fn parse_ago(text: &str) -> Option<f64> {
    re!(PART, r"(\d+)(ms|d|h|m|s)");
    let t = text.trim().trim_start_matches('+');
    let mut total = 0.0;
    let mut any = false;
    for c in PART.captures_iter(t) {
        let n: f64 = c[1].parse().ok()?;
        total += n * match &c[2] {
            "d" => 86_400.0,
            "h" => 3_600.0,
            "m" => 60.0,
            "s" => 1.0,
            _ => 0.001,
        };
        any = true;
    }
    any.then_some(total)
}

/// `cmd appops get <package>`: only the operations with a recorded access or
/// rejection (the rest are the default configuration).
pub fn parse_appops(text: &str) -> Vec<AppOpUse> {
    re!(LINE, r"^(?:Uid mode:\s*)?([A-Z][A-Z0-9_]+):\s*([a-z_]+)(.*)$");
    re!(TIME, r"\btime=(\+\S+?) ago");
    re!(REJECT, r"\brejectTime=(\+\S+?) ago");
    re!(DURATION, r"\bduration=(\+\S+?)(?:;|$)");
    let mut out: Vec<AppOpUse> = Vec::new();
    for line in text.lines() {
        let Some(c) = LINE.captures(line.trim()) else {
            continue;
        };
        let rest = &c[3];
        let last_access_ago = TIME.captures(rest).and_then(|m| parse_ago(&m[1]));
        let last_reject_ago = REJECT.captures(rest).and_then(|m| parse_ago(&m[1]));
        if last_access_ago.is_none() && last_reject_ago.is_none() {
            continue;
        }
        let op = c[1].to_string();
        if out.iter().any(|o| o.op == op) {
            continue;
        }
        out.push(AppOpUse { op, mode: c[2].to_string(), last_access_ago, last_reject_ago, duration: DURATION.captures(rest).and_then(|m| parse_ago(&m[1])) });
    }
    out.sort_by(|a, b| {
        let key = |o: &AppOpUse| o.last_access_ago.or(o.last_reject_ago).unwrap_or(f64::MAX);
        key(a).total_cmp(&key(b))
    });
    out
}

/// Operations that touch privacy (camera, microphone, location…).
pub fn is_sensitive_op(op: &str) -> bool {
    matches!(
        op,
        "CAMERA"
            | "RECORD_AUDIO"
            | "FINE_LOCATION"
            | "COARSE_LOCATION"
            | "MONITOR_LOCATION"
            | "MONITOR_HIGH_POWER_LOCATION"
            | "READ_CONTACTS"
            | "READ_CALL_LOG"
            | "READ_SMS"
            | "READ_CLIPBOARD"
            | "READ_MEDIA_IMAGES"
            | "READ_MEDIA_VIDEO"
            | "READ_MEDIA_AUDIO"
            | "READ_MEDIA_VISUAL_USER_SELECTED"
            | "READ_EXTERNAL_STORAGE"
            | "BODY_SENSORS"
            | "ACTIVITY_RECOGNITION"
            | "READ_PHONE_STATE"
            | "READ_CALENDAR"
            | "GET_ACCOUNTS"
    )
}

pub fn op_label(op: &str) -> String {
    let known = match op {
        "CAMERA" => tr("Camera"),
        "RECORD_AUDIO" => tr("Microphone"),
        "FINE_LOCATION" => tr("Precise location"),
        "COARSE_LOCATION" => tr("Approximate location"),
        "MONITOR_LOCATION" => tr("Location monitoring"),
        "MONITOR_HIGH_POWER_LOCATION" => tr("High-power location"),
        "READ_CONTACTS" => tr("Read contacts"),
        "READ_CALL_LOG" => tr("Read the call log"),
        "READ_SMS" => tr("Read SMS"),
        "READ_CLIPBOARD" => tr("Read the clipboard"),
        "WRITE_CLIPBOARD" => tr("Write to the clipboard"),
        "READ_MEDIA_IMAGES" => tr("Read photos"),
        "READ_MEDIA_VIDEO" => tr("Read videos"),
        "READ_MEDIA_AUDIO" => tr("Read audio"),
        "READ_MEDIA_VISUAL_USER_SELECTED" => tr("Photos chosen by the user"),
        "READ_EXTERNAL_STORAGE" => tr("Read storage"),
        "WRITE_EXTERNAL_STORAGE" => tr("Write to storage"),
        "BODY_SENSORS" => tr("Body sensors"),
        "ACTIVITY_RECOGNITION" => tr("Physical activity"),
        "READ_PHONE_STATE" => tr("Phone state"),
        "READ_CALENDAR" => tr("Read the calendar"),
        "GET_ACCOUNTS" => tr("See accounts"),
        "POST_NOTIFICATION" => tr("Show notifications"),
        "WAKE_LOCK" => tr("Keep the phone awake"),
        "START_FOREGROUND" => tr("Foreground service"),
        "SYSTEM_ALERT_WINDOW" => tr("Display over other apps"),
        "GET_USAGE_STATS" => tr("Usage statistics"),
        "TAKE_AUDIO_FOCUS" => tr("Take audio focus"),
        "PLAY_AUDIO" => tr("Play audio"),
        "VIBRATE" => tr("Vibrate"),
        "RUN_IN_BACKGROUND" => tr("Run in the background"),
        "RUN_ANY_IN_BACKGROUND" => tr("Run in the background (any task)"),
        "BLUETOOTH_SCAN" => tr("Scan for Bluetooth devices"),
        "BLUETOOTH_CONNECT" => tr("Connect to Bluetooth"),
        "NEARBY_WIFI_DEVICES" => tr("Nearby Wi-Fi devices"),
        "SCHEDULE_EXACT_ALARM" => tr("Exact alarms"),
        "REQUEST_INSTALL_PACKAGES" => tr("Install apps"),
        "MANAGE_EXTERNAL_STORAGE" => tr("All files access"),
        "CONTROL_AUDIO" => tr("Control audio"),
        "CONTROL_AUDIO_PARTIAL" => tr("Control audio (partial)"),
        _ => "",
    };
    if known.is_empty() {
        op.replace('_', " ").to_lowercase()
    } else {
        known.to_string()
    }
}

/// Battery optimisation exemption: `system`, `user`, `system-excidle`…
pub fn deviceidle_exemption(text: &str, package: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let mut f = l.trim().split(',');
        let kind = f.next()?;
        (f.next()? == package).then(|| kind.to_string())
    })
}

pub fn exemption_label(kind: &str) -> &'static str {
    match kind {
        "system" => tr("Exempted by the system"),
        "system-excidle" => tr("Exempted by the system (except in deep idle)"),
        "user" => tr("Exempted by the user"),
        _ => tr("Exempted"),
    }
}

pub fn parse_exit_info(text: &str) -> Vec<ExitRecord> {
    re!(HEAD, r"timestamp=(\S+ \S+)");
    re!(PROC, r"process=(\S+) reason=(\d+)(?: \([^)]*\))? subreason=\d+(?: \(([^)]*)\))? status=(-?\d+)");
    re!(IMP, r"importance=(\d+)");
    re!(RSS, r"\brss=(\S+)");
    re!(DESC, r"description=(.*?) state=");
    let mut out = Vec::new();
    for block in text.split("ApplicationExitInfo #").skip(1) {
        let Some(p) = PROC.captures(block) else {
            continue;
        };
        out.push(ExitRecord {
            timestamp: HEAD.captures(block).map(|c| c[1].to_string()).unwrap_or_default(),
            process: p[1].to_string(),
            reason: p[2].parse().unwrap_or(0),
            subreason: p.get(3).map(|m| m.as_str().to_string()).filter(|s| s != "UNKNOWN").unwrap_or_default(),
            status: p[4].parse().ok(),
            importance: IMP.captures(block).and_then(|c| c[1].parse().ok()),
            rss: RSS.captures(block).map(|c| c[1].to_string()).unwrap_or_default(),
            description: DESC.captures(block).map(|c| c[1].trim().to_string()).filter(|d| d != "null").unwrap_or_default(),
        });
    }
    out
}

/// `dumpsys activity lru`: state of each process according to Android.
pub fn parse_lru(text: &str) -> Vec<LruEntry> {
    re!(LINE, r"^\s*#\s*\d+:\s+([a-z]+(?:\+\s*\d+)?)\s+([A-Z]+)\s+\S+\s+(\d+):([^/\s]+)/\S+");
    text.lines()
        .filter_map(|l| LINE.captures(l))
        .map(|c| LruEntry { pid: c[3].parse().unwrap_or(0), process: c[4].to_string(), adj: c[1].split_whitespace().collect(), state: c[2].to_string() })
        .collect()
}
