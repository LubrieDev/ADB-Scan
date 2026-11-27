# Status

Version 1.4.1 · 26 November 2026 · Overview no longer opens blank when it
was the last section used. Version 1.4.0 added the Root page (with Magisk from its
official site), about twenty new features without root, and support for
Android 11, Samsung and several devices at once. Version 1.3.0 added favorites
in the sidebar, custom accent colour and intensity, and the Phone changes
page. Version 1.2.1 published the
Windows installer, which 1.2.0 could not build. Version 1.2.0 added updates from the app (“Update
now”) and the installer for Windows. Version 1.1.0 added the Windows version and the
sections to control the phone; 1.0.0 (24 November 2026) was the first stable
release.

Everything on this page was checked with a REDMAGIC 11S Pro (Android 16,
without root) on Linux Mint 22 Cinnamon, with the app's configuration isolated
so the real data was not touched. On 26 November every read was also run on a
second phone, a moto e20 (Android 11 Go, Unisoc, 32-bit, over wireless ADB),
and on a Samsung tablet (Galaxy Tab A9+ 5G, One UI on Android 16): see
“Second phone” and “Samsung tablet” below.

## Summary

| Area | Result |
| --- | --- |
| Core (`crates/core`) | 321 tests passing, with real phone outputs (anonymised: sample package names, network names and addresses). |
| Interface (`crates/app`) | 8 tests; builds without clippy warnings. |
| Languages | English (default) and Spanish: every interface string has a Spanish translation (checked by a test). |
| One-off snapshot | 1.4 s with the real phone. |
| Live telemetry | ~556 ms per block with a single `adb shell`. |
| ADB and USB monitor | Real detection; USB 3.2 Gen 2 (10 Gb/s), node accessible. |
| APK reader | Name, version, SDK and ABIs of real APKs. |
| Desktop | `.deb` package installed, native notifications and Cinnamon tray icon. |

## Ways to connect

- **Basic analysis** without developer options, in “File transfer” mode:
  `adb devices` empty. The app names the phone over USB, marks debugging as
  off and optional, recognises it from the history and reads the storage
  through GVfs, because Mint mounts it. The battery shows as not available,
  with the reason. With the MTP interface free, the direct read takes 5 ms:
  Android confirmed, battery and free space.
- **“Charging only”** after a restart: the phone does not appear on USB. The
  section explains what to do; the advanced sections offer “View basic
  analysis”.
- **USB debugging**: the “Ways to connect” dialog shows “Connected and
  authorised” live. With ADB authorised, the basic analysis works in parallel
  (MTP + ADB) and the other sections do not change.
- **Wireless debugging with a QR code**: scanned from “Pair device with QR
  code” → paired and connected over Wi-Fi; the connection port was found by
  itself (with Debian's `adb` 34.0.4, without mDNS, avahi finds it).
- **Manual wireless debugging**: IP address, pairing port and code, with the
  connection port empty → paired and connected.
- The QR code expires after 3 minutes; when closing, no search or pairing
  process is left running. “Continue with the basic analysis” hides the
  invitation and remembers it.
- The connection notification says USB or Wi-Fi according to the real
  transport.

## Sections verified with the phone

- **Interface performance**: no freezes when switching pages. Large lists
  (properties, processes, history) only create the visible rows and only the
  visible page processes the telemetry. History opens in under 16 ms with
  real data (59 ms in the worst measured case: 300 connections, 400 events,
  400 operations).
- **Network**: live Wi-Fi with `cmd wifi status` (on Android 16, `dumpsys wifi`
  takes 5 MB and changed format).
- **Apps**: real names (56 of 62 apps), technical details of each app (manifest
  read in all 62 user apps, 426 ms on average), confirmations with name and
  package, sorting and filters by source.
- **Marketing name**: without `ro.product.marketname`, it is taken from the USB
  product (“REDMAGIC 11S Pro”).
- **Terminal**: a real terminal (VTE) with two sessions and persistent history.
- **Flasher**: confirmation by typing the serial number for every flash, model
  check (`getvar product` against what ADB saw and what the ROM requires),
  vbmeta and slot warnings, entry in History.
- **Security**: the name of each app in the findings, “Revoke” for “Install
  unknown apps”, “View in Apps” and a USB debugging warning with a button to
  turn it off.
- **Performance**: GPU, FPS, grouped sensors, alerts and the session CSV.
- **Screen**: readable scrcpy title, notice and close if the video is cut,
  virtual webcam (`--v4l2-sink`).
- **Component health**: battery at 98.1 % (7360 of 7500 mAh) with 53 cycles,
  read from sysfs and checked against `batterystats`.
- **Phone data**: verified without showing the values (`--example identity`):
  IMEI 1 and 2, IMSI, ICCID, serial, Android ID, names, Bluetooth MAC, SIM and
  accounts in 0.4 s; public IP in 1.3 s. The public IP stays hidden like the
  rest until its eye is pressed.
- **Private mode**: hides serial numbers, IPs, MACs, Wi-Fi networks and
  nicknames on every page, the sidebar, the quick panel, the tray icon,
  notifications and tooltips. The database and the configuration contain no
  mask after using it.
- **Per-app usage and wakelocks**: 6 s with two queries in parallel; the
  figures match an independent calculation over the same output.
- **Platform sheet**: SELinux, Treble, A/B, verified boot, encryption, kernel
  (GKI), GPU, Vulkan, CPU clusters, thermal limits and hardware features
  (~0.4 s, with `--example inspect`).
- **Controls** (`--example controls --apply`): every switch changes and is
  restored (140–250 ms including the check); media volume 40 → 60 → 40; the
  final state is identical to the initial one. Showing the page changes
  nothing.
- **Languages**: Overview, Basic analysis, Ways to connect, Controls and
  Preferences checked in English and Spanish with the phone connected.
- **Features of 2026-11-26** (second batch and third batch), run on the
  phone on 26 November with temporary examples; every change was put back
  and the values compared before and after:
  - Read: Usage (screen time today and week, 30 days of data, 2 313
    notifications in 24 h from 28 apps, unlocks and screen-ons, unused apps
    among the ones with a launcher icon), Stability, default apps and
    keyboards, saved Wi-Fi networks (Android 16 prints one line per security
    type: joined), a settings snapshot (1 134 keys), app limits, per-app
    tuning (game modes, language, `gfxinfo`), alarms and background tasks
    (Battery → usage), memory per app (`procstats`, 102 apps), mobile
    signal and a Wi-Fi scan, and the copy of contacts (20), calls (3) and
    SMS (209) in about 1.5 s each, checked without showing their content.
  - Changed and restored: private DNS, bold and high contrast text,
    grayscale, colour inversion, text size, touch and hold delay (put back
    through the settings snapshot), a profile applied, the browser role set
    again, pause / restrict / block data / unrestricted battery on Chrome,
    the custom game mode on a game (70 % and 60 FPS, then standard), an app
    language, the keyboard and Night Light. Every setting ended identical,
    including the ones Android touches by itself (keyboard history, Night
    Light's last activation); the only difference left is Chrome's
    `RUN_ANY_IN_BACKGROUND`, which went from “not set” to an explicit
    “allow” (the same behaviour; it is what Settings writes).
  - Stability: debuggerd dumps of a process that went on running (signal
    35, fdsan warnings of the shell, seen on this phone) are not counted as
    crashes.
  - Not run, on purpose: forgetting a saved network (its password cannot be
    entered back from here), the bug report, Optimize apps now, and changing
    the home, phone, SMS or assistant app (it can reset carrier messaging or
    the assistant's setup).
- **Root page** (26 November), read on two phones over ADB: the REDMAGIC
  (Android 16, `init_boot`, GKI android16-6.12, locked) and a moto e20
  (Android 11 Go, Unisoc ums9230, 32-bit, `boot`, kernel 4.14, locked, OEM
  unlocking allowed: `sys.oem_unlock_allowed=1`; the REDMAGIC does not
  publish it). Magisk v31.1 downloaded from its official releases and
  checked against GitHub's SHA-256 (1.4 s; a second time reuses the
  verified copy); its APK carries `armeabi-v7a`, so it serves 32-bit
  phones. Not run on any phone: installing Magisk, sending images,
  restarting to the bootloader, unlocking or flashing.
- **Second phone** (26 November): a moto e20 (Android 11 Go, Unisoc ums9230,
  Mali-G57, 32-bit, 1.8 GB of RAM), connected only over wireless ADB while the
  REDMAGIC was on USB. Every read of the core and every page was run on it;
  nothing was changed on it. What Android 11 does differently, and was fixed:
  - Internal storage was empty: `df -k /data` answers with the obb bind mount
    of the same partition. The live script also sends the `/data` line of a
    plain `df -k`.
  - No temperatures: the zones have no type (`mType=-1`) and each name ends
    in a line break, which the live script's `grep` cut off. The zones of
    Unisoc (`ank0…5`, `prometheus6/7`: ARM core code names, `cluster0/1`) count
    as CPU, `soc` as SoC.
  - Battery per app was empty: there is no `batterystats --usage`; the
    `Estimated power use` block of `--charged` is read instead.
  - Volumes, default apps and the app on screen were not read: `volume --get`
    prints nothing (now `dumpsys audio`), there is no `cmd role
    get-role-holders` (now `dumpsys role`), and the resumed activity is
    written `ResumedActivity:`.
  - Wi-Fi: the standard comes as a number (`4` → Wi-Fi 4) and without the IP
    (taken from `wlan0`); saved networks say `wpa2`.
  - App names: tables of up to 72 MB (Settings) are not read over Wi-Fi, and a
    name that could not be read is not asked for again in the session (each
    try downloaded the whole table). Some system apps came out in the test
    pseudo-locale (“[Çåļéñðåŕ Šţöŕåĝé one two three]”): those variants are
    skipped and such names are dropped from the cache.
  - With two phones, Overview, the quick panel and Connection showed the
    REDMAGIC's USB link for the moto (they took the first USB phone). Now only
    the selected phone's own USB device is used (by serial, or by
    `ro.serialno` over Wi-Fi).
  - As Android 11 allows it, and shown as such: no call log for ADB, no game
    mode or per-app language (hidden), no GPU load (Mali, no root), no
    signing certificate (`get-app-links` does not exist), no single-app cache
    clearing.
- **Samsung tablet** (26 November): a Galaxy Tab A9+ 5G (SM-X218U, Snapdragon
  695, One UI on Android 16) over USB. Only reads: it is a device in daily
  use. Everything read as on the REDMAGIC; what Samsung does differently, and
  was fixed:
  - Battery health came out as 73 %: Android's learned capacity there is a
    placeholder (5000 mAh, min = max). Samsung's own figures in `dumpsys
    battery` are used instead: `mSavedBatteryAsoc` (89 %, what Samsung
    Members shows), `mSavedBatteryUsage` (life charge in %: 20179 → 202
    cycles) and `battery FirstUseDate` (shown as “Battery in use since”).
  - The name was “SAMSUNG Android”: Samsung's USB product name for every
    device, and it publishes no `marketname`. Generic USB names are ignored:
    the model (SM-X218U) is shown.
  - Root page: a US Samsung with Snapdragon (model ending in U or U1) is
    marked as not unlockable, with no fastboot buttons nor Magisk steps;
    Samsung without fastboot no longer offers “Restart to bootloader”; the
    chip shows `ro.soc.model` (SM6375 (blair)).
- **Research on ADB without root**: about 120 commands measured on the phone
  (time, size and whether Android blocks them), in `docs/ADB_RESEARCH.md`.

## Pending verification

- With the phone: uploading and downloading files, live logcat, the full
  security audit and reboots (actions that modify the phone).
- On the desktop: the quick panel and autostart with the session.
- The platform sheet on screen (the data was verified with
  `--example inspect`).
- Storage wear (UFS): it gives “Permission denied” without root.
- Added on 2026-11-25, read with the phone but not yet run on it (they
  change it): removing and restoring preinstalled apps, taking permissions
  away, deleting files from Storage, the developer switches, screen size and
  density with the automatic undo, demo mode, port rules, exporting and
  installing apps, and the photo backup as an automation. Verified with the
  phone: the Preinstalled apps, Permissions and Developer readings, a full
  Storage scan (50 GB, 140 000 files, 37 s) and an automation run on connect.
- The nubia/REDMAGIC entries of the preinstalled apps list were chosen from
  the packages of one NX809J; other models may name them differently.
- Interface performance (2026-11-26, measured with the phone): `GtkLabel`
  width-for-height answered cheaply (`layout_fix.rs`), Storage and Phone
  changes as virtualised lists, Storage totals in the background, private
  mode only redraws the visible page. Pending with the phone: Device with
  private mode and the first opening of Storage's tabs.

## Ideas for after 1.0

- Features with root and tests on rooted test phones. The main phone is not
  rooted; the current Security page is the read-only baseline without root.
- Android sources not used yet: recent location accesses, Bluetooth paired
  devices (the test phone never had Bluetooth on), other users and work
  profiles, and the navigation mode and refresh rate (vendor-specific on
  nubia: left out so as not to break the phone's own gestures).
- More interface languages (a `.po` file each).

## Needs installing with `sudo`

- `libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev`: a normal build (without
  them, `scripts/env-no-dev.sh`).
- `fastboot`, `scrcpy`: Flasher and Screen (without them they are shown
  disabled with instructions, or they are downloaded from the assistant).
- `avahi-utils`: QR pairing and finding phones on the network (installed on
  Linux Mint).
- `tshark`: network capture (optional).
