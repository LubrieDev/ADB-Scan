//! USB detection from the PC: sysfs + kernel events (netlink uevent).
//!
//! It lets us know a phone is connected before ADB authorises it, and tell ADB,
//! fastboot or MTP-only apart by the USB interfaces. It does not need libudev:
//! the `NETLINK_KOBJECT_UEVENT` socket is listened to directly.
//!
//! Linux only ([`HOST_USB`]): elsewhere the bus is not read and phones are only
//! seen through adb.

use crate::i18n::tr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
#[cfg(target_os = "linux")]
use std::time::Duration;

/// Common Android vendor IDs. Only a hint: the reliable part is the ADB
/// (ff/42/01) or fastboot (ff/42/03) interface descriptor.
pub const ANDROID_VENDOR_IDS: [&str; 23] =
    ["04e8", "0b05", "0fce", "1004", "12d1", "18d1", "19d2", "22b8", "2717", "2916", "2a70", "2b4c", "2d95", "339b", "04dd", "0489", "05c6", "1782", "1f3a", "2207", "0e8d", "2ae5", "0bb4"];

pub fn vendor_name(vid: &str) -> &'static str {
    match vid.to_lowercase().as_str() {
        "04e8" => "Samsung",
        "0b05" => "ASUS",
        "0fce" => "Sony",
        "1004" => "LG",
        "12d1" => "Huawei",
        "18d1" => "Google",
        "19d2" => "ZTE / nubia / REDMAGIC",
        "22b8" => "Motorola",
        "2717" => "Xiaomi",
        "2a70" => "OnePlus",
        "2b4c" => "Fairphone",
        "2d95" => "vivo",
        "05c6" | "339b" => "Qualcomm",
        "0e8d" => "MediaTek",
        "2ae5" => "Nothing",
        "0bb4" => "HTC",
        "1782" => "Unisoc",
        "2207" => "Rockchip",
        _ => "",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsbMode {
    /// ADB interface present (ff/42/01).
    Adb,
    /// fastboot interface present (ff/42/03).
    Fastboot,
    /// Qualcomm EDL (05c6:9008).
    Edl,
    /// Android vendor without a debugging interface (MTP, charging…).
    #[default]
    Other,
}

/// What the phone offers over USB, deduced from its interfaces. On Android it
/// depends on the USB mode chosen on the phone itself (“File transfer”, “USB
/// tethering”…) and on whether debugging is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UsbFunction {
    Adb,
    Fastboot,
    /// File transfer (MTP).
    Mtp,
    /// Photo transfer (PTP).
    Ptp,
    /// USB tethering (RNDIS/NCM/ECM).
    Tethering,
    Midi,
    /// Android 14+: the phone as a webcam (UVC).
    Webcam,
    MassStorage,
    Serial,
    /// Android Open Accessory.
    Accessory,
}

impl UsbFunction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Adb => tr("USB debugging (ADB)"),
            Self::Fastboot => "Fastboot",
            Self::Mtp => tr("File transfer (MTP)"),
            Self::Ptp => tr("Photo transfer (PTP)"),
            Self::Tethering => tr("USB tethering"),
            Self::Midi => "MIDI",
            Self::Webcam => tr("Webcam"),
            Self::MassStorage => tr("Mass storage"),
            Self::Serial => tr("Serial port"),
            Self::Accessory => tr("Android accessory"),
        }
    }
}

/// Endpoint of an interface (sysfs `ep_XX`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsbEndpoint {
    /// `bEndpointAddress`: bit 0x80 means input (phone → PC).
    pub address: u8,
    /// `Bulk`, `Interrupt`, `Isoc` or `Control`.
    pub transfer: String,
}

impl UsbEndpoint {
    pub fn is_in(&self) -> bool {
        self.address & 0x80 != 0
    }
}

/// USB interface as the phone describes it (sysfs `B-P:C.I`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsbInterface {
    pub number: u8,
    pub class: String,
    pub subclass: String,
    pub protocol: String,
    /// `iInterface` string (“MTP”, “ADB Interface”…), if there is one.
    pub name: String,
    pub endpoints: Vec<UsbEndpoint>,
}

impl UsbInterface {
    pub fn function(&self, vendor: &str, product: &str) -> Option<UsbFunction> {
        let (c, s, p) = (self.class.as_str(), self.subclass.as_str(), self.protocol.as_str());
        // Android describes MTP as a vendor-specific class (ff/ff/00) named
        // “MTP”; some vendors use the imaging class (06/01/01).
        if self.name.trim().eq_ignore_ascii_case("mtp") {
            return Some(UsbFunction::Mtp);
        }
        let f = match (c, s, p) {
            ("ff", "42", "01") => UsbFunction::Adb,
            ("ff", "42", "03") => UsbFunction::Fastboot,
            ("06", "01", "01") => UsbFunction::Ptp,
            ("e0", "01", "03") | ("ef", "04", "01") | ("02", "0d", _) | ("02", "06", _) => UsbFunction::Tethering,
            ("01", "03", _) => UsbFunction::Midi,
            ("0e", "01", _) => UsbFunction::Webcam,
            ("08", _, _) => UsbFunction::MassStorage,
            ("02", "02", _) => UsbFunction::Serial,
            _ if vendor.eq_ignore_ascii_case("18d1") && (0x2d00..=0x2d05).contains(&u32::from_str_radix(product, 16).unwrap_or(0)) => UsbFunction::Accessory,
            _ => return None,
        };
        Some(f)
    }

    fn bulk(&self, input: bool) -> Option<u8> {
        self.endpoints.iter().find(|e| e.transfer == "Bulk" && e.is_in() == input).map(|e| e.address)
    }

    /// Data input and output endpoints (bulk), if the interface has them.
    pub fn bulk_pair(&self) -> Option<(u8, u8)> {
        Some((self.bulk(true)?, self.bulk(false)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsbDevice {
    pub sysfs: PathBuf,
    pub name: String,
    pub vendor_id: String,
    pub product_id: String,
    pub manufacturer: String,
    pub product: String,
    pub serial: String,
    pub mode: UsbMode,
    pub busnum: String,
    pub devnum: String,
    /// Mbit/s according to sysfs (480, 5000, 10000…).
    pub speed_mbps: Option<u32>,
    /// Negotiated USB version (`version`: 2.00, 3.20…).
    pub version: String,
    /// Interfaces of the active configuration, by number.
    pub interfaces: Vec<UsbInterface>,
}

impl UsbDevice {
    pub fn has_adb_interface(&self) -> bool {
        self.mode == UsbMode::Adb
    }

    /// Functions it offers right now, without repeats and in interface order.
    pub fn functions(&self) -> Vec<UsbFunction> {
        let mut out = Vec::new();
        for i in &self.interfaces {
            if let Some(f) = i.function(&self.vendor_id, &self.product_id) {
                if !out.contains(&f) {
                    out.push(f);
                }
            }
        }
        out
    }

    /// MTP or PTP interface (the file or photo transfer one).
    pub fn transfer_interface(&self) -> Option<&UsbInterface> {
        let pick = |want: UsbFunction| self.interfaces.iter().find(|i| i.function(&self.vendor_id, &self.product_id) == Some(want) && i.bulk_pair().is_some());
        pick(UsbFunction::Mtp).or_else(|| pick(UsbFunction::Ptp))
    }

    /// Key that stays stable while it is plugged in: the USB serial number or,
    /// if it does not publish one, the port.
    pub fn key(&self) -> String {
        if self.serial.is_empty() {
            self.name.clone()
        } else {
            self.serial.clone()
        }
    }

    pub fn speed_label(&self) -> String {
        match self.speed_mbps {
            Some(1) | Some(2) => tr("USB 1.x (slow)").into(),
            Some(12) => "USB 1.1 · 12 Mb/s".into(),
            Some(480) => "USB 2.0 · 480 Mb/s".into(),
            Some(5000) => "USB 3.2 Gen 1 · 5 Gb/s".into(),
            Some(10000) => "USB 3.2 Gen 2 · 10 Gb/s".into(),
            Some(20000) => "USB 3.2 Gen 2×2 · 20 Gb/s".into(),
            Some(n) if n >= 40000 => format!("USB4 · {} Gb/s", n / 1000),
            Some(n) => format!("{n} Mb/s"),
            None => "—".into(),
        }
    }

    pub fn display_name(&self) -> String {
        let vendor = if self.manufacturer.is_empty() { vendor_name(&self.vendor_id).to_string() } else { self.manufacturer.clone() };
        match (vendor.is_empty(), self.product.is_empty()) {
            (false, false) => format!("{vendor} {}", self.product),
            (true, false) => self.product.clone(),
            (false, true) => vendor,
            (true, true) => format!("{}:{}", self.vendor_id, self.product_id),
        }
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn is_device_dir(name: &str) -> bool {
    // "1-14", "2-3.1": bus-port[.port…], without ':' (those are interfaces).
    let mut parts = name.splitn(2, '-');
    let (Some(bus), Some(ports)) = (parts.next(), parts.next()) else {
        return false;
    };
    !bus.is_empty() && bus.chars().all(|c| c.is_ascii_digit()) && !ports.is_empty() && ports.chars().all(|c| c.is_ascii_digit() || c == '.')
}

pub const SYSFS_USB: &str = "/sys/bus/usb/devices";

/// Whether ADB Scan reads the USB bus itself (sysfs, usbfs): only on Linux. On
/// Windows the USB, MTP and cable sections say they are not available.
pub const HOST_USB: bool = cfg!(target_os = "linux");

/// Android-related USB devices found in sysfs.
pub fn scan(root: &Path) -> Vec<UsbDevice> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_device_dir(&name) {
            continue;
        }
        let dir = root.join(&name);
        let vendor = read(&dir.join("idVendor"));
        if vendor.is_empty() {
            continue;
        }
        let product_id = read(&dir.join("idProduct"));
        let interfaces = read_interfaces(&dir, &name);
        let mut mode = UsbMode::Other;
        for i in &interfaces {
            if i.class == "ff" && i.subclass == "42" {
                match i.protocol.as_str() {
                    "01" => mode = UsbMode::Adb,
                    "03" if mode != UsbMode::Adb => mode = UsbMode::Fastboot,
                    _ => {}
                }
            }
        }
        if vendor == "05c6" && product_id == "9008" {
            mode = UsbMode::Edl;
        }
        let android_vendor = ANDROID_VENDOR_IDS.contains(&vendor.to_lowercase().as_str());
        // Android MTP: vendor-specific class (ff/ff) named “MTP”. That way a
        // phone from a vendor not on the list is also seen, without debugging.
        // Media players and cameras use 06/01/01.
        let android_mtp = interfaces.iter().any(|i| i.class == "ff" && i.subclass == "ff" && i.name.trim().eq_ignore_ascii_case("mtp"));
        if mode == UsbMode::Other && !android_vendor && !android_mtp {
            continue;
        }
        out.push(UsbDevice {
            name: name.clone(),
            vendor_id: vendor,
            product_id,
            manufacturer: read(&dir.join("manufacturer")),
            product: read(&dir.join("product")),
            serial: read(&dir.join("serial")),
            mode,
            busnum: read(&dir.join("busnum")),
            devnum: read(&dir.join("devnum")),
            speed_mbps: read(&dir.join("speed")).parse::<f64>().ok().map(|v| v as u32),
            version: read(&dir.join("version")),
            interfaces,
            sysfs: dir,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Interfaces of a device (`<dir>/<name>:C.I`) with their endpoints. The driver
/// that has claimed it is not stored: it changes when it is claimed (adb, the
/// file manager or the app itself) and would cause “device changed” notices
/// without anything changing; it is read when used with [`interface_driver`].
fn read_interfaces(dir: &Path, name: &str) -> Vec<UsbInterface> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let prefix = format!("{name}:");
    let mut out = Vec::new();
    for e in entries.flatten() {
        let iname = e.file_name().to_string_lossy().into_owned();
        if !iname.starts_with(&prefix) {
            continue;
        }
        let ip = dir.join(&iname);
        let mut endpoints = Vec::new();
        if let Ok(eps) = fs::read_dir(&ip) {
            for ep in eps.flatten() {
                if !ep.file_name().to_string_lossy().starts_with("ep_") {
                    continue;
                }
                let Ok(address) = u8::from_str_radix(&read(&ep.path().join("bEndpointAddress")), 16) else {
                    continue;
                };
                endpoints.push(UsbEndpoint { address, transfer: read(&ep.path().join("type")) });
            }
        }
        endpoints.sort_by_key(|e| e.address);
        out.push(UsbInterface {
            number: u8::from_str_radix(&read(&ip.join("bInterfaceNumber")), 16).unwrap_or(0),
            class: read(&ip.join("bInterfaceClass")).to_lowercase(),
            subclass: read(&ip.join("bInterfaceSubClass")).to_lowercase(),
            protocol: read(&ip.join("bInterfaceProtocol")).to_lowercase(),
            name: read(&ip.join("interface")),
            endpoints,
        });
    }
    out.sort_by_key(|i| i.number);
    out
}

/// Driver that holds the interface right now (`usbfs` = a user program: adb,
/// the file manager…), or `None` if it is free.
pub fn interface_driver(dev: &UsbDevice, number: u8) -> Option<String> {
    let entries = fs::read_dir(&dev.sysfs).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if !e.file_name().to_string_lossy().starts_with(&format!("{}:", dev.name)) {
            continue;
        }
        if u8::from_str_radix(&read(&p.join("bInterfaceNumber")), 16).ok() != Some(number) {
            continue;
        }
        return fs::read_link(p.join("driver")).ok().and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()));
    }
    None
}

/// Path of the device node (`/dev/bus/usb/BBB/DDD`).
pub fn node_path(dev: &UsbDevice) -> Option<String> {
    let bus: u32 = dev.busnum.parse().ok()?;
    let num: u32 = dev.devnum.parse().ok()?;
    Some(format!("/dev/bus/usb/{bus:03}/{num:03}"))
}

/// Checks whether the user can open the USB node (`/dev/bus/usb/BBB/DDD`). If
/// not, ADB will show “no permissions” and udev rules are needed.
#[cfg(target_os = "linux")]
pub fn node_accessible(dev: &UsbDevice) -> Option<bool> {
    let c = std::ffi::CString::new(node_path(dev)?).ok()?;
    // SAFETY: access(2) with a valid NUL-terminated path.
    Some(unsafe { libc::access(c.as_ptr(), libc::R_OK | libc::W_OK) } == 0)
}

/// There are no usbfs nodes outside Linux: unknown.
#[cfg(not(target_os = "linux"))]
pub fn node_accessible(_dev: &UsbDevice) -> Option<bool> {
    None
}

// ---------------------------------------------------------------------------
// Kernel event monitor
// ---------------------------------------------------------------------------

/// Listens to uevents from the USB subsystem and calls `on_change` with the new
/// list (grouping bursts within 300 ms). If netlink is not available, it polls
/// sysfs every 2 s. Outside Linux it reports an empty list once.
pub struct UsbMonitor {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl UsbMonitor {
    pub fn start<F>(on_change: F) -> Self
    where
        F: Fn(Vec<UsbDevice>) + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let handle = thread::Builder::new().name("usb-monitor".into()).spawn(move || run_monitor(s, on_change)).expect("usb thread");
        Self { stop, handle: Some(handle) }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for UsbMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "linux")]
fn open_uevent_socket() -> Option<i32> {
    // SAFETY: direct socket/bind/setsockopt calls with zero-initialised
    // structures; every return code is checked.
    unsafe {
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, libc::NETLINK_KOBJECT_UEVENT);
        if fd < 0 {
            return None;
        }
        let mut addr: libc::sockaddr_nl = std::mem::zeroed();
        addr.nl_family = libc::AF_NETLINK as u16;
        addr.nl_groups = 1; // kernel events
        if libc::bind(fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_nl>() as u32) < 0 {
            libc::close(fd);
            return None;
        }
        let tv = libc::timeval { tv_sec: 0, tv_usec: 250_000 };
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO, &tv as *const _ as *const libc::c_void, std::mem::size_of::<libc::timeval>() as u32);
        Some(fd)
    }
}

/// Without sysfs there is no bus to watch.
#[cfg(not(target_os = "linux"))]
fn run_monitor<F: Fn(Vec<UsbDevice>)>(_stop: Arc<AtomicBool>, on_change: F) {
    on_change(Vec::new());
}

#[cfg(target_os = "linux")]
fn run_monitor<F: Fn(Vec<UsbDevice>)>(stop: Arc<AtomicBool>, on_change: F) {
    let root = PathBuf::from(SYSFS_USB);
    let mut last = scan(&root);
    on_change(last.clone());
    let fd = open_uevent_socket();
    let mut buf = vec![0u8; 8192];
    let mut pending_since: Option<std::time::Instant> = None;
    while !stop.load(Ordering::SeqCst) {
        match fd {
            Some(fd) => {
                // SAFETY: valid buffer of known size.
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
                if n > 0 {
                    let msg = &buf[..n as usize];
                    if msg.windows(13).any(|w| w == b"SUBSYSTEM=usb") {
                        pending_since.get_or_insert_with(std::time::Instant::now);
                    }
                }
            }
            None => {
                thread::sleep(Duration::from_secs(2));
                pending_since = Some(std::time::Instant::now() - Duration::from_secs(1));
            }
        }
        if pending_since.is_some_and(|t| t.elapsed() >= Duration::from_millis(300)) {
            pending_since = None;
            let now = scan(&root);
            if now != last {
                last = now.clone();
                on_change(now);
            }
        }
    }
    if let Some(fd) = fd {
        // SAFETY: fd opened by us.
        unsafe {
            libc::close(fd);
        }
    }
}

/// A USB product name that says nothing about the model (Samsung calls every
/// phone and tablet “SAMSUNG_Android”): not used as the device's name.
pub fn is_generic_product(name: &str) -> bool {
    let n = name.trim().to_lowercase().replace(['_', '-'], " ");
    let n = n.split_whitespace().filter(|w| !matches!(*w, "samsung" | "android" | "phone" | "device" | "mtp" | "adb" | "interface" | "composite" | "usb")).count();
    n == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, v: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, v).unwrap();
    }

    #[test]
    fn device_dir_names() {
        assert!(is_device_dir("1-14"));
        assert!(is_device_dir("2-3.1.4"));
        assert!(!is_device_dir("1-14:1.0"));
        assert!(!is_device_dir("usb1"));
    }

    #[test]
    fn scans_fake_sysfs() {
        let root = std::env::temp_dir().join(format!("adbscan-sysfs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // Phone with ADB
        write(&root.join("2-3/idVendor"), "19d2\n");
        write(&root.join("2-3/idProduct"), "1352\n");
        write(&root.join("2-3/product"), "REDMAGIC 11S Pro\n");
        write(&root.join("2-3/speed"), "10000\n");
        write(&root.join("2-3/2-3:1.1/bInterfaceClass"), "ff");
        write(&root.join("2-3/2-3:1.1/bInterfaceSubClass"), "42");
        write(&root.join("2-3/2-3:1.1/bInterfaceProtocol"), "01");
        // Phone in fastboot
        write(&root.join("1-2/idVendor"), "18d1");
        write(&root.join("1-2/idProduct"), "4ee0");
        write(&root.join("1-2/1-2:1.0/bInterfaceClass"), "ff");
        write(&root.join("1-2/1-2:1.0/bInterfaceSubClass"), "42");
        write(&root.join("1-2/1-2:1.0/bInterfaceProtocol"), "03");
        // Mouse: ignored
        write(&root.join("1-5/idVendor"), "046d");
        let devs = scan(&root);
        assert_eq!(devs.len(), 2);
        let phone = devs.iter().find(|d| d.name == "2-3").unwrap();
        assert_eq!(phone.mode, UsbMode::Adb);
        assert_eq!(phone.speed_label(), "USB 3.2 Gen 2 · 10 Gb/s");
        assert_eq!(phone.display_name(), "ZTE / nubia / REDMAGIC REDMAGIC 11S Pro");
        assert_eq!(devs.iter().find(|d| d.name == "1-2").unwrap().mode, UsbMode::Fastboot);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn android_without_debugging_is_found_by_its_mtp_interface() {
        let root = std::env::temp_dir().join(format!("adbscan-sysfs-mtp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // Vendor not on the list, without debugging, in “File transfer” mode.
        write(&root.join("3-1/idVendor"), "abcd");
        write(&root.join("3-1/idProduct"), "0001");
        write(&root.join("3-1/serial"), "XYZ123");
        let i = root.join("3-1/3-1:1.0");
        write(&i.join("bInterfaceNumber"), "00");
        write(&i.join("bInterfaceClass"), "ff");
        write(&i.join("bInterfaceSubClass"), "ff");
        write(&i.join("bInterfaceProtocol"), "00");
        write(&i.join("interface"), "MTP");
        write(&i.join("ep_01/bEndpointAddress"), "01");
        write(&i.join("ep_01/type"), "Bulk");
        write(&i.join("ep_81/bEndpointAddress"), "81");
        write(&i.join("ep_81/type"), "Bulk");
        write(&i.join("ep_82/bEndpointAddress"), "82");
        write(&i.join("ep_82/type"), "Interrupt");
        // Photo camera (PTP 06/01/01) from another vendor: not a phone.
        write(&root.join("3-2/idVendor"), "04a9");
        write(&root.join("3-2/3-2:1.0/bInterfaceClass"), "06");
        write(&root.join("3-2/3-2:1.0/bInterfaceSubClass"), "01");
        write(&root.join("3-2/3-2:1.0/bInterfaceProtocol"), "01");
        let devs = scan(&root);
        assert_eq!(devs.len(), 1);
        let d = &devs[0];
        assert_eq!(d.mode, UsbMode::Other);
        assert_eq!(d.functions(), vec![UsbFunction::Mtp]);
        let t = d.transfer_interface().unwrap();
        assert_eq!(t.bulk_pair(), Some((0x81, 0x01)));
        assert_eq!(d.key(), "XYZ123");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn functions_from_interfaces() {
        let iface = |c: &str, s: &str, p: &str, n: &str| UsbInterface { class: c.into(), subclass: s.into(), protocol: p.into(), name: n.into(), ..Default::default() };
        let d = UsbDevice {
            vendor_id: "18d1".into(),
            product_id: "4ee7".into(),
            interfaces: vec![iface("ff", "ff", "00", "MTP"), iface("ff", "42", "01", "ADB Interface"), iface("e0", "01", "03", ""), iface("0a", "00", "00", ""), iface("01", "03", "00", "")],
            ..Default::default()
        };
        assert_eq!(d.functions(), vec![UsbFunction::Mtp, UsbFunction::Adb, UsbFunction::Tethering, UsbFunction::Midi]);
        let acc = UsbDevice { vendor_id: "18d1".into(), product_id: "2d01".into(), interfaces: vec![iface("ff", "ff", "00", "")], ..Default::default() };
        assert_eq!(acc.functions(), vec![UsbFunction::Accessory]);
    }

    #[test]
    fn generic_products() {
        for g in ["SAMSUNG_Android", "Android", "MTP", "Android Phone", "ADB Interface", "", "  "] {
            assert!(is_generic_product(g), "{g}");
        }
        for real in ["REDMAGIC 11S Pro", "moto e20", "Pixel 8", "Galaxy Tab A9+"] {
            assert!(!is_generic_product(real), "{real}");
        }
    }
}
