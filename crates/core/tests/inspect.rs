//! Platform sheet and app inspection with real outputs from a REDMAGIC 11S Pro
//! (Android 16). In `app_inspect.txt` the parts naming other installed apps
//! were removed (Queries, LRU list and battery whitelist trimmed).
//! `manifest_florisboard.bin` is the manifest of FlorisBoard, an open-source
//! app (Apache-2.0).

use adbscan_core::apk::{self, ComponentKind};
use adbscan_core::app_inspect::{self, AppInspection};
use adbscan_core::platform;
use adbscan_core::probe::Probe;

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

// -- platform -----------------------------------------------------------------

#[test]
fn platform_real() {
    let p = platform::parse_platform(&fixture("platform.txt"));
    assert_eq!(p.selinux, Probe::Ok("Enforcing".into()));
    let k = p.kernel.ok().unwrap();
    assert_eq!(k.release, "6.12.38-android16-5-g665eafb62659-ab14778838-4k");
    assert_eq!(k.compiler, "clang 19.0.1");
    assert_eq!(k.gki.as_deref(), Some("android16-6.12"));
    assert_eq!(k.flags, "SMP PREEMPT");
    assert_eq!(p.page_size, Probe::Ok(4096));
    let g = p.gles.ok().unwrap();
    assert_eq!((g.vendor.as_str(), g.renderer.as_str(), g.version.as_str()), ("Qualcomm", "Adreno (TM) 840", "OpenGL ES 3.2 V@0842.44"));
    let v = p.vulkan.ok().unwrap();
    assert_eq!(v.device, "Adreno (TM) 840");
    assert_eq!(v.api, "1.4.295");
    // Matches the GLES version (V@0842.44).
    assert_eq!(v.driver_version, "512.842.44");
    assert_eq!(v.driver_name, "Qualcomm Technologies Inc. Adreno Vulkan Driver");
    assert_eq!(p.gpu_model, Probe::Ok("Adreno840v2".into()));
    let c = p.clusters.ok().unwrap();
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].cpus_label(), "CPU 0–5");
    assert_eq!(c[1].cpus_label(), "CPU 6, 7");
    assert_eq!((c[0].min_khz, c[0].max_khz), (Some(384_000), Some(3_628_800)));
    assert_eq!(c[1].max_khz, Some(4_742_400));
    assert_eq!(c[0].governor, Probe::Ok("walt".into()));
    assert_eq!(c[0].steps, Probe::Ok(28));
    let t = p.thermal_limits.ok().unwrap();
    let kinds: Vec<_> = t.iter().map(|l| l.kind.as_str()).collect();
    assert_eq!(kinds, ["CPU", "GPU", "Battery", "Skin"]);
    assert_eq!(t[0].throttling, Some((105.0, 105.0)));
    assert_eq!(t[3].shutdown, Some((90.0, 90.0)));
    let f = p.features.ok().unwrap();
    assert_eq!(f.len(), 121);
    let hl = platform::feature_highlights(f);
    let get = |name: &str| hl.iter().find(|h| h.0 == name).unwrap().clone();
    assert!(get("NFC").1);
    assert!(!get("eSIM").1);
    assert!(get("Infrared emitter").1);
    assert_eq!(get("Hardware-backed keystore").2, "KeyMint 4.0");
    assert_eq!(get("Vulkan (guaranteed by Android)").2, "1.4.0");
    assert_eq!(p.mainline, Probe::Ok("2026-10-01".into()));
}

#[test]
fn platform_restricted_and_missing() {
    // Another manufacturer: no Adreno, SELinux without getenforce, no vkjson.
    let text = "@@@SELINUX@@@\n/system/bin/sh: getenforce: inaccessible or not found\n\
                @@@KERNEL@@@\ncat: /proc/version: Permission denied\n\n\
                @@@PAGESIZE@@@\n16384\n@@@GLES@@@\n@@@VULKAN@@@\nUnknown command: vkjson\n\
                @@@GPUMODEL@@@\ncat: /sys/class/kgsl/kgsl-3d0/gpu_model: No such file or directory\n\n\
                @@@CPUFREQ@@@\npolicy0|0 1 2 3|schedutil|300000|2000000|cat: x: Permission denied\n\
                @@@END@@@\n";
    let p = platform::parse_platform(text);
    assert!(matches!(p.selinux, Probe::Unsupported(_)));
    assert!(matches!(p.kernel, Probe::Restricted(_)));
    assert_eq!(p.page_size, Probe::Ok(16384));
    assert_eq!(p.gles, Probe::Missing);
    assert!(matches!(p.vulkan, Probe::Unsupported(_)));
    assert!(matches!(p.gpu_model, Probe::Unsupported(_)));
    assert!(matches!(p.clusters.ok().unwrap()[0].steps, Probe::Restricted(_)));
    // Sections the script never got to print.
    assert!(matches!(p.thermal_limits, Probe::Failed(_)));
    assert!(matches!(p.features, Probe::Failed(_)));
}

#[test]
fn platform_facts_from_props() {
    let props = adbscan_core::parsers::parse_getprop(&fixture("props.txt"));
    let facts = platform::boot_facts(&props);
    let get = |label: &str| facts.iter().find(|f| f.label == label).map(|f| f.value.clone()).unwrap();
    assert!(get("Verified boot").ok().unwrap().starts_with("Green"));
    let mut fake = std::collections::BTreeMap::new();
    fake.insert("ro.boot.verifiedbootstate".to_string(), "orange".to_string());
    fake.insert("ro.boot.flash.locked".to_string(), "0".to_string());
    fake.insert("ro.crypto.state".to_string(), "encrypted".to_string());
    fake.insert("ro.crypto.type".to_string(), "file".to_string());
    fake.insert("ro.vendor.api_level".to_string(), "202504".to_string());
    fake.insert("ro.product.first_api_level".to_string(), "34".to_string());
    let b = platform::boot_facts(&fake);
    assert!(b[0].value.ok().unwrap().starts_with("Orange"));
    assert_eq!(b[1].value, Probe::Ok("Unlocked".into()));
    assert_eq!(b[2].value, Probe::Ok("File-based encryption (FBE)".into()));
    // Without the property nothing is made up: “does not publish it”.
    assert_eq!(b[3].value, Probe::Missing);
    let p = platform::platform_facts(&fake);
    let get = |label: &str| p.iter().find(|f| f.label == label).map(|f| f.value.clone()).unwrap();
    assert_eq!(get("Shipped with"), Probe::Ok("Android 14 (API 34)".into()));
    assert_eq!(get("Vendor level"), Probe::Ok("202504 (April 2025)".into()));
    assert_eq!(get("Project Treble"), Probe::Missing);
    assert_eq!(platform::opengles_from_prop(&[("ro.opengles.version".to_string(), "196610".to_string())].into_iter().collect()).as_deref(), Some("3.2"));
}

// -- app inspection ---------------------------------------------------------

#[test]
fn inspection_real() {
    let pkg = "com.google.android.youtube";
    let text = fixture("app_inspect.txt");
    let a = app_inspect::parse_inspection(pkg, &text);
    let e = a.extras.ok().unwrap();
    assert_eq!(e.primary_abi.as_deref(), Some("arm64-v8a"));
    assert_eq!(e.secondary_abi, None);
    assert_eq!(e.splits, ["base", "config.arm64_v8a", "config.en", "config.es", "config.xxhdpi"]);
    assert_eq!(e.signing_scheme, Some(3));
    assert_eq!(e.installer.as_deref(), Some("com.android.vending"));
    assert_eq!(e.data_dir.as_deref(), Some("/data/user/0/com.google.android.youtube"));
    assert_eq!(e.user.installed, Some(true));
    assert_eq!(e.user.stopped, Some(false));
    assert_eq!(e.user.enabled, Some(1));
    assert_eq!(e.declared_permissions.len(), 2);
    assert_eq!(e.declared_permissions[0].1, "signature");
    assert_eq!(e.disabled_components.len(), 2);
    // It is an update of the stock app (“Hidden system packages” block).
    assert_eq!(e.factory_version, Some(("21.31.525".into(), "/product/app/YouTube".into())));
    assert_eq!(e.dexopt, Some(("speed-profile".into(), "bg-dexopt".into())));

    let l = a.links.ok().unwrap();
    assert_eq!(l.signatures, ["5A:AD:2B:EE:6D:B9:5D:17:E0:5A:08:D7:D1:E6:4C:10:A1:51:18:79:15:44:83:91:6B:6A:E6:C7:FD:9C:B0:C6"]);
    assert_eq!(l.domains.len(), 7);
    assert_eq!(l.domains[0], ("youtu.be".into(), "system_configured".into()));

    assert_eq!(a.standby_bucket, Probe::Ok(20));
    assert_eq!(a.battery_exemption, Probe::Ok(None));
    let procs = a.processes.ok().unwrap();
    assert_eq!(procs.len(), 1);
    assert_eq!((procs[0].pid, procs[0].state.as_str(), procs[0].adj.as_str()), (1886, "TPSL", "fg"));

    let ops = a.appops.ok().unwrap();
    // Only the operations with a recorded access or rejection, most recent first.
    assert_eq!(ops.len(), 8);
    assert_eq!(ops[0].op, "CONTROL_AUDIO");
    assert!(ops.iter().all(|o| o.last_access_ago.or(o.last_reject_ago).is_some()));
    let saw = ops.iter().find(|o| o.op == "SYSTEM_ALERT_WINDOW").unwrap();
    assert_eq!(saw.last_access_ago, None);
    assert!(saw.last_reject_ago.is_some());

    let exits = a.exits.ok().unwrap();
    assert_eq!(exits.len(), 16);
    assert_eq!(exits[0].reason, 10);
    assert_eq!(exits[0].reason_label(), "Closed by the user");
    assert_eq!(exits[0].subreason, "REMOVE TASK");
    assert_eq!(exits[0].rss, "544MB");
    assert!(exits.iter().any(|x| x.reason == 2 && x.reason_label() == "Stopped by a signal (SIGKILL)"));

    let sections = adbscan_core::parsers::split_sections(&text);
    let base = AppInspection::base_apk(&sections["PATH"], &a.extras).unwrap();
    assert!(base.ends_with("/base.apk"));
}

#[test]
fn inspection_old_android_and_errors() {
    let text = "@@@DUMP@@@\nUnable to find package: x.y\n@@@LINKS@@@\nUnknown command: get-app-links\n\
                @@@BUCKET@@@\nSecurity exception: requires android.permission.PACKAGE_USAGE_STATS\n\
                @@@APPOPS@@@\n@@@IDLE@@@\nsystem,x.y,10001\n@@@EXIT@@@\nBad activity command, or no activities match: exit-info\n\
                @@@LRU@@@\nACTIVITY MANAGER LRU PROCESSES (dumpsys activity lru)\n@@@END@@@\n";
    let a = app_inspect::parse_inspection("x.y", text);
    assert_eq!(a.extras, Probe::Missing);
    assert!(matches!(a.links, Probe::Unsupported(_)));
    assert!(matches!(a.standby_bucket, Probe::Restricted(_)));
    assert_eq!(a.battery_exemption, Probe::Ok(Some("system".into())));
    assert!(matches!(a.exits, Probe::Unsupported(_)));
    // LRU list read and no processes of the app: it is not running.
    assert_eq!(a.processes, Probe::Ok(vec![]));
}

#[test]
fn ago_and_labels() {
    assert_eq!(app_inspect::parse_ago("+2d6h11m6s649ms"), Some(2.0 * 86400.0 + 6.0 * 3600.0 + 11.0 * 60.0 + 6.649));
    assert_eq!(app_inspect::parse_ago("+12ms"), Some(0.012));
    assert_eq!(app_inspect::parse_ago("nada"), None);
    assert_eq!(app_inspect::proc_state_label("CEM"), "Cached (paused)");
    assert_eq!(app_inspect::standby_bucket_label(45).0, "Restricted");
    assert_eq!(app_inspect::dexopt_label("verify"), "Verified only: interpreted and compiled on the fly (JIT)");
}

#[test]
fn lru_real_format() {
    let text = "ACTIVITY MANAGER LRU PROCESSES (dumpsys activity lru)\n  Activities:\n\
  #132: fg     TPSL -------T 1886:com.google.android.youtube/u0a441 act:activities|recents\n\
  #128: cch+ 5 CEM  -------- 390:org.example.app:sandboxed_tab/u0a552\n\
  #130: vis    BFGS ---NFU-T 4438:com.example.launcher.adapter/1000 act:client\n\
  # 1: pers   PER  LCMNFUAT 3965:com.android.systemui/u0a468\n";
    let l = app_inspect::parse_lru(text);
    assert_eq!(l.len(), 4);
    assert_eq!((l[1].adj.as_str(), l[1].state.as_str(), l[1].package()), ("cch+5", "CEM", "org.example.app"));
    assert_eq!(l[2].pid, 4438);
    assert_eq!(l[3].state, "PER");
}

// -- manifest -------------------------------------------------------------------

#[test]
fn manifest_components_real() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/manifest_florisboard.bin");
    let m = apk::manifest_summary(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(m.package, "dev.patrickgold.florisboard");
    assert_eq!(m.target_sdk, Some(36));
    assert_eq!(m.components.len(), 11);
    let count = |k: ComponentKind| m.of_kind(k).count();
    assert_eq!((count(ComponentKind::Activity), count(ComponentKind::Service), count(ComponentKind::Receiver), count(ComponentKind::Provider)), (4, 3, 1, 3));
    let find = |n: &str| m.components.iter().find(|c| c.name.ends_with(n)).unwrap();
    // Keyboard service: exported, but only the system can bind to it.
    let ime = find(".FlorisImeService");
    assert!(ime.is_exported(m.target_sdk) && !ime.is_open(m.target_sdk));
    assert_eq!(ime.permission.as_deref(), Some("android.permission.BIND_INPUT_METHOD"));
    // Activity without `exported` or filters: private.
    let crash = find(".CrashDialogActivity");
    assert_eq!(crash.exported_attr, None);
    assert!(!crash.is_exported(m.target_sdk));
    // Exported without a permission: any app can open it.
    assert!(find(".FlorisCopyToClipboardActivity").is_open(m.target_sdk));
    let fp = find("androidx.core.content.FileProvider");
    assert_eq!(fp.authorities.as_deref(), Some("dev.patrickgold.florisboard.provider.file"));
    assert!(!fp.is_exported(m.target_sdk));
    assert_eq!(find(".FlorisImeService").short_name(&m.package), ".FlorisImeService");
    assert_eq!(fp.short_name(&m.package), "androidx.core.content.FileProvider");
}

#[test]
fn implicit_export_rules() {
    let c = |kind, exported_attr, has_intent_filter| apk::ManifestComponent {
        kind,
        name: "a.B".into(),
        exported_attr,
        has_intent_filter,
        permission: None,
        disabled_in_manifest: false,
        authorities: None,
    };
    assert!(c(ComponentKind::Activity, None, true).is_exported(Some(30)));
    assert!(!c(ComponentKind::Activity, None, false).is_exported(Some(30)));
    assert!(c(ComponentKind::Provider, None, false).is_exported(Some(16)));
    assert!(!c(ComponentKind::Provider, None, false).is_exported(Some(17)));
    assert!(!c(ComponentKind::Service, Some(false), true).is_exported(Some(30)));
}

#[test]
fn scripts_only_read() {
    // No verb that changes the phone: only queries.
    let app = app_inspect::inspect_script("com.example.app");
    for script in [platform::PLATFORM_SCRIPT, app.as_str()] {
        for forbidden in [
            "setprop",
            "settings put",
            " set ",
            "appops set",
            "set-standby-bucket",
            "force-stop",
            "pm clear",
            "pm disable",
            "pm enable",
            "pm grant",
            "pm revoke",
            "pm uninstall",
            "compile",
            "--reset",
            "rm ",
            "kill",
            "> /",
            ">/data",
            ">/sdcard",
        ] {
            assert!(!script.contains(forbidden), "“{forbidden}” in the script:\n{script}");
        }
    }
    // The package is quoted even though it has already been validated.
    assert!(app_inspect::inspect_script("a.b'; reboot; '").contains("'a.b'\"'\"'; reboot; '\"'\"''"));
}

#[test]
fn package_dump_uid_android16() {
    // Android 16 uses `appId=` and before it comes a permission's “uid=… gids=[] …
    // prot=…”.
    let text = fixture("app_inspect.txt");
    let dump = &adbscan_core::parsers::split_sections(&text)["DUMP"];
    let info = adbscan_core::parsers::parse_package_dump(dump, "com.google.android.youtube", "");
    assert_eq!(info.uid, "10441");
    assert_eq!(info.version_name, "21.46.522");
}

#[test]
fn op_labels_are_distinct() {
    // Two different operations must not look the same in the list.
    let ops = ["CONTROL_AUDIO", "CONTROL_AUDIO_PARTIAL", "RUN_IN_BACKGROUND", "RUN_ANY_IN_BACKGROUND", "CAMERA", "RECORD_AUDIO"];
    let labels: std::collections::HashSet<_> = ops.iter().map(|o| app_inspect::op_label(o)).collect();
    assert_eq!(labels.len(), ops.len(), "{labels:?}");
    assert_eq!(app_inspect::exit_subreason_label("REMOVE TASK"), "swiped away from Recents");
    assert_eq!(app_inspect::exit_subreason_label("SOMETHING NEW"), "something new");
}
