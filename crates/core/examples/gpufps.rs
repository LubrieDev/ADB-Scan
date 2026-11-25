//! Measures GPU and FPS of the foreground app. Usage: cargo run -p adbscan-core --example
//! gpufps
use adbscan_core::adb::Adb;
use adbscan_core::models::DeviceState;
use std::time::Instant;

fn main() {
    let adb = Adb::locate(None).expect("adb");
    let serial = adb.devices().expect("devices").into_iter().find(|d| d.state == DeviceState::Device).expect("no device").serial;
    let mut prev = None;
    for _ in 0..3 {
        let t = Instant::now();
        let m = adb.gpu_fps(&serial, prev);
        println!("{:>4} ms  GPU {:?} %  FPS {:?}  app {}", t.elapsed().as_millis(), m.gpu_busy.map(|g| (g * 10.0).round() / 10.0), m.fps, m.app);
        prev = m.newest_frame;
        std::thread::sleep(std::time::Duration::from_millis(800));
    }
}
