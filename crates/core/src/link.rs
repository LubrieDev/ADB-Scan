//! The physical link between the PC and the phone: the cable, the PC's USB
//! port, the phone's port, the power the PC offers and how stable the
//! connection is. Read-only.
//!
//! **PC side** (sysfs, without ADB): negotiated speed and lanes, the port it
//! hangs from and that port's limit, the hubs in between, the controller, the
//! port's location according to ACPI and, when the PC has a Type-C/PD
//! controller exposed by Linux (`/sys/class/typec`, UCSI), the cable's own
//! chip (e-marker): passive or active, copper or optical, rated speed and
//! current.
//!
//! **Phone side** (`dumpsys usb`, `dumpsys battery`, properties): roles, port
//! modes, moisture detection, compliance warnings, the current the PC offers
//! and the USB event log, where repeated cuts reveal a loose contact. Android
//! keeps the phone's own Type-C data (`/sys/class/typec`) behind SELinux.
//!
//! **Deductions** ([`assess`]): what can be told about the cable without its
//! chip (USB-C on both ends when the PC offers 3 A or two lanes, USB 2.0-only
//! wires when a phone that reached USB 3 before now links at 480 Mb/s) and
//! what limits the speed. They are marked as probable, not as facts.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::i18n::tr;
use crate::t;
use crate::usb::UsbDevice;

fn read(path: &Path) -> String {
    fs::read_to_string(path).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn read_num<T: std::str::FromStr>(path: &Path) -> Option<T> {
    read(path).split_whitespace().next()?.parse().ok()
}

fn hex(text: &str) -> Option<u32> {
    let t = text.trim();
    u32::from_str_radix(t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t), 16).ok()
}

/// Value in brackets of a sysfs choice (`[source] sink` → `source`), or the
/// whole text.
fn chosen(text: &str) -> String {
    match (text.find('['), text.find(']')) {
        (Some(a), Some(b)) if b > a => text[a + 1..b].to_string(),
        _ => text.trim().to_string(),
    }
}

// ---------------------------------------------------------------------------
// PC side
// ---------------------------------------------------------------------------

/// Where the port is on the PC, according to ACPI (`physical_location`).
/// Firmware tables are not always right: shown as the PC describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortLocation {
    pub panel: String,
    pub horizontal: String,
    pub vertical: String,
    pub dock: bool,
    pub lid: bool,
}

impl PortLocation {
    pub fn label(&self) -> String {
        let side = |s: &str| match s {
            "left" => tr("left"),
            "right" => tr("right"),
            "center" => tr("centre"),
            "top" | "upper" => tr("top"),
            "bottom" | "lower" => tr("bottom"),
            "front" => tr("front"),
            "back" => tr("back"),
            _ => "",
        };
        let mut parts = Vec::new();
        if !self.panel.is_empty() && self.panel != "unknown" {
            parts.push(t!("{} panel", side(&self.panel)));
        }
        if !side(&self.horizontal).is_empty() {
            parts.push(t!("{} side", side(&self.horizontal)));
        }
        if self.dock {
            parts.push(tr("on the dock").into());
        }
        if self.lid {
            parts.push(tr("on the lid").into());
        }
        parts.join(" · ")
    }
}

/// The PC's USB controller.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Controller {
    /// PCI address (`0000:00:14.0`).
    pub address: String,
    /// Name from the PCI database (`Intel Corporation Comet Lake USB 3.1 xHCI
    /// Host Controller`), or the ids if it is not there.
    pub name: String,
    /// Kernel driver (`xhci_hcd`).
    pub driver: String,
}

/// Connection data of the PC's USB-C port (Linux `typec` class).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TypeCPort {
    /// `default`, `1.5A`, `3.0A` or `usb_power_delivery`.
    pub power_mode: String,
    pub pd_revision: String,
    /// `normal` or `reverse` (which way round the plug went in).
    pub orientation: String,
    pub data_role: String,
    pub power_role: String,
    pub cable: Option<CableChip>,
}

/// What the cable's e-marker chip declares (USB PD Discover Identity).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CableChip {
    /// `active` or `passive`, as Linux reports it.
    pub kind: String,
    /// `type-c`, `captive`…
    pub plug: String,
    pub vendor_id: Option<u16>,
    pub speed: Option<&'static str>,
    pub current_a: Option<u8>,
    pub max_voltage_v: Option<u8>,
    /// Length estimated from the declared signal latency.
    pub length: Option<&'static str>,
    /// Only active cables say it: copper or optical fibre.
    pub optical: Option<bool>,
}

/// Decodes the cable VDOs (USB PD 3.x). `vdo2` exists only for active
/// cables; there bit 10 says copper (0) or optical (1).
pub fn decode_cable(active: bool, vdo1: u32, vdo2: Option<u32>) -> CableChip {
    let speed = match vdo1 & 0b111 {
        0 => Some("USB 2.0"),
        1 => Some("USB 3.2 Gen 1 · 5 Gb/s"),
        2 => Some("USB 3.2 / USB4 Gen 2 · 10–20 Gb/s"),
        3 => Some("USB4 Gen 3 · 40 Gb/s"),
        4 => Some("USB4 Gen 4 · 80 Gb/s"),
        _ => None,
    };
    let current_a = match (vdo1 >> 5) & 0b11 {
        1 => Some(3),
        2 => Some(5),
        _ => None,
    };
    let max_voltage_v = Some([20, 30, 40, 50][((vdo1 >> 9) & 0b11) as usize]);
    let length = match (vdo1 >> 13) & 0b1111 {
        1 => Some("~1 m"),
        2 => Some("~2 m"),
        3 => Some("~3 m"),
        4 => Some("~4 m"),
        5 => Some("~5 m"),
        6 => Some("~6 m"),
        7 => Some("~7 m"),
        8 => Some("> 7 m"),
        9 if active => Some("~100 m"),
        10 if active => Some("~200 m"),
        11 if active => Some("~300 m"),
        _ => None,
    };
    CableChip {
        kind: if active { "active".into() } else { "passive".into() },
        speed,
        current_a,
        max_voltage_v,
        length,
        optical: if active { vdo2.map(|v| v & (1 << 10) != 0) } else { Some(false) },
        ..Default::default()
    }
}

/// The phone's link as the PC sees it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PcLink {
    pub speed_mbps: Option<u32>,
    /// USB version the phone declares (`3.20`).
    pub version: String,
    pub rx_lanes: Option<u8>,
    pub tx_lanes: Option<u8>,
    /// Power the phone asks the port for (mA).
    pub max_power_ma: Option<u32>,
    /// Hubs between the PC's port and the phone, nearest to the PC first.
    pub hubs: Vec<String>,
    pub bus: String,
    /// The PC's own port (the first one, before any hub).
    pub root_port: Option<u32>,
    /// Fastest speed of that port (Mb/s): its bus or its USB 3 companion.
    pub port_max_mbps: Option<u32>,
    /// The port also has USB 3 wiring (a SuperSpeed companion).
    pub port_superspeed: bool,
    pub location: Option<PortLocation>,
    /// `hotplug` (a port you plug into), `hardwired` (internal)…
    pub connect_type: String,
    pub over_current: Option<u32>,
    /// Link power saving allowed by the port (`u1_u2`, `u1`, `u2`, `0`).
    pub lpm: String,
    pub controller: Option<Controller>,
    pub typec: Option<TypeCPort>,
}

impl PcLink {
    pub fn lanes(&self) -> u8 {
        self.rx_lanes.unwrap_or(1).max(self.tx_lanes.unwrap_or(1))
    }
}

pub const PCI_IDS: [&str; 2] = ["/usr/share/misc/pci.ids", "/usr/share/hwdata/pci.ids"];

/// Name of a PCI device in a `pci.ids` database.
pub fn pci_name(ids: &str, vendor: u16, device: u16) -> Option<String> {
    let (v, d) = (format!("{vendor:04x}"), format!("{device:04x}"));
    let mut vendor_name = None;
    for line in ids.lines() {
        if let Some(name) = &vendor_name {
            if !line.starts_with('\t') && !line.starts_with('#') && !line.is_empty() {
                return Some(String::clone(name));
            }
            if let Some(rest) = line.strip_prefix('\t').and_then(|l| l.strip_prefix(&d)) {
                if !line.starts_with("\t\t") {
                    return Some(format!("{name} {}", rest.trim()));
                }
            }
        } else if let Some(rest) = line.strip_prefix(&v) {
            if rest.starts_with(' ') {
                vendor_name = Some(rest.trim().to_string());
            }
        }
    }
    vendor_name
}

fn port_location(dir: &Path) -> Option<PortLocation> {
    let p = dir.join("physical_location");
    if !p.is_dir() {
        return None;
    }
    Some(PortLocation {
        panel: read(&p.join("panel")),
        horizontal: read(&p.join("horizontal_position")),
        vertical: read(&p.join("vertical_position")),
        dock: read(&p.join("dock")) == "yes",
        lid: read(&p.join("lid")) == "yes",
    })
}

fn typec_port(connector: &Path) -> TypeCPort {
    let mut port = TypeCPort {
        power_mode: read(&connector.join("power_operation_mode")),
        pd_revision: read(&connector.join("usb_power_delivery_revision")),
        orientation: read(&connector.join("orientation")),
        data_role: chosen(&read(&connector.join("data_role"))),
        power_role: chosen(&read(&connector.join("power_role"))),
        cable: None,
    };
    // The cable is a sibling device: `port0` → `port0-cable`.
    if let (Some(parent), Some(name)) = (connector.parent(), connector.file_name()) {
        let cable = parent.join(format!("{}-cable", name.to_string_lossy()));
        if cable.is_dir() {
            let kind = read(&cable.join("type"));
            let id = cable.join("identity");
            let header = hex(&read(&id.join("id_header")));
            let vdo1 = hex(&read(&id.join("product_type_vdo1")));
            let vdo2 = hex(&read(&id.join("product_type_vdo2")));
            let active = kind == "active";
            let mut chip = match vdo1.filter(|v| *v != 0) {
                Some(v) => decode_cable(active, v, vdo2),
                None => CableChip { kind: kind.clone(), ..Default::default() },
            };
            chip.kind = kind;
            chip.plug = read(&cable.join("plug_type"));
            chip.vendor_id = header.map(|h| (h & 0xffff) as u16).filter(|v| *v != 0);
            port.cable = Some(chip);
        }
    }
    port
}

/// Reads the link of `dev` from sysfs. `root` is `/sys/bus/usb/devices`;
/// `pci_ids` the PCI name database, if there is one.
pub fn pc_link(root: &Path, dev: &UsbDevice, pci_ids: Option<&str>) -> PcLink {
    let dir = root.join(&dev.name);
    let (bus, ports) = dev.name.split_once('-').unwrap_or((dev.name.as_str(), ""));
    let ports: Vec<&str> = ports.split('.').collect();
    let mut link = PcLink {
        speed_mbps: read(&dir.join("speed")).parse::<f64>().ok().map(|v| v as u32),
        version: read(&dir.join("version")),
        rx_lanes: read_num(&dir.join("rx_lanes")),
        tx_lanes: read_num(&dir.join("tx_lanes")),
        max_power_ma: read(&dir.join("bMaxPower")).trim_end_matches("mA").trim().parse().ok(),
        bus: bus.to_string(),
        root_port: ports.first().and_then(|p| p.parse().ok()),
        ..Default::default()
    };
    for i in 1..ports.len() {
        let hub = root.join(format!("{bus}-{}", ports[..i].join(".")));
        let name = [read(&hub.join("manufacturer")), read(&hub.join("product"))].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
        let id = format!("{}:{}", read(&hub.join("idVendor")), read(&hub.join("idProduct")));
        link.hubs.push(if name.is_empty() { id } else { name });
    }
    let root_hub = root.join(format!("usb{bus}"));
    let mut port_max = read(&root_hub.join("speed")).parse::<f64>().ok().map(|v| v as u32);
    if let Some(p) = link.root_port {
        let port = root.join(format!("{bus}-0:1.0")).join(format!("usb{bus}-port{p}"));
        link.connect_type = read(&port.join("connect_type"));
        link.over_current = read_num(&port.join("over_current_count"));
        link.lpm = read(&port.join("usb3_lpm_permit"));
        link.location = port_location(&port);
        // The USB 3 and USB 2 halves of one connector are “peers”.
        if let Ok(peer) = fs::canonicalize(port.join("peer")) {
            if let Some(peer_hub) = peer.parent().and_then(Path::parent) {
                let peer_speed = read(&peer_hub.join("speed")).parse::<f64>().ok().map(|v| v as u32);
                port_max = port_max.max(peer_speed);
                if link.location.is_none() {
                    link.location = port_location(&peer);
                }
            }
        }
        if let Ok(connector) = fs::canonicalize(port.join("connector")) {
            link.typec = Some(typec_port(&connector));
        }
    }
    link.port_superspeed = port_max.is_some_and(|m| m >= 5000);
    link.port_max_mbps = port_max;
    // The controller is the PCI device the root hub hangs from.
    if let Some(pci) = fs::canonicalize(&root_hub).ok().and_then(|p| p.parent().map(PathBuf::from)) {
        let (vendor, device) = (hex(&read(&pci.join("vendor"))), hex(&read(&pci.join("device"))));
        if let (Some(v), Some(d)) = (vendor, device) {
            let name = pci_ids.and_then(|ids| pci_name(ids, v as u16, d as u16)).unwrap_or_else(|| format!("{v:04x}:{d:04x}"));
            let driver = fs::read_link(pci.join("driver")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
            link.controller = Some(Controller { address: pci.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), name, driver });
        }
    }
    link
}

/// [`pc_link`] on this PC.
#[cfg(target_os = "linux")]
pub fn read_pc_link(dev: &UsbDevice) -> PcLink {
    let ids = PCI_IDS.iter().find_map(|p| fs::read_to_string(p).ok());
    pc_link(Path::new(crate::usb::SYSFS_USB), dev, ids.as_deref())
}

/// Without sysfs the PC side of the link is unknown (see [`crate::usb::HOST_USB`]).
#[cfg(not(target_os = "linux"))]
pub fn read_pc_link(_dev: &UsbDevice) -> PcLink {
    PcLink::default()
}

// ---------------------------------------------------------------------------
// Phone side
// ---------------------------------------------------------------------------

/// Read-only query of the phone's USB state.
pub const PHONE_SCRIPT: &str = "echo @@@USB@@@; dumpsys usb 2>&1
echo @@@BATTERY@@@; dumpsys battery 2>&1
echo @@@PROPS@@@; for p in sys.usb.controller ro.boot.usbcontroller sys.usb.config sys.usb.state persist.sys.usb.dp; do echo \"$p=$(getprop $p)\"; done
echo @@@CHARGE@@@; cd /sys/class/power_supply/battery 2>/dev/null && for f in current_now voltage_now status charge_type capacity charge_control_end_threshold; do echo \"$f=$(cat $f 2>/dev/null)\"; done; cd /
echo @@@UPTIME@@@; cat /proc/uptime
echo @@@NOW@@@; date +'%m-%d %H:%M:%S'
echo @@@END@@@
";

/// A USB state change from the phone's own log.
#[derive(Debug, Clone, PartialEq)]
pub struct UsbEvent {
    /// `MM-DD HH:MM:SS` as the phone wrote it.
    pub at: String,
    /// Seconds in a comparable scale (month and day included).
    pub t: f64,
    pub connected: bool,
}

/// What the phone says about its USB port.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhoneLink {
    /// `dual`, `audio_acc`…
    pub port_modes: Vec<String>,
    /// `ufp` (device), `dfp` (host)…
    pub current_mode: String,
    pub power_role: String,
    pub data_role: String,
    /// `not detected`, `detected`, `not supported`…
    pub contaminant: String,
    pub data_status: String,
    pub power_limited: Option<bool>,
    pub supports_compliance: Option<bool>,
    pub compliance: Vec<String>,
    /// Seconds since this cable was plugged in.
    pub connected_for_s: Option<f64>,
    pub hal_version: String,
    pub ac_powered: Option<bool>,
    pub usb_powered: Option<bool>,
    pub max_current_ua: Option<i64>,
    pub max_voltage_uv: Option<i64>,
    pub controller: String,
    pub usb_config: String,
    pub displayport: Option<bool>,
    /// Battery current right now (µA; the sign depends on the phone).
    pub battery_current_ua: Option<i64>,
    pub battery_voltage_uv: Option<i64>,
    /// `Charging`, `Discharging`, `Full`, `Not charging`.
    pub charge_status: String,
    /// `Fast`, `Standard`, `Trickle`…
    pub charge_type: String,
    pub battery_level: Option<i64>,
    /// Charging stops at this % (battery protection), if the phone has it.
    pub charge_limit: Option<i64>,
    pub events: Vec<UsbEvent>,
    /// The phone's clock when it was read (same scale as [`UsbEvent::t`]).
    pub now: Option<f64>,
}

fn event_time(text: &str) -> Option<f64> {
    // `11-24 15:24:23:715` or `11-24 15:24:23`
    let (date, time) = text.trim().split_once(' ')?;
    let (month, day) = date.split_once('-')?;
    let mut hms = time.split(':').map(|p| p.parse::<f64>().ok());
    let (h, m, s) = (hms.next()??, hms.next()??, hms.next()??);
    let ms = hms.next().flatten().unwrap_or(0.0);
    Some((month.parse::<f64>().ok()? * 31.0 + day.parse::<f64>().ok()?) * 86400.0 + h * 3600.0 + m * 60.0 + s + ms / 1000.0)
}

/// `key=value` lines of a section (first value of each key).
fn pairs(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        if let Some((k, v)) = line.trim().split_once(['=', ':']) {
            map.entry(k.trim().to_string()).or_insert_with(|| v.trim().to_string());
        }
    }
    map
}

pub fn parse_phone_link(text: &str) -> PhoneLink {
    let s = crate::parsers::split_sections(text);
    let get = |name: &str| s.get(name).map(String::as_str).unwrap_or("");
    let usb = get("USB");
    let mut link = PhoneLink::default();
    // Only the first port, outside the event log.
    let ports = usb.find("port_manager").map(|i| &usb[i..]).unwrap_or("");
    let kv = pairs(&ports.lines().filter(|l| !l.contains("UEVENT")).collect::<Vec<_>>().join("\n"));
    let val = |k: &str| kv.get(k).cloned().unwrap_or_default();
    link.port_modes = val("supported_modes").split(',').map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect();
    link.current_mode = val("current_mode");
    link.power_role = val("power_role");
    link.data_role = val("data_role");
    link.contaminant = val("contaminant_presence_status");
    link.data_status = val("usb_data_status");
    link.power_limited = kv.get("is_power_transfer_limited").map(|v| v == "true");
    link.supports_compliance = kv.get("supports_compliance_warnings").map(|v| v == "true");
    link.compliance =
        val("compliance_warning_status").trim_matches(|c| c == '[' || c == ']').split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect();
    link.hal_version = val("usb_hal_version");
    let uptime: Option<f64> = get("UPTIME").split_whitespace().next().and_then(|v| v.parse().ok());
    if let (Some(at), Some(up)) = (kv.get("connected_at_millis").and_then(|v| v.parse::<f64>().ok()), uptime) {
        if at > 0.0 && up * 1000.0 >= at {
            link.connected_for_s = Some(up - at / 1000.0);
        }
    }
    for line in usb.lines().filter(|l| l.contains("USB UEVENT")) {
        let at = line.trim().split(" USB UEVENT").next().unwrap_or("").trim();
        let state = line.split("USB_STATE=").nth(1).map(|r| r.split([',', '}']).next().unwrap_or("").trim()).unwrap_or("");
        if let (Some(t), true) = (event_time(at), matches!(state, "CONNECTED" | "DISCONNECTED")) {
            let short = at.rsplit_once(':').map(|(a, _)| a).unwrap_or(at).to_string();
            link.events.push(UsbEvent { at: short, t, connected: state == "CONNECTED" });
        }
    }
    link.now = event_time(get("NOW"));
    let battery = pairs(get("BATTERY"));
    link.ac_powered = battery.get("AC powered").map(|v| v == "true");
    link.usb_powered = battery.get("USB powered").map(|v| v == "true");
    link.max_current_ua = battery.get("Max charging current").and_then(|v| v.parse().ok()).filter(|v| *v > 0);
    link.max_voltage_uv = battery.get("Max charging voltage").and_then(|v| v.parse().ok()).filter(|v| *v > 0);
    let charge = pairs(get("CHARGE"));
    let num = |k: &str| charge.get(k).and_then(|v| v.parse::<i64>().ok());
    link.battery_current_ua = num("current_now");
    link.battery_voltage_uv = num("voltage_now").filter(|v| *v > 0);
    link.charge_status = charge.get("status").cloned().unwrap_or_default();
    link.charge_type = charge.get("charge_type").cloned().unwrap_or_default();
    link.battery_level = num("capacity");
    link.charge_limit = num("charge_control_end_threshold").filter(|l| (1..100).contains(l));
    let props = pairs(get("PROPS"));
    let prop = |k: &str| props.get(k).cloned().unwrap_or_default();
    link.controller = Some(prop("sys.usb.controller")).filter(|c| !c.is_empty()).unwrap_or_else(|| prop("ro.boot.usbcontroller"));
    link.usb_config = prop("sys.usb.config");
    link.displayport = match prop("persist.sys.usb.dp").as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    };
    link
}

/// What the battery is doing right now through the cable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChargeNow {
    /// Taking this power (W) at these amps and volts.
    Charging { watts: f64, amps: f64, volts: f64 },
    /// Stopped at the battery protection limit (%).
    PausedAtLimit(i64),
    Full,
    /// On battery: using this power (W), if it is known.
    Discharging(Option<f64>),
}

/// Below this current (µA) the battery is not really charging.
const IDLE_UA: i64 = 30_000;

pub fn charge_now(ph: &PhoneLink) -> Option<ChargeNow> {
    let volts = ph.battery_voltage_uv.map(|v| v as f64 / 1e6);
    let amps = ph.battery_current_ua.map(|c| c.unsigned_abs() as f64 / 1e6);
    let idle = ph.battery_current_ua.map_or(true, |c| c.abs() < IDLE_UA);
    match ph.charge_status.as_str() {
        "Full" => Some(ChargeNow::Full),
        "Charging" | "Not charging" if idle => match (ph.charge_limit, ph.battery_level) {
            (Some(limit), Some(level)) if level >= limit => Some(ChargeNow::PausedAtLimit(limit)),
            _ if ph.charge_status == "Not charging" => Some(ChargeNow::Discharging(None)),
            _ => None,
        },
        "Charging" => {
            let (a, v) = (amps?, volts?);
            Some(ChargeNow::Charging { watts: a * v, amps: a, volts: v })
        }
        "Discharging" | "Not charging" => Some(ChargeNow::Discharging(amps.zip(volts).map(|(a, v)| a * v))),
        _ => None,
    }
}

/// Cuts in the phone's USB log.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stability {
    pub disconnects: usize,
    /// Most cuts within one minute and when that minute started.
    pub worst_minute: usize,
    pub worst_at: String,
    /// Seconds since that minute, if the phone's clock is known.
    pub worst_ago: Option<f64>,
    /// Last cut and how long ago it was (s), if the phone's clock is known.
    pub last_cut: Option<(String, Option<f64>)>,
}

impl Stability {
    /// The worst minute happened within [`RECENT_SECS`] (or when is
    /// unknown): an older burst no longer describes this connection.
    pub fn is_recent(&self) -> bool {
        self.worst_ago.map_or(true, |a| a <= RECENT_SECS)
    }
}

/// How long a burst of cuts still counts as a current problem.
pub const RECENT_SECS: f64 = 3600.0;

/// From this many cuts in a minute, the connection is unstable.
pub const UNSTABLE_CUTS_PER_MINUTE: usize = 5;

pub fn stability(events: &[UsbEvent], now: Option<f64>) -> Stability {
    let cuts: Vec<&UsbEvent> = events.iter().filter(|e| !e.connected).collect();
    let mut st = Stability { disconnects: cuts.len(), ..Default::default() };
    let mut start = 0;
    for end in 0..cuts.len() {
        while cuts[end].t - cuts[start].t > 60.0 {
            start += 1;
        }
        if end + 1 - start > st.worst_minute {
            st.worst_minute = end + 1 - start;
            st.worst_at = cuts[start].at.clone();
            st.worst_ago = now.map(|n| n - cuts[start].t).filter(|d| *d >= 0.0);
        }
    }
    st.last_cut = cuts.last().map(|c| (c.at.clone(), now.map(|n| n - c.t).filter(|d| *d >= 0.0)));
    st
}

// ---------------------------------------------------------------------------
// Deductions
// ---------------------------------------------------------------------------

/// The two ends of the cable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CableEnds {
    UsbCToC,
    Unknown,
}

/// How sure a deduction is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Certainty {
    /// Read from the hardware.
    Read,
    /// Deduced from other data.
    Probable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CableGuess {
    pub ends: CableEnds,
    pub ends_certainty: Certainty,
    pub ends_reason: String,
    /// The cable carries USB 3 (SuperSpeed) wires: `Some(false)` when it only
    /// has USB 2.0 ones.
    pub superspeed: Option<bool>,
    pub superspeed_reason: String,
}

/// Severity of a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Info,
    Warning,
    Problem,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub level: Level,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assessment {
    pub cable: CableGuess,
    pub findings: Vec<Finding>,
}

pub fn speed_label(mbps: u32) -> String {
    match mbps {
        0..=12 => format!("USB 1.x · {mbps} Mb/s"),
        480 => "USB 2.0 · 480 Mb/s".into(),
        5000 => "USB 3.2 Gen 1 · 5 Gb/s".into(),
        10000 => "USB 3.2 Gen 2 · 10 Gb/s".into(),
        20000 => "USB 3.2 Gen 2×2 · 20 Gb/s".into(),
        n if n >= 40000 => format!("USB4 · {} Gb/s", n / 1000),
        n => format!("{n} Mb/s"),
    }
}

/// Explanation of an Android compliance warning (`compliance_warning_status`).
pub fn compliance_label(code: &str) -> String {
    match code.trim().to_ascii_uppercase().as_str() {
        "OTHER" => tr("A non-compliant cable or charger").into(),
        "DEBUG_ACCESSORY" => tr("A debug accessory is connected").into(),
        "BC_1_2" => tr("An old charger standard (BC 1.2) over USB-C").into(),
        "MISSING_RP" => tr("The cable or charger does not identify itself (missing Rp)").into(),
        "INPUT_POWER_LIMITED" => tr("The charger gives less power than the phone needs").into(),
        "MISSING_DATA_LINES" => tr("The cable has no data wires").into(),
        "ENUMERATION_FAIL" => tr("The PC could not recognise the phone").into(),
        "FLAKY_CONNECTION" => tr("The connection keeps dropping").into(),
        "UNRELIABLE_IO" => tr("Data transfers fail on this connection").into(),
        other => other.to_string(),
    }
}

/// What can be said about the cable and the connection. `best_mbps`: fastest
/// link this phone has reached on this PC before.
pub fn assess(pc: Option<&PcLink>, phone: Option<&PhoneLink>, best_mbps: Option<u32>) -> Assessment {
    let mut findings: Vec<Finding> = Vec::new();
    macro_rules! find {
        ($level:expr, $title:expr, $detail:expr $(,)?) => {
            findings.push(Finding { level: $level, title: $title, detail: $detail })
        };
    }
    let speed = pc.and_then(|p| p.speed_mbps);
    let chip = pc.and_then(|p| p.typec.as_ref()).and_then(|t| t.cable.as_ref());
    let amps = phone.and_then(|p| p.max_current_ua).map(|ua| ua as f64 / 1e6);
    let volts = phone.and_then(|p| p.max_voltage_uv).map(|uv| uv as f64 / 1e6);
    let pc_three_amps = pc.and_then(|p| p.typec.as_ref()).is_some_and(|t| t.power_mode == "3.0A");

    // -- ends ---------------------------------------------------------------
    let (ends, ends_certainty, ends_reason) = if chip.is_some() {
        (CableEnds::UsbCToC, Certainty::Read, tr("The cable has an identification chip (e-marker): only USB-C to USB-C cables carry one").to_string())
    } else if pc.is_some_and(|p| p.lanes() >= 2) {
        (CableEnds::UsbCToC, Certainty::Read, tr("The link uses two lanes, which needs USB-C on both ends").to_string())
    } else if pc_three_amps || (amps.is_some_and(|a| a >= 2.9) && volts.map_or(true, |v| v < 5.5)) {
        (CableEnds::UsbCToC, Certainty::Probable, tr("The PC offers 3 A at 5 V, which only a USB-C port announces through the cable").to_string())
    } else {
        (CableEnds::Unknown, Certainty::Probable, tr("Nothing in the link tells the plugs apart: a USB-A port and a USB-C port look the same at this current").to_string())
    };

    // -- data wires ------------------------------------------------------------
    let port_ss = pc.is_some_and(|p| p.port_superspeed);
    let (superspeed, superspeed_reason) = match (speed, chip.and_then(|c| c.speed)) {
        (_, Some("USB 2.0")) => (Some(false), tr("According to the cable's chip").to_string()),
        (_, Some(_)) => (Some(true), tr("According to the cable's chip").to_string()),
        (Some(s), _) if s >= 5000 => (Some(true), tr("The link runs at USB 3 speed, so the cable has the USB 3 wires").to_string()),
        (Some(s), _) if s <= 480 && port_ss && best_mbps.is_some_and(|b| b >= 5000) => {
            (Some(false), tr("This port and this phone have reached USB 3 before; now they link at USB 2.0").to_string())
        }
        (Some(s), _) if s <= 480 => (None, tr("At USB 2.0 speed it cannot be told whether the cable or the phone is the limit").to_string()),
        _ => (None, String::new()),
    };

    // -- findings ---------------------------------------------------------------
    if let (Some(s), Some(p)) = (speed, pc) {
        let limit = p.port_max_mbps;
        if s <= 480 && port_ss && best_mbps.is_some_and(|b| b >= 5000) {
            find!(
Level::Warning,
                tr("The cable limits the speed").into(),
                t!("The link runs at {}, but this phone reached {} on this PC before. Use a USB 3 cable (or plug it in directly, without adapters).", speed_label(s), speed_label(best_mbps.unwrap_or(0))),
            );
        } else if s <= 480 && port_ss {
            find!(
Level::Info,
                tr("USB 2.0 link").into(),
                tr("The PC's port supports USB 3, but the link runs at 480 Mb/s. Many phones only have USB 2.0; if yours has USB 3, try another cable.").into(),
            );
        } else if s <= 480 && !port_ss {
            find!(Level::Info, tr("USB 2.0 port").into(), tr("This PC port only supports USB 2.0 (480 Mb/s). A blue or USB-C port is usually faster.").into());
        } else if let Some(l) = limit.filter(|l| *l > s) {
            find!(Level::Info, t!("Link at {}", speed_label(s)), t!("The PC's port reaches {}: the phone or the cable sets the limit.", speed_label(l)));
        } else {
            find!(Level::Ok, t!("Link at {}", speed_label(s)), tr("The fastest this port allows.").into());
        }
        if !p.hubs.is_empty() {
            find!(Level::Info, tr("Through a hub").into(), t!("{}. A hub shares its speed and power among everything connected to it.", p.hubs.join(" → ")));
        }
        if p.over_current.is_some_and(|n| n > 0) {
            find!(
Level::Warning,
                tr("The PC port detected over-current").into(),
                t!("{} times since the PC started. A damaged cable or device can cause it.", p.over_current.unwrap_or(0)),
            );
        }
    }
    if let Some(ph) = phone {
        if ph.contaminant.contains("detected") && !ph.contaminant.contains("not") {
            find!(Level::Problem, tr("Moisture or dirt in the phone's connector").into(), tr("The phone detected something in its USB-C port. Unplug it, let it dry and clean it carefully.").into());
        }
        for w in &ph.compliance {
            find!(Level::Warning, tr("Warning from the phone about the cable").into(), compliance_label(w));
        }
        if ph.power_limited == Some(true) {
            find!(Level::Warning, tr("Charging is limited").into(), tr("The phone is limiting the power it takes through this connection.").into());
        }
        if !ph.data_status.is_empty() && !matches!(ph.data_status.as_str(), "enabled" | "unknown") {
            find!(Level::Warning, tr("Data over USB is restricted on the phone").into(), t!("State: {}", ph.data_status));
        }
        let st = stability(&ph.events, ph.now);
        if st.worst_minute >= UNSTABLE_CUTS_PER_MINUTE && st.is_recent() {
            find!(
                Level::Warning,
                tr("Unstable connection").into(),
                t!(
                    "The phone recorded {} cuts in one minute ({}). A loose plug, a worn cable or a dirty port usually causes it.",
                    st.worst_minute,
                    st.worst_at
                ),
            );
        }
    }
    if findings.iter().all(|f| f.level <= Level::Ok) && (pc.is_some() || phone.is_some()) {
        find!(Level::Ok, tr("No problems found").into(), tr("The cable, the ports and the connection look fine.").into());
    }
    findings.sort_by_key(|f| std::cmp::Reverse(f.level));
    Assessment { cable: CableGuess { ends, ends_certainty, ends_reason, superspeed, superspeed_reason }, findings }
}

// ---------------------------------------------------------------------------
// Watching the link over time
// ---------------------------------------------------------------------------

/// Something worth a notification.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkAlert {
    /// The phone dropped off the USB bus this many times within a minute.
    Unstable { key: String, cuts: usize },
    /// It came back at USB 2.0 although it reached USB 3 on this PC before.
    SlowCable { key: String, now_mbps: u32, best_mbps: u32 },
}

/// Follows the phones on the PC's USB bus (see [`crate::usb::UsbMonitor`]):
/// counts how often each one drops off and compares its speed with the best
/// one it reached.
#[derive(Debug, Default)]
pub struct LinkWatch {
    present: HashMap<String, u32>,
    drops: HashMap<String, Vec<f64>>,
    alerted_unstable: HashMap<String, f64>,
    alerted_slow: std::collections::HashSet<String>,
}

/// Drops within a minute that count as unstable. Higher than in the phone's
/// own log: switching the USB mode or rebooting also re-plugs it once or
/// twice.
pub const UNSTABLE_DROPS: usize = 4;

impl LinkWatch {
    /// `phones`: (key, speed) of the phones on the bus now. `best`: fastest
    /// known speed per key. Returns the alerts and the new best speeds.
    pub fn update(&mut self, phones: &[(String, Option<u32>)], best: &HashMap<String, u32>, now: f64) -> (Vec<LinkAlert>, Vec<(String, u32)>) {
        let mut alerts = Vec::new();
        let mut records = Vec::new();
        let gone: Vec<String> = self.present.keys().filter(|k| !phones.iter().any(|(p, _)| p == *k)).cloned().collect();
        for key in gone {
            self.present.remove(&key);
            let d = self.drops.entry(key.clone()).or_default();
            d.push(now);
            d.retain(|t| now - t <= 60.0);
            let quiet = self.alerted_unstable.get(&key).map_or(true, |t| now - t > 600.0);
            if d.len() >= UNSTABLE_DROPS && quiet {
                self.alerted_unstable.insert(key.clone(), now);
                alerts.push(LinkAlert::Unstable { key: key.clone(), cuts: d.len() });
            }
            // A new plug-in may be slow again: warn once more.
            self.alerted_slow.remove(&key);
        }
        for (key, speed) in phones {
            let Some(s) = *speed else { continue };
            let new = self.present.insert(key.clone(), s) != Some(s);
            let known = best.get(key).copied();
            if known.map_or(true, |b| s > b) {
                records.push((key.clone(), s));
            }
            if new && s <= 480 && known.is_some_and(|b| b >= 5000) && self.alerted_slow.insert(key.clone()) {
                alerts.push(LinkAlert::SlowCable { key: key.clone(), now_mbps: s, best_mbps: known.unwrap_or(0) });
            }
        }
        (alerts, records)
    }
}

// ---------------------------------------------------------------------------
// Real speed
// ---------------------------------------------------------------------------

/// Where the speed test writes on the phone (removed afterwards).
pub const SPEED_TEST_REMOTE: &str = "/data/local/tmp/adbscan-speedtest.bin";
/// Size of the test file: enough to fill the link for a few seconds.
pub const SPEED_TEST_BYTES: usize = 128 * 1024 * 1024;

/// Result of the speed test, in MB/s (10⁶ bytes per second).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpeedTest {
    pub upload_mbs: f64,
    pub download_mbs: f64,
}

/// `adb push`/`pull` summary: “… (134217728 bytes in 3.190s)”. Returns the
/// bytes and the seconds adb measured itself (without starting up).
pub fn parse_transfer(text: &str) -> Option<(f64, f64)> {
    let inner = &text[text.rfind('(')? + 1..];
    let (bytes, rest) = inner.split_once(" bytes in ")?;
    let secs = rest.trim_start().split('s').next()?.trim().parse::<f64>().ok()?;
    let bytes = bytes.trim().parse::<f64>().ok()?;
    (secs > 0.0).then_some((bytes, secs))
}

/// The most the link itself carries, in MB/s, after the line encoding
/// (8b/10b up to 5 Gb/s, 128b/132b above). ADB never gets close: it is the
/// usual bottleneck, along with the phone's storage.
pub fn link_ceiling_mbs(mbps: u32) -> f64 {
    let efficiency = if mbps <= 5000 { 0.8 } else { 128.0 / 132.0 };
    mbps as f64 * efficiency / 8.0
}

/// Test data that does not compress (ADB may compress what it sends).
pub fn speed_test_data(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

/// A speed test kept for comparing cables and ports.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpeedRecord {
    /// When (seconds since 1970).
    pub ts: f64,
    pub serial: String,
    /// How it was connected, as shown then (“Bus 2 · port 3”, “Wi-Fi”).
    pub via: String,
    /// Negotiated USB speed (Mb/s), if it was by cable.
    pub link_mbps: Option<u32>,
    pub upload_mbs: f64,
    pub download_mbs: f64,
}

impl SpeedRecord {
    /// The slower direction: what a copy back and forth can count on.
    pub fn score(&self) -> f64 {
        self.upload_mbs.min(self.download_mbs)
    }
}

/// How many speed tests are kept.
pub const SPEED_HISTORY: usize = 30;

/// Adds `record` at the front, keeping the latest [`SPEED_HISTORY`].
pub fn remember_speed(history: &mut Vec<SpeedRecord>, record: SpeedRecord) {
    history.insert(0, record);
    history.truncate(SPEED_HISTORY);
}
