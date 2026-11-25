//! Remote, watcher, notifications and photos against a real phone.
//!
//! `cargo run -p adbscan-core --example remote` **only reads** and does not
//! show private content: for notifications it gives counts (not titles or
//! texts) and for photos, how many there are and their total size.

use adbscan_core::adb::Adb;
use std::time::Instant;

fn main() {
    let adb = Adb::locate(None).expect("adb");
    let Some(dev) = adb.devices().expect("adb devices").into_iter().find(|d| d.connected()) else {
        println!("No authorised device");
        return;
    };
    let serial = dev.serial;

    let t = Instant::now();
    match adb.remote_status(&serial) {
        Ok((call, playing)) => {
            println!("Call: {} ({:?})", call.label(), t.elapsed());
            match playing {
                Some(p) => {
                    println!("Playing: app {}, {} · title {} chars · artist {} chars", p.package, if p.playing { "playing" } else { "paused" }, p.title.chars().count(), p.artist.chars().count())
                }
                None => println!("Playing: nothing"),
            }
        }
        Err(e) => println!("State: error {e}"),
    }

    let t = Instant::now();
    let top = adb.top_cpu(&serial, 3);
    println!("Top CPU ({:?}): {top:?}", t.elapsed());

    let t = Instant::now();
    match adb.phone_notifications(&serial) {
        Ok(list) => {
            let shown = list.iter().filter(|n| n.worth_showing()).count();
            let ongoing = list.iter().filter(|n| n.ongoing).count();
            let summaries = list.iter().filter(|n| n.summary).count();
            let with_text = list.iter().filter(|n| !n.title.is_empty() || !n.text.is_empty()).count();
            println!("Notifications ({:?}): {} in total · {with_text} with text · {ongoing} ongoing · {summaries} summaries · {shown} would alert", t.elapsed(), list.len());
        }
        Err(e) => println!("Notifications: error {e}"),
    }

    let t = Instant::now();
    match adb.list_media(&serial) {
        Ok(list) => {
            let bytes: u64 = list.iter().map(|m| m.size).sum();
            let folders: std::collections::BTreeSet<&str> = list.iter().filter_map(|m| m.path.rsplit_once('/').map(|(d, _)| d)).collect();
            println!("Photos and videos ({:?}): {} files, {:.1} GiB, {} folders", t.elapsed(), list.len(), bytes as f64 / 1_073_741_824.0, folders.len());
        }
        Err(e) => println!("Photos: error {e}"),
    }
}
