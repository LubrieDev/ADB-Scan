//! Quick diagnosis against a real device:
//! `cargo run -p adbscan-core --example probe [seconds]`

use adbscan_core::adb::Adb;
use adbscan_core::format;
use adbscan_core::live::{LiveEvent, LiveStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let adb = Adb::locate(None).expect("adb");
    let devices = adb.devices().expect("adb devices");
    println!("adb: {}", adb.path().display());
    for d in &devices {
        println!("  {} · {} · {:?}", d.serial, d.state_label(), d.transport);
    }
    let Some(dev) = devices.iter().find(|d| d.connected()) else {
        println!("No authorised device");
        return;
    };

    let t = Instant::now();
    let snap = adb.collect_snapshot(&dev.serial).expect("snapshot");
    println!("\nOne-off snapshot in {:?}", t.elapsed());
    println!("  {} {} · {}", snap.manufacturer(), snap.model(), snap.android_version());
    println!("  Security patch: {}", snap.security_patch());
    println!("  Battery {} · {} · {}", format::percent(snap.battery.percentage()), snap.battery.status(), format::temperature(snap.battery_temperature()));
    println!("  RAM {} of {}", format::human_kib(snap.memory.used_kib()), format::human_kib(snap.memory.total_kib));
    println!("  Data {} ({})", format::human_kib(snap.data_storage.total_kib), format::percent(snap.data_storage.percent()));
    println!("  CPU {} cores · max {}", snap.cpu.core_count.unwrap_or(0), format::frequency_khz(snap.cpu.max_freq_khz()));
    println!("  Display {} @ {:?} Hz · {} dpi", snap.display.resolution(), snap.display.refresh_hz, snap.display.density_dpi.unwrap_or(0));
    println!("  USB {} · Wi-Fi {:?}", snap.usb.label(), snap.network.wifi_ipv4());
    println!("  Temperatures: {}", snap.temperatures.len());
    let h = &snap.health;
    println!("  Battery health {} ({:?}) · {:?} cycles · storage {} ({})", format::percent(h.battery_percent()), h.battery_source, h.battery_cycles, h.storage_label(), h.storage_detail());
    let t = Instant::now();
    println!("  batterystats (fallback): {:?} in {:?}", adb.battery_capacity(&dev.serial), t.elapsed());

    let count = Arc::new(Mutex::new((0u32, None::<f64>)));
    let c2 = count.clone();
    let start = Instant::now();
    let mut stream = LiveStream::start(adb.clone(), dev.serial.clone(), 500, move |ev| match ev {
        LiveEvent::Snapshot(s) => {
            let mut g = c2.lock().unwrap();
            g.0 += 1;
            g.1 = s.cpu.load_percent.or(g.1);
            if g.0 % 4 == 1 {
                println!("  [{:>2}] display: {} (active {:?}, setting “{}”, state “{}”)", g.0, s.display.refresh_text(), s.display.refresh_hz, s.display.refresh_setting, s.display.screen_state);
                if let Some(w) = &s.wifi {
                    println!("       wifi: “{}” {:?} dBm · {} · {:?} MHz", w.ssid, w.rssi, w.link_speed, w.frequency_mhz);
                }
            }
        }
        LiveEvent::Notice(n) => eprintln!("notice: {n}"),
        LiveEvent::Stopped => {}
    });
    std::thread::sleep(Duration::from_secs(secs));
    stream.stop();
    let g = count.lock().unwrap();
    println!("\nLive: {} blocks in {:.1} s ({:.0} ms/block) · CPU load {}", g.0, start.elapsed().as_secs_f64(), start.elapsed().as_millis() as f64 / g.0.max(1) as f64, format::percent(g.1));
}
