//! Tests of the live telemetry stream.

use adbscan_core::live::*;
use std::collections::HashMap;

const FREQ_TABS: &str = "1000000\t900000\t3000000\t3000000\t500000\t400000\tschedutil\tschedutil";

fn base_sections() -> HashMap<String, String> {
    let pairs: [(&str, &str); 19] = [
        ("TICK", "1700000000000"),
        ("BATTERY", "AC powered: true\nUSB powered: false\nstatus: 2\npresent: true\nlevel: 80\nscale: 100\nvoltage: 4400\ntemperature: 250\ntechnology: Li-ion"),
        ("MEM", "MemTotal: 8000000 kB\nMemFree: 1000000 kB\nMemAvailable: 4000000 kB\nBuffers: 100000 kB\nCached: 2000000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB"),
        ("DF", "Filesystem 1K-blocks Used Available Use% Mounted on\n/dev/block/dm-1 100000 40000 60000 40% /data\n/dev/block/dm-2 200000 50000 150000 25% /storage/emulated/0"),
        ("THERMAL", "Thermal Status: 1\nCurrent temperatures from HAL:\nTemperature{mValue=42.5, mType=0, mName=CPU, mStatus=0}\nTemperature{mValue=38.0, mType=1, mName=GPU, mStatus=0}\n"),
        ("STAT", "cpu  100 0 100 800 0 0 0 0 0 0\n"),
        ("LOAD", "1.50 1.25 1.00 2/100 1234\n"),
        ("UPTIME", "1234.5 2000.0\n"),
        ("USB", "connected=true\nconfigured=true\nusb_charging=false\n"),
        ("USBSTATE", "adb,mtp\nadb\n"),
        ("BRIGHTNESS", "128\n"),
        ("FREQ", FREQ_TABS),
        ("IPADDR", "23: wlan0    inet 192.168.1.10/24 brd 192.168.1.255 scope global wlan0\n"),
        ("IPROUTE", "default via 192.168.1.1 dev wlan0\n"),
        ("PROPS", "[ro.product.model]: [Test Phone]\n[ro.build.version.release]: [16]\n[ro.board.platform]: [testboard]\n"),
        ("CPUINFO", "processor : 0\nprocessor : 1\nHardware : testboard\n"),
        ("SIZE", "Physical size: 1080x2400\n"),
        ("DENSITY", "Physical density: 420\n"),
        ("DISPLAY", "mBaseDisplayInfo=DisplayInfo{fps=120.0}\n"),
    ];
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn script_interval_is_clamped() {
    let s = build_live_script(750, 20);
    assert!(s.contains("TARGET_MS=750"));
    assert!(s.contains("@@@BATTERY@@@"));
    assert!(s.contains("@@@END@@@"));
    assert!(build_live_script(1, 20).contains(&format!("TARGET_MS={MIN_INTERVAL_MS}")));
    assert!(build_live_script(99_999, 20).contains(&format!("TARGET_MS={MAX_INTERVAL_MS}")));
}

#[test]
fn freq_columns_are_transposed() {
    let t = parse_freq_columns(FREQ_TABS);
    assert!(t.contains("###CORE0"));
    assert!(t.contains("cur=1000000"));
    assert!(t.contains("###CORE1"));
    assert!(t.contains("gov=schedutil"));
    assert_eq!(parse_freq_columns("1\t2\t3"), "");
}

#[test]
fn snapshot_builds_all_sections() {
    let mut acc = LiveAccumulator::default();
    let snap = build_snapshot("serial", &base_sections(), &mut acc);
    assert_eq!(snap.serial, "serial");
    assert_eq!(snap.battery.percentage(), Some(80.0));
    assert!(snap.battery.ac_powered);
    assert_eq!(snap.memory.total_kib, Some(8_000_000));
    assert_eq!(snap.external_storage.mount, "/storage/emulated/0");
    assert_eq!(snap.data_storage.mount, "/data");
    assert!(snap.cpu_temperature().is_some());
    assert_eq!(snap.cpu.core_count, Some(2));
    assert_eq!(snap.cpu.cores.len(), 2);
    assert_eq!(snap.props["ro.product.model"], "Test Phone");
    assert_eq!(snap.display.width, Some(1080));
    assert_eq!(snap.display.density_dpi, Some(420));
    assert_eq!(snap.display.refresh_hz, Some(120.0));
    assert_eq!(snap.display.brightness, Some(128));
    assert_eq!(snap.network.interfaces[0].name, "wlan0");
    assert_eq!(snap.network.wifi_ipv4().as_deref(), Some("192.168.1.10"));
    assert_eq!(snap.usb.functions, "adb,mtp");
    assert_eq!(snap.thermal_status, Some(1));
    assert_eq!(acc.blocks, 1);
    assert!((acc.last_tick - 1_700_000_000.0).abs() < 1e-6);
}

#[test]
fn static_data_survives_between_blocks_and_cpu_load_uses_delta() {
    let mut acc = LiveAccumulator::default();
    let first = build_snapshot("s", &base_sections(), &mut acc);
    assert_eq!(first.cpu.load_percent, None);
    let mut fast = base_sections();
    for k in ["PROPS", "CPUINFO", "SIZE", "DENSITY", "DISPLAY"] {
        fast.remove(k);
    }
    fast.insert("STAT".into(), "cpu  150 0 150 900 0 0 0 0 0 0\n".into());
    let second = build_snapshot("s", &fast, &mut acc);
    assert_eq!(second.props["ro.product.model"], "Test Phone");
    assert_eq!(second.display.width, Some(1080));
    assert_eq!(second.cpu.core_count, Some(2));
    assert!((second.cpu.load_percent.unwrap() - 50.0).abs() < 1e-9);
}

#[test]
fn empty_block_does_not_panic() {
    let mut acc = LiveAccumulator::default();
    let snap = build_snapshot("s", &HashMap::new(), &mut acc);
    assert_eq!(snap.battery.percentage(), None);
    assert_eq!(snap.memory.total_kib, None);
}

#[test]
fn wear_section_is_static_and_survives_between_blocks() {
    assert!(build_live_script(500, 20).contains("@@@WEAR@@@"));
    let mut acc = LiveAccumulator::default();
    let mut first = base_sections();
    first.insert("WEAR".into(), "batt_cycle_count=53\nbatt_charge_full=7360000\nbatt_charge_full_design=7500000\nkind=UFS\nlife= \neol=\n".into());
    let a = build_snapshot("s", &first, &mut acc);
    assert_eq!(a.health.battery_cycles, Some(53));
    // Fast block without a static section: the last read is kept.
    let mut fast = base_sections();
    fast.remove("PROPS");
    let b = build_snapshot("s", &fast, &mut acc);
    assert_eq!(b.health.battery_cycles, Some(53));
    assert!(b.health.probed);
    // Before the first read nothing is taken as known.
    let empty = build_snapshot("s", &base_sections(), &mut LiveAccumulator::default());
    assert!(!empty.health.probed);
}
