//! Screen time and data per app (read-only), from what Android itself counts:
//!
//! - **Screen time**: `dumpsys usagestats -c` (`-c`: times in milliseconds).
//!   Only the in-memory daily and weekly blocks are kept, filtered on the
//!   phone (the full dump also has every event of the last 24 hours, several
//!   megabytes). Per app: time visible on screen, time in the foreground,
//!   launches and last use.
//! - **Data**: `dumpsys netstats --checkin --uid --duration=…`, already added
//!   up by the phone per UID and network (`cell` = metered mobile data,
//!   `wifi`), in two-hour buckets. Android keeps per-app history for 90 days.
//! - **Interruptions**: the same dump, filtered on the phone with `grep -F`
//!   (fast; `awk` over the whole dump takes seconds): notifications per app in
//!   the last 24 hours (only the package of each one, never its content), and
//!   the daily and weekly counts of unlocks and screen-ons.
//! - **Unused apps**: the yearly block (last use of each app), the installed
//!   user apps (`pm list packages -3`) and when each was installed
//!   (`firstInstallTime` in `dumpsys package packages`).
//! - **Names**: `pm list packages -U` (UID → packages).
//!
//! Formats from Android 16's sources (`UserUsageStatsService.printIntervalStats`,
//! `NetworkStatsCollection.dumpCheckin`).

use std::collections::HashMap;

use crate::battery_usage::{parse_uid_packages, system_uid_name};
use crate::i18n::tr;
use crate::t;

/// Days of data usage read.
pub const DATA_DAYS: u64 = 30;

/// Read-only. The `sed` keeps the daily and weekly package blocks.
pub fn script() -> String {
    format!(
        "echo @@@SCREEN@@@; dumpsys usagestats -c 2>&1 | sed -n '/In-memory daily stats/,/ChooserCounts/p;/In-memory weekly stats/,/ChooserCounts/p;/In-memory yearly stats/,/ChooserCounts/p'
echo @@@EVENTS@@@; dumpsys usagestats -c 2>/dev/null | grep -F -e NOTIFICATION_INTERRUPTION -e 'In-memory' -e 'Last 24 hour events' -e 'screen-interactive:' -e 'keyguard-hidden:' | sed -E 's/^ *time=[0-9]+ type=NOTIFICATION_INTERRUPTION package=([^ ]+).*/N \\1/'
echo @@@THIRD@@@; pm list packages -3 2>&1
echo @@@LAUNCH@@@; cmd package query-activities --brief -a android.intent.action.MAIN -c android.intent.category.LAUNCHER 2>&1
echo @@@INSTALL@@@; dumpsys package packages 2>/dev/null | grep -F -e '  Package [' -e 'firstInstallTime='
echo @@@NET@@@; dumpsys netstats --checkin --uid --duration={} 2>&1
echo @@@UIDS@@@; pm list packages -U 2>&1
echo @@@END@@@
",
        DATA_DAYS * 86_400_000
    )
}

/// Screen time of one app in a period.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppTime {
    pub package: String,
    /// Visible on screen (what Digital Wellbeing counts).
    pub visible_ms: u64,
    /// In the foreground (focused).
    pub foreground_ms: u64,
    pub launches: u64,
    /// Unix time, ms (0: unknown).
    pub last_used_ms: u64,
}

impl AppTime {
    /// The time shown: visible, or in the foreground on older versions
    /// without the visible time.
    pub fn screen_ms(&self) -> u64 {
        self.visible_ms.max(self.foreground_ms)
    }
}

/// A period of screen time (today or this week, as Android splits them).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Period {
    /// Unix time, ms.
    pub begin_ms: u64,
    pub end_ms: u64,
    /// Apps with some screen time, most used first.
    pub apps: Vec<AppTime>,
}

impl Period {
    pub fn total_ms(&self) -> u64 {
        self.apps.iter().map(AppTime::screen_ms).sum()
    }
}

/// Data used by one UID (an app, or apps sharing it).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppData {
    pub uid: i64,
    pub packages: Vec<String>,
    pub mobile_bytes: u64,
    pub wifi_bytes: u64,
    /// Ethernet, Bluetooth, proxy.
    pub other_bytes: u64,
}

impl AppData {
    pub fn total(&self) -> u64 {
        self.mobile_bytes + self.wifi_bytes + self.other_bytes
    }

    /// An app's own UID (not the system's).
    pub fn is_app(&self) -> bool {
        self.uid >= 0 && self.uid % 100_000 >= 10_000
    }

    /// Readable name: the package (and how many more share it) or the system
    /// part that used it.
    pub fn label(&self) -> String {
        match self.uid {
            -5 => return tr("Hotspot and tethering").into(),
            -4 => return tr("Uninstalled apps").into(),
            u if u < 0 => return t!("Other (UID {})", u),
            _ => {}
        }
        if !self.is_app() {
            return system_uid_name(self.uid as u32);
        }
        match self.packages.as_slice() {
            [] => t!("Uninstalled app or another user's app (UID {})", self.uid),
            [one] => one.clone(),
            [first, rest @ ..] => format!("{first} (+{})", rest.len()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataUsage {
    /// Unix time, seconds.
    pub from: u64,
    pub to: u64,
    /// Most data first; without those that used none.
    pub apps: Vec<AppData>,
}

impl DataUsage {
    pub fn mobile_total(&self) -> u64 {
        self.apps.iter().map(|a| a.mobile_bytes).sum()
    }
    pub fn wifi_total(&self) -> u64 {
        self.apps.iter().map(|a| a.wifi_bytes).sum()
    }
}

/// Unlocks and screen-ons in a period.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pickups {
    pub unlocks: u64,
    pub screen_ons: u64,
}

/// An installed user app not opened for a long time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnusedApp {
    pub package: String,
    /// Unix ms; 0: not opened at all in the period Android keeps.
    pub last_used_ms: u64,
    pub installed_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    /// `None` if Android did not answer (or answered something else).
    pub today: Option<Period>,
    pub week: Option<Period>,
    /// Up to a year back (when its block starts: `begin_ms`).
    pub year: Option<Period>,
    pub data: Option<DataUsage>,
    /// Notifications that interrupted, per app, in the last 24 hours (most
    /// first). `None`: not read.
    pub notifications: Option<Vec<(String, u64)>>,
    pub pickups_today: Option<Pickups>,
    pub pickups_week: Option<Pickups>,
    /// Installed user apps and when each was installed (Unix ms).
    pub user_apps: Vec<String>,
    pub installed_ms: HashMap<String, u64>,
    /// Apps with an icon in the launcher (the ones that can be opened).
    pub launchable: std::collections::HashSet<String>,
}

impl Usage {
    pub fn notifications_total(&self) -> u64 {
        self.notifications.iter().flatten().map(|(_, n)| n).sum()
    }

    /// User apps not opened in `days` (installed before that). Oldest use
    /// first; empty if the yearly block was not read.
    pub fn unused(&self, now_ms: u64, days: u64) -> Vec<UnusedApp> {
        let Some(year) = &self.year else { return Vec::new() };
        let cutoff = now_ms.saturating_sub(days * 86_400_000);
        let mut last: HashMap<&str, u64> = HashMap::new();
        for p in [&self.year, &self.week, &self.today].into_iter().flatten() {
            for a in &p.apps {
                let e = last.entry(a.package.as_str()).or_insert(0);
                *e = (*e).max(a.last_used_ms);
            }
        }
        let mut out: Vec<UnusedApp> = self
            .user_apps
            .iter()
            .filter_map(|pkg| {
                // Apps without an icon (services, keyboards) are never “opened”.
                if !self.launchable.is_empty() && !self.launchable.contains(pkg) {
                    return None;
                }
                let installed = *self.installed_ms.get(pkg)?;
                let used = last.get(pkg.as_str()).copied().unwrap_or(0);
                // Never seen: only if the yearly block reaches back past the
                // cutoff (otherwise it is not known).
                let known = used > 0 || year.begin_ms <= cutoff;
                (installed > 0 && installed < cutoff && used < cutoff && known).then(|| UnusedApp { package: pkg.clone(), last_used_ms: used, installed_ms: installed })
            })
            .collect();
        out.sort_by(|a, b| a.last_used_ms.cmp(&b.last_used_ms).then(a.installed_ms.cmp(&b.installed_ms)).then(a.package.cmp(&b.package)));
        out
    }
}

/// The filtered events: notifications per app (24 h) and the daily and
/// weekly unlocks and screen-ons.
/// Notifications per app, today's and this week's pickups.
pub type Events = (Option<Vec<(String, u64)>>, Option<Pickups>, Option<Pickups>);

pub fn parse_events(text: &str) -> Events {
    let mut counts: HashMap<String, u64> = HashMap::new();
    let (mut in_events, mut seen_events) = (false, false);
    // 1: daily, 2: weekly; only the first block of each.
    let mut block = 0;
    let (mut today, mut week): (Option<Pickups>, Option<Pickups>) = (None, None);
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with("Last 24 hour events") {
            in_events = true;
            seen_events = true;
            continue;
        }
        if l.starts_with("In-memory") {
            in_events = false;
            block = match l {
                _ if l.starts_with("In-memory daily") && today.is_none() => 1,
                _ if l.starts_with("In-memory weekly") && week.is_none() => 2,
                _ => 0,
            };
            if block == 1 {
                today = Some(Pickups::default());
            } else if block == 2 {
                week = Some(Pickups::default());
            }
            continue;
        }
        if in_events {
            if let Some(pkg) = l.strip_prefix("N ") {
                *counts.entry(pkg.trim().to_string()).or_insert(0) += 1;
            }
            continue;
        }
        let slot = match block {
            1 => today.as_mut(),
            2 => week.as_mut(),
            _ => None,
        };
        let Some(p) = slot else { continue };
        // `keyguard-hidden: 24x for 9345860`
        let n = |prefix: &str| l.strip_prefix(prefix).and_then(|r| r.trim().split('x').next()).and_then(|v| v.parse::<u64>().ok());
        if let Some(v) = n("keyguard-hidden:") {
            p.unlocks = v;
        } else if let Some(v) = n("screen-interactive:") {
            p.screen_ons = v;
        }
    }
    let notifications = seen_events.then(|| {
        let mut v: Vec<(String, u64)> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    });
    (notifications, today, week)
}

/// `cmd package query-activities --brief`: the packages of the `pkg/.Class`
/// lines.
fn parse_activity_packages(text: &str) -> std::collections::HashSet<String> {
    text.lines().map(str::trim).filter(|l| !l.contains('=') && !l.contains(' ')).filter_map(|l| l.split_once('/')).map(|(p, _)| p.to_string()).filter(|p| !p.is_empty()).collect()
}

/// `pm list packages -3`.
fn parse_package_list(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.trim().strip_prefix("package:")).map(|p| p.trim().to_string()).collect()
}

/// `  Package [com.app] (…)` followed by `firstInstallTime=2026-09-15
/// 17:45:01` (the phone's local time; read as this computer's).
fn parse_install_times(text: &str) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("Package [") {
            current = rest.split(']').next().map(str::to_string);
            continue;
        }
        let Some(v) = l.strip_prefix("firstInstallTime=") else { continue };
        let Some(pkg) = current.take() else { continue };
        if let Some(ms) = local_datetime_ms(v.trim()) {
            out.entry(pkg).or_insert(ms);
        }
    }
    out
}

/// `2026-09-15 17:45:01` (local time) → Unix ms.
fn local_datetime_ms(text: &str) -> Option<u64> {
    let (date, time) = text.split_once(' ')?;
    let d: Vec<i64> = date.split('-').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let t: Vec<i64> = time.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    if d.len() != 3 || t.len() != 3 {
        return None;
    }
    let days = crate::format::days_from_civil(d[0], d[1], d[2]);
    let secs = days * 86_400 + t[0] * 3600 + t[1] * 60 + t[2] - crate::format::local_offset_secs();
    u64::try_from(secs).ok().map(|s| s * 1000)
}

/// Value of `key=` among `key=value` pairs.
fn pair<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace().find_map(|w| w.strip_prefix(key)?.strip_prefix('='))
}

/// The daily, weekly and yearly blocks of `dumpsys usagestats -c`: (today,
/// week, year).
pub fn parse_screen(text: &str) -> (Option<Period>, Option<Period>, Option<Period>) {
    let mut slots: [Option<Period>; 3] = [None, None, None];
    // Index of the block being read; only the first block of each (user 0).
    let mut block: Option<usize> = None;
    let mut current: Option<Period> = None;
    let finish = |current: &mut Option<Period>, block: Option<usize>, slots: &mut [Option<Period>; 3]| {
        if let (Some(mut p), Some(b)) = (current.take(), block) {
            // The yearly block keeps every app (for the last use).
            if b < 2 {
                p.apps.retain(|a| a.screen_ms() > 0 || a.launches > 0);
            }
            p.apps.sort_by(|a, b| b.screen_ms().cmp(&a.screen_ms()).then(a.package.cmp(&b.package)));
            if slots[b].is_none() {
                slots[b] = Some(p);
            }
        }
    };
    for line in text.lines() {
        let l = line.trim();
        let which = ["In-memory daily stats", "In-memory weekly stats", "In-memory yearly stats"].iter().position(|h| l.starts_with(h));
        if let Some(i) = which {
            finish(&mut current, block, &mut slots);
            block = Some(i);
            current = Some(Period::default());
            continue;
        }
        if l.starts_with("ChooserCounts") || l.starts_with("In-memory") {
            finish(&mut current, block, &mut slots);
            block = None;
            continue;
        }
        let Some(p) = current.as_mut() else { continue };
        if let (Some(b), Some(e)) = (pair(l, "beginTime"), pair(l, "endTime")) {
            p.begin_ms = b.parse().unwrap_or(0);
            p.end_ms = e.parse().unwrap_or(0);
            continue;
        }
        let Some(package) = pair(l, "package") else { continue };
        let num = |k: &str| pair(l, k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        p.apps.push(AppTime { package: package.to_string(), visible_ms: num("totalTimeVisible"), foreground_ms: num("totalTimeUsed"), launches: num("appLaunchCount"), last_used_ms: num("lastTimeUsed") });
    }
    finish(&mut current, block, &mut slots);
    let [today, week, year] = slots;
    (today, week, year)
}

/// The `uid` section of `dumpsys netstats --checkin --uid`: lines
/// `c,<network>,<uid>,<set>,<tag>` followed by `b,<start>,<rx bytes>,<rx
/// packets>,<tx bytes>,<tx packets>,<operations>` buckets.
pub fn parse_data(text: &str, uids: &HashMap<u32, Vec<String>>) -> Option<DataUsage> {
    let mut lines = text.lines().map(str::trim);
    let header = lines.find(|l| l.starts_with("v1,"))?;
    let mut h = header.split(',').skip(1);
    let from: u64 = h.next()?.parse().ok()?;
    let to: u64 = h.next()?.parse().ok()?;
    let mut by_uid: HashMap<i64, AppData> = HashMap::new();
    let mut section = "";
    // (network, uid) of the current `c,` line.
    let mut current: Option<(String, i64)> = None;
    for l in lines {
        match l {
            "xt" | "uid" | "tag" => {
                section = l;
                current = None;
                continue;
            }
            _ => {}
        }
        if section != "uid" {
            continue;
        }
        let f: Vec<&str> = l.split(',').collect();
        match f.first().copied() {
            Some("c") if f.len() >= 5 => {
                // Only the total (tag 0).
                current = (f[4] == "0").then(|| (f[1].to_string(), f[2].parse().unwrap_or(i64::MIN))).filter(|(_, u)| *u != i64::MIN);
            }
            Some("b") if f.len() >= 5 => {
                let Some((net, uid)) = current.as_ref() else { continue };
                let n = |s: &str| s.parse::<u64>().unwrap_or(0);
                let bytes = n(f[2]) + n(f[4]);
                let e = by_uid.entry(*uid).or_insert_with(|| AppData { uid: *uid, packages: u32::try_from(*uid).ok().and_then(|u| uids.get(&u)).cloned().unwrap_or_default(), ..Default::default() });
                match net.as_str() {
                    "cell" => e.mobile_bytes += bytes,
                    "wifi" => e.wifi_bytes += bytes,
                    _ => e.other_bytes += bytes,
                }
            }
            _ => {}
        }
    }
    let mut apps: Vec<AppData> = by_uid.into_values().filter(|a| a.total() > 0).collect();
    apps.sort_by(|a, b| b.total().cmp(&a.total()).then(a.uid.cmp(&b.uid)));
    Some(DataUsage { from, to, apps })
}

pub fn parse(text: &str) -> Usage {
    let s = crate::parsers::split_sections(text);
    let empty = String::new();
    let get = |k: &str| s.get(k).unwrap_or(&empty);
    let (today, week, year) = parse_screen(get("SCREEN"));
    let uids = parse_uid_packages(get("UIDS"));
    let (notifications, pickups_today, pickups_week) = parse_events(get("EVENTS"));
    Usage {
        today,
        week,
        year,
        data: parse_data(get("NET"), &uids),
        notifications,
        pickups_today,
        pickups_week,
        user_apps: parse_package_list(get("THIRD")),
        installed_ms: parse_install_times(get("INSTALL")),
        launchable: parse_activity_packages(get("LAUNCH")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: &str = "In-memory daily stats
  beginTime=1764115200000 endTime=1764158400000
    packages
      package=com.example.chat totalTimeUsed=3600000 lastTimeUsed=1764150000000 totalTimeVisible=3700000 lastTimeVisible=1764150000000 lastTimeComponentUsed=0 totalTimeFS=0 lastTimeFS=0 appLaunchCount=12
      package=com.example.idle totalTimeUsed=0 lastTimeUsed=0 totalTimeVisible=0 lastTimeVisible=0 lastTimeComponentUsed=0 totalTimeFS=0 lastTimeFS=0 appLaunchCount=0
      package=com.example.video totalTimeUsed=5400000 lastTimeUsed=1764140000000 totalTimeVisible=5500000 lastTimeVisible=1764140000000 lastTimeComponentUsed=0 totalTimeFS=0 lastTimeFS=0 appLaunchCount=3

    ChooserCounts
      package=com.example.chat
In-memory weekly stats
  beginTime=1763856000000 endTime=1764158400000
    packages
      package=com.example.chat totalTimeUsed=36000000 lastTimeUsed=1764150000000 totalTimeVisible=36500000 lastTimeVisible=0 lastTimeComponentUsed=0 totalTimeFS=0 lastTimeFS=0 appLaunchCount=80
    ChooserCounts
";

    const NET: &str = "v1,1761566400,1764158400
xt
c,cell,-1,A,0
d,3600
b,1764000000,999,1,999,1,0
uid
c,cell,10441,B,0
d,7200
b,1764000000,1000,10,500,5,0
b,1764007200,2000,20,500,5,0
c,cell,10441,F,0
d,7200
b,1764000000,100,1,100,1,0
c,wifi,10441,F,0
d,7200
b,1764000000,50000,50,10000,10,0
c,wifi,1000,B,0
d,7200
b,1764000000,700,7,300,3,0
c,cell,-5,B,0
d,7200
b,1764000000,4000,4,1000,1,0
c,wifi,10543,F,0
d,7200
b,1764000000,0,0,0,0,0
tag
c,cell,10441,B,1234
d,7200
b,1764000000,99999,1,99999,1,0
";

    #[test]
    fn screen_time() {
        let (today, week, _) = parse_screen(SCREEN);
        let today = today.unwrap();
        assert_eq!(today.begin_ms, 1_764_115_200_000);
        // Without the idle app, longest first.
        assert_eq!(today.apps.iter().map(|a| a.package.as_str()).collect::<Vec<_>>(), ["com.example.video", "com.example.chat"]);
        assert_eq!(today.apps[1].launches, 12);
        assert_eq!(today.total_ms(), 9_200_000);
        assert_eq!(week.unwrap().apps[0].screen_ms(), 36_500_000);
    }

    #[test]
    fn interruptions() {
        // As the phone prints them after its filter (REDMAGIC NX809J,
        // Android 16; sample packages).
        let events = "  Last 24 hour events (beginTime=1795645241576 endTime=1795731641576 )
N com.example.chat
N com.example.chat
N com.example.mail
N com.example.chat
  In-memory daily stats
      screen-interactive: 49x for 12267685
      keyguard-hidden: 24x for 9345860
  In-memory weekly stats
      screen-interactive: 501x for 125843323
      keyguard-hidden: 499x for 142372956
  In-memory monthly stats
      screen-interactive: 621x for 125843323
      keyguard-hidden: 413x for 116633103
";
        let (n, today, week) = parse_events(events);
        assert_eq!(n.unwrap(), [("com.example.chat".to_string(), 3), ("com.example.mail".to_string(), 1)]);
        assert_eq!(today, Some(Pickups { unlocks: 24, screen_ons: 49 }));
        assert_eq!(week, Some(Pickups { unlocks: 499, screen_ons: 501 }));
        assert_eq!(parse_events(""), (None, None, None));
    }

    #[test]
    fn unused_apps() {
        let day = 86_400_000u64;
        let now = 1_795_731_641_576u64;
        let screen = format!(
            "In-memory yearly stats\n  beginTime={} endTime={now}\n    packages\n      package=com.example.old totalTimeUsed=100 lastTimeUsed={} totalTimeVisible=100 appLaunchCount=1\n      package=com.example.used totalTimeUsed=100 lastTimeUsed={} totalTimeVisible=100 appLaunchCount=1\n    ChooserCounts\n",
            now - 300 * day,
            now - 200 * day,
            now - 2 * day
        );
        let (_, _, year) = parse_screen(&screen);
        let year = year.unwrap();
        assert_eq!(year.apps.len(), 2);
        let installs = "  Package [com.example.old] (abc):\n    firstInstallTime=2025-01-01 10:00:00\n  Package [com.example.used] (def):\n    firstInstallTime=2025-01-01 10:00:00\n  Package [com.example.never] (123):\n    firstInstallTime=2025-02-01 10:00:00\n    firstInstallTime=2025-03-01 10:00:00\n  Package [com.example.new] (456):\n    firstInstallTime=2099-01-01 10:00:00\n";
        let installed_ms = parse_install_times(installs);
        assert_eq!(installed_ms.len(), 4);
        let u = Usage {
            year: Some(year),
            user_apps: parse_package_list("package:com.example.old\npackage:com.example.used\npackage:com.example.never\npackage:com.example.new\n"),
            installed_ms,
            ..Default::default()
        };
        let unused: Vec<String> = u.unused(now, 90).into_iter().map(|a| a.package).collect();
        // Never opened first, then the oldest use; not the recent or new ones.
        assert_eq!(unused, ["com.example.never", "com.example.old"]);
        assert_eq!(local_datetime_ms("2026-01-01 00:00:00").map(|m| m % 1000), Some(0));
        // Only apps with an icon count.
        let launch = parse_activity_packages("3 activities found:\n  Activity #0:\n    priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=true\n    com.example.old/.Main\n  Activity #1:\n    com.example.new/com.example.new.Start\n");
        assert_eq!(launch.len(), 2);
        let u = Usage { launchable: launch, ..u };
        let unused: Vec<String> = u.unused(now, 90).into_iter().map(|a| a.package).collect();
        assert_eq!(unused, ["com.example.old"]);
    }

    #[test]
    fn data_per_app() {
        let mut uids = HashMap::new();
        uids.insert(10441, vec!["com.example.video".to_string()]);
        let d = parse_data(NET, &uids).unwrap();
        assert_eq!((d.from, d.to), (1_761_566_400, 1_764_158_400));
        let video = d.apps.iter().find(|a| a.uid == 10441).unwrap();
        // Background and foreground added up; the tag section is not counted.
        assert_eq!(video.mobile_bytes, 4200);
        assert_eq!(video.wifi_bytes, 60_000);
        assert_eq!(video.label(), "com.example.video");
        assert_eq!(d.apps[0].uid, 10441);
        assert!(d.apps.iter().all(|a| a.uid != 10543));
        assert_eq!(d.apps.iter().find(|a| a.uid == -5).unwrap().label(), "Hotspot and tethering");
        assert_eq!(d.apps.iter().find(|a| a.uid == 1000).unwrap().label(), "Android system");
        assert_eq!(d.mobile_total(), 4200 + 5000);
    }

    #[test]
    fn whole_script_output() {
        let out = format!("@@@SCREEN@@@\n{SCREEN}@@@NET@@@\n{NET}@@@UIDS@@@\npackage:com.example.video uid:10441\n@@@END@@@\n");
        let u = parse(&out);
        assert!(u.today.is_some() && u.week.is_some());
        assert_eq!(u.data.unwrap().apps[0].packages, ["com.example.video"]);
        // Nothing from Android: nothing made up.
        let u = parse("@@@SCREEN@@@\nPermission Denial\n@@@NET@@@\n\n@@@UIDS@@@\n@@@END@@@\n");
        assert_eq!(u, Usage::default());
    }
}
