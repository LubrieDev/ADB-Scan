//! Phone-wide clean-up actions, offered in Apps:
//!
//! - **Close open apps** ([`OpenApps`]): what “Clear all” does in Recents.
//!   Each app window (task) is removed with `am stack remove`, which is the
//!   same call as swiping a card away: Android closes the window and the
//!   app's process if nothing else uses it.
//! - **Stop all** ([`StopPlan`]): also stops apps running in the background,
//!   without a window, and asks Android to free the expendable processes.
//! - **Clear caches** ([`trim_caches_script`], [`clear_cache_script`]): the
//!   cache of every app, or of one, without touching its data.
//!
//! The rest of this note is about “Stop all”, which closes the user's
//! running apps and the background processes that can be closed safely
//! (`am kill-all`).
//!
//! **It changes the phone.** It only runs through an explicit, confirmed
//! action. These are never closed:
//!
//! - **System apps.** Among them are the mobile network, calls, Bluetooth and
//!   the interface itself (SystemUI): closing them can leave the phone without
//!   service. Of those, `am kill-all` only frees the ones Android marks as
//!   expendable (cached).
//! - **User apps the phone needs** ([`Keep`]): home screen, active keyboard,
//!   calls, SMS, accessibility and live wallpaper. A keyboard installed by the
//!   user (FlorisBoard, an updated Gboard…) is a user app: without this
//!   exception, the phone would be left without a keyboard.
//!
//! An app closed with `am force-stop` does not receive notifications or alarms
//! until it is opened again: the interface warns about it beforehand.

use crate::i18n::tr;
use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::adb::is_valid_package_name;
use crate::exec::shell_quote;
use crate::parsers::{parse_df, parse_meminfo, split_sections};

/// Read-only query: user apps, processes and which apps fulfil a role the phone
/// needs.
pub const PLAN_SCRIPT: &str = r#"echo @@@USER@@@; pm list packages -3 2>&1
echo @@@PS@@@; ps -A -o USER,NAME 2>&1
echo @@@HOME@@@; cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.HOME 2>&1
echo @@@IME@@@; settings get secure default_input_method 2>&1
echo @@@A11Y@@@; settings get secure enabled_accessibility_services 2>&1
echo @@@SMS@@@; settings get secure sms_default_application 2>&1
echo @@@DIALER@@@; telecom get-default-dialer 2>&1
echo @@@WALLPAPER@@@; dumpsys wallpaper 2>&1 | grep -o 'ComponentInfo{[^}]*}' | head -n 8
echo @@@MEM@@@; grep MemAvailable /proc/meminfo
echo @@@END@@@
"#;

/// Why a running user app is not closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Keep {
    Launcher,
    Keyboard,
    Dialer,
    Sms,
    Accessibility,
    Wallpaper,
}

impl Keep {
    pub fn label(self) -> &'static str {
        match self {
            Keep::Launcher => tr("Home screen"),
            Keep::Keyboard => tr("Keyboard"),
            Keep::Dialer => tr("Calls"),
            Keep::Sms => "SMS",
            Keep::Accessibility => tr("Accessibility"),
            Keep::Wallpaper => tr("Live wallpaper"),
        }
    }
}

/// What would be closed right now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StopPlan {
    /// Running user apps that get closed (alphabetical order).
    pub stop: Vec<String>,
    /// Running user apps that are kept, with the reason.
    pub keep: Vec<(String, Keep)>,
    /// Available memory before (KiB).
    pub mem_available_kib: Option<u64>,
}

/// Result of [`crate::adb::Adb::stop_all`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StopResult {
    pub stopped: usize,
    /// Apps that started again right away (the system relaunches them).
    pub restarted: Vec<String>,
    pub mem_before_kib: Option<u64>,
    pub mem_after_kib: Option<u64>,
}

impl StopResult {
    /// Memory freed (KiB); negative if other apps took more.
    pub fn freed_kib(&self) -> Option<i64> {
        Some(self.mem_after_kib? as i64 - self.mem_before_kib? as i64)
    }
}

/// Package of a component (`package/.Class`, `ComponentInfo{package/…}`).
fn component_package(text: &str) -> Option<String> {
    let text = text.trim().trim_start_matches("ComponentInfo{");
    let pkg = text.split('/').next()?.trim();
    is_valid_package_name(pkg).then(|| pkg.to_string())
}

pub fn parse_plan(text: &str) -> StopPlan {
    let s = crate::parsers::split_sections(text);
    let section = |name: &str| s.get(name).map(String::as_str).unwrap_or("");
    let user: HashSet<String> = crate::parsers::parse_package_names(section("USER"));

    // App processes (`u0_a123`): `package` or `package:process`. Isolated ones
    // (`u0_i…`, WebView) are not apps that can be closed.
    let running: BTreeSet<String> = section("PS")
        .lines()
        .filter_map(|l| {
            let mut cols = l.split_whitespace();
            let owner = cols.next()?;
            let name = cols.next()?;
            let (_, rest) = owner.split_once('_')?;
            if !owner.starts_with('u') || !rest.starts_with('a') {
                return None;
            }
            let pkg = name.split(':').next()?;
            user.contains(pkg).then(|| pkg.to_string())
        })
        .collect();

    let mut needed: BTreeMap<String, Keep> = BTreeMap::new();
    let mut need = |pkg: Option<String>, why: Keep| {
        if let Some(p) = pkg {
            needed.entry(p).or_insert(why);
        }
    };
    // The last line of `resolve-activity --brief` is `package/activity`.
    need(section("HOME").lines().rev().find(|l| l.contains('/')).and_then(component_package), Keep::Launcher);
    need(component_package(section("IME")), Keep::Keyboard);
    for c in section("A11Y").trim().split(':').filter(|c| !c.is_empty() && *c != "null") {
        need(component_package(c), Keep::Accessibility);
    }
    let plain = |name: &str| Some(section(name).trim().to_string()).filter(|p| is_valid_package_name(p));
    need(plain("SMS"), Keep::Sms);
    need(plain("DIALER"), Keep::Dialer);
    for c in section("WALLPAPER").lines() {
        need(component_package(c), Keep::Wallpaper);
    }

    let mut plan = StopPlan { mem_available_kib: crate::parsers::parse_meminfo(section("MEM")).available_kib.and_then(|v| u64::try_from(v).ok()), ..Default::default() };
    for pkg in running {
        match needed.get(&pkg) {
            Some(why) => plan.keep.push((pkg, *why)),
            None => plan.stop.push(pkg),
        }
    }
    plan
}

/// Commands to close `packages` and free the expendable processes. `None` if
/// any name is not a valid package (it is never sent).
pub fn stop_script(packages: &[String]) -> Option<String> {
    let mut script = String::new();
    for p in packages {
        if !is_valid_package_name(p) {
            return None;
        }
        script.push_str(&format!("am force-stop {} 2>&1\n", shell_quote(p)));
    }
    script.push_str("am kill-all 2>&1\n");
    Some(script)
}

// -- close open apps --------------------------------------------------------

/// Read-only query: app windows (tasks) and available memory.
pub const OPEN_APPS_SCRIPT: &str = "echo @@@STACKS@@@; am stack list 2>&1
echo @@@MEM@@@; grep MemAvailable /proc/meminfo
echo @@@END@@@
";

/// An open app window, as a card in Recents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTask {
    /// Root task id: what `am stack remove` takes.
    pub root_id: u32,
    /// Package of the app shown in it.
    pub package: String,
}

/// What “Close open apps” would close right now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpenApps {
    pub tasks: Vec<OpenTask>,
    /// Available memory before (KiB).
    pub mem_available_kib: Option<u64>,
}

impl OpenApps {
    /// Packages with an open window, without repeats, in Recents order.
    pub fn packages(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for t in &self.tasks {
            if !out.contains(&t.package) {
                out.push(t.package.clone());
            }
        }
        out
    }
}

/// Parses [`OPEN_APPS_SCRIPT`]. Only ordinary app windows count: the home
/// screen, Recents itself, the split-screen containers and a
/// picture-in-picture window are never closed.
pub fn parse_open_apps(text: &str) -> OpenApps {
    let s = split_sections(text);
    let stacks = s.get("STACKS").map(String::as_str).unwrap_or("");
    let mut tasks = Vec::new();
    // (root id, activity type, windowing mode, packages)
    let mut root: Option<(u32, String, String, Vec<String>)> = None;
    let flush = |root: &mut Option<(u32, String, String, Vec<String>)>, tasks: &mut Vec<OpenTask>| {
        if let Some((id, kind, mode, packages)) = root.take() {
            if kind == "standard" && mode != "pinned" {
                if let Some(package) = packages.into_iter().next() {
                    tasks.push(OpenTask { root_id: id, package });
                }
            }
        }
    };
    let field = |line: &str, key: &str| -> Option<String> {
        let rest = &line[line.find(key)? + key.len()..];
        Some(rest.split(|c: char| c.is_whitespace() || c == '}' || c == ',').next().unwrap_or("").to_string())
    };
    for line in stacks.lines() {
        let t = line.trim();
        // `RootTask id=12 …` (Android 12+) or `Stack id=12 …` (earlier).
        if let Some(rest) = t.strip_prefix("RootTask id=").or_else(|| t.strip_prefix("Stack id=")) {
            flush(&mut root, &mut tasks);
            root = rest.split_whitespace().next().and_then(|id| id.parse().ok()).map(|id| (id, String::new(), String::new(), Vec::new()));
        } else if let Some((_, kind, mode, packages)) = root.as_mut() {
            if t.starts_with("configuration=") || t.contains("mActivityType=") {
                if kind.is_empty() {
                    *kind = field(t, "mActivityType=").unwrap_or_default();
                }
                if mode.is_empty() {
                    *mode = field(t, "mWindowingMode=").unwrap_or_default();
                }
            } else if let Some(rest) = t.strip_prefix("taskId=") {
                // `taskId=182: package/.Activity bounds=…`; `unknown` when empty.
                let component = rest.split_once(':').map(|(_, c)| c.trim()).unwrap_or("");
                if let Some(pkg) = component.split_whitespace().next().and_then(|c| c.split_once('/')).map(|(p, _)| p) {
                    if is_valid_package_name(pkg) {
                        packages.push(pkg.to_string());
                    }
                }
            }
        }
    }
    flush(&mut root, &mut tasks);
    let mem = parse_meminfo(s.get("MEM").map(String::as_str).unwrap_or(""));
    OpenApps { tasks, mem_available_kib: mem.available_kib.and_then(|v| u64::try_from(v).ok()) }
}

/// Commands that close the windows in `tasks`, as swiping their cards away.
pub fn close_tasks_script(tasks: &[OpenTask]) -> String {
    tasks.iter().map(|t| format!("am stack remove {} 2>&1\n", t.root_id)).collect()
}

/// Result of [`crate::adb::Adb::close_open_apps`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CloseResult {
    pub closed: usize,
    pub mem_before_kib: Option<u64>,
    pub mem_after_kib: Option<u64>,
}

impl CloseResult {
    /// Memory freed (KiB); negative if other apps took more.
    pub fn freed_kib(&self) -> Option<i64> {
        Some(self.mem_after_kib? as i64 - self.mem_before_kib? as i64)
    }
}

// -- caches -----------------------------------------------------------------

/// Free space on `/data` before and after: the space a cache clean-up freed.
const DF: &str = "df -k /data 2>/dev/null";

/// Asks Android to delete the cache of every app (`pm trim-caches` with a
/// target no phone reaches, so it frees all it can). Data is not touched.
pub fn trim_caches_script() -> String {
    format!("echo @@@BEFORE@@@; {DF}\necho @@@RUN@@@; pm trim-caches 1000000G 2>&1\necho @@@AFTER@@@; {DF}\necho @@@END@@@\n")
}

/// Deletes the cache of one app (`pm clear --cache-only`). `None` if the
/// name is not a valid package (it is never sent).
///
/// **The option is checked first**: an Android that does not know
/// `--cache-only` ignores it and `pm clear` would then delete all the app's
/// data. On those versions nothing runs and the output says `UNSUPPORTED`.
pub fn clear_cache_script(package: &str) -> Option<String> {
    if !is_valid_package_name(package) {
        return None;
    }
    let p = shell_quote(package);
    Some(format!(
        "if pm help 2>&1 | grep -q -e '--cache-only'; then\n\
         echo @@@BEFORE@@@; {DF}\necho @@@RUN@@@; pm clear --cache-only {p} 2>&1\necho @@@AFTER@@@; {DF}\n\
         else echo @@@UNSUPPORTED@@@; fi\necho @@@END@@@\n"
    ))
}

/// Outcome of a cache clean-up script.
#[derive(Debug, Clone, PartialEq)]
pub enum CacheOutcome {
    /// Done; space freed on `/data` (KiB), if it could be measured.
    Freed(Option<i64>),
    /// This Android cannot clear a single app's cache through ADB.
    Unsupported,
    /// Android refused: its message.
    Failed(String),
}

/// Parses the output of [`trim_caches_script`] or [`clear_cache_script`].
/// `needs_success`: `pm clear` answers `Success`; `pm trim-caches` prints
/// nothing when it works.
pub fn parse_cache_outcome(text: &str, needs_success: bool) -> CacheOutcome {
    let s = split_sections(text);
    if s.contains_key("UNSUPPORTED") {
        return CacheOutcome::Unsupported;
    }
    let Some(run) = s.get("RUN") else {
        return CacheOutcome::Failed(text.trim().lines().last().unwrap_or("").to_string());
    };
    let problem = run.lines().find(|l| l.contains("Exception") || l.contains("Error") || l.contains("Failed") || l.contains("Unknown option"));
    if let Some(line) = problem.or_else(|| (needs_success && !run.contains("Success")).then(|| run.lines().last().unwrap_or(""))) {
        return CacheOutcome::Failed(line.trim().chars().take(140).collect());
    }
    let available = |name: &str| s.get(name).and_then(|t| parse_df(t).into_iter().next()).and_then(|d| d.available_kib);
    CacheOutcome::Freed(match (available("BEFORE"), available("AFTER")) {
        (Some(before), Some(after)) => Some(after - before),
        _ => None,
    })
}
