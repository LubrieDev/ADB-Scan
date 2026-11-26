//! ADB client: host commands, remote shell and device operations.
//!
//! Host commands always use an argument vector. Remote commands with user data
//! are quoted with [`shell_quote`]; they are never concatenated unescaped.

use crate::{i18n::tr, t};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::exec::{self, shell_join, shell_quote, CommandResult, StreamEvent, StreamHandle, StreamOptions};
use crate::models::*;
use crate::parsers;

const SHORT: Duration = Duration::from_secs(10);
const NORMAL: Duration = Duration::from_secs(20);

/// Clean shutdown (see [`Adb::power_off`]). `svc power shutdown` has existed
/// for many versions; if this Android does not list it in its help, `reboot -p`
/// is used.
const POWER_OFF_SCRIPT: &str = "if svc power 2>&1 | grep -q 'svc power shutdown'; then svc power shutdown 2>&1; else reboot -p 2>&1; fi";
const LONG: Duration = Duration::from_secs(120);
const TRANSFER: Duration = Duration::from_secs(3600);

// ---------------------------------------------------------------------------
// Tool lookup
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Adb,
    Fastboot,
    Scrcpy,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Adb => "adb",
            Tool::Fastboot => "fastboot",
            Tool::Scrcpy => "scrcpy",
        }
    }
}

#[cfg(unix)]
fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Folder of a tool inside a package (`platform-tools` or `scrcpy`).
fn package_folder(tool: Tool) -> &'static str {
    match tool {
        Tool::Scrcpy => "scrcpy",
        _ => "platform-tools",
    }
}

/// Usual locations besides `PATH`, including those managed by the app.
pub fn tool_candidates(tool: Tool) -> Vec<PathBuf> {
    let name = tool.name();
    let mut out = vec![crate::paths::managed_tools_dir().join(package_folder(tool)).join(exec::exe_name(name))];
    #[cfg(unix)]
    let h = home();
    #[cfg(unix)]
    match tool {
        Tool::Adb | Tool::Fastboot => {
            for base in [
                PathBuf::from("/usr/lib/android-sdk/platform-tools"),
                PathBuf::from("/opt/android-sdk/platform-tools"),
                PathBuf::from("/opt/android/platform-tools"),
                h.join("Android/Sdk/platform-tools"),
                h.join("android-sdk/platform-tools"),
                h.join("platform-tools"),
            ] {
                out.push(base.join(name));
            }
        }
        Tool::Scrcpy => {
            out.push(PathBuf::from("/snap/bin/scrcpy"));
            out.push(PathBuf::from("/usr/local/bin/scrcpy"));
            out.push(h.join(".local/bin/scrcpy"));
        }
    }
    // Android Studio's SDK (`%LOCALAPPDATA%\Android\Sdk`).
    #[cfg(windows)]
    if matches!(tool, Tool::Adb | Tool::Fastboot) {
        if let Some(local) = dirs::data_local_dir() {
            out.push(local.join("Android").join("Sdk").join("platform-tools").join(exec::exe_name(name)));
        }
    }
    out
}

/// The copy that travels with ADB Scan, next to its executable (the Windows
/// installer: `platform-tools\adb.exe`, `scrcpy\scrcpy.exe`). Only on Windows: on
/// Linux the executable is in `/usr/bin`, and “next to it” would be the
/// system's adb, which would override the path chosen in Preferences.
pub fn bundled_tool(tool: Tool) -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let file = exec::exe_name(tool.name());
    [dir.join(package_folder(tool)).join(&file), dir.join(&file)].into_iter().find(|p| exec::is_executable(p))
}

/// Looks for a tool: the copy next to the executable (Windows) → configured
/// path → PATH → usual locations.
pub fn find_tool(tool: Tool, configured: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = bundled_tool(tool) {
        return Some(p);
    }
    if let Some(cfg) = configured.map(str::trim).filter(|c| !c.is_empty()) {
        return exec::which(cfg);
    }
    exec::which(tool.name()).or_else(|| tool_candidates(tool).into_iter().find(|p| exec::is_executable(p)))
}

/// Looks for any tool (tshark, aapt…): configured path or PATH.
pub fn find_tool_named(name: &str, configured: &str) -> Option<PathBuf> {
    let c = configured.trim();
    if !c.is_empty() {
        return exec::which(c);
    }
    exec::which(name)
}

/// Version of a tool (`adb version`, `fastboot --version`, `scrcpy --version`).
pub fn tool_version(path: &Path, tool: Tool) -> Option<String> {
    let arg = match tool {
        Tool::Adb => "version",
        _ => "--version",
    };
    let r = exec::run(path, &[arg.into()], Duration::from_secs(5), None).ok()?;
    let text = r.combined();
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim().to_string();
    let detail = text.lines().find(|l| l.contains("Version ")).map(|l| l.trim().to_string());
    Some(match detail {
        Some(d) if d != line => format!("{line} · {d}"),
        _ => line,
    })
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Adb {
    path: Arc<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebootMode {
    System,
    Recovery,
    Bootloader,
    Fastbootd,
    Sideload,
    /// EDL mode (Qualcomm). Only on some devices.
    Edl,
}

impl RebootMode {
    pub fn arg(self) -> Option<&'static str> {
        match self {
            Self::System => None,
            Self::Recovery => Some("recovery"),
            Self::Bootloader => Some("bootloader"),
            Self::Fastbootd => Some("fastboot"),
            Self::Sideload => Some("sideload"),
            Self::Edl => Some("edl"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => tr("System"),
            Self::Recovery => "Recovery",
            Self::Bootloader => "Bootloader",
            Self::Fastbootd => "Fastbootd",
            Self::Sideload => "Sideload",
            Self::Edl => "EDL (Qualcomm)",
        }
    }

    pub const ALL: [RebootMode; 6] = [Self::System, Self::Recovery, Self::Bootloader, Self::Fastbootd, Self::Sideload, Self::Edl];
}

#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// `-r`: replace, keeping the data.
    pub replace: bool,
    /// `-d`: allow downgrade.
    pub downgrade: bool,
    /// `-g`: grant all runtime permissions.
    pub grant_all: bool,
    /// `-t`: allow test APKs.
    pub allow_test: bool,
}

/// Progress of a transfer (0.0–1.0) with bytes transferred and total.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

impl Progress {
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            (self.done as f64 / self.total as f64).clamp(0.0, 1.0)
        }
    }
}

/// Allows cancelling long transfers from the interface.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst)
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Paths that are never deleted or renamed from the interface.
pub fn is_protected_remote_path(path: &str) -> bool {
    let p = path.trim_end_matches('/');
    let p = if p.is_empty() { "/" } else { p };
    const PROTECTED: [&str; 14] =
        ["/", "/sdcard", "/storage", "/storage/emulated", "/storage/emulated/0", "/storage/self", "/storage/self/primary", "/data", "/system", "/vendor", "/product", "/apex", "/proc", "/sys"];
    PROTECTED.contains(&p)
}

pub fn validate_remote_path(path: &str) -> Result<String> {
    let p = path.trim();
    if p.is_empty() || !p.starts_with('/') {
        return Err(Error::Invalid(tr("the device path must be absolute").into()));
    }
    if p.contains('\0') || p.contains('\n') {
        return Err(Error::Invalid(tr("the path contains characters that are not allowed").into()));
    }
    Ok(p.to_string())
}

pub fn remote_join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub fn remote_parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => trimmed[..i].into(),
    }
}

pub fn is_valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name.contains('.')
        && name.split('.').all(|part| !part.is_empty() && part.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

fn check_package(name: &str) -> Result<()> {
    if is_valid_package_name(name) {
        Ok(())
    } else {
        Err(Error::Invalid(t!("invalid package name: {}", name)))
    }
}

impl Adb {
    pub fn new(path: PathBuf) -> Self {
        Self { path: Arc::new(path) }
    }

    pub fn locate(configured: Option<&str>) -> Result<Self> {
        find_tool(Tool::Adb, configured).map(Self::new).ok_or(Error::ToolNotFound { tool: "adb".into() })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn argv(serial: Option<&str>, args: &[&str]) -> Vec<String> {
        let mut v = Vec::with_capacity(args.len() + 2);
        if let Some(s) = serial {
            v.push("-s".into());
            v.push(s.into());
        }
        v.extend(args.iter().map(|a| a.to_string()));
        v
    }

    fn check_device(serial: Option<&str>, r: CommandResult) -> Result<CommandResult> {
        if let Some(s) = serial {
            let text = format!("{}{}", r.stderr, r.stdout).to_lowercase();
            if text.contains("device '") && text.contains("not found") || text.contains("device not found") {
                return Err(Error::DeviceNotFound(s.into()));
            }
        }
        Ok(r)
    }

    /// Runs `adb [-s serial] args…`.
    pub fn run(&self, serial: Option<&str>, args: &[&str], timeout: Duration) -> Result<CommandResult> {
        let r = exec::run(&self.path, &Self::argv(serial, args), timeout, None)?;
        Self::check_device(serial, r)
    }

    /// Like [`Self::run`], with the raw output. The output is not put in the
    /// result text: it is data (a PNG, a file from an APK), and a
    /// `resources.arsc` with the phrase “device not found” among its strings
    /// would look like a disconnected phone.
    pub fn run_bytes(&self, serial: Option<&str>, args: &[&str], timeout: Duration) -> Result<(CommandResult, Vec<u8>)> {
        let (mut r, bytes) = exec::run_raw(&self.path, &Self::argv(serial, args), timeout, None)?;
        r.stdout.clear();
        Ok((Self::check_device(serial, r)?, bytes))
    }

    /// `adb shell` with separate, quoted arguments.
    pub fn shell(&self, serial: &str, args: &[&str], timeout: Duration) -> Result<CommandResult> {
        let line = shell_join(args);
        self.run(Some(serial), &["shell", &line], timeout)
    }

    /// Text of `adb shell …`; empty if it fails.
    pub fn shell_text(&self, serial: &str, args: &[&str]) -> String {
        self.shell(serial, args, NORMAL).map(|r| r.stdout).unwrap_or_default()
    }

    /// Fixed remote script (written by us, without user data).
    pub fn shell_script(&self, serial: &str, script: &str, timeout: Duration) -> Result<CommandResult> {
        self.run(Some(serial), &["shell", script], timeout)
    }

    pub fn shell_stream<F>(&self, serial: &str, script: &str, on_event: F) -> Result<StreamHandle>
    where
        F: Fn(StreamEvent) + Send + Sync + 'static,
    {
        exec::stream(&self.path, &Self::argv(Some(serial), &["shell", script]), StreamOptions::default(), on_event)
    }

    pub fn stream<F>(&self, serial: Option<&str>, args: &[&str], with_stdin: bool, on_event: F) -> Result<StreamHandle>
    where
        F: Fn(StreamEvent) + Send + Sync + 'static,
    {
        exec::stream(&self.path, &Self::argv(serial, args), StreamOptions { with_stdin, ..Default::default() }, on_event)
    }

    // -- server -------------------------------------------------------------

    pub fn start_server(&self) -> Result<CommandResult> {
        self.run(None, &["start-server"], Duration::from_secs(15))
    }

    pub fn kill_server(&self) -> Result<CommandResult> {
        self.run(None, &["kill-server"], SHORT)
    }

    pub fn devices(&self) -> Result<Vec<AdbDevice>> {
        let r = self.run(None, &["devices", "-l"], SHORT)?.into_result()?;
        Ok(parsers::parse_devices(&r.stdout))
    }

    // -- properties and information
    // --------------------------------------------

    pub fn getprop(&self, serial: &str) -> Result<std::collections::BTreeMap<String, String>> {
        let r = self.shell(serial, &["getprop"], NORMAL)?.into_result()?;
        Ok(parsers::parse_getprop(&r.stdout))
    }

    pub fn prop(&self, serial: &str, key: &str) -> String {
        self.shell_text(serial, &["getprop", key]).trim().to_string()
    }

    /// Full snapshot in a single round trip (first paint or if the live stream
    /// cannot start).
    pub fn collect_snapshot(&self, serial: &str) -> Result<Snapshot> {
        let script = crate::live::build_live_script(1000, 1);
        // A single block: the loop is cut after the first END.
        let once = script.replace("  i=$((i + 1))\ndone", "  break\ndone");
        let r = self.shell_script(serial, &once, Duration::from_secs(30))?;
        let block = r.stdout.split(crate::live::BLOCK_END).next().unwrap_or("");
        let sections = parsers::split_sections(block);
        let mut acc = crate::live::LiveAccumulator::default();
        Ok(crate::live::build_snapshot(serial, &sections, &mut acc))
    }

    /// Battery capacity according to Android, (design, learned) in mAh.
    /// Fallback when sysfs is not readable: `batterystats` takes ~10 s, so it
    /// is queried only once per connection and never in the live loop.
    pub fn battery_capacity(&self, serial: &str) -> (Option<i64>, Option<i64>) {
        let script = "dumpsys batterystats | grep 'battery capacity'";
        match self.shell_script(serial, script, Duration::from_secs(40)) {
            Ok(r) => parsers::parse_batterystats_capacity(&r.stdout),
            Err(_) => (None, None),
        }
    }

    /// Private identifiers (IMEI, SIM, accounts…). Read-only and on demand: the
    /// output is never logged.
    pub fn private_info(&self, serial: &str) -> Result<crate::identity::PrivateInfo> {
        let r = self.shell_script(serial, crate::identity::PRIVATE_SCRIPT, Duration::from_secs(20))?;
        Ok(crate::identity::parse_private(&r.stdout))
    }

    /// Public IP queried from the phone (its network, not the PC's).
    pub fn public_ip(&self, serial: &str) -> std::result::Result<std::net::IpAddr, String> {
        match self.shell_script(serial, &crate::identity::public_ip_script(), Duration::from_secs(15)) {
            Ok(r) => crate::identity::parse_public_ip(&r.stdout),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Per-app usage and wakelocks since the last full charge. Two read-only
    /// queries in parallel (~4 s in total).
    pub fn battery_usage(&self, serial: &str) -> Result<crate::battery_usage::BatteryUsage> {
        use crate::battery_usage::{build, USAGE_SCRIPT, WAKELOCK_SCRIPT};
        let (usage, wakelocks) = std::thread::scope(|scope| {
            let w = scope.spawn(|| self.shell_script(serial, WAKELOCK_SCRIPT, Duration::from_secs(60)));
            let u = self.shell_script(serial, USAGE_SCRIPT, Duration::from_secs(60));
            (u, w.join().unwrap_or_else(|_| Err(crate::Error::Invalid("wakelocks thread".into()))))
        });
        let usage = usage?;
        // Without wakelocks the usage is still shown.
        let wakelocks = wakelocks.map(|r| r.stdout).unwrap_or_default();
        Ok(build(&usage.stdout, &wakelocks))
    }

    pub fn processes(&self, serial: &str) -> Result<Vec<ProcessInfo>> {
        let r = self.shell(serial, &["ps", "-A", "-o", "PID,USER,PPID,VSZ,RSS,S,NAME,ARGS"], NORMAL)?;
        Ok(parsers::parse_ps(&r.stdout))
    }

    /// GPU usage (Adreno) and FPS of the foreground app, without root.
    /// `prev_newest`: timestamp of the most recent frame in the previous
    /// reading; if it did not change, the screen is idle and 0 FPS is reported.
    pub fn gpu_fps(&self, serial: &str, prev_newest: Option<i64>) -> crate::models::GpuFps {
        let mut out = crate::models::GpuFps::default();
        let first = self.shell_script(serial, "dumpsys window | grep -m1 mCurrentFocus; echo @@@LAYERS@@@; dumpsys SurfaceFlinger --list", NORMAL).map(|r| r.stdout).unwrap_or_default();
        let (focus_text, layers_text) = first.split_once("@@@LAYERS@@@").unwrap_or((&first, ""));
        let focus = parsers::parse_focus(focus_text);
        let layers = focus.as_deref().map(|f| parsers::fps_layers(&parsers::parse_layer_list(layers_text), f)).unwrap_or_default();
        let mut script = String::from("cat /sys/class/kgsl/kgsl-3d0/gpubusy 2>/dev/null; echo");
        for l in &layers {
            script.push_str(&format!("; echo @@@LAYER@@@; dumpsys SurfaceFlinger --latency {}", shell_quote(l)));
        }
        let second = self.shell_script(serial, &script, NORMAL).map(|r| r.stdout).unwrap_or_default();
        let mut parts = second.split("@@@LAYER@@@");
        out.gpu_busy = parts.next().and_then(parsers::parse_gpubusy);
        // The layer with the most recent frames is the one being drawn.
        let best = parts.map(|p| parsers::parse_latency(p.trim_start())).filter_map(|t| parsers::fps_from_presents(&t)).max_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        if let Some((fps, newest)) = best {
            out.newest_frame = Some(newest);
            out.fps = Some(if prev_newest == Some(newest) { 0.0 } else { fps });
        }
        out.app = focus.map(|f| f.split('/').next().unwrap_or("").to_string()).unwrap_or_default();
        out
    }

    /// The `n` processes using the most CPU right now (name, % of one core). It
    /// takes about 2 s: `top` needs two samples.
    pub fn top_cpu(&self, serial: &str, n: usize) -> Vec<(String, f64)> {
        let n = (n + 1).to_string(); // +1: `top` itself, which is discarded
        parsers::parse_top_last(&self.shell_text(serial, &["top", "-b", "-n", "2", "-d", "2", "-s", "1", "-m", &n, "-o", "%CPU,NAME"]))
    }

    /// Processes with instantaneous %CPU and the list of user apps.
    pub fn process_overview(&self, serial: &str) -> Result<crate::models::ProcessOverview> {
        let mut processes = self.processes(serial)?;
        // Two samples: `top`'s first one has nothing to compare with and gives 0
        // % to almost everything. `parse_top_cpu` keeps the last value of each
        // PID.
        let cpu = parsers::parse_top_cpu(&self.shell_text(serial, &["top", "-b", "-n", "2", "-d", "1", "-o", "PID,%CPU"]));
        for p in &mut processes {
            p.cpu = cpu.get(&p.pid).copied();
        }
        // State of each process according to Android (on screen, service,
        // cached…).
        let states: std::collections::HashMap<i64, String> = crate::app_inspect::parse_lru(&self.shell_text(serial, &["dumpsys", "activity", "lru"])).into_iter().map(|e| (e.pid, e.state)).collect();
        for p in &mut processes {
            p.android_state = states.get(&p.pid).cloned();
        }
        let user_packages = self.shell_text(serial, &["pm", "list", "packages", "-3"]).lines().filter_map(|l| l.trim().strip_prefix("package:")).map(String::from).collect();
        Ok(crate::models::ProcessOverview { processes, user_packages })
    }

    pub fn sensors(&self, serial: &str) -> Result<Vec<SensorInfo>> {
        let r = self.shell(serial, &["dumpsys", "sensorservice"], NORMAL)?;
        Ok(parsers::parse_sensors(&r.stdout))
    }

    pub fn services(&self, serial: &str) -> Result<Vec<ServiceInfo>> {
        let r = self.shell(serial, &["service", "list"], NORMAL)?;
        Ok(parsers::parse_services(&r.stdout))
    }

    pub fn wifi_status(&self, serial: &str) -> WifiStatus {
        // `cmd wifi status` (Android 11+) is short; `dumpsys wifi` takes
        // several MB on Android 16 and its format changes between versions.
        let text = self.shell_text(serial, &["cmd", "wifi", "status"]);
        if text.contains("Wifi is") || text.contains("Wi-Fi is") {
            return parsers::parse_wifi_status(&text);
        }
        parsers::parse_wifi_status(&self.shell_text(serial, &["dumpsys", "wifi"]))
    }

    pub fn kernel_version(&self, serial: &str) -> String {
        self.shell_text(serial, &["uname", "-a"]).trim().to_string()
    }

    pub fn foreground_activity(&self, serial: &str) -> String {
        let text = self.shell_text(serial, &["dumpsys", "activity", "activities"]);
        static RES: std::sync::LazyLock<[regex::Regex; 2]> =
            std::sync::LazyLock::new(|| [regex::Regex::new(r"mResumedActivity:\s+\S+\s+\S+\s+([^\s/]+/[\w.$]+)").unwrap(), regex::Regex::new(r"topResumedActivity=.*?\s([\w.]+/[\w.$]+)").unwrap()]);
        RES.iter().find_map(|re| re.captures(&text).map(|c| c[1].to_string())).unwrap_or_default()
    }

    // -- control --------------------------------------------------------------

    pub fn reboot(&self, serial: &str, mode: RebootMode) -> Result<CommandResult> {
        let mut args = vec!["reboot"];
        if let Some(m) = mode.arg() {
            args.push(m);
        }
        self.run(Some(serial), &args, SHORT)
    }

    /// Shuts the phone down cleanly, like the power button: `svc power
    /// shutdown` tells apps to save their state. Versions without that command
    /// use `reboot -p`. The connection drops as soon as the phone starts
    /// shutting down, so losing it is not an error; only Android rejecting the
    /// command is.
    pub fn power_off(&self, serial: &str) -> Result<()> {
        let r = self.shell_script(serial, POWER_OFF_SCRIPT, SHORT)?;
        let out = r.combined();
        if out.contains("Exception") || out.contains("ermission denied") || out.contains("not allowed") {
            let line = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            return Err(Error::Command { summary: line.chars().take(140).collect(), code: r.code });
        }
        Ok(())
    }

    pub fn keyevent(&self, serial: &str, code: &str) -> Result<CommandResult> {
        if !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(Error::Invalid(t!("invalid key code: {}", code)));
        }
        self.shell(serial, &["input", "keyevent", code], SHORT)
    }

    /// Types text into the focused field (spaces are encoded as %s).
    pub fn input_text(&self, serial: &str, text: &str) -> Result<CommandResult> {
        let encoded = text.replace(' ', "%s");
        self.shell(serial, &["input", "text", &encoded], SHORT)
    }

    pub fn set_usb_function(&self, serial: &str, function: &str) -> Result<CommandResult> {
        if !["mtp", "ptp", "adb", "rndis", "midi", "none"].contains(&function) {
            return Err(Error::Invalid(function.into()));
        }
        self.shell(serial, &["svc", "usb", "setFunctions", function], Duration::from_secs(15))
    }

    pub fn reconnect(&self, serial: &str) -> Result<CommandResult> {
        self.run(Some(serial), &["reconnect", "device"], NORMAL)
    }

    pub fn screenshot_png(&self, serial: &str) -> Result<Vec<u8>> {
        let (r, bytes) = self.run_bytes(Some(serial), &["exec-out", "screencap", "-p"], Duration::from_secs(30))?;
        if !r.ok() || bytes.len() < 8 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
            return Err(Error::Command { summary: t!("invalid screenshot: {}", r.summary()), code: r.code });
        }
        Ok(bytes)
    }

    // -- wireless ----------------------------------------------------------

    pub fn connect(&self, address: &str) -> Result<CommandResult> {
        let addr = validate_host_port(address)?;
        let r = self.run(None, &["connect", &addr], Duration::from_secs(15))?;
        let out = r.combined().to_lowercase();
        if out.contains("connected to") || out.contains("already connected") {
            Ok(r)
        } else {
            Err(Error::Command { summary: r.summary(), code: r.code })
        }
    }

    pub fn disconnect(&self, address: &str) -> Result<CommandResult> {
        self.run(None, &["disconnect", address.trim()], SHORT)
    }

    /// Wireless debugging pairing (Android 11+) with the 6-digit code shown by
    /// the phone.
    pub fn pair(&self, address: &str, code: &str) -> Result<CommandResult> {
        let code = code.trim();
        if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
            return Err(Error::Invalid(tr("the pairing code has 6 digits").into()));
        }
        self.pair_secret(address, code)
    }

    /// QR pairing: the password was generated by ADB Scan
    /// (`wireless::QrPairing`), alphanumeric.
    pub fn pair_with_password(&self, address: &str, password: &str) -> Result<CommandResult> {
        let p = password.trim();
        if p.is_empty() || p.len() > 64 || !p.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(Error::Invalid(tr("invalid pairing password").into()));
        }
        self.pair_secret(address, p)
    }

    fn pair_secret(&self, address: &str, secret: &str) -> Result<CommandResult> {
        let addr = validate_host_port(address)?;
        let r = self.run(None, &["pair", &addr, secret], Duration::from_secs(30))?;
        if r.combined().to_lowercase().contains("successfully paired") {
            Ok(r)
        } else {
            Err(Error::Command { summary: r.summary(), code: r.code })
        }
    }

    /// Turns on ADB over TCP on the given port (requires USB to be connected).
    pub fn tcpip(&self, serial: &str, port: u16) -> Result<CommandResult> {
        self.run(Some(serial), &["tcpip", &port.to_string()], NORMAL)?.into_result()
    }

    /// Announced mDNS services (`adb mdns services`).
    pub fn mdns_services(&self) -> Vec<(String, String, String)> {
        let Ok(r) = self.run(None, &["mdns", "services"], SHORT) else {
            return Vec::new();
        };
        r.stdout
            .lines()
            .filter_map(|l| {
                let p: Vec<&str> = l.split_whitespace().collect();
                (p.len() >= 3 && p[2].contains(':')).then(|| (p[0].to_string(), p[1].to_string(), p[2].to_string()))
            })
            .collect()
    }

    // -- packages ---------------------------------------------------------------

    pub fn packages(&self, serial: &str) -> Result<Vec<PackageInfo>> {
        let all = self.shell(serial, &["pm", "list", "packages", "-f", "--show-versioncode"], LONG)?.into_result()?;
        let user = self.shell_text(serial, &["pm", "list", "packages", "-3"]);
        let disabled = self.shell_text(serial, &["pm", "list", "packages", "-d"]);
        let user = parsers::parse_package_names(&user);
        let disabled = parsers::parse_package_names(&disabled);
        let records = parsers::parse_package_records(&all.stdout);
        // Data for sorting and filtering, in bulk (three quick calls).
        let installers = parsers::parse_installers(&self.shell_text(serial, &["pm", "list", "packages", "-i"]));
        let sizes = parsers::parse_diskstats(&self.shell_text(serial, &["dumpsys", "diskstats"]));
        let mut mtimes = std::collections::HashMap::new();
        let paths: Vec<&str> = records.values().map(|r| r.path.as_str()).filter(|p| validate_remote_path(p).is_ok()).collect();
        for chunk in paths.chunks(80) {
            let script = format!("stat -c '%Y %n' {} 2>/dev/null", chunk.iter().map(|p| shell_quote(p)).collect::<Vec<_>>().join(" "));
            if let Ok(r) = self.shell_script(serial, &script, NORMAL) {
                mtimes.extend(parsers::parse_stat_mtimes(&r.stdout));
            }
        }
        Ok(records
            .into_iter()
            .map(|(name, rec)| PackageInfo {
                is_system: !user.contains(&name),
                enabled: Some(!disabled.contains(&name)),
                installer: installers.get(&name).cloned().unwrap_or_default(),
                total_bytes: sizes.get(&name).map(|d| d.total()).filter(|b| *b > 0),
                disk: sizes.get(&name).copied().filter(|d| d.total() > 0),
                updated_ts: mtimes.get(&rec.path).copied(),
                apk_path: rec.path,
                version_code: rec.version_code,
                package: name,
                ..Default::default()
            })
            .collect())
    }

    pub fn package_details(&self, serial: &str, package: &str, apk_path: &str) -> Result<PackageInfo> {
        check_package(package)?;
        let r = self.shell(serial, &["dumpsys", "package", package], NORMAL)?;
        let mut info = parsers::parse_package_dump(&r.stdout, package, apk_path);
        let path = if apk_path.is_empty() { info.apk_path.clone() } else { apk_path.to_string() };
        if !path.is_empty() {
            let du = self.shell_text(serial, &["du", "-sk", &remote_parent_if_apk(&path)]);
            if let Some(kib) = du.split_whitespace().next().and_then(|v| v.parse::<i64>().ok()) {
                info.size_bytes = Some(kib * 1024);
            }
        }
        Ok(info)
    }

    /// Paths of a package's APKs (base + splits).
    pub fn package_apk_paths(&self, serial: &str, package: &str) -> Result<Vec<String>> {
        check_package(package)?;
        let r = self.shell(serial, &["pm", "path", package], NORMAL)?;
        Ok(r.stdout.lines().filter_map(|l| l.trim().strip_prefix("package:")).map(String::from).collect())
    }

    /// Display name of an installed app, reading only the manifest and
    /// `resources.arsc` with `unzip -p` on the phone. `None` if there is no
    /// unzip.
    pub fn package_label(&self, serial: &str, apk_path: &str) -> Option<String> {
        let manifest = self.apk_entry(serial, apk_path, "AndroidManifest.xml").ok()?;
        let arsc = self.apk_entry(serial, apk_path, "resources.arsc").ok();
        crate::apk::label_from_parts(&manifest, arsc.as_deref())
    }

    /// A file from an installed APK, read on the phone itself with `unzip -p`
    /// (without copying the APK). The error says why it could not be read.
    pub fn apk_entry(&self, serial: &str, apk_path: &str, entry: &str) -> std::result::Result<Vec<u8>, crate::probe::Probe<()>> {
        use crate::probe::Probe;
        if validate_remote_path(apk_path).is_err() || !apk_path.ends_with(".apk") {
            return Err(Probe::Failed(tr("invalid APK path").into()));
        }
        // unzip's warnings go to stderr so they do not mix with the data.
        let cmd = format!("unzip -p {} {} 2>/dev/null", shell_quote(apk_path), shell_quote(entry));
        let (_, bytes) = self.run_bytes(Some(serial), &["exec-out", &cmd], Duration::from_secs(30)).map_err(|e| Probe::Failed(e.to_string()))?;
        // A binary XML or a resources.arsc starts with its chunk type (0x0003 /
        // 0x0002).
        if bytes.len() > 8 && bytes[1] == 0 && (bytes[0] == 3 || bytes[0] == 2) {
            return Ok(bytes);
        }
        // No data: the reason is found out with a separate query.
        let why = self.shell_text(serial, &["sh", "-c", &format!("command -v unzip >/dev/null || echo NOUNZIP; ls {} 2>&1 >/dev/null", shell_quote(apk_path))]);
        Err(if why.contains("NOUNZIP") { Probe::Unsupported(tr("unzip is not on the phone").into()) } else { crate::probe::classify_failure(&why).unwrap_or(Probe::Missing) })
    }

    /// A file from an installed APK (`unzip -p` on the phone), up to `max`
    /// bytes. `Ok(None)` if it is not there or is larger; `Err` if the
    /// connection with the phone failed (worth retrying).
    fn apk_read(&self, serial: &str, apk_path: &str, entry: &str, max: usize) -> Result<Option<Vec<u8>>> {
        if validate_remote_path(apk_path).is_err() || !apk_path.ends_with(".apk") || entry.contains("..") {
            return Ok(None);
        }
        // unzip's warnings go to /dev/null on the phone: whatever arrives on
        // stderr comes from adb (device disconnected, connection closed…).
        let cmd = format!("unzip -p {} {} 2>/dev/null", shell_quote(apk_path), shell_quote(entry));
        let (r, bytes) = self.run_bytes(Some(serial), &["exec-out", &cmd], Duration::from_secs(30))?;
        if r.timed_out || !r.stderr.trim().is_empty() {
            return Err(Error::Command { summary: format!("adb exec-out: {}", if r.timed_out { tr("timed out") } else { r.stderr.trim() }), code: r.code });
        }
        Ok((!bytes.is_empty() && bytes.len() <= max).then_some(bytes))
    }

    /// Density splits of an installed app (`split_config.xxhdpi.apk`…), next to
    /// its `base.apk`: with Android App Bundles, each density's images go
    /// there.
    fn density_splits(&self, serial: &str, base_apk: &str) -> Result<Vec<String>> {
        let Some(dir) = base_apk.rsplit_once('/').map(|(d, _)| d) else {
            return Ok(Vec::new());
        };
        let r = self.shell(serial, &["ls", "-1", dir], NORMAL)?;
        Ok(r.stdout.lines().map(str::trim).filter(|n| n.starts_with("split_config.") && n.ends_with("dpi.apk")).map(|n| format!("{dir}/{n}")).collect())
    }

    /// Real icon of an app (see [`crate::apk::resolve_icon`]), read on the
    /// phone without copying the APK. Read-only. `Ok(None)`: the app has no
    /// icon that can be drawn (or there is no `unzip`); `Err`: the connection
    /// failed and it is worth retrying.
    /// `apk_path` can be the APK or the app's folder (`codePath` from `dumpsys
    /// package`): for the folder its `base.apk` is used.
    pub fn app_icon(&self, serial: &str, apk_path: &str) -> Result<Option<crate::models::AppIcon>> {
        use crate::apk::{resolve_icon, IconLayer};
        const MAX_IMAGE: usize = 4 << 20;
        const MAX_TABLE: usize = 64 << 20;
        let base = if apk_path.ends_with(".apk") { apk_path.to_string() } else { format!("{}/base.apk", apk_path.trim_end_matches('/')) };
        let Some(manifest) = self.apk_read(serial, &base, "AndroidManifest.xml", MAX_TABLE)? else {
            return Ok(None);
        };
        let Some(arsc) = self.apk_read(serial, &base, "resources.arsc", MAX_TABLE)? else {
            return Ok(None);
        };
        // A connection failure while reading a layer: if in the end there is no
        // icon, it is returned so it gets retried (and is not remembered as “no
        // icon”).
        let failed = std::cell::RefCell::new(None);
        let read = |apk: &str, file: &str| match self.apk_read(serial, apk, file, MAX_IMAGE) {
            Ok(b) => b,
            Err(e) => {
                failed.borrow_mut().get_or_insert(e);
                None
            }
        };
        let mut icon = resolve_icon(&manifest, &[&arsc], &|_, f| read(&base, f));
        // No image in the main APK: look for it in the density splits.
        if icon.as_ref().map_or(true, |i| i.converted) {
            let mut apks = vec![base.clone()];
            let mut tables = vec![arsc.clone()];
            for split in self.density_splits(serial, &base)? {
                if let Some(t) = self.apk_read(serial, &split, "resources.arsc", MAX_TABLE)? {
                    apks.push(split);
                    tables.push(t);
                }
            }
            if apks.len() > 1 {
                let refs: Vec<&[u8]> = tables.iter().map(Vec::as_slice).collect();
                let with_splits = resolve_icon(&manifest, &refs, &|i, f| read(apks.get(i)?, f));
                if with_splits.as_ref().is_some_and(|w| icon.is_none() || !w.converted) {
                    icon = with_splits;
                }
            }
        }
        if let (None, Some(e)) = (&icon, failed.into_inner()) {
            return Err(e);
        }
        Ok(icon.map(|i| crate::models::AppIcon {
            image: i.front,
            adaptive: i.adaptive,
            background_color: match &i.background {
                Some(IconLayer::Color(c)) => Some(*c),
                _ => None,
            },
            background_image: match i.background {
                Some(IconLayer::Image(b)) => Some(b),
                _ => None,
            },
        }))
    }

    /// Technical details of an app (see [`crate::app_inspect`]). Read-only.
    pub fn inspect_package(&self, serial: &str, package: &str) -> Result<crate::app_inspect::AppInspection> {
        use crate::probe::Probe;
        check_package(package)?;
        let r = self.shell_script(serial, &crate::app_inspect::inspect_script(package), NORMAL)?;
        let mut info = crate::app_inspect::parse_inspection(package, &r.stdout);
        let sections = parsers::split_sections(&r.stdout);
        let base = crate::app_inspect::AppInspection::base_apk(sections.get("PATH").map(String::as_str).unwrap_or(""), &info.extras);
        info.manifest = match base {
            None => Probe::Missing,
            Some(apk) => match self.apk_entry(serial, &apk, "AndroidManifest.xml") {
                Ok(bytes) => match crate::apk::manifest_summary(&bytes) {
                    Some(m) => Probe::Ok(m),
                    None => Probe::Failed(tr("unrecognised binary manifest").into()),
                },
                Err(e) => e.map_never(),
            },
        };
        Ok(info)
    }

    // -- controls (they change the phone)
    // ------------------------------------------

    /// State of the quick controls. Read-only (~0.2 s).
    pub fn control_state(&self, serial: &str) -> Result<crate::controls::ControlState> {
        let r = self.shell_script(serial, crate::controls::STATE_SCRIPT, NORMAL)?;
        Ok(crate::controls::parse_state(&r.stdout))
    }

    /// Runs a control command and returns an error if Android rejects it.
    fn control_command(&self, serial: &str, command: &str) -> Result<CommandResult> {
        let r = self.shell_script(serial, &format!("{command} 2>&1"), NORMAL)?;
        let out = r.combined();
        let lower = out.to_lowercase();
        if let Some(p) = crate::probe::classify_failure(&out) {
            return Err(Error::Command { summary: p.status_text().unwrap_or_default(), code: r.code });
        }
        if lower.contains("exception") || lower.contains("killed") || lower.contains("error:") || lower.contains("failed") {
            let first = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().chars().take(140).collect();
            return Err(Error::Command { summary: first, code: r.code });
        }
        Ok(r)
    }

    /// Turns a control on or off and checks, by reading the state back, that
    /// Android applied it. Returns whether it was applied and the final state.
    pub fn set_toggle(&self, serial: &str, toggle: crate::controls::Toggle, on: bool, sdk: Option<i64>) -> Result<(bool, crate::controls::ControlState)> {
        let command = toggle.command(on, sdk).ok_or_else(|| Error::Invalid(t!("{} cannot be changed without root on this Android version", toggle.label())))?;
        self.control_command(serial, &command)?;
        let deadline = std::time::Instant::now() + Duration::from_millis(toggle.settle_ms());
        loop {
            let state = self.control_state(serial)?;
            let done = toggle.state(&state).ok() == Some(&on);
            if done || std::time::Instant::now() >= deadline {
                return Ok((done, state));
            }
            thread::sleep(Duration::from_millis(500));
        }
    }

    /// Changes a value (brightness, volume, Do Not Disturb…) and returns the
    /// new state. Error if Android did not apply it.
    pub fn set_level(&self, serial: &str, level: crate::controls::Level, current: &crate::controls::ControlState) -> Result<crate::controls::ControlState> {
        use crate::controls::Level;
        self.control_command(serial, &level.command(current))?;
        thread::sleep(Duration::from_millis(300));
        let state = self.control_state(serial)?;
        let volume = |s: &crate::controls::ControlState, ring: bool| (if ring { &s.ring_volume } else { &s.media_volume }).ok().map(|v| v.value);
        match level {
            // Some manufacturers ignore `--set` from the shell: media volume is
            // then moved with the volume keys.
            Level::MediaVolume(target) => {
                let before = volume(current, false);
                if volume(&state, false) == Some(target) || before.is_none() || before == Some(target) {
                    return Ok(state);
                }
                self.media_volume_by_keys(serial, before.unwrap_or(0), target, current)
            }
            Level::RingVolume(target) if volume(&state, true) != Some(target) && volume(current, true) != Some(target) => {
                Err(Error::Command { summary: tr("Android does not allow changing the ring volume over ADB on this phone").into(), code: None })
            }
            _ => Ok(state),
        }
    }

    /// Media volume with the volume keys (see
    /// [`crate::controls::volume_keys_script`]). If the keys moved the ring
    /// volume instead of media (another ROM, a call), the ring volume is
    /// restored and it is reported.
    fn media_volume_by_keys(&self, serial: &str, from: i64, target: i64, before: &crate::controls::ControlState) -> Result<crate::controls::ControlState> {
        use crate::controls::volume_keys_script;
        self.shell_script(serial, &volume_keys_script(3, from, target), Duration::from_secs(40))?;
        let state = self.control_state(serial)?;
        let ring = |s: &crate::controls::ControlState| s.ring_volume.ok().map(|v| v.value);
        if let (Some(was), Some(now)) = (ring(before), ring(&state)) {
            if was != now {
                let _ = self.shell_script(serial, &volume_keys_script(2, now, was), Duration::from_secs(40));
            }
        }
        if state.media_volume.ok().map(|v| v.value) == Some(from) {
            return Err(Error::Command { summary: tr("Android does not allow changing the media volume over ADB right now").into(), code: None });
        }
        self.control_state(serial)
    }

    /// Stateless action (open quick settings, a Settings screen…).
    pub fn control_action(&self, serial: &str, action: crate::controls::Action) -> Result<CommandResult> {
        self.control_command(serial, &action.command())
    }

    /// Phone notifications (see [`crate::notifications`]). Read-only.
    pub fn phone_notifications(&self, serial: &str) -> Result<Vec<crate::notifications::PhoneNotification>> {
        let r = self.shell_script(serial, crate::notifications::READ_SCRIPT, NORMAL)?;
        Ok(crate::notifications::parse(&r.stdout))
    }

    /// Phone photos and videos (see [`crate::photos`]). Read-only.
    pub fn list_media(&self, serial: &str) -> Result<Vec<crate::photos::RemoteMedia>> {
        let r = self.shell_script(serial, crate::photos::LIST_SCRIPT, Duration::from_secs(120))?;
        Ok(crate::photos::parse_list(&r.stdout))
    }

    // -- remote (it changes the phone)
    // ----------------------------------------------

    /// Call state and what is playing (see [`crate::remote`]). Read-only.
    pub fn remote_status(&self, serial: &str) -> Result<(crate::remote::CallState, Option<crate::remote::NowPlaying>)> {
        let r = self.shell_script(serial, crate::remote::STATUS_SCRIPT, NORMAL)?;
        let s = parsers::split_sections(&r.stdout);
        let get = |k: &str| s.get(k).map(String::as_str).unwrap_or("");
        Ok((crate::remote::parse_call_state(get("CALL")), crate::remote::parse_now_playing(get("MEDIA"))))
    }

    /// Runs a remote `am start` command and returns an error if Android rejects
    /// it (`am` prints the error but exits with code 0).
    fn remote_am(&self, serial: &str, command: &str) -> Result<()> {
        let r = self.shell_script(serial, command, SHORT)?;
        match crate::remote::am_error(&r.combined()) {
            Some(e) => Err(Error::Command { summary: e, code: r.code }),
            None => Ok(()),
        }
    }

    pub fn media_key(&self, serial: &str, key: crate::remote::MediaKey) -> Result<CommandResult> {
        self.keyevent(serial, key.keycode())
    }

    /// Calls `number`. If Android does not allow calling directly from ADB, it
    /// opens the dialer with the number. Returns whether the call started.
    pub fn dial(&self, serial: &str, number: &str) -> Result<bool> {
        let invalid = || Error::Invalid(t!("invalid number: {}", number));
        let call = crate::remote::dial_command(number, true).ok_or_else(invalid)?;
        match self.remote_am(serial, &call) {
            Ok(()) => Ok(true),
            Err(_) => {
                self.remote_am(serial, &crate::remote::dial_command(number, false).ok_or_else(invalid)?)?;
                Ok(false)
            }
        }
    }

    /// Hangs up (or rejects) the current call. Without a call it does nothing:
    /// the hang-up key would turn the screen off.
    pub fn end_call(&self, serial: &str) -> Result<()> {
        use crate::remote::CallState;
        match self.remote_status(serial)?.0 {
            CallState::Ringing | CallState::InCall => self.keyevent(serial, "KEYCODE_ENDCALL").map(|_| ()),
            _ => Err(Error::Invalid(tr("there is no call").into())),
        }
    }

    /// Answers the incoming call (headset key).
    pub fn answer_call(&self, serial: &str) -> Result<()> {
        match self.remote_status(serial)?.0 {
            crate::remote::CallState::Ringing => self.keyevent(serial, "KEYCODE_HEADSETHOOK").map(|_| ()),
            _ => Err(Error::Invalid(tr("there is no incoming call").into())),
        }
    }

    pub fn open_url(&self, serial: &str, url: &str) -> Result<()> {
        let cmd = crate::remote::open_url_command(url).ok_or_else(|| Error::Invalid(tr("invalid link (use http:// or https://)").into()))?;
        self.remote_am(serial, &cmd)
    }

    pub fn compose_sms(&self, serial: &str, number: &str, text: &str) -> Result<()> {
        let cmd = crate::remote::sms_command(number, text).ok_or_else(|| Error::Invalid(t!("invalid number: {}", number)))?;
        self.remote_am(serial, &cmd)
    }

    /// Makes the phone ring (see [`crate::remote::find_phone_command`]).
    pub fn find_phone(&self, serial: &str) -> Result<()> {
        self.remote_am(serial, &crate::remote::find_phone_command())
    }

    /// Runs a scheduled action (see [`crate::schedule`]) and returns a summary
    /// for History.
    pub fn run_scheduled(&self, serial: &str, action: crate::schedule::ScheduledAction) -> Result<String> {
        use crate::controls::{Action, Dnd, Level};
        use crate::schedule::ScheduledAction as A;
        match action {
            A::PowerOff => self.power_off(serial).map(|_| tr("Powering off").into()),
            A::Reboot => self.reboot(serial, RebootMode::System).map(|_| tr("Restarting").into()),
            A::StopAll => {
                let plan = self.stop_plan(serial)?;
                let r = self.stop_all(serial, &plan)?;
                Ok(t!("{} apps stopped", r.stopped))
            }
            A::DndOn | A::DndOff => {
                let state = self.control_state(serial)?;
                let mode = if action == A::DndOn { Dnd::Priority } else { Dnd::Off };
                self.set_level(serial, Level::Dnd(mode), &state).map(|_| mode.label().to_string())
            }
            A::ScreenOff => self.control_action(serial, Action::ScreenOff).map(|_| tr("Screen turned off").into()),
        }
    }

    /// Runs an automation (see [`crate::automation`]) and returns a summary
    /// for History. `photos_dir`: where the photo backup copies (empty = not
    /// chosen yet). [`AutoAction::Mirror`](crate::automation::AutoAction) is
    /// run by the interface.
    pub fn run_automation(&self, serial: &str, action: crate::automation::AutoAction, photos_dir: &Path) -> Result<String> {
        use crate::automation::AutoAction as A;
        use crate::schedule::ScheduledAction as S;
        match action {
            A::BackupPhotos => {
                if photos_dir.as_os_str().is_empty() {
                    return Err(Error::Invalid(tr("choose the photos folder first (Files → Copy photos)").into()));
                }
                let list = self.list_media(serial)?;
                let items = crate::photos::plan(&list, photos_dir, |p| std::fs::metadata(p).ok().map(|m| m.len()));
                let cancel = CancelToken::new();
                let (mut n, mut bytes) = (0usize, 0u64);
                for (remote, size, local) in items {
                    if let Some(parent) = local.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    self.pull(serial, &remote, &local, size, &cancel, |_| {})?.into_result()?;
                    n += 1;
                    bytes += size;
                }
                Ok(t!("{} new photos and videos copied ({})", n, crate::format::human_bytes(Some(bytes as f64))))
            }
            A::DndOn => self.run_scheduled(serial, S::DndOn),
            A::DndOff => self.run_scheduled(serial, S::DndOff),
            A::StopAll => self.run_scheduled(serial, S::StopAll),
            A::StayAwakeOn | A::StayAwakeOff => self.dev_switch(serial, crate::devtools::Switch::StayAwake, action == A::StayAwakeOn).map(|_| action.label().to_string()),
            A::ClearCaches => self.trim_caches(serial).map(|k| k.map(|k| t!("{} freed", crate::format::human_kib(Some(k)))).unwrap_or_else(|| tr("Caches cleared").into())),
            A::Mirror => Err(Error::Invalid(tr("the screen mirror is opened by the window").into())),
        }
    }

    // -- stop all (it changes the phone) ----------------------------------------

    /// Preinstalled apps from [`crate::debloat`] still present, and every
    /// app removed for the user. Read-only.
    pub fn debloat_state(&self, serial: &str) -> Result<crate::debloat::DebloatState> {
        let r = self.shell_script(serial, crate::debloat::STATE_SCRIPT, NORMAL)?;
        Ok(crate::debloat::parse_state_output(&r.stdout))
    }

    /// What “Stop all” would close right now (see [`crate::cleanup`]).
    /// Read-only.
    pub fn stop_plan(&self, serial: &str) -> Result<crate::cleanup::StopPlan> {
        let r = self.shell_script(serial, crate::cleanup::PLAN_SCRIPT, NORMAL)?;
        Ok(crate::cleanup::parse_plan(&r.stdout))
    }

    /// Closes the apps in `plan.stop` (the plan the user confirmed) and frees
    /// the expendable processes. Then it reads again to know which ones the
    /// system relaunched and how much memory was freed.
    pub fn stop_all(&self, serial: &str, plan: &crate::cleanup::StopPlan) -> Result<crate::cleanup::StopResult> {
        let script = crate::cleanup::stop_script(&plan.stop).ok_or_else(|| Error::Invalid(tr("invalid package name").into()))?;
        let r = self.shell_script(serial, &script, Duration::from_secs(120))?;
        if let Some(line) = r.combined().lines().find(|l| l.contains("Exception") || l.contains("Error:")) {
            return Err(Error::Command { summary: line.trim().chars().take(140).collect(), code: r.code });
        }
        // What Android relaunches (system services that depend on an app) comes
        // back instantly; one second is enough to see it.
        thread::sleep(Duration::from_millis(1200));
        let after = self.stop_plan(serial)?;
        let restarted: Vec<String> = plan.stop.iter().filter(|p| after.stop.contains(p)).cloned().collect();
        Ok(crate::cleanup::StopResult { stopped: plan.stop.len() - restarted.len(), restarted, mem_before_kib: plan.mem_available_kib, mem_after_kib: after.mem_available_kib })
    }

    /// App windows “Close open apps” would close (see [`crate::cleanup`]).
    /// Read-only.
    pub fn open_apps(&self, serial: &str) -> Result<crate::cleanup::OpenApps> {
        let r = self.shell_script(serial, crate::cleanup::OPEN_APPS_SCRIPT, NORMAL)?;
        Ok(crate::cleanup::parse_open_apps(&r.stdout))
    }

    /// Closes the windows in `apps` (the list the user confirmed), as “Clear
    /// all” in Recents, and measures the memory freed.
    pub fn close_open_apps(&self, serial: &str, apps: &crate::cleanup::OpenApps) -> Result<crate::cleanup::CloseResult> {
        let r = self.shell_script(serial, &crate::cleanup::close_tasks_script(&apps.tasks), LONG)?;
        let failed = r.combined().lines().filter(|l| l.contains("Exception") || l.contains("Error")).count();
        if failed > 0 && failed >= apps.tasks.len() {
            let line = r.combined().lines().find(|l| l.contains("Exception") || l.contains("Error")).unwrap_or("").trim().chars().take(140).collect();
            return Err(Error::Command { summary: line, code: r.code });
        }
        thread::sleep(Duration::from_millis(1200));
        let after = self.open_apps(serial)?;
        let closed = apps.tasks.iter().filter(|t| !after.tasks.contains(t)).count();
        Ok(crate::cleanup::CloseResult { closed, mem_before_kib: apps.mem_available_kib, mem_after_kib: after.mem_available_kib })
    }

    /// Deletes the cache of every app. Returns the space freed (KiB).
    pub fn trim_caches(&self, serial: &str) -> Result<Option<i64>> {
        let r = self.shell_script(serial, &crate::cleanup::trim_caches_script(), LONG)?;
        cache_result(crate::cleanup::parse_cache_outcome(&r.stdout, false), r.code)
    }

    /// Deletes the cache of one app, keeping its data. Returns the space
    /// freed (KiB). Fails without running anything on an Android that cannot
    /// do it (see [`crate::cleanup::clear_cache_script`]).
    pub fn clear_cache(&self, serial: &str, package: &str) -> Result<Option<i64>> {
        let script = crate::cleanup::clear_cache_script(package).ok_or_else(|| Error::Invalid(tr("invalid package name").into()))?;
        let r = self.shell_script(serial, &script, LONG)?;
        cache_result(crate::cleanup::parse_cache_outcome(&r.stdout, true), r.code)
    }

    /// When Android last measured the size of each app (`dumpsys diskstats`
    /// shows that measurement, not a live one), in seconds since 1970.
    pub fn disk_stats_time(&self, serial: &str) -> Option<i64> {
        self.shell_text(serial, &["stat", "-c", "%Y", "/data/system/diskstats_cache.json"]).trim().parse().ok()
    }

    /// The phone's side of the USB link (see [`crate::link`]). Read-only.
    pub fn phone_link(&self, serial: &str) -> Result<crate::link::PhoneLink> {
        let r = self.shell_script(serial, crate::link::PHONE_SCRIPT, NORMAL)?;
        Ok(crate::link::parse_phone_link(&r.stdout))
    }

    /// Measures the real transfer speed: sends a temporary file to the phone,
    /// reads it back and deletes both copies. It only writes in
    /// `/data/local/tmp`, the shell's own folder.
    pub fn speed_test(&self, serial: &str) -> Result<crate::link::SpeedTest> {
        use crate::link::{parse_transfer, speed_test_data, SpeedTest, SPEED_TEST_BYTES, SPEED_TEST_REMOTE};
        let dir = std::env::temp_dir();
        let up = dir.join(format!("adbscan-speedtest-{}.bin", std::process::id()));
        let down = dir.join(format!("adbscan-speedtest-{}-back.bin", std::process::id()));
        std::fs::write(&up, speed_test_data(SPEED_TEST_BYTES))?;
        let rate = |r: &CommandResult| {
            let (bytes, secs) = parse_transfer(&r.combined()).unwrap_or((SPEED_TEST_BYTES as f64, r.duration.as_secs_f64().max(0.001)));
            bytes / secs / 1e6
        };
        let result = (|| -> Result<SpeedTest> {
            let pushed = self.run(Some(serial), &["push", &up.display().to_string(), SPEED_TEST_REMOTE], LONG)?;
            if !pushed.ok() {
                return Err(Error::Command { summary: pushed.summary(), code: pushed.code });
            }
            let pulled = self.run(Some(serial), &["pull", SPEED_TEST_REMOTE, &down.display().to_string()], LONG)?;
            if !pulled.ok() {
                return Err(Error::Command { summary: pulled.summary(), code: pulled.code });
            }
            Ok(SpeedTest { upload_mbs: rate(&pushed), download_mbs: rate(&pulled) })
        })();
        let _ = self.shell(serial, &["rm", "-f", SPEED_TEST_REMOTE], SHORT);
        let _ = std::fs::remove_file(&up);
        let _ = std::fs::remove_file(&down);
        result
    }

    /// Whether this Android can clear a single app's cache through ADB
    /// (`pm clear --cache-only`). Read-only.
    pub fn supports_clear_cache(&self, serial: &str) -> bool {
        self.shell_script(serial, "pm help 2>&1 | grep -c -e '--cache-only'", NORMAL).is_ok_and(|r| r.stdout.trim().parse::<u32>().unwrap_or(0) > 0)
    }

    /// Platform sheet (see [`crate::platform`]). Read-only.
    pub fn platform_report(&self, serial: &str) -> Result<crate::platform::PlatformReport> {
        let r = self.shell_script(serial, crate::platform::PLATFORM_SCRIPT, NORMAL)?;
        Ok(crate::platform::parse_platform(&r.stdout))
    }

    pub fn package_pids(&self, serial: &str, package: &str) -> Vec<i64> {
        self.shell_text(serial, &["pidof", package]).split_whitespace().filter_map(|p| p.parse().ok()).collect()
    }

    pub fn install(&self, serial: &str, apks: &[PathBuf], opts: &InstallOptions) -> Result<CommandResult> {
        if apks.is_empty() {
            return Err(Error::Invalid(tr("there is no APK to install").into()));
        }
        for apk in apks {
            if !apk.is_file() {
                return Err(Error::Invalid(t!("{} does not exist", apk.display())));
            }
        }
        let mut args: Vec<String> = vec![if apks.len() > 1 { "install-multiple" } else { "install" }.into()];
        if opts.replace {
            args.push("-r".into());
        }
        if opts.downgrade {
            args.push("-d".into());
        }
        if opts.grant_all {
            args.push("-g".into());
        }
        if opts.allow_test {
            args.push("-t".into());
        }
        args.extend(apks.iter().map(|p| p.display().to_string()));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let r = self.run(Some(serial), &refs, Duration::from_secs(600))?;
        if r.combined().contains("Success") {
            Ok(r)
        } else {
            Err(Error::Command { summary: install_error_hint(&r.combined()), code: r.code })
        }
    }

    pub fn uninstall(&self, serial: &str, package: &str, keep_data: bool) -> Result<CommandResult> {
        check_package(package)?;
        let mut args = vec!["pm", "uninstall"];
        if keep_data {
            args.push("-k");
        }
        args.push(package);
        pm_result(self.shell(serial, &args, LONG)?)
    }

    /// Uninstalls only for user 0 (system apps; reversible with
    /// [`Adb::reinstall_existing`]).
    pub fn uninstall_for_user(&self, serial: &str, package: &str) -> Result<CommandResult> {
        check_package(package)?;
        pm_result(self.shell(serial, &["pm", "uninstall", "-k", "--user", "0", package], LONG)?)
    }

    pub fn reinstall_existing(&self, serial: &str, package: &str) -> Result<CommandResult> {
        check_package(package)?;
        pm_result(self.shell(serial, &["cmd", "package", "install-existing", package], LONG)?)
    }

    pub fn set_enabled(&self, serial: &str, package: &str, enabled: bool) -> Result<CommandResult> {
        check_package(package)?;
        let args: &[&str] = if enabled { &["pm", "enable", package] } else { &["pm", "disable-user", "--user", "0", package] };
        let r = self.shell(serial, args, NORMAL)?;
        if r.combined().contains("new state") {
            Ok(r)
        } else {
            Err(Error::Command { summary: r.summary(), code: r.code })
        }
    }

    pub fn clear_data(&self, serial: &str, package: &str) -> Result<CommandResult> {
        check_package(package)?;
        pm_result(self.shell(serial, &["pm", "clear", package], NORMAL)?)
    }

    /// Removes an app's “Install unknown apps” permission (`appops set …
    /// REQUEST_INSTALL_PACKAGES deny`). Reversible from Settings → Apps →
    /// Special app access.
    pub fn deny_install_packages(&self, serial: &str, package: &str) -> Result<CommandResult> {
        check_package(package)?;
        self.shell(serial, &["appops", "set", "--user", "0", package, "REQUEST_INSTALL_PACKAGES", "deny"], SHORT)?.into_result()
    }

    /// Turns off USB debugging. The ADB connection is cut immediately.
    pub fn disable_usb_debugging(&self, serial: &str) -> Result<CommandResult> {
        self.shell(serial, &["settings", "put", "global", "adb_enabled", "0"], SHORT)?.into_result()
    }

    pub fn force_stop(&self, serial: &str, package: &str) -> Result<CommandResult> {
        check_package(package)?;
        self.shell(serial, &["am", "force-stop", package], SHORT)?.into_result()
    }

    pub fn launch(&self, serial: &str, package: &str) -> Result<CommandResult> {
        check_package(package)?;
        let r = self.shell(serial, &["monkey", "-p", package, "-c", "android.intent.category.LAUNCHER", "1"], NORMAL)?;
        if r.combined().contains("No activities found") {
            return Err(Error::Command { summary: tr("The app has no main activity").into(), code: r.code });
        }
        Ok(r)
    }

    pub fn grant_permission(&self, serial: &str, package: &str, permission: &str, grant: bool) -> Result<CommandResult> {
        check_package(package)?;
        if !permission.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') {
            return Err(Error::Invalid(permission.into()));
        }
        let verb = if grant { "grant" } else { "revoke" };
        self.shell(serial, &["pm", verb, package, permission], NORMAL)?.into_result()
    }

    /// What changed on the phone: apps installed, updated and removed,
    /// restarts and shutdowns (see [`crate::changes`]). Read-only.
    pub fn phone_changes(&self, serial: &str) -> Result<crate::changes::Timeline> {
        let r = self.shell_script(serial, crate::changes::SCRIPT, LONG)?;
        Ok(crate::changes::parse(&r.stdout))
    }

    /// Every sensitive permission and special access, by app (see
    /// [`crate::permissions`]). Read-only.
    pub fn permission_overview(&self, serial: &str) -> Result<crate::permissions::Overview> {
        let r = self.shell_script(serial, crate::permissions::SCRIPT, LONG)?;
        Ok(crate::permissions::parse(&r.stdout))
    }

    /// Sets an App Op from [`crate::permissions::APP_OPS`] to allow or deny
    /// for user 0 (the same switch as in Settings → Special app access).
    pub fn set_app_op(&self, serial: &str, package: &str, op: &str, allow: bool) -> Result<CommandResult> {
        check_package(package)?;
        if !crate::permissions::APP_OPS.contains(&op) {
            return Err(Error::Invalid(op.into()));
        }
        self.shell(serial, &["appops", "set", "--user", "0", package, op, if allow { "allow" } else { "deny" }], SHORT)?.into_result()
    }

    /// Turns off one accessibility service: rewrites the list of enabled
    /// services without it.
    pub fn disable_accessibility_service(&self, serial: &str, component: &str) -> Result<CommandResult> {
        if !crate::permissions::is_valid_component(component) {
            return Err(Error::Invalid(component.into()));
        }
        let current = self.shell(serial, &["settings", "get", "secure", "enabled_accessibility_services"], SHORT)?.into_result()?;
        let rest = crate::permissions::without_component(&current.stdout, component);
        let r = if rest.is_empty() {
            self.shell(serial, &["settings", "delete", "secure", "enabled_accessibility_services"], SHORT)?
        } else {
            self.shell(serial, &["settings", "put", "secure", "enabled_accessibility_services", &rest], SHORT)?
        };
        r.into_result()
    }

    /// Takes away an app's access to notifications.
    pub fn disallow_notification_listener(&self, serial: &str, component: &str) -> Result<CommandResult> {
        if !crate::permissions::is_valid_component(component) {
            return Err(Error::Invalid(component.into()));
        }
        self.shell(serial, &["cmd", "notification", "disallow_listener", component], SHORT)?.into_result()
    }

    // -- developer tools (see `crate::devtools`) ------------------------------

    pub fn dev_state(&self, serial: &str) -> Result<crate::devtools::DevState> {
        let r = self.shell_script(serial, crate::devtools::STATE_SCRIPT, NORMAL)?;
        Ok(crate::devtools::parse_state(&r.stdout))
    }

    pub fn dev_switch(&self, serial: &str, switch: crate::devtools::Switch, on: bool) -> Result<CommandResult> {
        self.shell_script(serial, &switch.script(on), SHORT)?.into_result()
    }

    /// Sets the three animation scales (window, transition, animator).
    pub fn set_animation_scale(&self, serial: &str, scale: &str) -> Result<CommandResult> {
        if !crate::devtools::ANIMATION_SCALES.contains(&scale) {
            return Err(Error::Invalid(scale.into()));
        }
        let script = format!("settings put global window_animation_scale {scale}; settings put global transition_animation_scale {scale}; settings put global animator_duration_scale {scale}");
        self.shell_script(serial, &script, SHORT)?.into_result()
    }

    /// Screen resolution: `Some` sets an override, `None` goes back to the
    /// physical one.
    pub fn set_display_size(&self, serial: &str, size: Option<(u32, u32)>) -> Result<CommandResult> {
        let arg = match size {
            Some((w, h)) => {
                let (w, h) = crate::devtools::check_size(w, h)?;
                format!("{w}x{h}")
            }
            None => "reset".into(),
        };
        self.shell(serial, &["wm", "size", &arg], SHORT)?.into_result()
    }

    pub fn set_density(&self, serial: &str, dpi: Option<u32>) -> Result<CommandResult> {
        let arg = match dpi {
            Some(d) => crate::devtools::check_density(d)?.to_string(),
            None => "reset".into(),
        };
        self.shell(serial, &["wm", "density", &arg], SHORT)?.into_result()
    }

    pub fn demo_mode(&self, serial: &str, on: bool) -> Result<CommandResult> {
        self.shell_script(serial, crate::devtools::demo_script(on), NORMAL)?.into_result()
    }

    /// Forwarding rules of this phone: `adb forward` (PC → phone) and `adb
    /// reverse` (phone → PC).
    pub fn port_rules(&self, serial: &str) -> Vec<crate::devtools::PortRule> {
        let fwd = self.run(None, &["forward", "--list"], SHORT).map(|r| r.stdout).unwrap_or_default();
        let rev = self.run(Some(serial), &["reverse", "--list"], SHORT).map(|r| r.stdout).unwrap_or_default();
        let mut v = crate::devtools::parse_rules(&fwd, false, serial);
        v.extend(crate::devtools::parse_rules(&rev, true, serial));
        v
    }

    /// Adds a TCP rule. `reverse`: the phone's `local` port reaches the PC's
    /// `remote` port (a web server on the PC seen as localhost on the phone).
    pub fn add_port_rule(&self, serial: &str, reverse: bool, local: u32, remote: u32) -> Result<CommandResult> {
        let (l, r) = (crate::devtools::check_port(local)?, crate::devtools::check_port(remote)?);
        let (l, r) = (format!("tcp:{l}"), format!("tcp:{r}"));
        self.run(Some(serial), &[if reverse { "reverse" } else { "forward" }, &l, &r], SHORT)?.into_result()
    }

    pub fn remove_port_rule(&self, serial: &str, rule: &crate::devtools::PortRule) -> Result<CommandResult> {
        let ok = |s: &str| s.split_once(':').is_some_and(|(k, v)| k.chars().all(|c| c.is_ascii_lowercase()) && !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/')));
        if !ok(&rule.local) {
            return Err(Error::Invalid(rule.local.clone()));
        }
        self.run(Some(serial), &[if rule.reverse { "reverse" } else { "forward" }, "--remove", &rule.local], SHORT)?.into_result()
    }

    // -- app migration (see `crate::migration`) ------------------------------

    /// The user's apps with their version code. Read-only.
    pub fn user_packages(&self, serial: &str) -> Result<Vec<(String, Option<i64>)>> {
        let r = self.shell(serial, &["pm", "list", "packages", "-3", "--show-versioncode"], NORMAL)?.into_result()?;
        Ok(crate::migration::parse_user_packages(&r.stdout))
    }

    /// Copies an app's APKs to `<dir>/<package>/` keeping their names, and
    /// returns them relative to `dir` (as the manifest stores them).
    pub fn export_app(&self, serial: &str, package: &str, dir: &Path) -> Result<Vec<String>> {
        let remote = self.package_apk_paths(serial, package)?;
        if remote.is_empty() {
            return Err(Error::Command { summary: tr("No APKs were found").into(), code: None });
        }
        let folder = dir.join(package);
        std::fs::create_dir_all(&folder)?;
        let mut out = Vec::new();
        for r in remote {
            let Some(name) = crate::migration::apk_file_name(&r) else { continue };
            let local = folder.join(&name);
            self.run(Some(serial), &["pull", &r, &local.display().to_string()], TRANSFER)?.into_result()?;
            out.push(format!("{package}/{name}"));
        }
        if out.is_empty() {
            return Err(Error::Command { summary: tr("No APKs were found").into(), code: None });
        }
        Ok(out)
    }

    /// Extracts all of a package's APKs to `dest_dir`.
    pub fn pull_apks(&self, serial: &str, package: &str, dest_dir: &Path) -> Result<Vec<PathBuf>> {
        let paths = self.package_apk_paths(serial, package)?;
        if paths.is_empty() {
            return Err(Error::Command { summary: tr("No APKs were found").into(), code: None });
        }
        std::fs::create_dir_all(dest_dir)?;
        let mut out = Vec::new();
        for remote in paths {
            let name = remote.rsplit('/').next().unwrap_or("base.apk");
            let local = if name == "base.apk" { dest_dir.join(format!("{package}.apk")) } else { dest_dir.join(format!("{package}-{name}")) };
            self.run(Some(serial), &["pull", &remote, &local.display().to_string()], TRANSFER)?.into_result()?;
            out.push(local);
        }
        Ok(out)
    }

    // -- files ---------------------------------------------------------------

    pub fn list_dir(&self, serial: &str, path: &str) -> Result<Vec<FileEntry>> {
        let path = validate_remote_path(path)?;
        let dir = if path.ends_with('/') { path.clone() } else { format!("{path}/") };
        let r = self.shell(serial, &["ls", "-la", &dir], NORMAL)?;
        let entries = parsers::parse_ls(&r.stdout);
        if entries.is_empty() && !r.ok() {
            return Err(Error::Command { summary: permission_hint(&r.combined()), code: r.code });
        }
        Ok(entries)
    }

    pub fn remote_size(&self, serial: &str, path: &str) -> Option<u64> {
        self.shell_text(serial, &["stat", "-c", "%s", path]).trim().parse().ok()
    }

    pub fn make_dir(&self, serial: &str, path: &str) -> Result<CommandResult> {
        let path = validate_remote_path(path)?;
        self.shell(serial, &["mkdir", "-p", &path], NORMAL)?.into_result()
    }

    pub fn delete(&self, serial: &str, path: &str) -> Result<CommandResult> {
        let path = validate_remote_path(path)?;
        if is_protected_remote_path(&path) {
            return Err(Error::Guarded(t!("{} is a protected path", path)));
        }
        self.shell(serial, &["rm", "-rf", &path], LONG)?.into_result()
    }

    /// Deletes one file of the shared storage (never a folder: `rm -f`
    /// without `-r`). Used by the storage analyzer.
    pub fn delete_shared_file(&self, serial: &str, path: &str) -> Result<CommandResult> {
        let path = validate_remote_path(path)?;
        if !crate::storage::deletable(&path) {
            return Err(Error::Guarded(t!("{} is a protected path", path)));
        }
        let r = self.shell(serial, &["rm", "-f", "--", &path], NORMAL)?.into_result()?;
        // `rm -f` does not fail on a folder or a missing file: check.
        let left = self.shell(serial, &["ls", "-d", "--", &path], SHORT)?;
        if left.ok() {
            return Err(Error::Command { summary: t!("{} was not deleted", path), code: left.code });
        }
        Ok(r)
    }

    pub fn rename(&self, serial: &str, from: &str, to: &str) -> Result<CommandResult> {
        let from = validate_remote_path(from)?;
        let to = validate_remote_path(to)?;
        if is_protected_remote_path(&from) {
            return Err(Error::Guarded(t!("{} is a protected path", from)));
        }
        self.shell(serial, &["mv", "-n", &from, &to], NORMAL)?.into_result()
    }

    /// Reads a small text file from the device (preview).
    pub fn read_text(&self, serial: &str, path: &str, max_bytes: usize) -> Result<String> {
        let path = validate_remote_path(path)?;
        let r = self.shell(serial, &["head", "-c", &max_bytes.to_string(), &path], NORMAL)?.into_result()?;
        Ok(r.stdout)
    }

    /// `adb push` with progress measured by querying the remote size.
    pub fn push<F>(&self, serial: &str, local: &Path, remote: &str, cancel: &CancelToken, on_progress: F) -> Result<CommandResult>
    where
        F: Fn(Progress) + Send + Sync + 'static,
    {
        let remote = validate_remote_path(remote)?;
        let total = local.metadata()?.len();
        let target = if local.is_dir() { None } else { Some(remote.clone()) };
        let watcher = {
            let adb = self.clone();
            let serial = serial.to_string();
            move || target.as_ref().and_then(|t| adb.remote_size(&serial, t))
        };
        self.transfer(serial, &["push", &local.display().to_string(), &remote], total, cancel, watcher, on_progress)
    }

    /// `adb pull` with progress measured by the local file's size.
    pub fn pull<F>(&self, serial: &str, remote: &str, local: &Path, total: u64, cancel: &CancelToken, on_progress: F) -> Result<CommandResult>
    where
        F: Fn(Progress) + Send + Sync + 'static,
    {
        let remote = validate_remote_path(remote)?;
        let local_owned = local.to_path_buf();
        let watcher = move || local_owned.metadata().ok().map(|m| m.len());
        self.transfer(serial, &["pull", "-a", &remote, &local.display().to_string()], total, cancel, watcher, on_progress)
    }

    fn transfer<W, F>(&self, serial: &str, args: &[&str], total: u64, cancel: &CancelToken, watcher: W, on_progress: F) -> Result<CommandResult>
    where
        W: Fn() -> Option<u64> + Send + 'static,
        F: Fn(Progress) + Send + Sync + 'static,
    {
        let done_flag = Arc::new(AtomicBool::new(false));
        let on_progress = Arc::new(on_progress);
        let poll = {
            let done_flag = done_flag.clone();
            let cb = on_progress.clone();
            thread::spawn(move || {
                while !done_flag.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(400));
                    if let Some(size) = watcher() {
                        if !done_flag.load(Ordering::SeqCst) {
                            cb(Progress { done: size.min(total), total });
                        }
                    }
                }
            })
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let stderr = Arc::new(std::sync::Mutex::new(String::new()));
        let stdout = Arc::new(std::sync::Mutex::new(String::new()));
        let (se, so) = (stderr.clone(), stdout.clone());
        let handle = self.stream(Some(serial), args, false, move |ev| match ev {
            StreamEvent::Stdout(l) => {
                let mut s = so.lock().unwrap();
                s.push_str(&l);
                s.push('\n');
            }
            StreamEvent::Stderr(l) => {
                let mut s = se.lock().unwrap();
                s.push_str(&l);
                s.push('\n');
            }
            StreamEvent::Exit { code, cancelled } => {
                let _ = tx.lock().unwrap().send((code, cancelled));
            }
        })?;
        let (code, cancelled) = loop {
            if cancel.is_cancelled() {
                handle.cancel();
            }
            match rx.recv_timeout(Duration::from_millis(150)) {
                Ok(v) => break v,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break (None, true),
            }
        };
        done_flag.store(true, Ordering::SeqCst);
        let _ = poll.join();
        let result = CommandResult {
            argv: args.iter().map(|a| a.to_string()).collect(),
            code,
            stdout: stdout.lock().unwrap().clone(),
            stderr: stderr.lock().unwrap().clone(),
            timed_out: false,
            duration: Duration::ZERO,
        };
        if cancelled || cancel.is_cancelled() {
            return Err(Error::Command { summary: tr("Transfer cancelled").into(), code });
        }
        if result.ok() {
            on_progress(Progress { done: total, total });
            Ok(result)
        } else {
            Err(Error::Command { summary: permission_hint(&result.combined()), code })
        }
    }

    // -- logcat -----------------------------------------------------------------

    pub fn logcat_dump(&self, serial: &str, lines: u32) -> Result<Vec<LogEntry>> {
        let r = self.run(Some(serial), &["logcat", "-d", "-v", "threadtime", "-t", &lines.to_string()], NORMAL)?;
        Ok(parsers::parse_logcat_text(&r.stdout))
    }

    pub fn logcat_clear(&self, serial: &str) -> Result<CommandResult> {
        self.run(Some(serial), &["logcat", "-c"], SHORT)
    }

    /// Streaming `adb logcat -v threadtime`. `buffers` e.g. `main,system,crash`.
    pub fn logcat_stream<F>(&self, serial: &str, buffers: &str, on_event: F) -> Result<StreamHandle>
    where
        F: Fn(StreamEvent) + Send + Sync + 'static,
    {
        let buffers = if buffers.split(',').all(|b| ["main", "system", "crash", "events", "radio", "kernel", "all", "default"].contains(&b.trim())) { buffers } else { "default" };
        self.stream(Some(serial), &["logcat", "-v", "threadtime", "-b", buffers], false, on_event)
    }

    /// Sends a ZIP through `adb sideload` (the device must be in sideload mode).
    pub fn sideload<F>(&self, serial: &str, zip: &Path, on_event: F) -> Result<StreamHandle>
    where
        F: Fn(StreamEvent) + Send + Sync + 'static,
    {
        if !zip.is_file() {
            return Err(Error::Invalid(t!("{} does not exist", zip.display())));
        }
        self.stream(Some(serial), &["sideload", &zip.display().to_string()], false, on_event)
    }

    pub fn quote(arg: &str) -> String {
        shell_quote(arg)
    }
}

fn remote_parent_if_apk(path: &str) -> String {
    if path.ends_with(".apk") {
        remote_parent(path)
    } else {
        path.to_string()
    }
}

fn pm_result(r: CommandResult) -> Result<CommandResult> {
    if r.combined().contains("Success") {
        Ok(r)
    } else {
        Err(Error::Command { summary: r.summary(), code: r.code })
    }
}

fn cache_result(outcome: crate::cleanup::CacheOutcome, code: Option<i32>) -> Result<Option<i64>> {
    use crate::cleanup::CacheOutcome;
    match outcome {
        CacheOutcome::Freed(kib) => Ok(kib),
        CacheOutcome::Unsupported => Err(Error::Command { summary: tr("this Android version cannot clear the cache of a single app through ADB").into(), code }),
        CacheOutcome::Failed(line) => Err(Error::Command { summary: line, code }),
    }
}

pub fn validate_host_port(address: &str) -> Result<String> {
    let a = address.trim();
    let (host, port) = a.rsplit_once(':').ok_or_else(|| Error::Invalid(tr("the address must have the IP:port format").into()))?;
    let port: u16 = port.parse().map_err(|_| Error::Invalid(tr("invalid port").into()))?;
    let host_ok = !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '[' || c == ']' || c == ':');
    if !host_ok || port == 0 {
        return Err(Error::Invalid(tr("invalid address").into()));
    }
    Ok(format!("{host}:{port}"))
}

/// Translates common `adb install` errors into clear messages.
pub fn install_error_hint(output: &str) -> String {
    const HINTS: [(&str, &str); 10] = [
        ("INSTALL_FAILED_VERSION_DOWNGRADE", "The installed version is newer. Turn on “Allow downgrade”."),
        ("INSTALL_FAILED_UPDATE_INCOMPATIBLE", "The signature does not match the installed app. Uninstall it first."),
        ("INSTALL_FAILED_ALREADY_EXISTS", "The app is already installed. Turn on “Replace”."),
        ("INSTALL_FAILED_INSUFFICIENT_STORAGE", "There is not enough space on the device."),
        ("INSTALL_FAILED_OLDER_SDK", "The APK requires a newer Android version."),
        ("INSTALL_FAILED_NO_MATCHING_ABIS", "The APK is not compatible with the processor architecture."),
        ("INSTALL_FAILED_TEST_ONLY", "It is a test APK. Turn on “Allow test APKs”."),
        ("INSTALL_PARSE_FAILED", "The APK file is damaged or invalid."),
        ("INSTALL_FAILED_USER_RESTRICTED", "The device blocked the installation. Check “Install via USB” in developer options."),
        ("INSTALL_FAILED_VERIFICATION_FAILURE", "Play Protect or another verification rejected the APK."),
    ];
    for (code, hint) in HINTS {
        if output.contains(code) {
            return format!("{} ({code})", tr(hint));
        }
    }
    output.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(tr("Unknown error")).trim().to_string()
}

/// Explains why Android blocks a path.
pub fn permission_hint(output: &str) -> String {
    let lower = output.to_lowercase();
    if lower.contains("permission denied") {
        tr("Android does not allow access to this path with the shell user (without root).").into()
    } else if lower.contains("no such file") {
        tr("The path does not exist on the device.").into()
    } else if lower.contains("read-only file system") {
        tr("The file system is read-only.").into()
    } else {
        output.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(tr("Unknown error")).trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packages_validation() {
        assert!(is_valid_package_name("com.android.settings"));
        assert!(is_valid_package_name("com.example_app.v2"));
        assert!(!is_valid_package_name("settings"));
        assert!(!is_valid_package_name("com.x;rm -rf /"));
        assert!(!is_valid_package_name("com..x"));
        assert!(!is_valid_package_name("com.1x"));
    }

    #[test]
    fn remote_paths() {
        assert!(is_protected_remote_path("/sdcard/"));
        assert!(is_protected_remote_path("/"));
        assert!(!is_protected_remote_path("/sdcard/Download"));
        assert!(validate_remote_path("relative").is_err());
        assert_eq!(remote_join("/sdcard", "a b"), "/sdcard/a b");
        assert_eq!(remote_join("/", "x"), "/x");
        assert_eq!(remote_parent("/sdcard/Download/"), "/sdcard");
        assert_eq!(remote_parent("/sdcard"), "/");
    }

    #[test]
    fn host_port() {
        assert_eq!(validate_host_port(" 192.168.1.5:5555 ").unwrap(), "192.168.1.5:5555");
        assert!(validate_host_port("192.168.1.5").is_err());
        assert!(validate_host_port("1.2.3.4:0").is_err());
        assert!(validate_host_port("a b:5555").is_err());
        assert!(validate_host_port("1.2.3.4:5555;rm").is_err());
    }

    #[test]
    fn hints() {
        assert!(install_error_hint("Failure [INSTALL_FAILED_VERSION_DOWNGRADE]").contains("downgrade"));
        assert!(permission_hint("ls: /data: Permission denied").contains("without root"));
    }
}
