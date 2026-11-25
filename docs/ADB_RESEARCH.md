# Research: what ADB allows without root

Date: 23 November 2026. Reference phone: REDMAGIC 11S Pro (NX809J, Snapdragon
SM8850, Android 16, without root, SELinux enforcing). `adb shell` user:
`uid=2000(shell)`, groups `log`, `readproc`, `net_bw_stats`, `sdcard_rw`…,
context `u:r:shell:s0`.

Every command was run **read-only** and its time (full round trip over USB)
and output size were measured. “Blocked” means Android denies it to the
`shell` user, not that the command fails. The results come from a single
phone: they change on other manufacturers and versions (see
“Compatibility”).

## 1. Starting point

Live telemetry (battery, memory, `df`, temperatures, CPU, network, USB,
brightness, Wi-Fi, refresh rate, battery and UFS health), full `getprop`,
`uname`, processes (`ps` + `top`), sensors, services (`service list`),
packages (`pm list`, `dumpsys package`, `diskstats`, name from the APK),
permissions, per-app usage (`batterystats`), security audit, private
identifiers, logcat, files, GPU/FPS (Adreno), scrcpy, fastboot.

## 2. Commands investigated

### System and platform

| Command | Time | Output | Result | Use |
| --- | --- | --- | --- | --- |
| `getprop` | 61 ms | 74 KB | Available | Already in the telemetry; now also Treble, A/B, verified boot, encryption, vendor level |
| `uname -a` · `cat /proc/version` | 36 ms | 0.3 KB | Available | **New**: compiler, date, GKI kernel branch |
| `getenforce` · `/sys/fs/selinux/enforce` | 40 ms | 9 B | Available | **New**: SELinux mode |
| `getconf PAGESIZE` | <40 ms | 4 B | Available | **New**: 4 KB / 16 KB page (Android 15+) |
| `cat /proc/cpuinfo` · `/proc/meminfo` | 35 ms | 4 KB | Available | Already in the telemetry |
| `cat /proc/vmstat`, `/proc/zoneinfo`, `/proc/stat`, `/proc/loadavg` | 35 ms | 3 KB | Available | Pending (approximate memory pressure) |
| `cat /proc/pressure/{memory,cpu,io}` (PSI) | — | — | **Blocked** (SELinux) | Not usable |
| `cat /proc/swaps`, `/sys/block/zram0/*` | — | — | **Blocked** | Not usable |
| `cat /proc/partitions` | — | — | **Blocked** | `ls /dev/block/by-name` works (names, not sizes) |
| `dmesg` | — | — | **Blocked** (`klogctl`) | Not usable |
| `pm list features` | 53 ms | 5 KB | Available | **New**: NFC, eSIM, UWB, StrongBox, IR… |
| `pm list libraries`, `pm list permission-groups` | 56 ms | 3 KB | Available | Low value |
| `dumpsys -l` · `cmd -l` | 190 ms | 13 KB | Available | List of services (Tools already lists `service list`) |

### Hardware

| Command | Time | Result | Use |
| --- | --- | --- | --- |
| `/sys/devices/system/cpu/cpufreq/policy*/{related_cpus,scaling_governor,cpuinfo_min_freq,cpuinfo_max_freq,scaling_available_frequencies}` | <50 ms | Available | **New**: clusters, governor (`walt`), limits and steps |
| `dumpsys SurfaceFlinger \| grep GLES` | 60 ms | Available | **New**: GPU, vendor and the driver's OpenGL ES version |
| `cmd gpu vkjson` (Android 10+) | 184 ms · 74 KB | Available | **New**: Vulkan 1.4.295, driver 512.842.44 |
| `/sys/class/kgsl/kgsl-3d0/gpu_model` | 37 ms | Available (Adreno only) | **New** |
| `/sys/class/kgsl/kgsl-3d0/{gpuclk,max_gpuclk}` | — | **Blocked** | GPU frequency not readable |
| `dumpsys hardware_properties` | 58 ms | Available | **New**: throttling and shutdown thresholds (CPU 105/125 °C, battery 80/90 °C, skin 50/90 °C) |
| `dumpsys thermalservice` | 49 ms | Available | Already in the telemetry |
| `dumpsys sensorservice` | 60 ms · 137 KB | Available | Already in Tools |
| `dumpsys media.camera` | 261 ms · **555 KB** | Available | Pending: very heavy and very vendor-dependent |
| `cat /vendor/etc/media_codecs*.xml` | 38 ms | Available | Pending: hardware codecs |
| `dumpsys input`, `dumpsys vibrator_manager`, `dumpsys fingerprint` | 40–66 ms | Available | Low value for the user |
| `lshal` | 208 ms | Partial | Adds nothing without root |

### Apps

| Command | Time | Result | Use |
| --- | --- | --- | --- |
| `dumpsys package <pkg>` | 143 ms (Settings: 342 KB) | Available | **Extended**: ABI, splits, signature scheme, initiating installer, data folder, per-user state, declared permissions, disabled components, factory version, ART compilation |
| `pm path <pkg>` | 59 ms | Available | Already there; now it locates the base APK |
| `unzip -p <base.apk> AndroidManifest.xml` | 68 ms | Available (toybox `unzip`) | **New**: activities, services, receivers and providers with their export and permission |
| `pm get-app-links <pkg>` (Android 12+) | 68 ms | Available | **New**: verified domains and the **SHA-256 of the signing certificate** (only apps with web links) |
| `am get-standby-bucket <pkg>` (Android 9+) | 63 ms | Available | **New**: standby bucket |
| `cmd appops get <pkg>` | 46 ms | Available | **New**: last access to the camera, microphone, location, clipboard… |
| `dumpsys deviceidle whitelist` | 44 ms | Available | **New**: battery optimisation exemption |
| `dumpsys activity exit-info <pkg>` (Android 11+) | 41 ms | Available | **New**: reason for each exit (crash, ANR, memory, user…) |
| `dumpsys gfxinfo <pkg>` | 52 ms | Partial | Failed to dump the foreground app (“Failure while dumping the app”): pending |
| `dumpsys meminfo <pkg>` | 206 ms | Available | Pending: real PSS per app (on demand) |
| `dumpsys package dexopt` (global) | 1.9 s | Available | Each app's block is used instead, much cheaper |
| `dumpsys usagestats` | 1.1 s · **9 MB** | Available | Discarded: cost and privacy |
| `dumpsys appops` (global) | 324 ms · 2.3 MB | Available | Discarded: used per app |

Across the phone's 62 user apps: manifest read in all 62, full inspection in
426 ms on average (maximum 1.1 s).

### Processes and memory

| Command | Time | Result | Use |
| --- | --- | --- | --- |
| `ps -A -o …` · `top -b -n 1` | 116 / 459 ms | Available | Already there |
| `dumpsys activity lru` | 43 ms · 9 KB | Available | **New**: Android's state for each app process (on screen, foreground service, perceptible, cached…) |
| `dumpsys activity processes` | 132 ms · **942 KB** | Available | Discarded: `lru` gives the same in 1 % of the size |
| `dumpsys meminfo` (global) | **10 s** | Available | Discarded for periodic use; pending as a manual action |
| `dumpsys cpuinfo` | 36 ms | Available | Pending: CPU per process over the last minutes (computed by Android) |
| `dumpsys procstats --hours 1` | 39 ms | Empty | Adds nothing on this phone |

### System services (`dumpsys`)

Useful and cheap: `battery`, `thermalservice`, `hardware_properties`,
`deviceidle`, `cpuinfo`, `diskstats`, `dropbox`, `uimode`, `webviewupdate`,
`role`. Heavy (>100 KB): `power` (117 KB), `display` (143 KB), `connectivity`
(169 KB), `telephony.registry` (397 KB), `media.camera` (555 KB),
`jobscheduler` (1.5 MB), `appops` (2.3 MB), `wifi` (3.9 MB), `usagestats`
(9 MB). `dnsresolver` and `cpu_monitor` do not exist on this phone (“Can't
find service”).

### Network and connectivity

| Command | Time | Result | Use |
| --- | --- | --- | --- |
| `ip addr`, `ip route`, `ip -6 route show table all`, `ip rule` | 35 ms | Available | Interfaces and routes were already there |
| `cat /proc/net/tcp{,6}` · `netstat -tuln` | 40 ms | Available | Pending: listening ports and connections (toybox `netstat` also shows established ones) |
| `ss -tuln` | — | **Blocked** (netlink socket) | Use `netstat` |
| `dumpsys connectivity` | 67 ms · 169 KB | Available | Pending: DNS, validated network, VPN |
| `getprop net.dns1` | — | Empty since Android 8 | Not usable |
| `cmd wifi status` | 53 ms | Available | Already there |
| `dumpsys telephony.registry` | 68 ms · 397 KB | Available | Pending: network type (LTE/NR) and service state |
| `dumpsys bluetooth_manager` | 75 ms | Available | Pending |
| `dumpsys usb` | 42 ms | Available | Already there |
| Real Wi-Fi MAC | — | Random per network since Android 10 | Already documented |

### Storage

| Command | Time | Result | Use |
| --- | --- | --- | --- |
| `df`, `/proc/mounts` | 40 ms | Available | Already there |
| `sm list-volumes all` · `sm list-disks` | 594 / 416 ms | Available | Low value (no SD) |
| `dumpsys diskstats` | 52 ms | Available | Already there; includes write latency and FBE |
| `dumpsys storaged` | 96 ms · 309 KB | Available | Pending: I/O per app |
| `ls /dev/block/by-name` | 45 ms | Available | Partition names (useful for the Flasher) |

### Logs and diagnosis

| Source | Result |
| --- | --- |
| `logcat` (main, system, events, crash) | Available (`log` group). `-b crash` was empty |
| `dmesg` | **Blocked** |
| `dumpsys dropbox` | Available: list of entries (boots, `SYSTEM_LAST_KMSG`, audits, crashes). Pending: show it |
| `/data/anr`, `/data/tombstones` | **Blocked** |
| `dumpsys activity exit-info` | **Used**: crashes and ANRs of each app |

### Display and interface

| Command | Time | Result |
| --- | --- | --- |
| `wm size` · `wm density` | 53 ms | Already there |
| `dumpsys window displays` | 52 ms | Available: rotation, modes |
| `dumpsys activity activities` | 47 ms | Already used for the foreground app |
| `dumpsys activity top` | **5 s** | Discarded |
| `dumpsys window windows` | 86 ms | Available, low value |
| `settings list global/secure/system` | 55–70 ms | Available (read-only); the audit already uses it |

## 3. Prioritisation

| Feature | Value | Cost | Risk | Decision |
| --- | --- | --- | --- | --- |
| Technical details of each app | High: turns every app into an inspectable object | ~0.4 s on demand | None (read) | **Done** |
| Components and exposure from the manifest | High (security) | 68 ms | None | **Done** |
| Android state of processes | High: “cached” versus “service” explains usage | 43 ms per refresh | None | **Done** |
| Platform sheet (SELinux, GPU, CPU, thermal limits, features, verified boot) | Medium-high | ~0.4 s when Device opens | None | **Done** |
| Listening ports (`netstat`) | Medium (security) | 40 ms | Low (shows remote IPs: private mode) | Pending |
| System crash log (`dropbox`) | Medium | 60 ms | Low | Pending |
| DNS and validated network (`connectivity`) | Medium | 67 ms · 169 KB | Low | Pending |
| PSS per app (`meminfo <pkg>`) | Medium | 200 ms | None | Pending (“Measure” button) |
| Smoothness per app (`gfxinfo`) | Medium | 52 ms | None | Pending: failed on this phone |
| Cameras (`media.camera`) | Low-medium | 555 KB | None | Discarded for now |
| PSI, `dmesg`, zram, `/proc/partitions` | — | — | — | Impossible without root |

## 4. Compatibility across versions

| Source | Since | If missing |
| --- | --- | --- |
| `am get-standby-bucket` | Android 9 | “Not supported” |
| `cmd gpu vkjson` | Android 10 | GLES only |
| `dumpsys activity exit-info` | Android 11 | “Not supported (Android 11 or later)” |
| `pm get-app-links` (current format) | Android 12 | “Not supported” |
| `ro.vendor.api_level` in YYYYMM format | Android 15 | Shown as “API n” |
| `appId=` in `dumpsys package` | Android 16 | `userId=` before (both are accepted) |
| `getconf PAGESIZE` 16 KB | Android 15+ (optional) | The real value is shown |
| `/sys/class/kgsl` | Qualcomm Adreno only | “Not supported” |
| `unzip` on the phone | Modern toybox | “Not supported (unzip is not on the phone)” |

Each item is stored as a `Probe` (`crates/core/src/probe.rs`): available, not
published, restricted, not supported or an error. The classification is done
by the error text in the first three lines, because `adb shell` does not
always propagate the exit code.

## 5. Controls (they change the phone)

Tested one by one on the REDMAGIC (Android 16, without root): read → change →
check → restore.

| Control | Command | Result |
| --- | --- | --- |
| Wi-Fi | `cmd wifi set-wifi-enabled enabled\|disabled` | Works |
| Mobile data | `cmd phone data enable\|disable` | Works (state in `mobile_data<SIM>`) |
| Bluetooth | `cmd bluetooth_manager enable\|disable` | Works |
| Airplane mode | `cmd connectivity airplane-mode enable\|disable` | Works |
| Location | `cmd location set-location-enabled true\|false` | Works |
| Dark theme | `cmd uimode night yes\|no` | Works |
| Auto-rotate | `settings put system accelerometer_rotation` | Works |
| Brightness | `settings put system screen_brightness 1–255` | Works |
| Do Not Disturb | `cmd notification set_dnd off\|priority\|alarms\|none` | Works |
| Data saver | `cmd netpolicy set restrict-background` | Works |
| Battery saver | `cmd power set-mode 1\|0` | Does not turn on while charging (Android rule) |
| Volume | `cmd media_session volume --set` | **Accepted and ignored**; media volume does change with `input keyevent KEYCODE_VOLUME_UP/DOWN` |
| NFC | `svc nfc enable` | The system kills the process (code 137) |
| Wi-Fi hotspot | `cmd wifi start-softap` | Does not exist in user builds |
| Flashlight | — | There is no shell command |
| `svc wifi\|data\|bluetooth` | — | No longer exist on Android 16 |

The `shell` user has `TETHER_PRIVILEGED`, `NETWORK_SETTINGS`,
`MODIFY_PHONE_STATE`, `BLUETOOTH_PRIVILEGED` and `WRITE_SECURE_SETTINGS`: the
limits above come from the shell tools, not from permissions.
