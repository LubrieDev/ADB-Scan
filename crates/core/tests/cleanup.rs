//! “Stop all”: what gets closed and what is kept. Output in the format of a
//! REDMAGIC 11S Pro (Android 16); the user packages are made up.

use adbscan_core::cleanup::*;

const PLAN: &str = "@@@USER@@@
package:com.example.chat
package:com.example.music
package:dev.patrickgold.florisboard
package:com.example.notrunning
package:com.example.dialer
package:com.example.reader
package:com.example.wall
@@@PS@@@
USER           NAME
root           init
system         system_server
u0_a468        com.android.systemui
u0_a463        com.zte.mifavor.launcher
u0_a301        com.example.chat
u0_a301        com.example.chat:pushservice
u0_a302        com.example.music:player
u0_a303        dev.patrickgold.florisboard
u0_a304        com.example.dialer
u0_a305        com.example.reader
u0_a306        com.example.wall
u0_i12         com.example.reader:sandboxed_process0
@@@HOME@@@
priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=true
com.zte.mifavor.launcher/com.android.launcher3.uioverrides.QuickstepLauncher
@@@IME@@@
dev.patrickgold.florisboard/.FlorisImeService
@@@A11Y@@@
com.example.reader/com.example.reader.ReadAloudService
@@@SMS@@@
null
@@@DIALER@@@
com.example.dialer
@@@WALLPAPER@@@
ComponentInfo{com.android.systemui/com.android.systemui.wallpapers.ImageWallpaper}
ComponentInfo{com.example.wall/com.example.wall.LiveWallpaper}
@@@MEM@@@
MemAvailable:    6535292 kB
@@@END@@@
";

#[test]
fn plan_stops_only_user_apps_not_needed() {
    let p = parse_plan(PLAN);
    // Once per app even if it has several processes; system ones (SystemUI, the
    // stock launcher) and those not started do not appear.
    assert_eq!(p.stop, vec!["com.example.chat".to_string(), "com.example.music".to_string()]);
    assert_eq!(
        p.keep,
        vec![
            ("com.example.dialer".to_string(), Keep::Dialer),
            ("com.example.reader".to_string(), Keep::Accessibility),
            ("com.example.wall".to_string(), Keep::Wallpaper),
            ("dev.patrickgold.florisboard".to_string(), Keep::Keyboard),
        ]
    );
    assert_eq!(p.mem_available_kib, Some(6_535_292));
}

#[test]
fn user_launcher_is_kept() {
    let text = PLAN.replace("com.zte.mifavor.launcher/", "com.example.chat/");
    let p = parse_plan(&text);
    assert!(!p.stop.contains(&"com.example.chat".to_string()));
    assert!(p.keep.contains(&("com.example.chat".to_string(), Keep::Launcher)));
}

#[test]
fn empty_output_stops_nothing() {
    let p = parse_plan("");
    assert!(p.stop.is_empty() && p.keep.is_empty());
}

#[test]
fn script_quotes_and_rejects_bad_names() {
    let s = stop_script(&["com.example.chat".into()]).unwrap();
    assert_eq!(s, "am force-stop com.example.chat 2>&1\nam kill-all 2>&1\n");
    assert_eq!(stop_script(&[]).unwrap(), "am kill-all 2>&1\n");
    assert!(stop_script(&["com.x; reboot".into()]).is_none());
}

#[test]
fn freed_memory() {
    let r = StopResult { mem_before_kib: Some(1000), mem_after_kib: Some(1500), ..Default::default() };
    assert_eq!(r.freed_kib(), Some(500));
    assert_eq!(StopResult::default().freed_kib(), None);
}

// -- close open apps --------------------------------------------------------

/// `am stack list` of a REDMAGIC 11S Pro (Android 16) with two apps open, one
/// of them in picture-in-picture, and the split-screen containers.
const STACKS: &str = "@@@STACKS@@@
RootTask id=1 bounds=[0,0][1216,2688] displayId=0 userId=0
 configuration={1.0 334mcc20mnc [es_MX] ldltr sw374dp winConfig={ mBounds=Rect(0, 0 - 1216, 2688) mWindowingMode=fullscreen mActivityType=home mAlwaysOnTop=undefined mRotation=ROTATION_0} s.980}
  taskId=2: com.zte.mifavor.launcher/com.android.launcher3.uioverrides.QuickstepLauncher bounds=[0,0][1216,2688] userId=0 visible=true topActivity=ComponentInfo{com.zte.mifavor.launcher/com.android.launcher3.uioverrides.QuickstepLauncher}

RootTask id=3 bounds=[0,0][1216,2688] displayId=0 userId=0
 configuration={1.0 334mcc20mnc [es_MX] ldltr sw374dp winConfig={ mBounds=Rect(0, 0 - 1216, 2688) mWindowingMode=fullscreen mActivityType=undefined mAlwaysOnTop=undefined mRotation=ROTATION_0} s.980}
  taskId=4: unknown bounds=[0,0][1216,2688] userId=0 visible=false
  taskId=5: unknown bounds=[0,2688][1216,4032] userId=0 visible=false

RootTask id=190 bounds=[0,0][1216,2688] displayId=0 userId=0
 configuration={1.0 334mcc20mnc [es_MX] ldltr sw374dp winConfig={ mBounds=Rect(0, 0 - 1216, 2688) mWindowingMode=fullscreen mActivityType=standard mAlwaysOnTop=undefined mRotation=ROTATION_0} s.980}
  taskId=190: com.example.chat/.MainActivity bounds=[0,0][1216,2688] userId=0 visible=false topActivity=ComponentInfo{com.example.chat/com.example.chat.MainActivity}

RootTask id=191 bounds=[0,0][1216,2688] displayId=0 userId=0
 configuration={1.0 334mcc20mnc [es_MX] ldltr sw374dp winConfig={ mBounds=Rect(0, 0 - 1216, 2688) mWindowingMode=fullscreen mActivityType=standard mAlwaysOnTop=undefined mRotation=ROTATION_0} s.980}
  taskId=191: com.android.settings/.Settings bounds=[0,0][1216,2688] userId=0 visible=false topActivity=ComponentInfo{com.android.settings/com.android.settings.Settings}

RootTask id=192 bounds=[800,2000][1216,2400] displayId=0 userId=0
 configuration={1.0 334mcc20mnc [es_MX] ldltr sw374dp winConfig={ mBounds=Rect(800, 2000 - 1216, 2400) mWindowingMode=pinned mActivityType=standard mAlwaysOnTop=on mRotation=ROTATION_0} s.980}
  taskId=192: com.example.video/.PlayerActivity bounds=[800,2000][1216,2400] userId=0 visible=true
@@@MEM@@@
MemAvailable:    6535292 kB
@@@END@@@
";

#[test]
fn open_apps_are_the_ordinary_app_windows() {
    let o = parse_open_apps(STACKS);
    assert_eq!(
        o.tasks,
        vec![
            OpenTask { root_id: 190, package: "com.example.chat".into() },
            OpenTask { root_id: 191, package: "com.android.settings".into() },
        ]
    );
    assert_eq!(o.packages(), vec!["com.example.chat".to_string(), "com.android.settings".to_string()]);
    assert_eq!(o.mem_available_kib, Some(6_535_292));
    assert_eq!(close_tasks_script(&o.tasks), "am stack remove 190 2>&1\nam stack remove 191 2>&1\n");
}

#[test]
fn nothing_open_closes_nothing() {
    let home_only: String = STACKS.split("\nRootTask id=3").next().unwrap().to_string();
    assert!(parse_open_apps(&home_only).tasks.is_empty());
    assert!(parse_open_apps("").tasks.is_empty());
    assert_eq!(close_tasks_script(&[]), "");
}

// -- caches -----------------------------------------------------------------

const DF_BEFORE: &str = "Filesystem       1K-blocks     Used Available Use% Mounted on\n/dev/block/dm-59 226696012 93426688 133138252  42% /data/user/0";
const DF_AFTER: &str = "Filesystem       1K-blocks     Used Available Use% Mounted on\n/dev/block/dm-59 226696012 93026688 133538252  42% /data/user/0";

#[test]
fn clear_cache_checks_the_option_before_running() {
    let s = clear_cache_script("com.example.chat").unwrap();
    // The check comes before the command, and without it nothing runs.
    let check = s.find("grep -q -e '--cache-only'").unwrap();
    let run = s.find("pm clear --cache-only com.example.chat").unwrap();
    assert!(check < run);
    assert!(s.contains("else echo @@@UNSUPPORTED@@@"));
    assert!(!s.replace("pm clear --cache-only", "").contains("pm clear"));
    assert!(clear_cache_script("com.x; reboot").is_none());
}

#[test]
fn cache_outcomes() {
    let ok = format!("@@@BEFORE@@@\n{DF_BEFORE}\n@@@RUN@@@\nSuccess\n@@@AFTER@@@\n{DF_AFTER}\n@@@END@@@\n");
    assert_eq!(parse_cache_outcome(&ok, true), CacheOutcome::Freed(Some(400_000)));
    // `pm trim-caches` prints nothing when it works.
    let trim = format!("@@@BEFORE@@@\n{DF_BEFORE}\n@@@RUN@@@\n@@@AFTER@@@\n{DF_AFTER}\n@@@END@@@\n");
    assert_eq!(parse_cache_outcome(&trim, false), CacheOutcome::Freed(Some(400_000)));
    assert_eq!(parse_cache_outcome("@@@RUN@@@\nSuccess\n@@@END@@@\n", true), CacheOutcome::Freed(None));
    assert_eq!(parse_cache_outcome("@@@UNSUPPORTED@@@\n@@@END@@@\n", true), CacheOutcome::Unsupported);
    let refused = "@@@BEFORE@@@\n@@@RUN@@@\nException occurred while executing 'clear':\njava.lang.SecurityException: denied\n@@@AFTER@@@\n@@@END@@@\n";
    assert!(matches!(parse_cache_outcome(refused, true), CacheOutcome::Failed(l) if l.contains("Exception")));
    assert!(matches!(parse_cache_outcome("@@@RUN@@@\nFailed\n@@@END@@@\n", true), CacheOutcome::Failed(_)));
    assert!(trim_caches_script().contains("pm trim-caches 1000000G"));
}
