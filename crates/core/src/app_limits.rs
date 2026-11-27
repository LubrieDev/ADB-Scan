//! Limits on one app (**they change the phone**, and every one is undone the
//! same way it is set):
//!
//! - **Pause** (`pm suspend --user 0`): the app is greyed out, cannot be
//!   opened and its notifications are hidden until it is resumed (`pm
//!   unsuspend`). Nothing is deleted. Android refuses it for the apps the
//!   phone needs (launcher, dialer, installer…).
//! - **Background activity**: `RUN_ANY_IN_BACKGROUND` set to `ignore` and the
//!   standby bucket set to `restricted`, the same as Settings → Battery →
//!   Restricted. It only runs while it is in use.
//! - **Background mobile data**: the app's UID in the netpolicy deny list
//!   (`POLICY_REJECT_METERED_BACKGROUND`), the switch Settings → Apps → Mobile
//!   data → Background data. Wi-Fi is not affected.
//! - **Unrestricted battery**: the app in Doze's user allowlist (`dumpsys
//!   deviceidle whitelist +pkg`) and background activity allowed, the same as
//!   Settings → Battery → Unrestricted: for apps whose notifications or alarms
//!   arrive late. Apps the system itself exempts are shown and not changed.
//!
//! Commands and outputs come from Android 16's sources
//! (`PackageManagerShellCommand`, `NetworkPolicyManagerShellCommand`).

use crate::i18n::tr;
use crate::probe::{self, Probe};

/// Limits of an app as read from the phone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppLimits {
    pub suspended: Probe<bool>,
    /// `RUN_ANY_IN_BACKGROUND` is allowed (Android's default).
    pub background_allowed: Probe<bool>,
    /// App Standby Bucket (10 active … 45 restricted).
    pub bucket: Probe<u32>,
    /// Background mobile data blocked.
    pub data_blocked: Probe<bool>,
    /// In Doze's allowlist (Settings → Battery → Unrestricted).
    pub unrestricted: Probe<bool>,
    /// Exempted by the system itself (not removable).
    pub system_exempt: bool,
}

/// Read-only script for one app. `package` must be valid
/// ([`crate::adb::is_valid_package_name`]); it is the only value it carries.
pub fn state_script(package: &str) -> String {
    format!(
        "p={package}
echo @@@USER@@@; dumpsys package \"$p\" 2>/dev/null | grep -m1 -E '^ *User 0:'
echo @@@RUNANY@@@; cmd appops get \"$p\" RUN_ANY_IN_BACKGROUND 2>&1
echo @@@BUCKET@@@; am get-standby-bucket \"$p\" 2>&1
echo @@@DENY@@@; cmd netpolicy list restrict-background-blacklist 2>&1
echo @@@IDLE@@@; dumpsys deviceidle whitelist 2>&1 | grep -F \",$p,\"
echo @@@END@@@
"
    )
}

/// `User 0: ceDataInode=… installed=true hidden=false suspended=true …`
fn parse_suspended(text: &str) -> Option<bool> {
    let v = text.split_whitespace().find_map(|w| w.strip_prefix("suspended="))?;
    v.parse().ok()
}

/// `RUN_ANY_IN_BACKGROUND: ignore; time=…`; `No operations.` when it was never
/// changed (allowed).
fn parse_run_any(text: &str) -> Option<bool> {
    let t = text.trim();
    if t.starts_with("No operations") {
        return Some(true);
    }
    let mode = t.lines().find_map(|l| l.trim().strip_prefix("RUN_ANY_IN_BACKGROUND:"))?;
    let mode = mode.split(';').next()?.trim();
    Some(!matches!(mode, "ignore" | "deny" | "errored"))
}

/// `Restrict background blacklisted UIDs: 10123 10456 ` (or `none`).
pub fn parse_deny_list(text: &str) -> Option<Vec<u32>> {
    let (_, rest) = text.lines().find(|l| l.contains("UIDs:"))?.split_once("UIDs:")?;
    Some(rest.split_whitespace().filter_map(|w| w.parse().ok()).collect())
}

/// `dumpsys deviceidle whitelist` lines of the app (`user,pkg,uid`,
/// `system,pkg,uid`, `system-excidle,pkg,uid`): (user, system).
fn parse_allowlist(text: &str) -> (bool, bool) {
    let kinds: Vec<&str> = text.lines().filter_map(|l| l.trim().split(',').next()).collect();
    (kinds.contains(&"user"), kinds.iter().any(|k| k.starts_with("system")))
}

pub fn parse(text: &str, uid: Option<u32>) -> AppLimits {
    let s = crate::parsers::split_sections(text);
    let idle = s.get("IDLE");
    let (user_allowed, system_exempt) = idle.map(|t| parse_allowlist(t)).unwrap_or((false, false));
    AppLimits {
        // An empty section is the app not being in the list.
        unrestricted: if idle.is_some() { Probe::Ok(user_allowed || system_exempt) } else { Probe::Missing },
        system_exempt,
        suspended: probe::from_section(s.get("USER"), parse_suspended),
        background_allowed: probe::from_section(s.get("RUNANY"), parse_run_any),
        bucket: probe::from_section(s.get("BUCKET"), |t| t.trim().parse().ok()),
        data_blocked: match uid {
            Some(uid) => probe::from_section(s.get("DENY"), |t| parse_deny_list(t).map(|l| l.contains(&uid))),
            None => Probe::Missing,
        },
    }
}

/// A limit that can be switched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Pause,
    Background,
    BackgroundData,
    Unrestricted,
}

impl Limit {
    /// Commands (arguments for `adb shell`) that put the limit on (`on`) or
    /// take it off. `uid` is needed for data.
    pub fn commands(self, package: &str, uid: Option<u32>, on: bool) -> Option<Vec<Vec<String>>> {
        let v = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<String>>();
        Some(match self {
            Limit::Pause => vec![v(&["pm", if on { "suspend" } else { "unsuspend" }, "--user", "0", package])],
            // Restricted and unrestricted exclude each other, as in Settings.
            Limit::Background if on => vec![
                v(&["cmd", "appops", "set", package, "RUN_ANY_IN_BACKGROUND", "ignore"]),
                v(&["am", "set-standby-bucket", package, "restricted"]),
                v(&["dumpsys", "deviceidle", "whitelist", &format!("-{package}")]),
            ],
            // `active`: Android lowers it again as the app goes unused.
            Limit::Background => vec![v(&["cmd", "appops", "set", package, "RUN_ANY_IN_BACKGROUND", "allow"]), v(&["am", "set-standby-bucket", package, "active"])],
            Limit::BackgroundData => {
                let uid = uid?.to_string();
                vec![v(&["cmd", "netpolicy", if on { "add" } else { "remove" }, "restrict-background-blacklist", &uid])]
            }
            Limit::Unrestricted if on => vec![v(&["dumpsys", "deviceidle", "whitelist", &format!("+{package}")]), v(&["cmd", "appops", "set", package, "RUN_ANY_IN_BACKGROUND", "allow"])],
            Limit::Unrestricted => vec![v(&["dumpsys", "deviceidle", "whitelist", &format!("-{package}")])],
        })
    }

    /// Whether the limit is on in what was read.
    pub fn is_on(self, l: &AppLimits) -> Option<bool> {
        match self {
            Limit::Pause => l.suspended.ok().copied(),
            Limit::Background => l.background_allowed.ok().map(|a| !a),
            Limit::BackgroundData => l.data_blocked.ok().copied(),
            Limit::Unrestricted => l.unrestricted.ok().copied(),
        }
    }

    /// Whether turning it on restricts the app (refused for the apps the
    /// phone needs).
    pub fn restricts(self) -> bool {
        self != Limit::Unrestricted
    }

    pub fn label(self) -> &'static str {
        match self {
            Limit::Pause => tr("Paused"),
            Limit::Background => tr("Background activity restricted"),
            Limit::BackgroundData => tr("Background mobile data blocked"),
            Limit::Unrestricted => tr("Unrestricted battery use"),
        }
    }
}

/// Apps that are never paused or restricted here: without them the phone
/// cannot be used or ADB Scan loses the connection.
pub fn is_protected(package: &str) -> bool {
    crate::debloat::is_protected(package)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "@@@USER@@@\n    User 0: ceDataInode=123 installed=true hidden=false suspended=true distractionFlags=0 stopped=false notLaunched=false enabled=0 instant=false virtual=false\n\
@@@RUNANY@@@\nRUN_ANY_IN_BACKGROUND: ignore; time=+2h3m ago\n@@@BUCKET@@@\n45\n@@@DENY@@@\nRestrict background blacklisted UIDs: 10123 10456 \n@@@END@@@\n";

    #[test]
    fn reads_every_limit() {
        let l = parse(OUT, Some(10456));
        assert_eq!(l.suspended, Probe::Ok(true));
        assert_eq!(l.background_allowed, Probe::Ok(false));
        assert_eq!(l.bucket, Probe::Ok(45));
        assert_eq!(l.data_blocked, Probe::Ok(true));
        assert_eq!(Limit::Background.is_on(&l), Some(true));
        assert_eq!(parse(OUT, Some(10999)).data_blocked, Probe::Ok(false));
    }

    #[test]
    fn defaults_and_empty_lists() {
        let out = "@@@USER@@@\n  User 0: installed=true hidden=false suspended=false stopped=false\n@@@RUNANY@@@\nNo operations.\n@@@BUCKET@@@\n10\n@@@DENY@@@\nRestrict background blacklisted UIDs: none\n@@@END@@@\n";
        let l = parse(out, Some(10123));
        assert_eq!(l.suspended, Probe::Ok(false));
        assert_eq!(l.background_allowed, Probe::Ok(true));
        assert_eq!(l.data_blocked, Probe::Ok(false));
        assert_eq!(parse_deny_list("Restrict background blacklisted UIDs: none\n"), Some(vec![]));
        assert_eq!(parse_run_any("RUN_ANY_IN_BACKGROUND: allow\n"), Some(true));
    }

    #[test]
    fn commands() {
        let c = Limit::Pause.commands("com.example.app", None, true).unwrap();
        assert_eq!(c[0].join(" "), "pm suspend --user 0 com.example.app");
        let c = Limit::Background.commands("com.example.app", None, true).unwrap();
        assert_eq!(c[1].join(" "), "am set-standby-bucket com.example.app restricted");
        let c = Limit::BackgroundData.commands("com.example.app", Some(10123), false).unwrap();
        assert_eq!(c[0].join(" "), "cmd netpolicy remove restrict-background-blacklist 10123");
        assert!(Limit::BackgroundData.commands("com.example.app", None, true).is_none());
        assert!(is_protected("com.android.shell"));
        let c = Limit::Unrestricted.commands("com.example.app", None, true).unwrap();
        assert_eq!(c[0].join(" "), "dumpsys deviceidle whitelist +com.example.app");
        assert_eq!(Limit::Unrestricted.commands("com.example.app", None, false).unwrap()[0].join(" "), "dumpsys deviceidle whitelist -com.example.app");
        assert!(!Limit::Unrestricted.restricts());
    }

    #[test]
    fn battery_allowlist() {
        // As Android 16 prints it (filtered to the app on the phone).
        let base = "@@@USER@@@\n  User 0: suspended=false\n@@@RUNANY@@@\nNo operations.\n@@@BUCKET@@@\n10\n@@@DENY@@@\nRestrict background blacklisted UIDs: none\n";
        let user = parse(&format!("{base}@@@IDLE@@@\nuser,com.example.app,10413\n@@@END@@@\n"), None);
        assert_eq!((user.unrestricted.clone(), user.system_exempt), (Probe::Ok(true), false));
        let system = parse(&format!("{base}@@@IDLE@@@\nsystem-excidle,com.example.app,10285\nsystem,com.example.app,10285\n@@@END@@@\n"), None);
        assert_eq!((system.unrestricted, system.system_exempt), (Probe::Ok(true), true));
        let none = parse(&format!("{base}@@@IDLE@@@\n@@@END@@@\n"), None);
        assert_eq!(none.unrestricted, Probe::Ok(false));
        assert_eq!(parse(&format!("{base}@@@END@@@\n"), None).unrestricted, Probe::Missing);
    }
}
