//! Reads the display name of the user apps of a real phone.
//! Usage: cargo run -p adbscan-core --example labels [maximum]
use adbscan_core::adb::Adb;
use std::time::Instant;

fn main() {
    let max: usize = std::env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let adb = Adb::locate(None).expect("adb");
    let serial = adb.devices().expect("devices").into_iter().find(|d| d.state == adbscan_core::models::DeviceState::Device).expect("no device").serial;
    let text = adb.shell_text(&serial, &["pm", "list", "packages", "-3", "-f"]);
    let (mut ok, mut total) = (0, 0);
    let t0 = Instant::now();
    for line in text.lines().take(max) {
        let Some(rest) = line.trim().strip_prefix("package:") else {
            continue;
        };
        let Some((path, pkg)) = rest.rsplit_once('=') else {
            continue;
        };
        total += 1;
        let t = Instant::now();
        let label = adb.package_label(&serial, path);
        if label.is_some() {
            ok += 1;
        }
        println!("{:>5} ms  {pkg:<45} {:?}", t.elapsed().as_millis(), label);
    }
    println!("{ok}/{total} names in {:.1} s", t0.elapsed().as_secs_f64());
}
