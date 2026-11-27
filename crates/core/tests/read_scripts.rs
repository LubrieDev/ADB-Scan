//! The read scripts of the newer pages (Preinstalled apps, Permissions,
//! Storage, Developer, Phone changes, Usage, Stability, default apps, app
//! limits, settings snapshots, per-app tuning, keyboards, radio, memory,
//! alarms) only read: no verb that changes the phone.

use adbscan_core::{app_limits, app_tuning, battery_usage, changes, debloat, devtools, keyboards, permissions, procstats, radio, roles, settings_snapshot, stability, storage, usage};

#[test]
fn read_scripts_only_read() {
    let built = [usage::script(), stability::script(), roles::script(), app_limits::state_script("com.example.app"), app_tuning::script("com.example.app")];
    let fixed = [
        debloat::STATE_SCRIPT,
        permissions::SCRIPT,
        devtools::STATE_SCRIPT,
        storage::SCAN_SCRIPT,
        changes::SCRIPT,
        settings_snapshot::SCRIPT,
        keyboards::SCRIPT,
        radio::WIFI_SCRIPT,
        radio::MOBILE_SCRIPT,
        procstats::SCRIPT,
        battery_usage::ALARM_SCRIPT,
        battery_usage::WAKELOCK_SCRIPT,
    ];
    for script in fixed.iter().copied().chain(built.iter().map(String::as_str)) {
        for forbidden in [
            "setprop",
            "settings put",
            "settings delete",
            " set ",
            "appops set",
            "pm uninstall",
            "pm install",
            "install-existing",
            "pm grant",
            "pm revoke",
            "pm clear",
            "pm disable",
            "pm enable",
            "disallow_listener",
            "am broadcast",
            "wm size ",
            "wm density ",
            "reverse",
            "forward",
            "kill",
            "suspend",
            "add-role-holder",
            "netpolicy add",
            "netpolicy remove",
            "set-standby-bucket",
            "forget-network",
            "bg-dexopt",
            "--poll",
            "whitelist +",
            "whitelist -",
            "ime set",
            "ime enable",
            "game mode",
            "game set",
            "game reset",
            "set-app-locales",
            "procstats --reset",
            "--clear",
            "> /",
            ">/data",
            ">/sdcard",
        ] {
            assert!(!script.contains(forbidden), "“{forbidden}” in the script:\n{script}");
        }
        // `rm` as a command, not inside a word (“alarm”).
        assert!(!script.split(|c: char| !c.is_ascii_alphanumeric() && c != '-').any(|w| w == "rm"), "“rm” in the script:\n{script}");
    }
}

/// The personal export only queries.
#[test]
fn personal_export_only_queries() {
    for k in adbscan_core::personal_export::Kind::ALL {
        let q = k.query().join(" ");
        assert!(q.starts_with("content query --uri content://"), "{q}");
        for verb in ["insert", "update", "delete", "call"] {
            assert!(!q.contains(&format!(" {verb} ")), "{q}");
        }
    }
}
