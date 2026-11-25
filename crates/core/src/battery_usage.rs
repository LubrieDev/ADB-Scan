//! Battery usage per app and wakelocks (without root, read-only).
//!
//! - Usage: `dumpsys batterystats --usage` (BatteryUsageStats, the same as
//!   Android's battery screen). Each app only adds up the periods **on
//!   battery**: the total in each UID's header includes charging time and is no
//!   use for knowing what drains the battery.
//! - Wakelocks: readable summary of `dumpsys batterystats --charged`, filtered
//!   on the phone (lines with a two-space indent are the global summary; those
//!   with more indentation, the per-UID detail).
//! - Names: `pm list packages -U` (UID → packages).
//!
//! The options that clear or write statistics (`--reset`, `--reset-all`,
//! `--write`, `enable`, `disable`) are never used.

use crate::{i18n::tr, t};
use std::collections::HashMap;

/// Per-app usage and package names (in a single call).
pub const USAGE_SCRIPT: &str = "echo '@@@USAGE@@@'; dumpsys batterystats --usage; echo '@@@UIDS@@@'; pm list packages -U";

/// Time on battery and wakelocks since the last full charge.
pub const WAKELOCK_SCRIPT: &str = "dumpsys batterystats --charged | grep -E '^  (Time on battery|All partial wake locks:|All kernel wake locks:|Wake lock |Kernel Wake lock )'";

/// Usage of a UID on battery, in mAh.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppUsage {
    pub uid: u32,
    /// Packages sharing the UID (empty for system ones).
    pub packages: Vec<String>,
    pub mah: f64,
    pub screen_on_mah: f64,
    pub screen_off_mah: f64,
}

impl AppUsage {
    pub fn is_app(&self) -> bool {
        self.uid % 100_000 >= 10_000
    }

    /// Readable name: package (and how many more share the UID) or the system
    /// component.
    pub fn label(&self) -> String {
        // System UIDs share dozens of packages (“android”,
        // “com.android.shell”…): the component's name is better.
        if !self.is_app() {
            return system_uid_name(self.uid);
        }
        match self.packages.as_slice() {
            [] => t!("Uninstalled app or another user's app (u{}a{})", self.uid / 100_000, self.uid % 100_000 - 10_000),
            [one] => one.clone(),
            [first, rest @ ..] => format!("{first} (+{})", rest.len()),
        }
    }
}

/// Names of the most common system UIDs.
pub fn system_uid_name(uid: u32) -> String {
    match uid % 100_000 {
        0 => tr("Kernel and root services").into(),
        1000 => tr("Android system").into(),
        1001 => tr("Phone (radio)").into(),
        1002 => "Bluetooth".into(),
        1013 => tr("Media server").into(),
        1021 => "GPS".into(),
        1041 => tr("Audio server").into(),
        1046 => tr("Media codecs").into(),
        1047 => tr("Camera server").into(),
        1073 => tr("Network stack").into(),
        2000 => tr("ADB shell (includes ADB Scan's telemetry)").into(),
        u => t!("System process (UID {})", u),
    }
}

/// A wakelock: keeps the CPU awake (`partial`) or the kernel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WakeLock {
    /// Owner UID (only for app ones).
    pub uid: Option<u32>,
    pub tag: String,
    pub ms: u64,
    pub count: u64,
    /// The longest single one, if Android reports it.
    pub max_ms: Option<u64>,
}

impl WakeLock {
    /// Short tag: scheduled jobs carry the full service path
    /// (“*job*r/com.app/...SystemJobService”).
    pub fn short_tag(&self) -> String {
        // Grouped: several tags, one per line; the first one wins.
        let t = self.tag.lines().next().unwrap_or("");
        if let Some(rest) = t.strip_prefix("*job*") {
            let service = rest.rsplit('/').next().unwrap_or(rest);
            let service = service.rsplit('.').next().unwrap_or(service);
            return t!("Background job ({})", service);
        }
        if let Some(rest) = t.strip_prefix("*alarm*:") {
            return t!("Alarm ({})", rest.trim());
        }
        if let Some(rest) = t.strip_prefix("*sync*/") {
            return t!("Sync ({})", rest.split('/').next().unwrap_or(rest));
        }
        t.to_string()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BatteryUsage {
    /// Time on battery since the last full charge.
    pub on_battery_ms: Option<u64>,
    pub screen_off_ms: Option<u64>,
    /// Total usage on battery (apps + unattributed), in mAh.
    pub total_mah: f64,
    /// Sorted from highest to lowest usage; without those using 0.
    pub apps: Vec<AppUsage>,
    pub wakelocks: Vec<WakeLock>,
    pub kernel_wakelocks: Vec<WakeLock>,
    /// UID → packages, to name the wakelock owners.
    pub uid_packages: HashMap<u32, Vec<String>>,
}

impl BatteryUsage {
    /// Owner of a UID: package (and how many more share it) or system
    /// component.
    pub fn owner(&self, uid: u32) -> String {
        AppUsage { uid, packages: self.uid_packages.get(&uid).cloned().unwrap_or_default(), ..Default::default() }.label()
    }

    pub fn percent_of(&self, mah: f64) -> Option<f64> {
        (self.total_mah > 0.0).then(|| (mah / self.total_mah * 100.0).clamp(0.0, 100.0))
    }

    /// App wakelocks grouped by owner and task: Android splits the same job
    /// into normal (`*job*r/`) and expedited (`*job*e/`). Time and counts are
    /// added up; `tag` keeps the original tags.
    pub fn grouped_wakelocks(&self) -> Vec<WakeLock> {
        let mut out: Vec<WakeLock> = Vec::new();
        for w in &self.wakelocks {
            let key = w.short_tag();
            match out.iter_mut().find(|g| g.uid == w.uid && g.short_tag() == key) {
                Some(g) => {
                    g.ms += w.ms;
                    g.count += w.count;
                    g.max_ms = g.max_ms.max(w.max_ms);
                    if !g.tag.split('\n').any(|t| t == w.tag) {
                        g.tag = format!("{}\n{}", g.tag, w.tag);
                    }
                }
                None => out.push(w.clone()),
            }
        }
        out.sort_by_key(|w| std::cmp::Reverse(w.ms));
        out
    }

    /// What is not attributed to any app (radio, GPU, idle…).
    pub fn unattributed_mah(&self) -> f64 {
        (self.total_mah - self.apps.iter().map(|a| a.mah).sum::<f64>()).max(0.0)
    }
}

/// “u0a441” → 10441; “u10a5” → 1010005; “1000” → 1000.
pub fn parse_uid(token: &str) -> Option<u32> {
    if let Some(rest) = token.strip_prefix('u') {
        let (user, app) = rest.split_once('a')?;
        let user: u32 = user.parse().ok()?;
        let app: u32 = app.parse().ok()?;
        return Some(user * 100_000 + 10_000 + app);
    }
    token.parse().ok()
}

/// “1d 8h 35m 37s 295ms” → milliseconds.
pub fn parse_duration_ms(text: &str) -> Option<u64> {
    let mut total = 0u64;
    let mut any = false;
    for part in text.split_whitespace() {
        let (num, unit) = part.find(|c: char| !c.is_ascii_digit()).map(|i| part.split_at(i))?;
        let n: u64 = num.parse().ok()?;
        total += n * match unit {
            "d" => 86_400_000,
            "h" => 3_600_000,
            "m" => 60_000,
            "s" => 1000,
            "ms" => 1,
            _ => return None,
        };
        any = true;
    }
    any.then_some(total)
}

/// Adds up the top-level components (“cpu=58.3”; not “cpu:fg=…”).
fn sum_components(text: &str) -> f64 {
    text.split_whitespace().filter_map(|t| t.split_once('=')).filter(|(name, _)| !name.contains(':') && !name.is_empty()).filter_map(|(_, v)| v.parse::<f64>().ok()).sum()
}

/// `pm list packages -U`: UID → packages.
pub fn parse_uid_packages(text: &str) -> HashMap<u32, Vec<String>> {
    let mut map: HashMap<u32, Vec<String>> = HashMap::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("package:") else {
            continue;
        };
        let Some((pkg, uid)) = rest.rsplit_once(" uid:") else {
            continue;
        };
        // With several users, “uid:10441,1010441”.
        for u in uid.split(',') {
            if let Ok(u) = u.trim().parse::<u32>() {
                let list = map.entry(u).or_default();
                if !list.iter().any(|p| p == pkg) {
                    list.push(pkg.to_string());
                }
            }
        }
    }
    for list in map.values_mut() {
        list.sort();
    }
    map
}

/// `dumpsys batterystats --usage`: (total on battery, apps).
pub fn parse_usage(text: &str) -> (f64, Vec<AppUsage>) {
    #[derive(PartialEq)]
    enum Block {
        None,
        Global,
        GlobalOnBattery,
        Uid,
    }
    let mut block = Block::None;
    let mut total = 0.0;
    let mut apps: Vec<AppUsage> = Vec::new();
    for line in text.lines() {
        let s = line.trim();
        if s == "Global" {
            block = Block::Global;
            continue;
        }
        if let Some(rest) = s.strip_prefix("UID ") {
            block = Block::None;
            let Some((uid, _)) = rest.split_once(':') else {
                continue;
            };
            if let Some(uid) = parse_uid(uid) {
                apps.push(AppUsage { uid, ..Default::default() });
                block = Block::Uid;
            }
            continue;
        }
        match block {
            Block::Global | Block::GlobalOnBattery => {
                if s.starts_with("(on battery") {
                    block = Block::GlobalOnBattery;
                } else if s.starts_with('(') {
                    block = Block::Global;
                } else if block == Block::GlobalOnBattery {
                    // “screen: 1914 apps: 1914”: the first number is the total.
                    if let Some(v) = s.split_whitespace().nth(1).and_then(|v| v.parse::<f64>().ok()) {
                        total += v;
                    }
                }
            }
            Block::Uid => {
                if let (Some(app), Some(state)) = (apps.last_mut(), s.strip_prefix("(on battery, ")) {
                    let Some((kind, rest)) = state.split_once(')') else {
                        continue;
                    };
                    let mah = sum_components(rest);
                    app.mah += mah;
                    if kind.starts_with("screen on") {
                        app.screen_on_mah += mah;
                    } else {
                        app.screen_off_mah += mah;
                    }
                }
            }
            Block::None => {}
        }
    }
    // The same UID can appear in several blocks (e.g. per user).
    let mut merged: Vec<AppUsage> = Vec::new();
    for a in apps {
        match merged.iter_mut().find(|m| m.uid == a.uid) {
            Some(m) => {
                m.mah += a.mah;
                m.screen_on_mah += a.screen_on_mah;
                m.screen_off_mah += a.screen_off_mah;
            }
            None => merged.push(a),
        }
    }
    merged.retain(|a| a.mah >= 0.005);
    merged.sort_by(|a, b| b.mah.total_cmp(&a.mah));
    (total, merged)
}

/// Summary of `--charged`: (time on battery, with the screen off, app
/// wakelocks, kernel wakelocks), sorted by time.
pub fn parse_wakelocks(text: &str) -> (Option<u64>, Option<u64>, Vec<WakeLock>, Vec<WakeLock>) {
    let mut on_battery = None;
    let mut screen_off = None;
    let mut partial = Vec::new();
    let mut kernel = Vec::new();
    for line in text.lines() {
        let s = line.trim();
        if let Some(rest) = s.strip_prefix("Time on battery screen off: ") {
            screen_off = rest.split(" (").next().and_then(parse_duration_ms);
        } else if let Some(rest) = s.strip_prefix("Time on battery: ") {
            on_battery = rest.split(" (").next().and_then(parse_duration_ms);
        } else if let Some(rest) = s.strip_prefix("Kernel Wake lock ") {
            if let Some(w) = parse_wakelock_line(rest, false) {
                kernel.push(w);
            }
        } else if let Some(rest) = s.strip_prefix("Wake lock ") {
            if let Some(w) = parse_wakelock_line(rest, true) {
                partial.push(w);
            }
        }
    }
    partial.retain(|w| w.ms > 0);
    kernel.retain(|w| w.ms > 0);
    partial.sort_by_key(|w| std::cmp::Reverse(w.ms));
    kernel.sort_by_key(|w| std::cmp::Reverse(w.ms));
    (on_battery, screen_off, partial, kernel)
}

/// “u0a543 TAG: 1h 1m 51s 609ms (15 times) max=1807241 actual=… realtime”.
/// The tag may contain “: ”, so the time is searched from the right: it is what
/// follows the last “: ” before “(N times)”.
fn parse_wakelock_line(rest: &str, with_uid: bool) -> Option<WakeLock> {
    let (uid, rest) = if with_uid {
        let (u, r) = rest.split_once(' ')?;
        (Some(parse_uid(u)?), r)
    } else {
        (None, rest)
    };
    let times_at = rest.rfind(" times)")?;
    let open = rest[..times_at].rfind(" (")?;
    let count: u64 = rest[open + 2..times_at].trim().parse().ok()?;
    let colon = rest[..open].rfind(": ")?;
    let tag = rest[..colon].trim().to_string();
    let ms = parse_duration_ms(&rest[colon + 2..open])?;
    let max_ms = rest[times_at..].split_whitespace().find_map(|t| t.strip_prefix("max=")).and_then(|v| v.parse().ok());
    Some(WakeLock { uid, tag: tag.trim_matches('"').to_string(), ms, count, max_ms })
}

/// Joins the two outputs. `usage_text` carries the USAGE and UIDS sections.
pub fn build(usage_text: &str, wakelock_text: &str) -> BatteryUsage {
    let sections = crate::parsers::split_sections(usage_text);
    let get = |k: &str| sections.get(k).map(String::as_str).unwrap_or("");
    let packages = parse_uid_packages(get("UIDS"));
    let (total, mut apps) = parse_usage(get("USAGE"));
    for a in &mut apps {
        if a.is_app() {
            a.packages = packages.get(&a.uid).cloned().unwrap_or_default();
        }
    }
    let (on_battery_ms, screen_off_ms, wakelocks, kernel_wakelocks) = parse_wakelocks(wakelock_text);
    let apps_total: f64 = apps.iter().map(|a| a.mah).sum();
    BatteryUsage {
        on_battery_ms,
        screen_off_ms,
        // If the global total is missing or does not add up, at least the apps'
        // one.
        total_mah: total.max(apps_total),
        apps,
        wakelocks,
        kernel_wakelocks,
        uid_packages: packages,
    }
}
