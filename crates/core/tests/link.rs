//! The USB link: sysfs on the PC (a made-up tree with the shape of a
//! Comet Lake laptop), the phone's `dumpsys usb`/`dumpsys battery` (REDMAGIC
//! 11S Pro, Android 16) and the deductions.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use adbscan_core::link::*;
use adbscan_core::usb::UsbDevice;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("{text}\n")).unwrap();
}

/// `/sys/devices/…` and `/sys/bus/usb/devices` with symlinks, as Linux has them.
fn fake_sysfs(name: &str, phone: &str, speed: &str, with_typec: bool) -> (PathBuf, UsbDevice) {
    let base = std::env::temp_dir().join(format!("adbscan-link-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let pci = base.join("devices/pci0000:00/0000:00:14.0");
    write(&pci.join("vendor"), "0x8086");
    write(&pci.join("device"), "0x06ed");
    fs::create_dir_all(base.join("bus/pci/drivers/xhci_hcd")).unwrap();
    std::os::unix::fs::symlink(base.join("bus/pci/drivers/xhci_hcd"), pci.join("driver")).unwrap();
    let usb1 = pci.join("usb1");
    let usb2 = pci.join("usb2");
    write(&usb1.join("speed"), "480");
    write(&usb2.join("speed"), "10000");
    let port2 = usb2.join("2-0:1.0/usb2-port3");
    let port1 = usb1.join("1-0:1.0/usb1-port4");
    write(&port2.join("connect_type"), "hotplug");
    write(&port2.join("over_current_count"), "0");
    write(&port2.join("usb3_lpm_permit"), "u1_u2");
    write(&port1.join("connect_type"), "hotplug");
    for (k, v) in [("panel", "back"), ("horizontal_position", "left"), ("vertical_position", "center"), ("dock", "no"), ("lid", "no")] {
        write(&port2.join("physical_location").join(k), v);
    }
    std::os::unix::fs::symlink(&port1, port2.join("peer")).unwrap();
    std::os::unix::fs::symlink(&port2, port1.join("peer")).unwrap();
    let (bus, parent) = if phone.starts_with("1-") { ("1", &usb1) } else { ("2", &usb2) };
    let dev = parent.join(phone);
    write(&dev.join("speed"), speed);
    write(&dev.join("version"), if speed == "480" { " 2.10" } else { " 3.20" });
    write(&dev.join("rx_lanes"), "1");
    write(&dev.join("tx_lanes"), "1");
    write(&dev.join("bMaxPower"), "896mA");
    let root = base.join("bus/usb/devices");
    fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(&usb1, root.join("usb1")).unwrap();
    std::os::unix::fs::symlink(&usb2, root.join("usb2")).unwrap();
    std::os::unix::fs::symlink(usb1.join("1-0:1.0"), root.join("1-0:1.0")).unwrap();
    std::os::unix::fs::symlink(usb2.join("2-0:1.0"), root.join("2-0:1.0")).unwrap();
    std::os::unix::fs::symlink(&dev, root.join(phone)).unwrap();
    if phone.contains('.') {
        // A hub on the PC's port.
        let hub = parent.join(phone.split('.').next().unwrap());
        write(&hub.join("manufacturer"), "Generic");
        write(&hub.join("product"), "USB3.1 Hub");
        std::os::unix::fs::symlink(&hub, root.join(phone.split('.').next().unwrap())).unwrap();
    }
    if with_typec {
        let tc = base.join("devices/platform/USBC000:00/typec");
        let port = tc.join("port0");
        write(&port.join("power_operation_mode"), "3.0A");
        write(&port.join("usb_power_delivery_revision"), "3.0");
        write(&port.join("orientation"), "reverse");
        write(&port.join("data_role"), "[host] device");
        write(&port.join("power_role"), "[source] sink");
        let cable = tc.join("port0-cable");
        write(&cable.join("type"), "active");
        write(&cable.join("plug_type"), "type-c");
        // Active cable: VID 0x2b1d, 5 A, 50 V, ~2 m, Gen 2; VDO2 bit 10: optical.
        write(&cable.join("identity/id_header"), "0x20002b1d");
        write(&cable.join("identity/product_type_vdo1"), &format!("{:#x}", 0b10 | (2 << 5) | (3 << 9) | (2 << 13)));
        write(&cable.join("identity/product_type_vdo2"), &format!("{:#x}", 1 << 10));
        std::os::unix::fs::symlink(&port, port2.join("connector")).unwrap();
    }
    let device = UsbDevice { name: phone.into(), busnum: bus.into(), speed_mbps: speed.parse().ok(), ..Default::default() };
    (root, device)
}

const PCI_IDS: &str = "# comment\n8086  Intel Corporation\n\t06e0  Comet Lake HECI Controller\n\t06ed  Comet Lake USB 3.1 xHCI Host Controller\n\t\t1025 1300  Acer\n8087  Intel Corp.\n";

#[test]
fn pci_names() {
    assert_eq!(pci_name(PCI_IDS, 0x8086, 0x06ed).as_deref(), Some("Intel Corporation Comet Lake USB 3.1 xHCI Host Controller"));
    assert_eq!(pci_name(PCI_IDS, 0x8086, 0x1234).as_deref(), Some("Intel Corporation"));
    assert_eq!(pci_name(PCI_IDS, 0x10de, 0x1234), None);
}

#[test]
fn reads_the_pc_side() {
    let (root, dev) = fake_sysfs("direct", "2-3", "10000", false);
    let l = pc_link(&root, &dev, Some(PCI_IDS));
    assert_eq!(l.speed_mbps, Some(10000));
    assert_eq!(l.version, "3.20");
    assert_eq!((l.rx_lanes, l.tx_lanes, l.lanes()), (Some(1), Some(1), 1));
    assert_eq!(l.max_power_ma, Some(896));
    assert_eq!((l.bus.as_str(), l.root_port), ("2", Some(3)));
    assert_eq!(l.port_max_mbps, Some(10000));
    assert!(l.port_superspeed);
    assert!(l.hubs.is_empty());
    assert_eq!(l.connect_type, "hotplug");
    assert_eq!((l.over_current, l.lpm.as_str()), (Some(0), "u1_u2"));
    let loc = l.location.clone().unwrap();
    assert_eq!((loc.panel.as_str(), loc.horizontal.as_str()), ("back", "left"));
    let c = l.controller.clone().unwrap();
    assert_eq!((c.address.as_str(), c.driver.as_str()), ("0000:00:14.0", "xhci_hcd"));
    assert!(c.name.contains("Comet Lake USB 3.1"));
    assert!(l.typec.is_none());
}

#[test]
fn usb2_half_of_a_usb3_port_and_hubs() {
    // On the USB 2.0 bus, but the connector also has USB 3 (its peer).
    let (root, dev) = fake_sysfs("usb2", "1-4", "480", false);
    let l = pc_link(&root, &dev, None);
    assert_eq!(l.port_max_mbps, Some(10000));
    assert!(l.port_superspeed);
    assert_eq!(l.location.unwrap().horizontal, "left", "the location comes from the peer");
    assert_eq!(l.controller.unwrap().name, "8086:06ed");

    let (root, dev) = fake_sysfs("hub", "2-3.1", "5000", false);
    let l = pc_link(&root, &dev, None);
    assert_eq!(l.hubs, vec!["Generic USB3.1 Hub".to_string()]);
    assert_eq!(l.root_port, Some(3));
}

#[test]
fn reads_the_cable_chip_when_linux_has_it() {
    let (root, dev) = fake_sysfs("typec", "2-3", "10000", true);
    let t = pc_link(&root, &dev, None).typec.unwrap();
    assert_eq!((t.power_mode.as_str(), t.orientation.as_str(), t.data_role.as_str(), t.power_role.as_str()), ("3.0A", "reverse", "host", "source"));
    let c = t.cable.unwrap();
    assert_eq!((c.kind.as_str(), c.plug.as_str()), ("active", "type-c"));
    assert_eq!(c.vendor_id, Some(0x2b1d));
    assert_eq!((c.current_a, c.max_voltage_v, c.length), (Some(5), Some(50), Some("~2 m")));
    assert_eq!(c.optical, Some(true));
    assert!(c.speed.unwrap().contains("Gen 2"));
}

#[test]
fn decodes_passive_cables() {
    let c = decode_cable(false, 0b001 | (1 << 5) | (1 << 13), None);
    assert_eq!(c.speed, Some("USB 3.2 Gen 1 · 5 Gb/s"));
    assert_eq!((c.current_a, c.max_voltage_v, c.length, c.optical), (Some(3), Some(20), Some("~1 m"), Some(false)));
    assert_eq!(decode_cable(false, 0, None).speed, Some("USB 2.0"));
}

/// `dumpsys usb`, `dumpsys battery` and the properties of a REDMAGIC 11S Pro
/// (Android 16), with a short excerpt of a burst of cuts.
const PHONE: &str = "@@@USB@@@
USB MANAGER STATE (dumpsys usb):
{
  device_manager={
    handler={
      connected=true
      configured=true
      usb_charging=false
      kernel_state=CONFIGURED
    }
    USB Event=[
      11-24 15:24:23:715 USB UEVENT: {SUBSYSTEM=android_usb, SEQNUM=16503, ACTION=change, USB_STATE=CONNECTED, DEVPATH=/devices/virtual/android_usb/android0}
      11-24 15:24:24:857 USB UEVENT: {SUBSYSTEM=android_usb, SEQNUM=16504, ACTION=change, USB_STATE=DISCONNECTED, DEVPATH=/devices/virtual/android_usb/android0}
      11-24 15:24:24:939 USB UEVENT: {SUBSYSTEM=android_usb, SEQNUM=16505, ACTION=change, USB_STATE=CONNECTED, DEVPATH=/devices/virtual/android_usb/android0}
      11-24 15:24:25:049 USB UEVENT: {SUBSYSTEM=android_usb, SEQNUM=16506, ACTION=change, USB_STATE=DISCONNECTED, DEVPATH=/devices/virtual/android_usb/android0}
      11-24 15:24:25:134 USB UEVENT: {SUBSYSTEM=android_usb, SEQNUM=16507, ACTION=change, USB_STATE=CONNECTED, DEVPATH=/devices/virtual/android_usb/android0}
      11-24 15:24:25:923 USB UEVENT: {SUBSYSTEM=android_usb, SEQNUM=16508, ACTION=change, USB_STATE=DISCONNECTED, DEVPATH=/devices/virtual/android_usb/android0}
      11-24 15:24:26:047 USB UEVENT: {SUBSYSTEM=android_usb, SEQNUM=16509, ACTION=change, USB_STATE=CONNECTED, DEVPATH=/devices/virtual/android_usb/android0}
      11-24 15:24:26:166 USB UEVENT: {SUBSYSTEM=android_usb, SEQNUM=16510, ACTION=change, USB_STATE=DISCONNECTED, DEVPATH=/devices/virtual/android_usb/android0}
      11-24 15:24:26:245 USB UEVENT: {SUBSYSTEM=android_usb, SEQNUM=16511, ACTION=change, USB_STATE=CONNECTED, DEVPATH=/devices/virtual/android_usb/android0}
      11-24 15:24:27:388 USB UEVENT: {SUBSYSTEM=android_usb, SEQNUM=16512, ACTION=change, USB_STATE=DISCONNECTED, DEVPATH=/devices/virtual/android_usb/android0}
      11-24 15:24:27:470 USB UEVENT: {SUBSYSTEM=android_usb, SEQNUM=16513, ACTION=change, USB_STATE=CONNECTED, DEVPATH=/devices/virtual/android_usb/android0}
      11-24 17:40:02:100 USB UEVENT: {SUBSYSTEM=android_usb, SEQNUM=16700, ACTION=change, USB_STATE=DISCONNECTED, DEVPATH=/devices/virtual/android_usb/android0}
      11-24 17:40:03:100 USB UEVENT: {SUBSYSTEM=android_usb, SEQNUM=16701, ACTION=change, USB_STATE=CONFIGURED, DEVPATH=/devices/virtual/android_usb/android0}
    ]
  }
  port_manager={
    is_simulation_active=false
    usb_ports={
      port={
        id=port0
        supported_modes=dual, audio_acc
        supports_compliance_warnings=false
      }
      status={
        connected=true
        current_mode=ufp
        power_role=sink
        data_role=device
        role_combinations=[
          {
            power_role=source
            data_role=host
          }
        ]
        contaminant_presence_status=not detected
        usb_data_status=enabled
        is_power_transfer_limited=false
        usb_power_brick_status=unknown
        compliance_warning_status=[]
      }
      can_change_mode=true
      connected_at_millis=15437966
      last_connect_duration_millis=0
    }
    usb_hal_version=20
  }
}
@@@BATTERY@@@
Current Battery Service state:
  AC powered: true
  USB powered: false
  Wireless powered: false
  Max charging current: 3000000
  Max charging voltage: 5000000
  level: 80
@@@PROPS@@@
sys.usb.controller=a600000.dwc3
ro.boot.usbcontroller=a600000.dwc3
sys.usb.config=adb
sys.usb.state=adb
persist.sys.usb.dp=true
@@@CHARGE@@@
current_now=0
voltage_now=4189000
status=Charging
charge_type=Fast
capacity=80
charge_control_end_threshold=80
@@@UPTIME@@@
19037.97 75000.10
@@@NOW@@@
11-24 18:00:00
@@@END@@@
";

#[test]
fn reads_the_phone_side() {
    let p = parse_phone_link(PHONE);
    assert_eq!(p.port_modes, vec!["dual".to_string(), "audio_acc".to_string()]);
    assert_eq!((p.current_mode.as_str(), p.power_role.as_str(), p.data_role.as_str()), ("ufp", "sink", "device"));
    assert_eq!(p.contaminant, "not detected");
    assert_eq!(p.data_status, "enabled");
    assert_eq!((p.power_limited, p.supports_compliance), (Some(false), Some(false)));
    assert!(p.compliance.is_empty());
    assert_eq!(p.hal_version, "20");
    assert!((p.connected_for_s.unwrap() - 3600.0).abs() < 1.0);
    assert_eq!((p.ac_powered, p.usb_powered), (Some(true), Some(false)));
    assert_eq!((p.max_current_ua, p.max_voltage_uv), (Some(3_000_000), Some(5_000_000)));
    assert_eq!((p.controller.as_str(), p.usb_config.as_str(), p.displayport), ("a600000.dwc3", "adb", Some(true)));
    // CONFIGURED is not a cut nor a plug-in.
    assert_eq!(p.events.len(), 12);
    assert_eq!(p.events[1].at, "11-24 15:24:24");
    assert!(!p.events[1].connected);

    let st = stability(&p.events, p.now);
    assert_eq!(st.disconnects, 6);
    assert_eq!((st.worst_minute, st.worst_at.as_str()), (5, "11-24 15:24:24"));
    assert!(!st.is_recent(), "the burst was more than an hour before");
    let (at, ago) = st.last_cut.unwrap();
    assert_eq!(at, "11-24 17:40:02");
    assert!((ago.unwrap() - 1197.9).abs() < 1.0);
}

#[test]
fn reads_compliance_warnings_and_moisture() {
    let text = PHONE
        .replace("compliance_warning_status=[]", "compliance_warning_status=[MISSING_DATA_LINES, flaky_connection]")
        .replace("contaminant_presence_status=not detected", "contaminant_presence_status=detected");
    let p = parse_phone_link(&text);
    assert_eq!(p.compliance, vec!["MISSING_DATA_LINES".to_string(), "flaky_connection".to_string()]);
    let a = assess(None, Some(&p), None);
    assert_eq!(a.findings[0].level, Level::Problem, "moisture comes first");
    assert_eq!(a.findings.iter().filter(|f| f.level == Level::Warning).count(), 2, "the burst of cuts is old: no warning");
    // The same burst a few minutes ago is a warning.
    let recent = parse_phone_link(&text.replace("11-24 18:00:00", "11-24 15:30:00"));
    let a = assess(None, Some(&recent), None);
    assert!(a.findings.iter().any(|f| f.level == Level::Warning && f.title == "Unstable connection"));
}

fn link(speed: u32, port_max: u32) -> PcLink {
    PcLink { speed_mbps: Some(speed), port_max_mbps: Some(port_max), port_superspeed: port_max >= 5000, ..Default::default() }
}

#[test]
fn deduces_the_cable() {
    let phone = parse_phone_link(PHONE);
    // 3 A at 5 V: a USB-C port, so USB-C on both ends (probable).
    let a = assess(Some(&link(10000, 10000)), Some(&phone), None);
    assert_eq!((a.cable.ends, a.cable.ends_certainty), (CableEnds::UsbCToC, Certainty::Probable));
    assert_eq!(a.cable.superspeed, Some(true));
    // Without the phone the plugs cannot be told apart.
    let a = assess(Some(&link(10000, 10000)), None, None);
    assert_eq!(a.cable.ends, CableEnds::Unknown);
    // Two lanes: read, not deduced.
    let two = PcLink { rx_lanes: Some(2), tx_lanes: Some(2), ..link(20000, 20000) };
    assert_eq!(assess(Some(&two), None, None).cable.ends_certainty, Certainty::Read);
}

#[test]
fn spots_a_usb2_cable() {
    // It reached 10 Gb/s here before; now 480 Mb/s.
    let a = assess(Some(&link(480, 10000)), None, Some(10000));
    assert_eq!(a.cable.superspeed, Some(false));
    assert_eq!(a.findings[0].level, Level::Warning);
    // Never seen faster: maybe the phone itself is USB 2.0.
    let a = assess(Some(&link(480, 10000)), None, None);
    assert_eq!(a.cable.superspeed, None);
    assert_eq!(a.findings[0].level, Level::Info);
    // A USB 2.0-only port.
    let a = assess(Some(&link(480, 480)), None, None);
    assert_eq!(a.findings[0].level, Level::Info);
}

#[test]
fn reports_hubs_over_current_and_all_clear() {
    let p = PcLink { hubs: vec!["Generic USB3.1 Hub".into()], over_current: Some(2), ..link(5000, 10000) };
    let a = assess(Some(&p), None, None);
    assert_eq!(a.findings[0].level, Level::Warning);
    assert!(a.findings.iter().any(|f| f.detail.contains("Generic USB3.1 Hub")));
    let a = assess(Some(&link(10000, 10000)), None, None);
    assert!(a.findings.iter().all(|f| f.level == Level::Ok));
    assert!(assess(None, None, None).findings.is_empty());
}

#[test]
fn watch_counts_drops_and_slow_plugs() {
    let mut w = LinkWatch::default();
    let best: HashMap<String, u32> = HashMap::from([("SER".to_string(), 10000)]);
    let on = vec![("SER".to_string(), Some(10000))];
    let (alerts, records) = w.update(&on, &best, 0.0);
    assert!(alerts.is_empty() && records.is_empty());
    let mut seen = Vec::new();
    for i in 1..=4 {
        let t = i as f64 * 5.0;
        seen.extend(w.update(&[], &best, t).0);
        seen.extend(w.update(&on, &best, t + 1.0).0);
    }
    assert_eq!(seen, vec![LinkAlert::Unstable { key: "SER".into(), cuts: 4 }]);
    // Back at USB 2.0: once per plug-in.
    w.update(&[], &best, 100.0);
    let slow = vec![("SER".to_string(), Some(480))];
    assert_eq!(w.update(&slow, &best, 101.0).0, vec![LinkAlert::SlowCable { key: "SER".into(), now_mbps: 480, best_mbps: 10000 }]);
    assert!(w.update(&slow, &best, 102.0).0.is_empty());
    // A new phone records its best speed.
    let (_, records) = w.update(&[("NEW".to_string(), Some(5000))], &best, 200.0);
    assert_eq!(records, vec![("NEW".to_string(), 5000)]);
}

#[test]
fn speed_test_helpers() {
    let push = "/tmp/x.bin: 1 file pushed, 0 skipped. 41.2 MB/s (134217728 bytes in 3.104s)";
    let (bytes, secs) = parse_transfer(push).unwrap();
    assert_eq!((bytes, secs), (134217728.0, 3.104));
    assert_eq!(parse_transfer("error: no devices"), None);
    assert!((link_ceiling_mbs(10000) - 1212.1).abs() < 0.1);
    assert!((link_ceiling_mbs(480) - 48.0).abs() < 0.01);
    let d = speed_test_data(1000);
    assert_eq!(d.len(), 1000);
    assert_ne!(d[..8], d[8..16]);
}

#[test]
fn charging_right_now() {
    // The REDMAGIC at 80 % with its charge limit at 80 %: paused, not broken.
    let p = parse_phone_link(PHONE);
    assert_eq!((p.battery_current_ua, p.battery_voltage_uv, p.battery_level, p.charge_limit), (Some(0), Some(4_189_000), Some(80), Some(80)));
    assert_eq!(p.charge_type, "Fast");
    assert_eq!(charge_now(&p), Some(ChargeNow::PausedAtLimit(80)));
    // Charging at 1.5 A (some phones report it negative).
    let charging = parse_phone_link(&PHONE.replace("current_now=0", "current_now=-1500000").replace("capacity=80", "capacity=55"));
    match charge_now(&charging) {
        Some(ChargeNow::Charging { watts, amps, volts }) => {
            assert!((amps - 1.5).abs() < 1e-9 && (volts - 4.189).abs() < 1e-9);
            assert!((watts - 6.2835).abs() < 1e-3);
        }
        other => panic!("{other:?}"),
    }
    let full = parse_phone_link(&PHONE.replace("status=Charging", "status=Full"));
    assert_eq!(charge_now(&full), Some(ChargeNow::Full));
    let on_battery = parse_phone_link(&PHONE.replace("status=Charging", "status=Discharging").replace("current_now=0", "current_now=500000"));
    assert!(matches!(charge_now(&on_battery), Some(ChargeNow::Discharging(Some(w))) if (w - 2.0945).abs() < 1e-3));
    // Without the battery data (older phones), nothing is claimed.
    assert_eq!(charge_now(&parse_phone_link("")), None);
}

#[test]
fn speed_history_keeps_the_latest() {
    let rec = |i: usize| SpeedRecord { ts: i as f64, serial: "S".into(), via: "Bus 2 · port 3".into(), link_mbps: Some(10000), upload_mbs: 160.0, download_mbs: 150.0 + i as f64 };
    let mut h = Vec::new();
    for i in 0..SPEED_HISTORY + 5 {
        remember_speed(&mut h, rec(i));
    }
    assert_eq!(h.len(), SPEED_HISTORY);
    assert_eq!(h[0].ts, (SPEED_HISTORY + 4) as f64, "newest first");
    assert_eq!(rec(3).score(), 153.0);
    let json = serde_json::to_string(&h[0]).unwrap();
    assert_eq!(serde_json::from_str::<SpeedRecord>(&json).unwrap(), h[0]);
}
