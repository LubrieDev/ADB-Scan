//! Runs the security audit (read-only) against a real phone.
//! Usage: cargo run -p adbscan-core --example audit
use adbscan_core::adb::Adb;
use adbscan_core::models::{now_secs, DeviceState};
use adbscan_core::security;
use std::time::Duration;

fn main() {
    let adb = Adb::locate(None).expect("adb");
    let serial = adb.devices().expect("devices").into_iter().find(|d| d.state == DeviceState::Device).expect("no device").serial;
    let out = adb.shell_script(&serial, security::SCRIPT, Duration::from_secs(60)).expect("script").stdout;
    let now = now_secs();
    let r = security::evaluate(&serial, &out, (now / 86400.0) as i64, now as i64);
    println!("Score {} ({})", r.score, r.grade());
    for f in &r.findings {
        println!("[{:?}] {} · {}", f.severity, f.category, f.title);
        if !f.packages.is_empty() {
            println!("    apps: {}", f.packages.join(", "));
        }
    }
}
