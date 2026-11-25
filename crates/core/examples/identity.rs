//! Checks which private identifiers can be read, **without showing them**: it
//! only says whether each one was obtained and its length.
//! Usage: cargo run -p adbscan-core --example identity
use adbscan_core::adb::Adb;
use adbscan_core::models::DeviceState;
use std::time::Instant;

fn shape(v: &Option<String>) -> String {
    match v {
        Some(s) => format!("yes ({} characters)", s.chars().count()),
        None => "—".into(),
    }
}

fn main() {
    let adb = Adb::locate(None).expect("adb");
    let serial = adb.devices().expect("devices").into_iter().find(|d| d.state == DeviceState::Device).expect("no device").serial;
    let t = Instant::now();
    let i = adb.private_info(&serial).expect("read");
    println!("Read in {:?}", t.elapsed());
    println!("  IMEI 1: {}", shape(&i.imei[0]));
    println!("  IMEI 2: {}", shape(&i.imei[1]));
    println!("  IMSI: {}", shape(&i.imsi));
    println!("  ICCID: {}", shape(&i.iccid));
    println!("  Number: {}", shape(&i.phone_number));
    println!("  Serial: {}", shape(&i.serial));
    println!("  Android ID: {}", shape(&i.android_id));
    println!("  Device name: {}", shape(&i.device_name));
    println!("  Bluetooth name: {}", shape(&i.bluetooth_name));
    println!("  Bluetooth MAC: {}", shape(&i.bluetooth_mac));
    for (n, s) in i.sims.iter().enumerate() {
        println!("  SIM {}: {} · carrier {}", n + 1, s.state_label(), if s.operator.is_empty() { "—" } else { "yes" });
    }
    println!("  Accounts: {} ({})", i.accounts.len(), i.accounts.iter().map(|a| a.kind_label()).collect::<Vec<_>>().join(", "));
    if std::env::args().any(|a| a == "--ip") {
        println!(
            "  Public IP: {}",
            match adb.public_ip(&serial) {
                Ok(ip) => format!("yes ({})", if ip.is_ipv4() { "IPv4" } else { "IPv6" }),
                Err(e) => e,
            }
        );
    }
}
