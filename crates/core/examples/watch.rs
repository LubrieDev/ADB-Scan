//! `cargo run -p adbscan-core --example watch [seconds]`: ADB and USB monitors.
use adbscan_core::{
    adb::Adb,
    monitor::{DeviceMonitor, MonitorEvent},
    usb::{self, UsbMonitor},
};
use std::time::Duration;

fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let adb = Adb::locate(None).expect("adb");
    let _m = DeviceMonitor::start(adb, |ev| match ev {
        MonitorEvent::Devices(d) => println!("ADB: {:?}", d.iter().map(|x| (&x.serial, x.state_label())).collect::<Vec<_>>()),
        MonitorEvent::Problem(p) => println!("ADB notice: {p}"),
    });
    let _u = UsbMonitor::start(|devs| {
        for d in devs {
            println!("USB: {} · {:?} · {} · node accessible: {:?}", d.display_name(), d.mode, d.speed_label(), usb::node_accessible(&d));
        }
    });
    std::thread::sleep(Duration::from_secs(secs));
}
