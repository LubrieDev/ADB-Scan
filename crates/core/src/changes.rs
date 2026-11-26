//! Phone changes: a timeline of what changed on the phone, from what Android
//! itself keeps. Read-only, and without root:
//!
//! - apps installed and updated: the dates the package manager keeps for each
//!   app (`firstInstallTime`, `lastUpdateTime`);
//! - apps removed: the battery history records each uninstall, but only since
//!   it was last reset (usually the last full charge or a few days);
//! - restarts and shutdowns: `persist.sys.boot.reason.history`, the last few,
//!   with their reason.
//!
//! Android keeps no dated history of settings or permission changes on a
//! normal phone (only development builds do), so those are not shown. What
//! ADB Scan itself changed is in its History (Operations).

use crate::format::days_from_civil;

/// Read-only script; its parts are separated by `@@@NAME@@@` lines.
pub const SCRIPT: &str = "echo @@@NOW@@@; date +%s; date +%z; \
echo @@@BOOTS@@@; getprop persist.sys.boot.reason.history; \
echo @@@BATTERY@@@; dumpsys batterystats --history 2>/dev/null | grep -E 'TIME: |pkgunin='; \
echo @@@PACKAGES@@@; dumpsys package packages 2>/dev/null | grep -E '^  Package \\[|firstInstallTime=|lastUpdateTime=|versionName='; \
echo @@@USER@@@; pm list packages -3 2>/dev/null";

/// Dates before this (seconds since 1970, 2010-01-01) are the factory
/// placeholders of preinstalled apps, not real installs.
const REAL_DATES_FROM: i64 = 1_262_304_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// An app was installed (apps the user installed).
    Installed { package: String, version: String },
    /// An app was updated (last update only: Android keeps no earlier ones).
    Updated { package: String, version: String },
    /// An app was uninstalled.
    Removed { package: String },
    /// The phone was restarted, with Android's reason (`reboot,userrequested`).
    Restarted { reason: String },
    /// The phone was turned off.
    TurnedOff { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Seconds since 1970 (UTC).
    pub time: i64,
    pub change: Change,
    /// A preinstalled app (not installed by the user).
    pub system: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Timeline {
    /// Newest first.
    pub entries: Vec<Entry>,
    /// From when removed apps are known (the start of the battery history).
    pub removals_since: Option<i64>,
}

impl Change {
    pub fn package(&self) -> Option<&str> {
        match self {
            Change::Installed { package, .. } | Change::Updated { package, .. } | Change::Removed { package } => Some(package),
            _ => None,
        }
    }
}

fn sections(text: &str) -> std::collections::HashMap<&str, Vec<&str>> {
    let mut out: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    let mut current = "";
    for line in text.lines() {
        let t = line.trim_end_matches('\r');
        if let Some(name) = t.trim().strip_prefix("@@@").and_then(|r| r.strip_suffix("@@@")) {
            current = name;
            out.entry(current).or_default();
        } else if !current.is_empty() {
            out.entry(current).or_default().push(t);
        }
    }
    out
}

/// `+0100` / `-0600` → seconds east of UTC.
fn parse_offset(s: &str) -> Option<i64> {
    let s = s.trim();
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => return None,
    };
    if rest.len() != 4 || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let h: i64 = rest[..2].parse().ok()?;
    let m: i64 = rest[2..].parse().ok()?;
    Some(sign * (h * 3600 + m * 60))
}

/// Phone-local `YYYY-MM-DD HH:MM:SS` → seconds since 1970 (UTC).
fn local_to_unix(date: &str, time: &str, offset: i64) -> Option<i64> {
    let d: Vec<i64> = date.split('-').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let t: Vec<i64> = time.split(':').map(|p| p.split('.').next().unwrap_or(p).parse().ok()).collect::<Option<_>>()?;
    if d.len() != 3 || t.len() != 3 || !(1..=12).contains(&d[1]) || !(1..=31).contains(&d[2]) || t[0] > 23 || t[1] > 59 || t[2] > 60 {
        return None;
    }
    Some(days_from_civil(d[0], d[1], d[2]) * 86400 + t[0] * 3600 + t[1] * 60 + t[2] - offset)
}

/// `reboot,userrequested,1795643910` → (time, reason).
fn parse_boot(line: &str) -> Option<(i64, String)> {
    let (reason, time) = line.trim().rsplit_once(',')?;
    let time: i64 = time.trim().parse().ok()?;
    (time > REAL_DATES_FROM).then(|| (time, reason.trim_end_matches(',').to_string()))
}

/// Uninstalls in the battery history. Its event lines carry `MM-DD
/// HH:MM:SS.mmm` in the phone's time; the year comes from the `TIME:
/// YYYY-MM-DD-HH-MM-SS` lines in between. Returns the uninstalls and the
/// first time in the history.
fn parse_battery(lines: &[&str], offset: i64) -> (Vec<(i64, String)>, Option<i64>) {
    let mut year: Option<(i64, i64)> = None; // (year, month) of the last TIME line
    let mut first = None;
    let mut out: Vec<(i64, String)> = Vec::new();
    for line in lines {
        let mut words = line.split_whitespace();
        let (Some(md), Some(clock)) = (words.next(), words.next()) else { continue };
        if let Some(stamp) = line.split("TIME: ").nth(1) {
            let p: Vec<&str> = stamp.trim().split('-').collect();
            if p.len() >= 6 {
                if let (Ok(y), Ok(m)) = (p[0].parse::<i64>(), p[1].parse::<i64>()) {
                    year = Some((y, m));
                    if first.is_none() {
                        first = local_to_unix(&format!("{}-{}-{}", p[0], p[1], p[2]), &format!("{}:{}:{}", p[3], p[4], p[5]), offset);
                    }
                }
            }
            continue;
        }
        let Some(pkg) = line.split("pkgunin=").nth(1).and_then(|r| r.split('"').nth(1)) else { continue };
        let Some((y, last_month)) = year else { continue };
        let Some((m, _)) = md.split_once('-') else { continue };
        let Ok(m) = m.parse::<i64>() else { continue };
        // A history that crosses New Year: January after a December TIME.
        let y = if m < last_month { y + 1 } else { y };
        if let Some(t) = local_to_unix(&format!("{y}-{md}"), clock, offset) {
            if !out.iter().any(|(tt, p)| *tt == t && p == pkg) {
                out.push((t, pkg.to_string()));
            }
        }
    }
    (out, first)
}

/// Reads [`SCRIPT`]'s output.
pub fn parse(text: &str) -> Timeline {
    let s = sections(text);
    let get = |name: &str| s.get(name).cloned().unwrap_or_default();
    let now_lines = get("NOW");
    let offset = now_lines.get(1).and_then(|l| parse_offset(l)).unwrap_or(0);
    let user: std::collections::HashSet<&str> = get("USER").iter().filter_map(|l| l.trim().strip_prefix("package:")).collect();
    let mut entries = Vec::new();

    for line in get("BOOTS") {
        if let Some((time, reason)) = parse_boot(line) {
            let change = if reason.starts_with("shutdown") { Change::TurnedOff { reason } } else { Change::Restarted { reason } };
            entries.push(Entry { time, change, system: false });
        }
    }

    let (removed, removals_since) = parse_battery(&get("BATTERY"), offset);
    for (time, package) in removed {
        entries.push(Entry { time, system: false, change: Change::Removed { package } });
    }

    // One package at a time: its version, last update and (first user's)
    // install date.
    let mut push_package = |pkg: &str, version: &str, first: Option<i64>, last: Option<i64>| {
        if pkg.is_empty() {
            return;
        }
        let system = !user.contains(pkg);
        let version = if version == "null" { String::new() } else { version.to_string() };
        let first = first.filter(|t| *t > REAL_DATES_FROM);
        let last = last.filter(|t| *t > REAL_DATES_FROM);
        if let (Some(t), false) = (first, system) {
            entries.push(Entry { time: t, system, change: Change::Installed { package: pkg.to_string(), version: version.clone() } });
        }
        if let Some(t) = last {
            // An install is also its “last update”: only a later date is an update.
            if first.is_none_or_older(t) {
                entries.push(Entry { time: t, system, change: Change::Updated { package: pkg.to_string(), version } });
            }
        }
    };
    let (mut pkg, mut version, mut first, mut last) = (String::new(), String::new(), None, None);
    for line in get("PACKAGES") {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Package [") {
            push_package(&pkg, &version, first, last);
            pkg = rest.split(']').next().unwrap_or("").to_string();
            version.clear();
            first = None;
            last = None;
        } else if let Some(v) = t.strip_prefix("versionName=") {
            version = v.to_string();
        } else if let Some(v) = t.strip_prefix("lastUpdateTime=") {
            last = v.split_once(' ').and_then(|(d, c)| local_to_unix(d, c, offset));
        } else if let Some(v) = t.strip_prefix("firstInstallTime=") {
            // Several users: the first one listed is user 0.
            if first.is_none() {
                first = v.split_once(' ').and_then(|(d, c)| local_to_unix(d, c, offset));
            }
        }
    }
    push_package(&pkg, &version, first, last);

    entries.sort_by_key(|e| std::cmp::Reverse(e.time));
    Timeline { entries, removals_since }
}

/// `Option<i64>::is_none_or` for Rust 1.80: true when there is no install
/// date or it is more than a minute before `t`.
trait OlderThan {
    fn is_none_or_older(&self, t: i64) -> bool;
}

impl OlderThan for Option<i64> {
    fn is_none_or_older(&self, t: i64) -> bool {
        !matches!(self, Some(first) if t - *first < 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
@@@NOW@@@
1795667708
-0600
@@@BOOTS@@@
reboot,userrequested,1795643910
reboot,,1795547341
shutdown,,1795482731
@@@BATTERY@@@
  12-30 23:00:00.000 TIME: 2026-12-30-23-00-00
  12-31 10:00:00.000 067 pkgunin=1001:\"com.example.old\"
  12-31 10:00:00.001 067 -wifi pkgunin=1001:\"com.example.old\"
  01-02 08:30:00.000 067 pkgunin=1002:\"com.example.gone\"
@@@PACKAGES@@@
  Package [com.vendor.tools] (33a10d5):
    versionName=16
    lastUpdateTime=1970-01-02 09:54:56
      firstInstallTime=1970-01-02 09:54:56
  Package [com.vendor.store] (aa):
    versionName=5.1
    lastUpdateTime=2026-11-20 10:00:00
      firstInstallTime=1970-01-02 09:54:56
  Package [com.example.chat] (bb):
    versionName=2.26
    lastUpdateTime=2026-11-25 02:14:21
      firstInstallTime=2026-11-19 11:30:12
      firstInstallTime=2026-11-21 09:00:00
  Package [com.example.fresh] (cc):
    versionName=null
    lastUpdateTime=2026-11-24 08:00:20
      firstInstallTime=2026-11-24 08:00:00
@@@USER@@@
package:com.example.chat
package:com.example.fresh
";

    fn at(date: &str, time: &str) -> i64 {
        local_to_unix(date, time, -6 * 3600).unwrap()
    }

    #[test]
    fn reads_every_kind_of_change() {
        let t = parse(SAMPLE);
        let kinds: Vec<String> = t
            .entries
            .iter()
            .map(|e| match &e.change {
                Change::Installed { package, .. } => format!("+{package}"),
                Change::Updated { package, version } => format!("^{package} {version}"),
                Change::Removed { package } => format!("-{package}"),
                Change::Restarted { reason } => format!("restart {reason}"),
                Change::TurnedOff { reason } => format!("off {reason}"),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "-com.example.gone",
                "-com.example.old",
                "restart reboot,userrequested",
                "^com.example.chat 2.26",
                "restart reboot",
                "+com.example.fresh",
                "off shutdown",
                "^com.vendor.store 5.1",
                "+com.example.chat",
            ]
        );
        // Newest first.
        assert!(t.entries.windows(2).all(|w| w[0].time >= w[1].time));
    }

    #[test]
    fn dates_are_the_phones_local_time() {
        let t = parse(SAMPLE);
        let chat = t.entries.iter().find(|e| e.change == Change::Installed { package: "com.example.chat".into(), version: "2.26".into() }).unwrap();
        // 11:30:12 at UTC−6 is 17:30:12 UTC; the first user (user 0) counts.
        assert_eq!(chat.time, at("2026-11-19", "11:30:12"));
        assert_eq!(chat.time % 86400, 17 * 3600 + 30 * 60 + 12);
        assert!(!chat.system);
    }

    #[test]
    fn factory_dates_and_installs_are_not_updates() {
        let t = parse(SAMPLE);
        // A preinstalled app never updated: nothing; updated: one entry, marked
        // as system; a fresh install is not also an update.
        assert!(t.entries.iter().all(|e| e.change.package() != Some("com.vendor.tools")));
        let store: Vec<&Entry> = t.entries.iter().filter(|e| e.change.package() == Some("com.vendor.store")).collect();
        assert_eq!(store.len(), 1);
        assert!(store[0].system);
        assert_eq!(t.entries.iter().filter(|e| e.change.package() == Some("com.example.fresh")).count(), 1);
        // "null" versions are left empty.
        assert!(t.entries.iter().any(|e| e.change == Change::Installed { package: "com.example.fresh".into(), version: String::new() }));
    }

    #[test]
    fn removals_cross_new_year_without_repeats() {
        let t = parse(SAMPLE);
        let gone = t.entries.iter().find(|e| e.change == Change::Removed { package: "com.example.gone".into() }).unwrap();
        assert_eq!(gone.time, at("2027-01-02", "08:30:00"));
        assert_eq!(t.entries.iter().filter(|e| e.change == Change::Removed { package: "com.example.old".into() }).count(), 1);
        assert_eq!(t.removals_since, Some(at("2026-12-30", "23:00:00")));
    }

    #[test]
    fn odd_output_gives_an_empty_timeline() {
        for text in ["", "garbage", "@@@NOW@@@\nx\ny\n@@@BOOTS@@@\nreboot,,abc\n,\n@@@PACKAGES@@@\n  Package [\n    lastUpdateTime=2026-13-45 99:00:00\n"] {
            assert_eq!(parse(text).entries, vec![], "{text}");
        }
    }

    #[test]
    fn the_script_only_reads() {
        for verb in ["setprop", "settings put", "pm uninstall", "pm install", "rm ", " > "] {
            assert!(!SCRIPT.contains(verb), "{verb}");
        }
    }
}
