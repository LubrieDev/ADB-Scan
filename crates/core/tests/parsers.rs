//! Tests with real outputs captured from a REDMAGIC 11S Pro (Android 16).

use adbscan_core::models::*;
use adbscan_core::parsers::{self, StorageKind};

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

// -- devices -------------------------------------------------------------

#[test]
fn real_devices() {
    let devices = parsers::parse_devices(&fixture("devices.txt"));
    assert_eq!(devices.len(), 1);
    let d = &devices[0];
    assert_eq!(d.serial, "A1B2C3D4E5F6");
    assert_eq!(d.state, DeviceState::Device);
    assert_eq!(d.model, "NX809J");
    assert_eq!(d.product, "NX809J-UN");
    assert_eq!(d.transport_id, "1");
    assert!(d.connected());
}

#[test]
fn unauthorized_offline_no_permissions() {
    let text = "List of devices attached\n\
                ABC123\tunauthorized usb:1-1\n\
                emulator-5554\toffline\n\
                XYZ\tno permissions (missing udev rules?); see [http://x]\n\
                192.168.1.5:5555\tdevice product:p model:Pixel_8 device:d transport_id:3\n";
    let devices = parsers::parse_devices(text);
    let states: Vec<_> = devices.iter().map(|d| d.state).collect();
    assert_eq!(states, vec![DeviceState::Unauthorized, DeviceState::Offline, DeviceState::NoPermissions, DeviceState::Device]);
    assert_eq!(devices[1].transport, Transport::Emulator);
    assert_eq!(devices[3].transport, Transport::Tcp);
    assert_eq!(devices[3].display_name(), "Pixel 8");
}

#[test]
fn unknown_state_is_kept_for_diagnostics() {
    let devices = parsers::parse_devices("ABC\thost\n");
    assert_eq!(devices[0].state, DeviceState::Unknown);
    assert_eq!(devices[0].state_label(), "host");
}

#[test]
fn empty_devices() {
    assert!(parsers::parse_devices("List of devices attached\n\n").is_empty());
}

#[test]
fn fastboot_devices_and_vars() {
    let d = parsers::parse_fastboot_devices("A1B2C3D4E5F6\tfastboot\nABCD fastbootd\n");
    assert_eq!(d.len(), 2);
    assert_eq!(d[0].state, DeviceState::Fastboot);
    assert_eq!(d[1].state, DeviceState::Fastbootd);
    let vars = parsers::parse_fastboot_vars("(bootloader) current-slot:a\n(bootloader) unlocked:no\n(bootloader) partition-size:boot_a: 0x6000000\nall: \nFinished. Total time: 0.050s\n");
    assert_eq!(vars.get("current-slot").map(String::as_str), Some("a"));
    assert_eq!(vars.get("unlocked").map(String::as_str), Some("no"));
    assert_eq!(vars.get("partition-size").map(String::as_str), Some("boot_a: 0x6000000"));
}

// -- properties, battery, memory ----------------------------------------------

#[test]
fn real_props() {
    let props = parsers::parse_getprop(&fixture("props.txt"));
    assert!(props.len() > 100);
    assert_eq!(props["ro.product.model"], "NX809J");
    assert_eq!(props["ro.build.version.release"], "16");
    assert!(!props.contains_key("Physical size"));
    assert_eq!(parsers::parse_getprop("[a.b]: []\n")["a.b"], "");
}

#[test]
fn real_battery() {
    let b = parsers::parse_battery(&fixture("battery.txt"));
    assert_eq!(b.level, Some(78));
    assert_eq!(b.scale, 100);
    assert!(b.ac_powered);
    assert!(!b.usb_powered);
    assert!((b.temperature_c.unwrap() - 28.0).abs() < 1e-9);
    assert_eq!(b.percentage(), Some(78.0));
    assert_eq!(b.charge_counter_mah, Some(5448));
}

#[test]
fn battery_edge_cases() {
    let b = parsers::parse_battery("level: 50\nscale: 100\ntemperature: 305\n");
    assert_eq!(b.percentage(), Some(50.0));
    assert!((b.temperature_c.unwrap() - 30.5).abs() < 1e-9);
    let empty = parsers::parse_battery("");
    assert_eq!(empty.percentage(), None);
    assert_eq!(empty.scale, 100);
}

#[test]
fn real_meminfo() {
    let m = parsers::parse_meminfo(&fixture("meminfo.txt"));
    assert!(m.total_kib.unwrap() > 0);
    assert!(m.available_kib.is_some());
    assert!(m.used_kib().is_some());
    let p = m.percent().unwrap();
    assert!(p > 0.0 && p <= 100.0);
    let m = parsers::parse_meminfo("MemTotal: 1000 kB\nMemFree: 200 kB\nCached: 300 kB\n");
    assert_eq!(m.available_kib, Some(500));
}

// -- storage, thermals, CPU -------------------------------------------------

#[test]
fn real_df() {
    let entries = parsers::parse_df(&fixture("df.txt"));
    assert!(!entries.is_empty());
    assert!(parsers::select_storage(&entries, StorageKind::Data).total_kib.is_some());
    assert_eq!(parsers::select_storage(&entries, StorageKind::Shared).label, "Shared storage");
    assert!(parsers::parse_df("Filesystem 1K-blocks Used Available Use% Mounted on\nbad line\n").is_empty());
}

#[test]
fn external_sd_detection() {
    let entries = parsers::parse_df("/dev/fuse 100 50 50 50% /storage/emulated/0\n/dev/fuse 200 100 100 50% /storage/1A2B-3C4D\n");
    let sd = parsers::external_sd_mounts(&entries);
    assert_eq!(sd.len(), 1);
    assert_eq!(sd[0].mount, "/storage/1A2B-3C4D");
}

#[test]
fn real_thermal() {
    let (temps, status) = parsers::parse_thermalservice(&fixture("thermal.txt"));
    assert!(!temps.is_empty());
    assert_eq!(status, Some(0));
    let names: Vec<_> = temps.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"battery"));
    assert!(!names.contains(&"vbat"));
    assert!(!names.contains(&"ibat"));
    assert!(temps.iter().all(|t| t.value_c < 200.0));
}

/// Android 11, Unisoc (moto e20): no type (`mType=-1`), and each name ends
/// in a line break, so `, mStatus=0}` comes on the next line. The live script
/// keeps those lines.
#[test]
fn unisoc_thermal() {
    let text = "Thermal Status: 0\nCached temperatures:\n\tTemperature{mValue=24.9, mType=-1, mName=soc-thmzone\n, mStatus=0}\nHAL Ready: true\n\
Current temperatures from HAL:\n\tTemperature{mValue=37.4, mType=-1, mName=cputop1-thmzone\n, mStatus=0}\n\tTemperature{mValue=38.1, mType=-1, mName=ank2-thmzone\n, mStatus=0}\n\
\tTemperature{mValue=36.3, mType=-1, mName=prometheus6-thmzone\n, mStatus=0}\n\tTemperature{mValue=37.7, mType=-1, mName=soc-thmzone\n, mStatus=0}\n\
\tTemperature{mValue=35.5, mType=-1, mName=gpu-thmzone\n, mStatus=0}\n\tTemperature{mValue=29.8, mType=-1, mName=board-thmzone\n, mStatus=0}\n";
    let (temps, status) = parsers::parse_thermalservice(text);
    assert_eq!(status, Some(0));
    // The current block wins over the cached one.
    assert_eq!(temps.len(), 6);
    let kind = |n: &str| temps.iter().find(|t| t.name == n).map(|t| t.kind()).unwrap();
    assert_eq!(kind("ank2-thmzone"), "CPU");
    assert_eq!(kind("prometheus6-thmzone"), "CPU");
    assert_eq!(kind("cputop1-thmzone"), "CPU");
    assert_eq!(kind("soc-thmzone"), "SoC");
    assert_eq!(kind("gpu-thmzone"), "GPU");
    let snap = adbscan_core::models::Snapshot { temperatures: temps, ..Default::default() };
    assert_eq!(snap.cpu_temperature(), Some(38.1));
}

/// Android 11 (moto e20): `df -k /data` answers with the obb bind mount of the
/// same partition; the plain `df -k` line for `/data` is added by the script.
#[test]
fn android_11_data_partition() {
    let text = "Filesystem       1K-blocks     Used Available Use% Mounted on\n/dev/block/dm-14  25077760 14801524  10071436  60% /storage/emulated/0/Android/obb\n\
/dev/fuse         25077760 14801524  10071436  60% /storage/emulated\n/dev/block/dm-14       25077760 14801552  10071408  60% /data\n";
    let entries = parsers::parse_df(text);
    let data = parsers::select_storage(&entries, StorageKind::Data);
    assert_eq!(data.mount, "/data");
    assert_eq!(data.total_kib, Some(25077760));
    assert!(parsers::select_storage(&entries, StorageKind::Shared).total_kib.is_some());
}

#[test]
fn real_cpuinfo() {
    let cpu = parsers::parse_cpuinfo(&fixture("cpuinfo.txt"));
    assert_eq!(cpu.core_count, Some(8));
    assert_eq!(cpu.model_name, "");
    let cpu = parsers::parse_cpuinfo("processor: 0\nHardware: Qualcomm SM8650\nprocessor: 1\n");
    assert_eq!(cpu.core_count, Some(2));
    assert_eq!(cpu.model_name, "Qualcomm SM8650");
}

#[test]
fn cpufreq_blocks() {
    let text = "###CORE0\ncur=787200\nmax=3628800\nmin=300000\ngov=schedutil\n###CORE1\ncur=300000\n";
    let cores = parsers::parse_cpu_freq_blocks(text);
    assert_eq!(cores.keys().copied().collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(cores[&0]["cur"], "787200");
    let mut cpu = CpuInfo::default();
    parsers::apply_cpu_freqs(&mut cpu, text);
    assert_eq!(cpu.cores.len(), 2);
    assert_eq!(cpu.core_count, Some(2));
    assert_eq!(cpu.cores[0].governor, "schedutil");
}

#[test]
fn proc_stat_and_load() {
    let prev = parsers::parse_proc_stat("cpu  100 0 100 800 0 0 0 0 0 0\n");
    let cur = parsers::parse_proc_stat("cpu  150 0 150 900 0 0 0 0 0 0\n");
    assert_eq!(prev, Some((200, 1000)));
    assert_eq!(cur, Some((300, 1200)));
    assert!((parsers::compute_cpu_load(prev, cur).unwrap() - 50.0).abs() < 1e-9);
    assert_eq!(parsers::compute_cpu_load(None, cur), None);
    assert_eq!(parsers::parse_loadavg("1.5 2.0 3.0"), Some((1.5, 2.0, 3.0)));
}

// -- processes, packages, files ----------------------------------------------------

#[test]
fn real_ps() {
    let ps = parsers::parse_ps(&fixture("ps.txt"));
    assert!(ps.len() > 50);
    let init = ps.iter().find(|p| p.pid == 1).unwrap();
    assert_eq!(init.name, "init");
    assert_eq!(init.user, "root");
}

#[test]
fn real_package_list() {
    let pkgs = parsers::parse_package_list(&fixture("packages.txt"));
    assert!(pkgs.len() > 100);
    assert!(pkgs["com.android.settings"].ends_with(".apk"));
}

#[test]
fn package_records_with_versions() {
    let text = "package:/system/app/A/A.apk=com.example.a versionCode:12\n\
                package:/data/app/B/base.apk=com.example.b versionCode:34\n\
                package:/data/app/~~x==/c-y==/base.apk=com.example.c versionCode:5\n";
    let r = parsers::parse_package_records(text);
    assert_eq!(r["com.example.a"].version_code, "12");
    assert_eq!(r["com.example.b"].path, "/data/app/B/base.apk");
    // Paths with '=' (recent installs) are kept whole.
    assert_eq!(r["com.example.c"].path, "/data/app/~~x==/c-y==/base.apk");
}

#[test]
fn package_names() {
    let names = parsers::parse_package_names("package:com.a\npackage:/x/base.apk=com.b\n\n");
    assert!(names.contains("com.a"));
    assert!(names.contains("com.b"));
}

#[test]
fn real_package_dump() {
    let info = parsers::parse_package_dump(&fixture("package_dump.txt"), "com.android.settings", "");
    assert_eq!(info.version_code, "160000");
    assert_eq!(info.version_name, "16.5.000.2610191704");
    assert_eq!(info.min_sdk, "31");
    assert_eq!(info.target_sdk, "36");
    assert!(info.is_system);
    assert!(info.flags.iter().any(|f| f == "SYSTEM"));
    assert!(!info.requested_permissions.is_empty());
}

#[test]
fn uid_map() {
    let text = "  Package [com.example.a] (abc):\n    userId=10123\n  Package [com.example.b] (def):\n    userId=10124\n";
    let m = parsers::package_uid_map(text);
    assert_eq!(m["com.example.a"], 10123);
    assert_eq!(m["com.example.b"], 10124);
}

#[test]
fn real_ls() {
    let entries = parsers::parse_ls(&fixture("ls_sdcard.txt"));
    let link = &entries[0];
    assert!(link.is_link);
    assert_eq!(link.link_target, "/storage/self/primary");
    assert_eq!(link.name, "/sdcard");
}

#[test]
fn ls_regular_and_symlink() {
    let text = "drwxr-xr-x 2 root root 4096 2026-01-02 03:04 DCIM\n\
                lrwxrwxrwx 1 root root   21 2026-01-02 03:04 link -> /sdcard/real\n\
                -rw-r--r-- 1 root root 1234 2026-01-02 03:04 file.txt\n\
                -rw-r--r-- 1 root root 1 2026-01-02 03:04 name with spaces.txt\n";
    let e = parsers::parse_ls(text);
    assert_eq!(e[0].name, "DCIM");
    assert!(e[0].is_dir);
    let link = e.iter().find(|x| x.name == "link").unwrap();
    assert_eq!(link.link_target, "/sdcard/real");
    assert!(e.iter().any(|x| x.name == "name with spaces.txt"));
}

// -- sensors, network, services, display, USB, logcat
// -------------------------------------

#[test]
fn real_sensors() {
    let s = parsers::parse_sensors(&fixture("sensors.txt"));
    assert!(s.len() > 20);
    let acc = s.iter().find(|x| x.name.contains("Accelerometer")).unwrap();
    assert_eq!(acc.kind, "android.sensor.accelerometer");
    assert_eq!(acc.type_code, 1);
    assert_eq!(acc.max_rate, "479.85");
}

#[test]
fn real_ip_addr() {
    let ifs = parsers::parse_ip_addr(&fixture("ipaddr.txt"));
    let names: Vec<_> = ifs.iter().map(|i| i.name.as_str()).collect();
    assert!(names.contains(&"lo"));
    assert!(names.contains(&"wlan0"));
    let wlan = ifs.iter().find(|i| i.name == "wlan0").unwrap();
    assert!(!wlan.addresses.is_empty());
    assert!(!wlan.mac.is_empty());
    assert_eq!(wlan.state, "UP");
}

#[test]
fn real_ip_route() {
    let r = parsers::parse_ip_route(&fixture("iproute.txt"));
    assert!(!r.is_empty());
    assert!(r.iter().any(|x| x.dev.is_some()));
}

#[test]
fn wifi_status() {
    let text = "Wifi is enabled\nWifi is connected to \"MiRed\"\n\
                WifiInfo: SSID: \"MiRed\", BSSID: aa:bb:cc:dd:ee:ff, Supplicant state: COMPLETED, RSSI: -54, \
                Link speed: 866Mbps, Frequency: 5180MHz, IP: /192.168.1.20\n";
    let s = parsers::parse_wifi_status(text);
    assert_eq!(s.enabled, Some(true));
    assert_eq!(s.ssid, "MiRed");
    assert_eq!(s.ip, "192.168.1.20");
    assert_eq!(s.rssi, Some(-54));
    assert_eq!(s.frequency_mhz, Some(5180));
}

#[test]
fn real_cmd_wifi_status() {
    let s = parsers::parse_wifi_status(&fixture("cmd_wifi_status.txt"));
    assert_eq!(s.enabled, Some(true));
    assert_eq!(s.ssid, "Hogar");
    assert_eq!(s.ip, "192.168.1.228");
    assert_eq!(s.bssid, "aa:bb:cc:dd:ee:ff");
    assert_eq!(s.state, "COMPLETED");
    assert_eq!(s.security, "WPA/WPA2-Personal");
    assert_eq!(s.standard, "Wi-Fi 5 (802.11ac)");
    assert_eq!(s.link_speed, "234 Mb/s");
    assert_eq!(s.frequency_mhz, Some(5745));
    assert!(s.rssi.is_some_and(|r| r < 0));
}

#[test]
fn real_services() {
    let s = parsers::parse_services(&fixture("services.txt"));
    assert!(s.len() > 100);
    assert!(s.iter().any(|x| x.name == "activity"));
}

#[test]
fn real_display() {
    let props = fixture("props.txt");
    let info = parsers::parse_display_info(&props, &props, &fixture("display.txt"));
    assert_eq!(info.width, Some(1216));
    assert_eq!(info.height, Some(2688));
    assert_eq!(info.density_dpi, Some(520));
    assert_eq!(info.density_bucket, "xxhdpi");
    assert_eq!(parsers::density_bucket(Some(160)), "mdpi");
    assert_eq!(parsers::density_bucket(None), "");
}

#[test]
fn display_override() {
    let (w, h, ov) = parsers::parse_wm_size("Physical size: 1080x2400\nOverride size: 720x1600\n");
    assert_eq!((w, h), (Some(720), Some(1600)));
    assert!(!ov.is_empty());
    assert_eq!(parsers::parse_brightness("128\n"), Some(128));
    assert_eq!(parsers::parse_brightness("auto"), None);
}

#[test]
fn real_usb_dump() {
    let u = parsers::parse_usb_dump(&fixture("usb.txt"));
    assert_eq!(u.connected, Some(true));
    assert_eq!(u.configured, Some(true));
    assert_eq!(u.charging, Some(false));
    assert_eq!(u.host_connected, Some(false));
    assert_eq!(u.state, "CONFIGURED");
    let p = parsers::parse_usb_props("mtp,adb", "adb", "CONFIGURED");
    assert_eq!(p.connected, Some(true));
    assert_eq!(p.functions, "mtp,adb");
    assert_eq!(p.label(), "Connected (ADB + MTP)");
}

#[test]
fn logcat_lines() {
    let e = parsers::parse_logcat_line("11-13 21:42:24.070  1234  5678 I ActivityManager: Start proc 1234");
    assert_eq!(e.level, LogLevel::Info);
    assert_eq!(e.pid, "1234");
    assert_eq!(e.tid, "5678");
    assert_eq!(e.tag, "ActivityManager");
    assert!(e.message.starts_with("Start proc"));
    let raw = parsers::parse_logcat_line("\tat com.example.Foo(Foo.java:10)");
    assert_eq!(raw.level, LogLevel::Raw);
    assert_eq!(raw.message, "\tat com.example.Foo(Foo.java:10)");
    let all = parsers::parse_logcat_text("--------- beginning of main\n11-13 21:42:24.070  1  2 D Tag: hola\n");
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].tag, "Tag");
}

#[test]
fn sections_split() {
    let s = parsers::split_sections("@@@A@@@\nuno\ndos\n@@@B2@@@\ntres\n");
    assert_eq!(s["A"], "uno\ndos");
    assert_eq!(s["B2"], "tres");
}

#[test]
fn humanize() {
    let d = AdbDevice { model: "Pixel_8".into(), ..AdbDevice::new("X", DeviceState::Device) };
    let t = parsers::humanize_adb_device(&d);
    assert!(t.contains("Pixel 8"));
    assert!(t.contains("Connected"));
}

#[test]
fn top_cpu_and_kernel_threads() {
    let top = "Tasks: 922 total\n  PID[%CPU]NAME\n 1047 13.3 top\n32483  2.0 com.google.android.youtube\n";
    let m = parsers::parse_top_cpu(top);
    assert_eq!(m.get(&1047), Some(&13.3));
    assert_eq!(m.get(&32483), Some(&2.0));
    let ps = parsers::parse_ps(&fixture("ps.txt"));
    assert!(ps.iter().any(|p| p.is_kernel_thread()));
    assert!(ps.iter().any(|p| !p.is_kernel_thread() && p.name == "init"));
    let app = ProcessInfo { name: "com.app:remote".into(), ..Default::default() };
    assert_eq!(app.package(), "com.app");
}

#[test]
fn real_gpu_and_fps() {
    let gpu = parsers::parse_gpubusy(&fixture("gpubusy.txt")).expect("gpubusy");
    assert!((0.0..=100.0).contains(&gpu));
    let focus = parsers::parse_focus(&fixture("focus.txt")).expect("focus");
    assert!(focus.starts_with("com.google.android.youtube/"));
    let layers = parsers::parse_layer_list(&fixture("sf_list.txt"));
    assert!(layers.iter().any(|l| l.starts_with("NavigationBar0#")));
    let cands = parsers::fps_layers(&layers, &focus);
    assert!(!cands.is_empty());
    assert!(cands.iter().all(|c| !c.starts_with("Background for") && !c.starts_with("Bounds for")));
    let presents = parsers::parse_latency(&fixture("sf_latency.txt"));
    assert!(presents.len() > 10);
    let (fps, newest) = parsers::fps_from_presents(&presents).unwrap();
    assert!(fps >= 1.0 && newest > 0);
    assert_eq!(parsers::fps_from_presents(&[1_000_000_000, 1_500_000_000, 1_900_000_000, 2_000_000_000]).unwrap().0, 3.0);
}

#[test]
fn bulk_package_data() {
    let i = parsers::parse_installers("package:com.a  installer=com.android.vending\npackage:com.b  installer=null\n");
    assert_eq!(i["com.a"], "com.android.vending");
    assert_eq!(i["com.b"], "");
    let d = parsers::parse_diskstats("Latency: 1ms\nPackage Names: [\"com.a\",\"com.b\"]\nApp Sizes: [100,200]\nApp Data Sizes: [10,20]\nCache Sizes: [1,2]\n");
    assert_eq!(d["com.a"].total(), 111);
    assert_eq!((d["com.b"].app, d["com.b"].data, d["com.b"].cache), (200, 20, 2));
    let m = parsers::parse_stat_mtimes("1784894735 /data/app/x/base.apk\n");
    assert_eq!(m["/data/app/x/base.apk"], 1784894735);
}

#[test]
fn refresh_rate_is_the_active_mode() {
    // Active mode 4 (90 Hz); the first fps= in the list is 60 (mode 1).
    let line = "mBaseDisplayInfo=DisplayInfo{\"Pantalla\", mode 4, defaultMode 2, supportedModes [{id=1, width=1216, height=2688, fps=60.000004, vsync=60.0}, {id=4, width=1216, height=2688, fps=90.0, vsync=90.0}], state ON}";
    assert_eq!(parsers::active_refresh_in_info(line), Some(90.0));
    let line2 = "mBaseDisplayInfo=DisplayInfo{mode 1, renderFrameRate 120.00001, supportedModes [{id=1, fps=60.0}]}";
    assert_eq!(parsers::active_refresh_in_info(line2).map(|v| v.round()), Some(120.0));
    let info = parsers::parse_display_info("", "", &fixture("display.txt"));
    assert!(info.refresh_hz.is_some());
    let (hz, state) = parsers::parse_refresh_live("mActiveRenderFrameRate=90.0\nmScreenState=ON\n");
    assert_eq!((hz, state.as_str()), (Some(90.0), "ON"));
    assert_eq!(parsers::parse_refresh_setting("null\nnull\n2\nauto,60,90,120,144\n"), "90 Hz");
    assert_eq!(parsers::parse_refresh_setting("null\nnull\n0\nauto,60,90,120,144\n"), "Automatic");
    assert_eq!(parsers::parse_refresh_setting("120.0\n60.0\nnull\nnull\n"), "60–120 Hz");
    assert_eq!(parsers::parse_refresh_setting("null\nnull\nnull\nnull\n"), "");
}

#[test]
fn refresh_text_explains_differences() {
    let mut d = DisplayInfo { refresh_hz: Some(60.000004), refresh_setting: "90 Hz".into(), screen_state: "DOZE".into(), ..Default::default() };
    assert_eq!(d.refresh_text(), "60 Hz · set to 90 Hz · screen dozing");
    d.refresh_hz = Some(90.0);
    d.screen_state = "ON".into();
    assert_eq!(d.refresh_text(), "90 Hz");
}

// -- component health -------------------------------------------------

#[test]
fn real_wear_battery_from_sysfs_and_ufs_denied() {
    let h = parsers::parse_wear(&fixture("wear.txt"));
    assert!(h.probed);
    assert_eq!(h.battery_cycles, Some(53));
    assert_eq!(h.battery_full_mah, Some(7360));
    assert_eq!(h.battery_design_mah, Some(7500));
    assert_eq!(h.battery_source, CapacitySource::Sysfs);
    let pct = h.battery_percent().unwrap();
    assert!((pct - 98.13).abs() < 0.01, "{pct}");
    assert_eq!(h.battery_level(), Some(HealthLevel::Good));
    // The UFS descriptor exists but cannot be read without root.
    assert_eq!(h.storage, StorageWear::Denied { kind: "UFS".into() });
    assert_eq!(h.storage_range(), None);
    assert_eq!(h.storage_label(), "—");
    assert!(h.storage_detail().contains("root"));
}

#[test]
fn wear_storage_levels_follow_jedec() {
    let h = parsers::parse_wear("kind=UFS\nlife=0x01 0x03\neol=0x01\n");
    // The worse of A and B counts: 0x03 = 20–30 % used.
    assert_eq!(h.storage_range(), Some((70, 80)));
    assert_eq!(h.storage_label(), "70–80 %");
    assert_eq!(h.storage_level(), Some(HealthLevel::Good));
    let emmc = parsers::parse_wear("kind=eMMC\nlife=0x0a 0x09\neol=0x02\n");
    assert_eq!(emmc.storage_range(), Some((0, 10)));
    assert_eq!(emmc.storage_level(), Some(HealthLevel::Poor));
    let gone = parsers::parse_wear("kind=UFS\nlife=0x0B 0x01\neol=0x03\n");
    assert_eq!(gone.storage_label(), "Exhausted");
    // No descriptor and no readable battery: nothing made up.
    let none = parsers::parse_wear("batt_cycle_count=\nbatt_charge_full=\nbatt_charge_full_design=\n");
    assert_eq!(none.storage, StorageWear::Unknown);
    assert_eq!(none.battery_percent(), None);
    assert_eq!(none.battery_cycles, None);
    assert_eq!(none.battery_source, CapacitySource::None);
}

#[test]
fn wear_rejects_incoherent_battery_values() {
    // Capacity in mAh (not µAh), accepted.
    let mah = parsers::parse_wear("batt_charge_full=4100\nbatt_charge_full_design=5000\n");
    assert_eq!(mah.battery_percent().map(|p| p.round()), Some(82.0));
    // Design at 0 or absurd capacity: no percentage.
    for text in ["batt_charge_full=4000000\nbatt_charge_full_design=0\n", "batt_charge_full=50\nbatt_charge_full_design=5000000\n"] {
        assert_eq!(parsers::parse_wear(text).battery_percent(), None, "{text}");
    }
    // A new battery can measure slightly more than the design capacity: capped
    // at 100 %.
    let new = parsers::parse_wear("batt_charge_full=5100000\nbatt_charge_full_design=5000000\n");
    assert_eq!(new.battery_percent(), Some(100.0));
    let neg = parsers::parse_wear("batt_cycle_count=-1\n");
    assert_eq!(neg.battery_cycles, None);
}

#[test]
fn real_batterystats_capacity() {
    let (design, learned) = parsers::parse_batterystats_capacity(&fixture("batterystats_capacity.txt"));
    assert_eq!(design, Some(7500));
    assert_eq!(learned, Some(7360));
    assert_eq!(parsers::parse_batterystats_capacity(""), (None, None));
}

#[test]
fn top_last_sample() {
    // Format of `top -b -n 2 -o %CPU,NAME` on Android 16: the first sample
    // gives 0 % to everything.
    let text = "Tasks: 992 total,   5 running\n  Mem:    11232M total\n800%cpu 223%user\n[%CPU] NAME\n  0.0 system_server\n  0.0 adbd\n\
Tasks: 992 total,   6 running\n  Mem:    11232M total\n800%cpu 190%user\n[%CPU] NAME\n 13.6 system_server\n 11.6 adbd\n  3.3 top\n  1.0 sh\n";
    assert_eq!(parsers::parse_top_last(text), vec![("system_server".to_string(), 13.6), ("adbd".to_string(), 11.6), ("sh".to_string(), 1.0)]);
}

/// Android 11 (moto e20): the standard is a number and there is no IP.
#[test]
fn android_11_wifi_status() {
    let text = "Wifi is enabled\nWifi is connected to \"home\"\nWifiInfo: SSID: home, BSSID: 02:00:00:00:00:01, MAC: 02:00:00:00:00:02, Supplicant state: COMPLETED, Wi-Fi standard: 4, RSSI: -49, Link speed: 65Mbps, Frequency: 2462MHz, Net ID: 2\n";
    let s = parsers::parse_wifi_status(text);
    assert_eq!(s.standard, "Wi-Fi 4 (802.11n)");
    assert_eq!(s.ip, "");
    assert_eq!(parsers::interface_ipv4("23: wlan0    inet 192.168.1.35/24 brd 192.168.1.255 scope global wlan0\\       valid_lft forever"), Some("192.168.1.35".into()));
    assert_eq!(parsers::interface_ipv4("Device \"wlan0\" does not exist."), None);
}

/// Samsung (Galaxy Tab A9+, One UI, Android 16): its own battery health in
/// `dumpsys battery`. Android's learned capacity there is a placeholder
/// (5000 mAh, min = max), so Samsung's figure is the one used.
#[test]
fn samsung_battery_health() {
    let text = "Current Battery Service state:\n  level: 35\n  FEATURE_SAVE_BATTERY_CYCLE: true\n  battery FirstUseDate: [20240813]\n  mSavedBatteryAsoc: [89]\n  mSavedBatteryUsage: [20179]\n  mSavedFullStatusDuration: unsupported\n";
    let h = parsers::parse_samsung_battery(text);
    assert_eq!(h.battery_asoc, Some(89));
    assert_eq!(h.battery_cycles, Some(202));
    assert_eq!(h.battery_first_use.as_deref(), Some("2024-08-13"));
    assert_eq!(h.battery_percent(), Some(89.0));
    // Measured mAh win over the manufacturer's figure.
    let measured = ComponentHealth { battery_full_mah: Some(4500), battery_design_mah: Some(5000), battery_asoc: Some(89), ..Default::default() };
    assert_eq!(measured.battery_percent(), Some(90.0));
    // Other phones: nothing.
    let none = parsers::parse_samsung_battery("  level: 80\n  mSavedBatteryAsoc: [0]\n");
    assert_eq!((none.battery_asoc, none.battery_cycles, none.battery_first_use), (None, None, None));
}
