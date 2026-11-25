//! Persistent sub-second telemetry.
//!
//! A single `adb shell` runs a lightweight loop on the device and prints a
//! complete block every `interval_ms`. The host does not start processes per
//! tick: it only reads the lines ADB is already forwarding.
//!
//! Static data (properties, CPU, resolution, density) is sent every
//! `static_every` ticks and kept in an accumulator between blocks.

use crate::{i18n::tr, t};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::adb::Adb;
use crate::exec::{StreamEvent, StreamHandle};
use crate::models::*;
use crate::parsers::{self, StorageKind};

pub const DEFAULT_INTERVAL_MS: u32 = 500;
pub const MIN_INTERVAL_MS: u32 = 250;
pub const MAX_INTERVAL_MS: u32 = 5000;
pub const BLOCK_END: &str = "@@@END@@@";
const MAX_BUFFER_LINES: usize = 20_000;

pub fn clamp_interval(ms: u32) -> u32 {
    ms.clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS)
}

/// Remote script. Only integers are interpolated, so it never contains user
/// text.
pub fn build_live_script(interval_ms: u32, static_every: u32) -> String {
    let interval = clamp_interval(interval_ms);
    let static_every = static_every.max(1);
    format!(
        r#"
TARGET_MS={interval}
STATIC_EVERY={static_every}
i=0
while true; do
  t0=$(date +%s%3N)
  echo '@@@TICK@@@'
  echo "$t0"
  echo '@@@BATTERY@@@'
  dumpsys battery
  echo '@@@MEM@@@'
  cat /proc/meminfo
  echo '@@@DF@@@'
  df -k /data /storage/emulated/0
  echo '@@@THERMAL@@@'
  dumpsys thermalservice | grep -E "Thermal Status:|temperatures from HAL:|Cached temperatures:|Temperature"
  echo '@@@STAT@@@'
  head -n 1 /proc/stat
  echo '@@@LOAD@@@'
  cat /proc/loadavg
  echo '@@@UPTIME@@@'
  cat /proc/uptime
  echo '@@@USB@@@'
  dumpsys usb | grep -E "^[[:space:]]*(connected|configured|usb_charging|host_connected|kernel_state)="
  echo '@@@USBSTATE@@@'
  getprop sys.usb.state
  getprop sys.usb.config
  echo '@@@BRIGHTNESS@@@'
  settings get system screen_brightness
  echo '@@@FREQ@@@'
  paste /sys/devices/system/cpu/cpu*/cpufreq/scaling_cur_freq \
        /sys/devices/system/cpu/cpu*/cpufreq/cpuinfo_max_freq \
        /sys/devices/system/cpu/cpu*/cpufreq/cpuinfo_min_freq \
        /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor 2>/dev/null
  if [ $((i % 2)) -eq 0 ]; then
    echo '@@@WIFI@@@'
    cmd wifi status 2>/dev/null | head -6
  fi
  if [ $((i % 4)) -eq 0 ]; then
    echo '@@@REFRESH@@@'
    dumpsys display | grep -m2 -o -E 'mActiveRenderFrameRate=[0-9.]+|mScreenState=[A-Z_]+'
  fi
  echo '@@@IPADDR@@@'
  ip -o addr 2>/dev/null
  ip -o link 2>/dev/null
  echo '@@@IPROUTE@@@'
  ip route 2>/dev/null
  if [ $((i % STATIC_EVERY)) -eq 0 ]; then
    echo '@@@PROPS@@@'
    getprop
    echo '@@@CPUINFO@@@'
    cat /proc/cpuinfo
    echo '@@@SIZE@@@'
    wm size
    echo '@@@DENSITY@@@'
    wm density
    echo '@@@DISPLAY@@@'
    dumpsys display | sed -n '/mBaseDisplayInfo/p'
    echo '@@@REFRESHCFG@@@'
    settings get system peak_refresh_rate
    settings get system min_refresh_rate
    settings get system refresh_rate_mode
    settings get system all_refresh_rate
    echo '@@@WEAR@@@'
    for f in cycle_count charge_full charge_full_design; do
      echo "batt_$f=$(cat /sys/class/power_supply/battery/$f 2>/dev/null)"
    done
    found=0
    for d in /sys/bus/platform/devices/*/health_descriptor; do
      case $d in *'*'*) break;; esac
      echo "kind=UFS"
      echo "life=$(cat $d/life_time_estimation_a 2>/dev/null) $(cat $d/life_time_estimation_b 2>/dev/null)"
      echo "eol=$(cat $d/eol_info 2>/dev/null)"
      found=1
      break
    done
    if [ "$found" -eq 0 ]; then
      for d in /sys/class/mmc_host/mmc0/mmc0:*; do
        case $d in *'*'*) break;; esac
        ls "$d" 2>/dev/null | grep -qx life_time || continue
        echo "kind=eMMC"
        echo "life=$(cat $d/life_time 2>/dev/null)"
        echo "eol=$(cat $d/pre_eol_info 2>/dev/null)"
        break
      done
    fi
  fi
  echo '@@@END@@@'
  elapsed=$(( $(date +%s%3N) - t0 ))
  wait_ms=$((TARGET_MS - elapsed))
  if [ "$wait_ms" -gt 20 ]; then
    sleep "$(printf '%d.%03d' $((wait_ms / 1000)) $((wait_ms % 1000)))"
  fi
  i=$((i + 1))
done
"#
    )
}

/// Turns the `paste` row (4 × cores columns) into `###COREn` blocks.
pub fn parse_freq_columns(text: &str) -> String {
    let values: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).flat_map(|l| l.split('\t').map(str::trim)).collect();
    if values.is_empty() || values.len() % 4 != 0 {
        return String::new();
    }
    let n = values.len() / 4;
    (0..n).map(|i| format!("###CORE{i}\ncur={}\nmax={}\nmin={}\ngov={}", values[i], values[n + i], values[2 * n + i], values[3 * n + i])).collect::<Vec<_>>().join("\n")
}

/// State that survives between blocks of the same stream.
#[derive(Debug, Default, Clone)]
pub struct LiveAccumulator {
    pub props: std::collections::BTreeMap<String, String>,
    pub static_cpu: CpuInfo,
    pub display: DisplayInfo,
    /// Last live value (every 2 s): active rate and screen state.
    pub refresh_live: (Option<f64>, String),
    pub refresh_setting: String,
    pub wifi: Option<WifiStatus>,
    pub health: ComponentHealth,
    pub previous_cpu_times: Option<(i64, i64)>,
    pub last_tick: f64,
    pub blocks: u64,
}

fn build_cpu(static_cpu: &CpuInfo, props: &std::collections::BTreeMap<String, String>, freq_text: &str) -> CpuInfo {
    let mut cpu = static_cpu.clone();
    cpu.cores.clear();
    if let Some(abi) = props.get("ro.product.cpu.abi").filter(|v| !v.is_empty()) {
        cpu.abi = abi.clone();
    }
    if cpu.model_name.is_empty() {
        cpu.model_name = props.get("ro.board.platform").or_else(|| props.get("ro.hardware")).cloned().unwrap_or_default();
    }
    if cpu.hardware.is_empty() {
        cpu.hardware = props.get("ro.hardware").cloned().unwrap_or_default();
    }
    parsers::apply_cpu_freqs(&mut cpu, &parse_freq_columns(freq_text));
    if cpu.core_count.is_none() && !cpu.cores.is_empty() {
        cpu.core_count = Some(cpu.cores.len());
    }
    cpu
}

/// Turns a block into a [`Snapshot`]. Pure with respect to the interface.
pub fn build_snapshot(serial: &str, sections: &std::collections::HashMap<String, String>, acc: &mut LiveAccumulator) -> Snapshot {
    let get = |k: &str| sections.get(k).map(String::as_str).unwrap_or("");
    let mut snap = Snapshot::new(serial);

    if sections.contains_key("PROPS") {
        let props = parsers::parse_getprop(get("PROPS"));
        if !props.is_empty() {
            acc.props = props;
        }
    }
    if sections.contains_key("CPUINFO") {
        let info = parsers::parse_cpuinfo(get("CPUINFO"));
        if info.core_count.is_some() || !info.hardware.is_empty() || !info.model_name.is_empty() {
            acc.static_cpu = info;
        }
    }
    if ["SIZE", "DENSITY", "DISPLAY"].iter().any(|k| sections.contains_key(*k)) {
        acc.display = parsers::parse_display_info(get("SIZE"), get("DENSITY"), get("DISPLAY"));
    }
    snap.props = acc.props.clone();
    snap.battery = parsers::parse_battery(get("BATTERY"));
    snap.memory = parsers::parse_meminfo(get("MEM"));
    let df = parsers::parse_df(get("DF"));
    snap.data_storage = parsers::select_storage(&df, StorageKind::Data);
    snap.external_storage = parsers::select_storage(&df, StorageKind::Shared);
    let (temps, status) = parsers::parse_thermalservice(get("THERMAL"));
    snap.temperatures = temps;
    snap.thermal_status = status;

    snap.usb = parsers::parse_usb_dump(get("USB"));
    let usb_lines: Vec<&str> = get("USBSTATE").lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if !usb_lines.is_empty() {
        let props = parsers::parse_usb_props(usb_lines[0], usb_lines.get(1).copied().unwrap_or(""), "");
        snap.usb.functions = props.functions;
        snap.usb.config = props.config;
        if snap.usb.connected.is_none() {
            snap.usb.connected = props.connected;
        }
    }
    if sections.contains_key("WIFI") {
        let text = get("WIFI");
        // Without `cmd wifi` (Android < 11) the block arrives empty: it is not
        // used.
        acc.wifi = (text.contains("Wifi is") || text.contains("Wi-Fi is")).then(|| parsers::parse_wifi_status(text));
    }
    snap.wifi = acc.wifi.clone();
    if sections.contains_key("WEAR") {
        acc.health = parsers::parse_wear(get("WEAR"));
    }
    snap.health = acc.health.clone();
    if sections.contains_key("REFRESH") {
        acc.refresh_live = parsers::parse_refresh_live(get("REFRESH"));
    }
    if sections.contains_key("REFRESHCFG") {
        acc.refresh_setting = parsers::parse_refresh_setting(get("REFRESHCFG"));
    }
    snap.display = acc.display.clone();
    if let Some(hz) = acc.refresh_live.0 {
        snap.display.refresh_hz = Some(hz);
    }
    snap.display.screen_state = acc.refresh_live.1.clone();
    snap.display.refresh_setting = acc.refresh_setting.clone();
    snap.display.brightness = parsers::parse_brightness(get("BRIGHTNESS"));
    snap.uptime_seconds = parsers::parse_uptime(get("UPTIME"));
    snap.cpu = build_cpu(&acc.static_cpu, &snap.props, get("FREQ"));
    snap.cpu.times = parsers::parse_proc_stat(get("STAT"));
    snap.cpu.load_percent = parsers::compute_cpu_load(acc.previous_cpu_times, snap.cpu.times);
    if snap.cpu.times.is_some() {
        acc.previous_cpu_times = snap.cpu.times;
    }
    snap.cpu.loadavg = parsers::parse_loadavg(get("LOAD"));
    snap.network = NetworkInfo { interfaces: parsers::parse_ip_addr(get("IPADDR")), routes: parsers::parse_ip_route(get("IPROUTE")), timestamp: snap.timestamp };
    if let Some(first) = get("TICK").trim().lines().next() {
        if let Ok(ms) = first.trim().parse::<i64>() {
            acc.last_tick = ms as f64 / 1000.0;
        }
    }
    acc.blocks += 1;
    snap
}

// ---------------------------------------------------------------------------
// Stream with watchdog and retries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum LiveEvent {
    Snapshot(Box<Snapshot>),
    /// Message for the interface (recoverable errors, retries).
    Notice(String),
    Stopped,
}

struct Shared {
    buffer: Mutex<Vec<String>>,
    acc: Mutex<LiveAccumulator>,
    last_data_ms: AtomicU64,
    generation: AtomicU64,
    stopping: AtomicBool,
    epoch: Instant,
}

impl Shared {
    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}

/// A persistent `adb shell` process that produces snapshots. Supervised by a
/// thread with a watchdog (4 s without data) and an increasing backoff (1–8 s).
pub struct LiveStream {
    shared: Arc<Shared>,
    supervisor: Option<thread::JoinHandle<()>>,
}

impl LiveStream {
    pub fn start<F>(adb: Adb, serial: String, interval_ms: u32, on_event: F) -> Self
    where
        F: Fn(LiveEvent) + Send + Sync + 'static,
    {
        let shared = Arc::new(Shared {
            buffer: Mutex::new(Vec::new()),
            acc: Mutex::new(LiveAccumulator::default()),
            last_data_ms: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            stopping: AtomicBool::new(false),
            epoch: Instant::now(),
        });
        let on_event = Arc::new(on_event);
        let sup_shared = shared.clone();
        let interval = clamp_interval(interval_ms);
        let supervisor = thread::Builder::new().name(format!("live-{serial}")).spawn(move || supervise(adb, serial, interval, sup_shared, on_event)).expect("telemetry thread");
        Self { shared, supervisor: Some(supervisor) }
    }

    pub fn blocks(&self) -> u64 {
        self.shared.acc.lock().unwrap().blocks
    }

    pub fn stop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        if let Some(h) = self.supervisor.take() {
            let _ = h.join();
        }
    }
}

impl Drop for LiveStream {
    fn drop(&mut self) {
        self.stop();
    }
}

fn supervise(adb: Adb, serial: String, interval: u32, shared: Arc<Shared>, on_event: Arc<dyn Fn(LiveEvent) + Send + Sync>) {
    let mut retry_delay = 1u64;
    let script = build_live_script(interval, (10_000 / interval).max(1));
    while !shared.stopping.load(Ordering::SeqCst) {
        let generation = shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        shared.buffer.lock().unwrap().clear();
        shared.last_data_ms.store(shared.now_ms(), Ordering::SeqCst);
        let exited = Arc::new(AtomicBool::new(false));
        let handle: Option<StreamHandle> = {
            let shared = shared.clone();
            let cb = on_event.clone();
            let serial2 = serial.clone();
            let exited2 = exited.clone();
            adb.shell_stream(&serial, &script, move |ev| {
                if shared.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                match ev {
                    StreamEvent::Stdout(line) => on_line(&shared, &serial2, line, &*cb),
                    StreamEvent::Stderr(l) => log::debug!("live {serial2}: {l}"),
                    StreamEvent::Exit { .. } => exited2.store(true, Ordering::SeqCst),
                }
            })
            .map_err(|e| on_event(LiveEvent::Notice(t!("Could not start the real-time reading: {}", e))))
            .ok()
        };
        let mut healthy_since: Option<Instant> = None;
        if let Some(handle) = handle {
            loop {
                thread::sleep(Duration::from_millis(100));
                if shared.stopping.load(Ordering::SeqCst) {
                    handle.cancel();
                    on_event(LiveEvent::Stopped);
                    return;
                }
                if exited.load(Ordering::SeqCst) {
                    on_event(LiveEvent::Notice(tr("The real-time reading stopped. Retrying…").into()));
                    break;
                }
                let idle = shared.now_ms().saturating_sub(shared.last_data_ms.load(Ordering::SeqCst));
                if idle > 4000 {
                    on_event(LiveEvent::Notice(tr("The real-time reading is not responding. Restarting…").into()));
                    handle.cancel();
                    break;
                }
                if idle < 2000 && healthy_since.is_none() {
                    healthy_since = Some(Instant::now());
                }
            }
        }
        if healthy_since.is_some_and(|t| t.elapsed() > Duration::from_secs(10)) {
            retry_delay = 1;
        }
        let wait = Duration::from_secs(retry_delay.clamp(1, 8));
        retry_delay = (retry_delay * 2).min(8);
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if shared.stopping.load(Ordering::SeqCst) {
                on_event(LiveEvent::Stopped);
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    on_event(LiveEvent::Stopped);
}

fn on_line(shared: &Shared, serial: &str, line: String, cb: &(dyn Fn(LiveEvent) + Send + Sync)) {
    if shared.stopping.load(Ordering::SeqCst) {
        return;
    }
    if line == BLOCK_END {
        shared.last_data_ms.store(shared.now_ms(), Ordering::SeqCst);
        let block = std::mem::take(&mut *shared.buffer.lock().unwrap()).join("\n");
        if block.trim().is_empty() {
            return;
        }
        let sections = parsers::split_sections(&block);
        let snap = build_snapshot(serial, &sections, &mut shared.acc.lock().unwrap());
        cb(LiveEvent::Snapshot(Box::new(snap)));
        return;
    }
    let mut buf = shared.buffer.lock().unwrap();
    buf.push(line);
    if buf.len() > MAX_BUFFER_LINES {
        let cut = buf.len() - 1000;
        let keep = buf.split_off(cut);
        *buf = keep;
    }
}
