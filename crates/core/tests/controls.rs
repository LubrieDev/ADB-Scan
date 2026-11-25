//! Quick controls with the real output of a REDMAGIC 11S Pro (Android 16). From
//! `settings list` only the keys in use are kept: the rest include phone
//! identifiers (`android_id`…).

use adbscan_core::controls::*;
use adbscan_core::probe::Probe;

fn fixture() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/controls_state.txt");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn state_real() {
    let s = parse_state(&fixture());
    assert_eq!(s.sdk, Some(36));
    assert_eq!(s.wifi, Probe::Ok(true));
    // Per SIM: `mobile_data1=0` even if the global one says 1.
    assert_eq!(s.mobile_data, Probe::Ok(false));
    assert_eq!(s.bluetooth, Probe::Ok(false));
    assert_eq!(s.airplane, Probe::Ok(false));
    assert_eq!(s.location, Probe::Ok(false));
    assert_eq!(s.dark_mode, Probe::Ok(true));
    assert_eq!(s.auto_rotate, Probe::Ok(true));
    assert_eq!(s.auto_brightness, Probe::Ok(false));
    assert_eq!(s.battery_saver, Probe::Ok(false));
    assert_eq!(s.data_saver, Probe::Ok(false));
    assert_eq!(s.dnd, Probe::Ok(Dnd::Priority));
    assert_eq!(s.brightness, Probe::Ok(26));
    assert_eq!(s.screen_timeout_ms, Probe::Ok(600_000));
    assert_eq!(s.media_volume, Probe::Ok(Volume { value: 40, min: 0, max: 150 }));
    assert_eq!(s.ring_volume, Probe::Ok(Volume { value: 15, min: 0, max: 15 }));
    assert_eq!(s.nfc, Probe::Ok(false));
    assert_eq!(s.hotspot, Probe::Ok(false));
    assert_eq!(s.charging, Some(true));
}

#[test]
fn hotspot_and_fallbacks() {
    let text = "@@@SDK@@@\n29\n@@@GLOBAL@@@\nwifi_on=2\nairplane_mode_on=1\nmobile_data=1\n@@@SYSTEM@@@\n@@@SECURE@@@\nlocation_mode=3\n\
                @@@WIFI@@@\nUnknown command: status\n@@@AIRPLANE@@@\nUnknown command: airplane-mode\n@@@LOCATION@@@\nUnknown command\n\
                @@@TETHER@@@\n  Tether state:\n    wlan1 - TetheredState - lastError = 0\n  Hardware offload:\n@@@END@@@\n";
    let s = parse_state(text);
    // Android 10: without `cmd`, the settings are used.
    assert_eq!(s.wifi, Probe::Ok(true));
    assert_eq!(s.airplane, Probe::Ok(true));
    assert_eq!(s.location, Probe::Ok(true));
    assert_eq!(s.mobile_data, Probe::Ok(true));
    assert_eq!(s.hotspot, Probe::Ok(true));
    // Settings read without the key: no value is made up.
    assert_eq!(s.auto_rotate, Probe::Failed("could not read the settings".into()));
    assert_eq!(s.bluetooth, Probe::Missing);
    // Missing section.
    assert!(matches!(s.nfc, Probe::Failed(_)));
}

#[test]
fn commands_by_version() {
    assert_eq!(Toggle::Wifi.command(false, Some(36)).unwrap(), "cmd wifi set-wifi-enabled disabled");
    assert_eq!(Toggle::Wifi.command(true, Some(29)).unwrap(), "svc wifi enable");
    assert_eq!(Toggle::MobileData.command(true, Some(36)).unwrap(), "cmd phone data enable");
    assert_eq!(Toggle::MobileData.command(true, Some(30)).unwrap(), "svc data enable");
    assert_eq!(Toggle::Bluetooth.command(true, Some(36)).unwrap(), "cmd bluetooth_manager enable");
    assert_eq!(Toggle::Bluetooth.command(false, Some(31)).unwrap(), "svc bluetooth disable");
    assert_eq!(Toggle::Airplane.command(true, Some(36)).unwrap(), "cmd connectivity airplane-mode enable");
    assert_eq!(Toggle::Airplane.command(true, Some(29)), None);
    assert_eq!(Toggle::Location.command(true, Some(36)).unwrap(), "cmd location set-location-enabled true");
    assert_eq!(Toggle::DarkMode.command(false, Some(36)).unwrap(), "cmd uimode night no");
    assert_eq!(Toggle::BatterySaver.command(true, Some(36)).unwrap(), "cmd power set-mode 1");
    assert_eq!(Toggle::DataSaver.command(true, Some(36)).unwrap(), "cmd netpolicy set restrict-background true");
}

#[test]
fn levels_are_clamped() {
    let s = parse_state(&fixture());
    assert_eq!(Level::Brightness(999).command(&s), "settings put system screen_brightness 255");
    assert_eq!(Level::Brightness(0).command(&s), "settings put system screen_brightness 1");
    assert_eq!(Level::MediaVolume(500).command(&s), "cmd media_session volume --stream 3 --set 150");
    assert_eq!(Level::RingVolume(-3).command(&s), "cmd media_session volume --stream 2 --set 0");
    assert_eq!(Level::ScreenTimeout(1).command(&s), "settings put system screen_off_timeout 10000");
    assert_eq!(Level::Dnd(Dnd::Silence).command(&s), "cmd notification set_dnd none");
}

#[test]
fn state_script_only_reads() {
    // Whole words: “is-location-enabled” is a read.
    let words: Vec<&str> = STATE_SCRIPT.split(|c: char| c.is_whitespace() || c == ';' || c == '|').collect();
    for forbidden in ["put", "enable", "disable", "--set", "set", "svc", "start", "input", "set-mode", "set-wifi-enabled", "set-location-enabled", "set_dnd"] {
        assert!(!words.contains(&forbidden), "“{forbidden}” in the read script");
    }
}

#[test]
fn settings_screens_are_public_actions() {
    for (_, action) in SETTINGS_SCREENS {
        assert!(action.starts_with("android.settings.") || action.starts_with("android.intent.action."), "{action}");
        assert!(action.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_'));
    }
}

#[test]
fn hints() {
    let s = parse_state(&fixture());
    assert!(not_applied_hint(Toggle::BatterySaver, true, &s).contains("while the phone is charging"));
    assert_eq!(parse_volume("[V] volume is 7 in range [1..15]"), Some(Volume { value: 7, min: 1, max: 15 }));
}

#[test]
fn volume_keys() {
    let up = volume_keys_script(3, 40, 80);
    assert!(up.contains("KEYCODE_VOLUME_UP") && up.contains("[ $v -ge 80 ]") && up.contains("--stream 3"));
    let down = volume_keys_script(2, 15, 10);
    assert!(down.contains("KEYCODE_VOLUME_DOWN") && down.contains("[ $v -le 10 ]") && down.contains("--stream 2"));
    // Never more than 40 presses, and it only reads the volume (it does not set
    // it).
    assert!(up.contains("-lt 40") && !up.contains("--set"));
}
