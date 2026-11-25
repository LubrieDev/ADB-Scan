//! Quick controls against a real phone.
//!
//! `cargo run -p adbscan-core --example controls` only reads the state. With
//! `--apply` it changes harmless controls (Bluetooth, rotation, dark theme,
//! data saver, brightness, volume, Do Not Disturb) the same way the interface
//! does and **restores each one** to its original value.

use adbscan_core::adb::Adb;
use adbscan_core::controls::{Level, Toggle};
use std::time::Instant;

fn main() {
    let adb = Adb::locate(None).expect("adb");
    let Some(dev) = adb.devices().expect("adb devices").into_iter().find(|d| d.connected()) else {
        println!("No authorised device");
        return;
    };
    let serial = dev.serial;
    let t = Instant::now();
    let s = adb.control_state(&serial).expect("state");
    println!("State in {:?} (API {:?})", t.elapsed(), s.sdk);
    println!("{s:#?}");
    if !std::env::args().any(|a| a == "--apply") {
        return;
    }
    let sdk = s.sdk;
    for toggle in [Toggle::Bluetooth, Toggle::AutoRotate, Toggle::DarkMode, Toggle::DataSaver] {
        let Some(&orig) = toggle.state(&s).ok() else {
            println!("{}: no state, skipped", toggle.label());
            continue;
        };
        let t = Instant::now();
        let (applied, _) = adb.set_toggle(&serial, toggle, !orig, sdk).expect("change");
        let ms = t.elapsed().as_millis();
        let (restored, _) = adb.set_toggle(&serial, toggle, orig, sdk).expect("restore");
        println!("{:<22} {orig} → {} applied={applied} in {ms} ms · restored={restored}", toggle.label(), !orig);
    }
    let levels: Vec<(Level, Level, &str)> = vec![
        (Level::Brightness(s.brightness.ok().copied().unwrap_or(100) + 10), Level::Brightness(s.brightness.ok().copied().unwrap_or(100)), "brightness"),
        (Level::MediaVolume(s.media_volume.ok().map(|v| v.value).unwrap_or(5) + 20), Level::MediaVolume(s.media_volume.ok().map(|v| v.value).unwrap_or(5)), "media volume"),
        (Level::Dnd(adbscan_core::controls::Dnd::Off), Level::Dnd(s.dnd.ok().copied().unwrap_or(adbscan_core::controls::Dnd::Off)), "Do Not Disturb"),
    ];
    for (change, restore, name) in levels {
        let after = adb.set_level(&serial, change, &s).expect("change");
        let back = adb.set_level(&serial, restore, &after).expect("restore");
        println!(
            "{name:<22} brightness {:?}→{:?} · volume {:?}→{:?} · dnd {:?}→{:?}",
            after.brightness.ok(),
            back.brightness.ok(),
            after.media_volume.ok().map(|v| v.value),
            back.media_volume.ok().map(|v| v.value),
            after.dnd.ok(),
            back.dnd.ok()
        );
    }
    // Ring volume: on the REDMAGIC Android ignores it and it must be reported,
    // not faked as a success.
    let ring = s.ring_volume.ok().map(|v| v.value).unwrap_or(1);
    match adb.set_level(&serial, Level::RingVolume((ring - 1).max(0)), &s) {
        Ok(st) => {
            println!("ring volume            applied: {:?}", st.ring_volume.ok().map(|v| v.value));
            let _ = adb.set_level(&serial, Level::RingVolume(ring), &st);
        }
        Err(e) => println!("ring volume            not applied: {e}"),
    }
    let fin = adb.control_state(&serial).expect("final state");
    let same = |a: &adbscan_core::controls::ControlState, b: &adbscan_core::controls::ControlState| {
        (
            a.wifi.clone(),
            a.mobile_data.clone(),
            a.bluetooth.clone(),
            a.airplane.clone(),
            a.location.clone(),
            a.dark_mode.clone(),
            a.auto_rotate.clone(),
            a.data_saver.clone(),
            a.dnd.clone(),
            a.brightness.clone(),
            a.media_volume.clone(),
        ) == (
            b.wifi.clone(),
            b.mobile_data.clone(),
            b.bluetooth.clone(),
            b.airplane.clone(),
            b.location.clone(),
            b.dark_mode.clone(),
            b.auto_rotate.clone(),
            b.data_saver.clone(),
            b.dnd.clone(),
            b.brightness.clone(),
            b.media_volume.clone(),
        )
    };
    println!("\nFinal state equal to the initial one: {}", same(&s, &fin));
}
