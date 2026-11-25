//! Per-app usage and wakelocks since the last full charge (read-only).
//! Usage: cargo run -p adbscan-core --example battery_usage
use adbscan_core::adb::Adb;
use adbscan_core::format;
use adbscan_core::models::DeviceState;
use std::time::Instant;

fn main() {
    let adb = Adb::locate(None).expect("adb");
    let serial = adb.devices().expect("devices").into_iter().find(|d| d.state == DeviceState::Device).expect("no device").serial;
    let t = Instant::now();
    let u = adb.battery_usage(&serial).expect("read");
    println!("Read in {:?}", t.elapsed());
    let secs = |ms: Option<u64>| format::human_duration(ms.map(|m| m as f64 / 1000.0));
    println!("On battery {} (screen off {}) · total {:.0} mAh · unattributed {:.0} mAh", secs(u.on_battery_ms), secs(u.screen_off_ms), u.total_mah, u.unattributed_mah());
    for a in u.apps.iter().take(8) {
        println!("  {:6.1} mAh {:5.1} %  (screen off {:5.1})  {}", a.mah, u.percent_of(a.mah).unwrap_or(0.0), a.screen_off_mah, a.label());
    }
    println!("App wakelocks:");
    for w in u.wakelocks.iter().take(5) {
        println!("  {} · {} times · {} · {}", secs(Some(w.ms)), w.count, u.owner(w.uid.unwrap_or(0)), w.short_tag());
    }
    println!("Kernel wakelocks: {}", u.kernel_wakelocks.len());
}
