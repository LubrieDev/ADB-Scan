//! Per-app usage and wakelocks. Real Android 16 formats (REDMAGIC 11S Pro);
//! package names were replaced with sample ones. The expected values were
//! computed separately with a prototype script.

use adbscan_core::battery_usage::*;

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn scripts_only_read() {
    // No option that clears or writes statistics.
    for script in [USAGE_SCRIPT, WAKELOCK_SCRIPT, ALARM_SCRIPT] {
        for forbidden in ["--reset", "--write", "--new-daily", " enable", " disable", "--history-create"] {
            assert!(!script.contains(forbidden), "{forbidden} in {script}");
        }
    }
}

#[test]
fn uid_and_duration_parsing() {
    assert_eq!(parse_uid("u0a441"), Some(10441));
    assert_eq!(parse_uid("u10a5"), Some(1_010_005));
    assert_eq!(parse_uid("1000"), Some(1000));
    assert_eq!(parse_uid("uXa1"), None);
    assert_eq!(parse_duration_ms("1d 8h 35m 37s 295ms"), Some(117_337_295));
    assert_eq!(parse_duration_ms("609ms"), Some(609));
    assert_eq!(parse_duration_ms(""), None);
    assert_eq!(parse_duration_ms("3 times"), None);
}

#[test]
fn real_usage_counts_only_time_on_battery() {
    let (total, apps) = parse_usage(&fixture("batterystats_usage.txt"));
    assert!(close(total, 3323.585), "{total}");
    let by = |uid: u32| apps.iter().find(|a| a.uid == uid).unwrap_or_else(|| panic!("UID {uid}"));
    // The header says 1035 mAh, but it includes charging time.
    let video = by(10441);
    assert!(close(video.mah, 989.2528), "{}", video.mah);
    assert!(close(video.screen_on_mah, 984.6008));
    assert!(close(video.screen_off_mah, 4.652));
    // The ADB shell: 817 mAh in the header, almost all of it while plugged in.
    assert!(close(by(2000).mah, 36.9401));
    assert!(close(by(1000).mah, 96.2579));
    assert!(close(by(10543).screen_off_mah, 45.442));
    // Sorted from highest to lowest.
    assert!(apps.windows(2).all(|w| w[0].mah >= w[1].mah));
    assert_eq!(apps[0].uid, 10441);
}

#[test]
fn real_wakelocks_and_time_on_battery() {
    let (on, off, partial, kernel) = parse_wakelocks(&fixture("batterystats_wakelocks.txt"));
    assert_eq!(on, Some(117_337_295));
    assert_eq!(off, Some(90_913_882));
    assert_eq!(kernel.len(), 5);
    assert_eq!(kernel[0].tag, "PowerManagerService.WakeLocks");
    assert_eq!(kernel[0].count, 8);
    assert_eq!(kernel[0].uid, None);
    assert_eq!(partial.len(), 4);
    // Sorted by time: the 1 h 1 min job first.
    assert_eq!(partial[0].uid, Some(10543));
    assert_eq!(partial[0].ms, 3_711_609);
    assert_eq!(partial[0].count, 15);
    assert_eq!(partial[0].max_ms, Some(1_807_241));
    assert_eq!(partial[0].short_tag(), "Background job (SystemJobService)");
    assert_eq!(partial[1].tag, "AudioMix");
    assert!(partial.windows(2).all(|w| w[0].ms >= w[1].ms));
}

#[test]
fn same_job_normal_and_expedited_are_grouped() {
    let usage = format!("@@@USAGE@@@\n{}\n@@@UIDS@@@\n{}", fixture("batterystats_usage.txt"), fixture("pm_packages_uid.txt"));
    let u = build(&usage, &fixture("batterystats_wakelocks.txt"));
    let g = u.grouped_wakelocks();
    // *job*r (1 h 1 min 51 s, 15) + *job*e (8 min 54 s, 10) of the same
    // service.
    assert_eq!(g.len(), 3);
    assert_eq!(g[0].uid, Some(10543));
    assert_eq!(g[0].ms, 3_711_609 + 534_310);
    assert_eq!(g[0].count, 25);
    assert_eq!(g[0].tag.lines().count(), 2, "keeps both tags");
    assert_eq!(g[0].short_tag(), "Background job (SystemJobService)");
    assert!(g.windows(2).all(|w| w[0].ms >= w[1].ms));
}

#[test]
fn wakelock_tags_may_contain_colons() {
    let (_, _, partial, _) = parse_wakelocks("  Wake lock u0a543 WorkManager: ProcessorForegroundLck: 6m 10s 187ms (3 times) max=437100 actual=619734 realtime\n");
    assert_eq!(partial[0].tag, "WorkManager: ProcessorForegroundLck");
    assert_eq!(partial[0].ms, 370_187);
    assert_eq!(partial[0].count, 3);
}

#[test]
fn build_names_owners_and_unattributed() {
    let usage = format!("@@@USAGE@@@\n{}\n@@@UIDS@@@\n{}", fixture("batterystats_usage.txt"), fixture("pm_packages_uid.txt"));
    let u = build(&usage, &fixture("batterystats_wakelocks.txt"));
    assert_eq!(u.apps[0].label(), "com.example.video");
    // Two packages with the same UID.
    assert_eq!(u.owner(10543), "com.example.chat (+1)");
    assert_eq!(u.owner(1000), "Android system");
    assert!(u.owner(2000).contains("ADB Scan"), "the ADB shell includes our telemetry");
    // App UID without a package: it is not “system”.
    assert!(u.owner(10578).starts_with("Uninstalled app"));
    let pct = u.percent_of(u.apps[0].mah).unwrap();
    assert!((pct - 29.76).abs() < 0.05, "{pct}");
    assert!(u.unattributed_mah() > 0.0);
    assert_eq!(u.on_battery_ms, Some(117_337_295));
    // No data: nothing made up.
    let empty = build("", "");
    assert!(empty.apps.is_empty());
    assert_eq!(empty.percent_of(10.0), None);
}

#[test]
fn real_alarms_who_wakes_the_phone() {
    let (since, alarms) = parse_alarms(&fixture("alarm_stats.txt"));
    assert_eq!(since.as_deref(), Some("2026-11-26 14:30:59"));
    assert_eq!((alarms[0].package.as_str(), alarms[0].uid, alarms[0].wakeups), ("android", Some(1000), 37));
    let gms = &alarms[1];
    assert_eq!((gms.package.as_str(), gms.uid, gms.wakeups, gms.running_ms), ("com.google.android.gms", Some(10405), 34, 810));
    assert_eq!(gms.top_tag, "com.google.android.chimera.IntentOperation.TARGETED_INTENT");
    assert_eq!(gms.alarms, 26 + 7 + 5 + 10 + 1 + 1 + 1);
    // Sorted by wakeups; the statistics block after them is not an app.
    assert!(alarms.windows(2).all(|w| w[0].wakeups >= w[1].wakeups));
    assert!(alarms.iter().all(|a| !a.package.contains(' ')));
    assert_eq!(parse_compact_duration_ms("+1h2m3s4ms"), Some(3_723_004));
    assert_eq!(parse_compact_duration_ms("+1s145ms"), Some(1145));
    assert_eq!(parse_compact_duration_ms("12x"), None);
}

#[test]
fn real_background_tasks_per_app() {
    let jobs = parse_jobs(&fixture("batterystats_jobs.txt"));
    assert_eq!(jobs.len(), 80);
    assert_eq!((jobs[0].package.as_str(), jobs[0].ms, jobs[0].count), ("com.example.app17", 30_897_364, 719));
    assert_eq!(jobs[1].package, "com.android.vending");
    // `@namespace@android/…` belongs to Android.
    assert!(jobs.iter().any(|j| j.package == "android"));
    assert!(jobs.iter().all(|j| !j.package.starts_with('@') && !j.package.contains('/')));
}

/// Android 11 (moto e20): no `--usage`; the `Estimated power use` block of
/// `--charged` (real lines; UIDs only).
#[test]
fn android_11_estimate() {
    let text = "  Estimated power use (mAh):\n    Capacity: 4000, Computed drain: 7888, actual drain: 9680-10120\n    Cell standby: 2386 ( radio=2386 ) Excluded from smearing\n\
    Screen: 1328 Excluded from smearing\n    Uid 0: 455 ( cpu=455 ) Excluded from smearing\n\
    Uid u0a160: 299 ( cpu=65.5 wake=3.56 ) Including smearing: 2201 ( screen=1041 proportional=860 )\n\
    Uid u0a189: 285 ( cpu=246 wake=38.7 ) Excluded from smearing\n\
    Uid u0a112: 0.00000450 ( wake=0.00000450 ) Including smearing: 0.00000738 ( proportional=0.00000288 )\n\n";
    let (total, apps) = parse_estimated(text);
    assert!(close(total, 7888.0));
    assert_eq!(apps.len(), 3);
    // Its own use plus its share of the screen.
    assert_eq!(apps[0].uid, 10160);
    assert!(close(apps[0].mah, 1340.0));
    assert_eq!(apps[1].uid, 0);
    assert_eq!(apps[2].uid, 10189);
    let usage = format!("@@@ESTIMATE@@@\n{text}@@@UIDS@@@\npackage:com.example.app1 uid:10160\n");
    let u = build(&usage, "");
    assert!(close(u.total_mah, 7888.0));
    assert_eq!(u.apps[0].packages, ["com.example.app1"]);
}
