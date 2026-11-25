//! Basic analysis: what can be known about an Android phone **without**
//! developer options or USB or wireless debugging.
//!
//! Sources, all read-only and with no new permissions:
//!
//! - USB descriptors the phone publishes when plugged in (sysfs): manufacturer,
//!   product, serial number, speed and the chosen USB mode.
//! - MTP/PTP ([`crate::mtp`]) if the user chose “File transfer” and the phone is
//!   unlocked: model, name, battery and storage.
//! - ADB Scan's local history: whether this serial number was connected before
//!   with authorised debugging.
//!
//! Nothing is made up: each item is [`Status::Available`],
//! [`Status::Unavailable`] with the reason, or [`Status::NeedsAdb`] (only with
//! the advanced analysis). This module does not use ADB and is independent of
//! the advanced analysis.

use crate::{i18n::{tr, trc}, t};
use crate::db::KnownDevice;
use crate::format;
use crate::models::DeviceState;
use crate::mtp::{self, GvfsStorage, MtpError, MtpReport};
use crate::probe::Probe;
use crate::usb::{self, UsbDevice, UsbFunction, UsbMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Read from the phone just now (or from the history, if the source says
    /// so).
    Available,
    /// Cannot be obtained in this situation; the value is the reason.
    Unavailable,
    /// Only with authorised USB debugging (advanced analysis).
    NeedsAdb,
    /// Read in progress.
    Pending,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    pub label: String,
    /// The data, or the reason it is missing.
    pub value: String,
    pub status: Status,
    /// Where it comes from (shown as a tooltip).
    pub source: &'static str,
    /// Identifying data that private mode hides.
    pub private: bool,
}

impl Fact {
    fn ok(label: impl Into<String>, value: impl Into<String>, source: &'static str) -> Self {
        Self { label: label.into(), value: value.into(), status: Status::Available, source, private: false }
    }

    fn missing(label: impl Into<String>, reason: impl Into<String>, source: &'static str) -> Self {
        Self { label: label.into(), value: reason.into(), status: Status::Unavailable, source, private: false }
    }

    fn private(mut self) -> Self {
        self.private = true;
        self
    }

    /// Optional data: empty counts as “does not publish it”.
    fn text(label: &str, value: &str, source: &'static str, reason: &str) -> Self {
        if value.trim().is_empty() {
            Self::missing(label, reason, source)
        } else {
            Self::ok(label, value.trim(), source)
        }
    }

    fn from_probe<T>(label: &str, p: &Probe<T>, source: &'static str, f: impl FnOnce(&T) -> String) -> Self {
        match p {
            Probe::Ok(v) => Self::ok(label, f(v), source),
            Probe::Missing => Self::missing(label, tr("The phone does not publish it"), source),
            other => Self::missing(label, other.status_text().unwrap_or_default(), source),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub title: &'static str,
    pub description: &'static str,
    pub facts: Vec<Fact>,
}

/// How certain it is that the device is an Android.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AndroidEvidence {
    /// Announces Android's MTP extension (`android.com`).
    MtpExtension,
    /// Has an Android debugging or fastboot interface.
    DebugInterface,
    /// Android phone manufacturer, with no further evidence.
    Vendor,
    Unconfirmed,
}

impl AndroidEvidence {
    pub fn label(self) -> &'static str {
        match self {
            Self::MtpExtension => tr("Android (confirmed by the phone itself over MTP)"),
            Self::DebugInterface => tr("Android (Android debugging interface present)"),
            Self::Vendor => tr("Probably Android (Android phone manufacturer)"),
            Self::Unconfirmed => tr("Unconfirmed"),
        }
    }
}

/// Result of a phone's MTP read.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // one per connected phone: not worth a box
pub enum MtpState {
    /// Not read yet (or being read).
    Pending,
    Read(Result<MtpReport, MtpError>),
}

/// Everything that goes into a phone's report.
pub struct Inputs<'a> {
    pub usb: &'a UsbDevice,
    pub node_access: Option<bool>,
    pub mtp: &'a MtpState,
    /// Storages seen by GVfs if the MTP interface was busy.
    pub gvfs: Option<&'a [GvfsStorage]>,
    pub known: Option<&'a KnownDevice>,
    pub nickname: Option<&'a str>,
    /// State in ADB if ADB sees it (only to say whether debugging is on; this
    /// report does not use ADB).
    pub adb_state: Option<DeviceState>,
}

pub fn evidence(usb: &UsbDevice, mtp: &MtpState) -> AndroidEvidence {
    if let MtpState::Read(Ok(r)) = mtp {
        if r.info.is_android() {
            return AndroidEvidence::MtpExtension;
        }
    }
    if matches!(usb.mode, UsbMode::Adb | UsbMode::Fastboot) {
        return AndroidEvidence::DebugInterface;
    }
    if usb::ANDROID_VENDOR_IDS.contains(&usb.vendor_id.to_lowercase().as_str()) {
        return AndroidEvidence::Vendor;
    }
    AndroidEvidence::Unconfirmed
}

const SRC_USB: &str = "USB descriptor published by the phone (sysfs)";
const SRC_IFACES: &str = "The phone's USB interfaces (sysfs)";
const SRC_MTP: &str = "MTP: information the phone shares in “File transfer” mode";
const SRC_GVFS: &str = "GVfs (the file manager has the phone mounted)";
const SRC_HISTORY: &str = "ADB Scan's local history (previous connections; not live data)";

fn bytes(v: u64) -> String {
    format::human_bytes(Some(v as f64))
}

/// Basic analysis report of a phone.
pub fn report(i: &Inputs) -> Vec<Group> {
    vec![identity(i), connection(i), transfer(i), recognition(i)]
}

fn identity(i: &Inputs) -> Group {
    let u = i.usb;
    let vendor_table = usb::vendor_name(&u.vendor_id);
    let manufacturer = if !u.manufacturer.trim().is_empty() {
        Fact::ok(tr("Manufacturer"), u.manufacturer.trim(), SRC_USB)
    } else if !vendor_table.is_empty() {
        Fact::ok(tr("Manufacturer"), vendor_table, tr("USB vendor identifier (VID)"))
    } else {
        Fact::missing(tr("Manufacturer"), tr("The phone does not publish it over USB"), SRC_USB)
    };
    let facts = vec![
        Fact::ok(tr("System"), evidence(u, i.mtp).label(), tr("USB interfaces and, if available, MTP")),
        manufacturer,
        Fact::text(tr("Product"), &u.product, SRC_USB, tr("The phone does not publish it over USB")),
        Fact::text(tr("Serial number"), &u.serial, SRC_USB, tr("The phone does not publish it over USB")).private(),
        Fact::ok(tr("USB identifier"), format!("{}:{}", u.vendor_id, u.product_id), SRC_USB),
    ];
    Group { title: tr("Identification"), description: tr("What the phone says about itself when plugged in, in any USB mode with data."), facts }
}

fn connection(i: &Inputs) -> Group {
    let u = i.usb;
    let funcs = u.functions();
    let mut facts = Vec::new();
    let data: Vec<&str> = funcs.iter().filter(|f| **f != UsbFunction::Adb).map(|f| f.label()).collect();
    facts.push(if data.is_empty() {
        Fact::ok(tr("USB mode"), tr("No data transfer (charging only)"), SRC_IFACES)
    } else {
        Fact::ok(tr("USB mode"), data.join(" · "), SRC_IFACES)
    });
    let debug = if funcs.contains(&UsbFunction::Adb) {
        match i.adb_state {
            Some(DeviceState::Device) => tr("On and authorised on this computer"),
            Some(DeviceState::Unauthorized) | Some(DeviceState::Authorizing) => tr("On, waiting for authorisation on the phone"),
            _ => tr("On in the phone"),
        }
    } else if u.mode == UsbMode::Fastboot {
        tr("Not applicable: the phone is in fastboot")
    } else {
        tr("Off (it is optional)")
    };
    facts.push(Fact::ok(tr("USB debugging"), debug, SRC_IFACES));
    facts.push(Fact::text(tr("Speed"), &if u.speed_mbps.is_some() { u.speed_label() } else { String::new() }, SRC_USB, tr("Not available")));
    facts.push(Fact::text(tr("USB version"), &u.version, SRC_USB, tr("Not available")));
    facts.push(Fact::ok(tr("Computer port"), t!("bus {} · port {}", u.busnum, u.name), "sysfs"));
    match i.node_access {
        Some(true) => facts.push(Fact::ok(tr("USB access"), tr("Allowed for your user"), tr("Permissions of /dev/bus/usb"))),
        Some(false) => facts.push(Fact::missing(tr("USB access"), tr("No permission: install ADB Scan's udev rules"), tr("Permissions of /dev/bus/usb"))),
        None => {}
    }
    Group { title: tr("USB connection"), description: tr("How it is connected and which USB mode the phone chose."), facts }
}

fn transfer(i: &Inputs) -> Group {
    const TITLE: &str = "File transfer (MTP)";
    const DESC: &str = "With “File transfer” chosen on the phone and the screen unlocked. Only descriptive data is read: no files.";
    let mut facts = Vec::new();
    let need = tr("Needs “File transfer” on the phone");
    match i.mtp {
        MtpState::Pending => {
            for label in [tr("Status"), tr("Battery"), tr("Storage")] {
                facts.push(Fact { label: label.into(), value: tr("Reading…").into(), status: Status::Pending, source: SRC_MTP, private: false });
            }
        }
        MtpState::Read(Ok(r)) => {
            let proto = if r.function == UsbFunction::Ptp { tr("PTP (photo transfer)") } else { "MTP" };
            facts.push(Fact::ok(tr("Status"), t!("Read over {}", proto), SRC_MTP));
            facts.push(Fact::text(tr("Manufacturer (according to the phone)"), &r.info.manufacturer, SRC_MTP, tr("Does not publish it")));
            facts.push(Fact::text(tr("Model"), &r.info.model, SRC_MTP, tr("Does not publish it")));
            facts.push(Fact::from_probe(tr("Device name"), &r.friendly_name, SRC_MTP, |n| n.clone()).private());
            facts.push(Fact::text(tr("Declared version"), &r.info.device_version, SRC_MTP, trc("fem", "Does not publish it")));
            if let Some(l) = r.perceived_type.ok().and_then(|c| mtp::perceived_type_label(*c)) {
                facts.push(Fact::ok(tr("Device type"), l, SRC_MTP));
            }
            facts.push(Fact::from_probe(tr("Battery"), &r.battery, SRC_MTP, |b| format!("{b} %")));
            match &r.storages {
                Probe::Ok(list) if list.is_empty() => {
                    facts.push(Fact::missing(tr("Storage"), tr("The phone does not share it: unlock it and press Refresh"), SRC_MTP));
                }
                Probe::Ok(list) => {
                    for s in list {
                        let mut v = t!("{} free of {}", bytes(s.free), bytes(s.capacity));
                        let kind = s.kind_label();
                        if !kind.is_empty() {
                            v.push_str(&format!(" · {kind}"));
                        }
                        if s.read_only() {
                            v.push_str(tr(" · read-only"));
                        }
                        facts.push(Fact::ok(s.name(), v, SRC_MTP));
                    }
                }
                other => facts.push(Fact::from_probe(tr("Storage"), other, SRC_MTP, |_: &Vec<_>| String::new())),
            }
        }
        MtpState::Read(Err(e)) => {
            facts.push(Fact::missing(tr("Status"), e.message(), SRC_MTP));
            let reason = match e {
                MtpError::NotOffered => need.to_string(),
                MtpError::Busy => tr("Not available while the file manager has the phone mounted. If you unmount it (without unplugging the cable), press Refresh").to_string(),
                other => other.message(),
            };
            facts.push(Fact::missing(tr("Battery"), reason.clone(), SRC_MTP));
            match (e, i.gvfs) {
                (MtpError::Busy, Some(list)) if !list.is_empty() => {
                    for s in list {
                        let v = match (s.free, s.capacity) {
                            (Some(f), Some(c)) => t!("{} free of {}", bytes(f), bytes(c)),
                            (None, Some(c)) => t!("Capacity {}", bytes(c)),
                            _ => String::new(),
                        };
                        facts.push(Fact::text(&s.name, &v, SRC_GVFS, tr("The file manager does not report the space")));
                    }
                }
                _ => facts.push(Fact::missing(tr("Storage"), reason, SRC_MTP)),
            }
        }
    }
    Group { title: TITLE, description: DESC, facts }
}

fn recognition(i: &Inputs) -> Group {
    let mut facts = Vec::new();
    match i.known {
        Some(k) => {
            let when = k.last_seen.map(|t| t!(" · last seen {}", format::relative_time(Some(t)))).unwrap_or_default();
            let times = if k.sessions == 1 { tr("1 connection").to_string() } else { t!("{} connections", k.sessions) };
            facts.push(Fact::ok(tr("Seen before"), t!("Yes · {}{}", times, when), SRC_HISTORY));
            if !k.model.is_empty() {
                facts.push(Fact::ok(tr("Recorded name"), k.model.clone(), SRC_HISTORY));
            }
        }
        None if i.usb.serial.is_empty() => facts.push(Fact::missing(tr("Seen before"), tr("Cannot tell: the phone does not publish a serial number"), SRC_HISTORY)),
        None => facts.push(Fact::ok(tr("Seen before"), tr("Not on record (the history only records connections with authorised debugging)"), SRC_HISTORY)),
    }
    if let Some(n) = i.nickname.filter(|n| !n.trim().is_empty()) {
        facts.push(Fact::ok(tr("Nickname"), n.trim(), tr("Your ADB Scan settings")).private());
    }
    Group { title: tr("Recognised on this computer"), description: tr("By its USB serial number, compared with the local history. Nothing is sent outside the computer."), facts }
}

/// What only the advanced analysis provides (authorised USB debugging).
pub fn advanced_group() -> Group {
    const NEEDS: &str = "Requires USB debugging and your authorisation";
    let facts = [
        tr("Android version and security patch"),
        tr("Live processor, memory and temperatures"),
        tr("Detailed battery: temperature, voltage, cycles and capacity"),
        tr("Network: IP, Wi-Fi and carrier"),
        tr("Installed apps, permissions and usage"),
        tr("File browser, screenshots and screen mirroring"),
        tr("Security audit and platform sheet"),
        tr("Private identifiers (IMEI, SIM)"),
    ]
    .iter()
    .map(|l| Fact { label: (*l).into(), value: tr(NEEDS).into(), status: Status::NeedsAdb, source: tr("ADB (advanced analysis)"), private: false })
    .collect();
    Group {
        title: tr("Advanced analysis"),
        description: tr("Android does not share this data without USB debugging. You can turn it on whenever you want; it is completely voluntary."),
        facts,
    }
}

/// What can be known without a USB cable.
pub fn without_usb() -> Group {
    Group {
        title: tr("No USB connection"),
        description: tr("Without a cable, Android does not identify itself to the computer."),
        facts: vec![
            Fact::missing(
                tr("Over the network (Wi-Fi)"),
                tr("Only with wireless debugging (advanced analysis). Without it, the phone does not announce itself on the network in an identifiable way and ADB Scan does not scan the network"),
                tr("Wireless debugging mDNS"),
            ),
            Fact::missing(tr("No connection at all"), tr("There is no live data. History keeps what was recorded in previous connections"), "—"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mtp::{DeviceInfo, StorageInfo};
    use crate::usb::{UsbEndpoint, UsbInterface};

    fn mtp_iface() -> UsbInterface {
        UsbInterface {
            number: 0,
            class: "ff".into(),
            subclass: "ff".into(),
            protocol: "00".into(),
            name: "MTP".into(),
            endpoints: vec![UsbEndpoint { address: 0x01, transfer: "Bulk".into() }, UsbEndpoint { address: 0x81, transfer: "Bulk".into() }, UsbEndpoint { address: 0x82, transfer: "Interrupt".into() }],
        }
    }

    fn phone(ifaces: Vec<UsbInterface>, mode: UsbMode) -> UsbDevice {
        UsbDevice {
            name: "2-3".into(),
            vendor_id: "19d2".into(),
            product_id: "1351".into(),
            manufacturer: "nubia".into(),
            product: "REDMAGIC 11S Pro".into(),
            serial: "A1B2C3D4E5F6".into(),
            busnum: "2".into(),
            speed_mbps: Some(5000),
            version: "3.20".into(),
            interfaces: ifaces,
            mode,
            ..Default::default()
        }
    }

    fn inputs<'a>(usb: &'a UsbDevice, mtp: &'a MtpState) -> Inputs<'a> {
        Inputs { usb, node_access: Some(true), mtp, gvfs: None, known: None, nickname: None, adb_state: None }
    }

    fn find<'a>(groups: &'a [Group], label: &str) -> &'a Fact {
        groups.iter().flat_map(|g| &g.facts).find(|f| f.label == label).unwrap_or_else(|| panic!("missing {label}"))
    }

    fn report_ok() -> MtpReport {
        MtpReport {
            function: UsbFunction::Mtp,
            info: DeviceInfo { vendor_extension: "microsoft.com: 1.0; android.com: 1.0;".into(), manufacturer: "nubia".into(), model: "NX809J".into(), device_version: "1.0".into(), ..Default::default() },
            battery: Probe::Ok(87),
            friendly_name: Probe::Ok("Ana's REDMAGIC".into()),
            perceived_type: Probe::Missing,
            storages: Probe::Ok(vec![StorageInfo { id: 0x10001, storage_type: 3, capacity: 512_000_000_000, free: 100_000_000_000, description: "Internal shared storage".into(), ..Default::default() }]),
        }
    }

    #[test]
    fn no_debugging_with_file_transfer() {
        let usb = phone(vec![mtp_iface()], UsbMode::Other);
        let mtp = MtpState::Read(Ok(report_ok()));
        let g = report(&inputs(&usb, &mtp));
        assert_eq!(find(&g, "System").value, AndroidEvidence::MtpExtension.label());
        assert_eq!(find(&g, "USB debugging").value, "Off (it is optional)");
        assert_eq!(find(&g, "USB mode").value, "File transfer (MTP)");
        assert_eq!(find(&g, "Battery").value, "87 %");
        assert_eq!(find(&g, "Battery").status, Status::Available);
        assert!(find(&g, "Internal shared storage").value.contains("free of"));
        assert!(find(&g, "Serial number").private);
        assert!(find(&g, "Device name").private);
        // Nothing that was read requires ADB.
        assert!(g.iter().flat_map(|g| &g.facts).all(|f| f.status != Status::NeedsAdb));
    }

    #[test]
    fn charging_only_marks_mtp_data_unavailable() {
        let usb = phone(vec![], UsbMode::Other);
        let mtp = MtpState::Read(Err(MtpError::NotOffered));
        let g = report(&inputs(&usb, &mtp));
        assert_eq!(find(&g, "USB mode").value, "No data transfer (charging only)");
        assert_eq!(find(&g, "System").value, AndroidEvidence::Vendor.label());
        let b = find(&g, "Battery");
        assert_eq!(b.status, Status::Unavailable);
        assert!(b.value.contains("File transfer"), "says what is needed, does not make up a level");
        assert_eq!(find(&g, "Storage").status, Status::Unavailable);
    }

    #[test]
    fn busy_interface_uses_gvfs_storage_but_not_battery() {
        let usb = phone(vec![mtp_iface()], UsbMode::Other);
        let mtp = MtpState::Read(Err(MtpError::Busy));
        let gvfs = vec![GvfsStorage { name: "Internal shared storage".into(), capacity: Some(512_000_000_000), free: Some(1_000_000_000) }];
        let g = report(&Inputs { gvfs: Some(&gvfs), ..inputs(&usb, &mtp) });
        assert_eq!(find(&g, "Battery").status, Status::Unavailable);
        let s = find(&g, "Internal shared storage");
        assert_eq!(s.status, Status::Available);
        assert_eq!(s.source, SRC_GVFS);
    }

    #[test]
    fn locked_phone_shares_no_storage() {
        let usb = phone(vec![mtp_iface()], UsbMode::Other);
        let mut r = report_ok();
        r.storages = Probe::Ok(vec![]);
        r.battery = Probe::Missing;
        let mtp = MtpState::Read(Ok(r));
        let g = report(&inputs(&usb, &mtp));
        assert_eq!(find(&g, "Storage").status, Status::Unavailable);
        assert_eq!(find(&g, "Battery").value, "The phone does not publish it");
    }

    #[test]
    fn debugging_state_is_reported_without_using_adb() {
        let adb_iface = UsbInterface { number: 1, class: "ff".into(), subclass: "42".into(), protocol: "01".into(), name: "ADB Interface".into(), ..Default::default() };
        let usb = phone(vec![mtp_iface(), adb_iface], UsbMode::Adb);
        let mtp = MtpState::Pending;
        let g = report(&Inputs { adb_state: Some(DeviceState::Unauthorized), ..inputs(&usb, &mtp) });
        assert_eq!(find(&g, "USB debugging").value, "On, waiting for authorisation on the phone");
        assert_eq!(find(&g, "USB mode").value, "File transfer (MTP)");
        assert_eq!(find(&g, "Battery").status, Status::Pending);
    }

    #[test]
    fn recognition_from_history() {
        let usb = phone(vec![], UsbMode::Other);
        let mtp = MtpState::Pending;
        let k = KnownDevice { serial: usb.serial.clone(), model: "REDMAGIC 11S Pro".into(), last_seen: Some(1.0), sessions: 3 };
        let g = report(&Inputs { known: Some(&k), nickname: Some("My phone"), ..inputs(&usb, &mtp) });
        assert!(find(&g, "Seen before").value.starts_with("Yes · 3 connections"));
        assert!(find(&g, "Nickname").private);
        let g = report(&inputs(&usb, &mtp));
        assert!(find(&g, "Seen before").value.starts_with("Not on record"));
    }

    #[test]
    fn advanced_only_needs_adb() {
        assert!(advanced_group().facts.iter().all(|f| f.status == Status::NeedsAdb));
        assert!(without_usb().facts.iter().all(|f| f.status == Status::Unavailable));
    }
}
