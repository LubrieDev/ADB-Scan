# Status

Version 1.1.0 · 25 November 2026 · Windows version and new sections to
control the phone. Version 1.0.0 (24 November 2026) was the first stable
release.

Everything on this page was checked with a REDMAGIC 11S Pro (Android 16,
without root) on Linux Mint 22 Cinnamon, with the app's configuration isolated
so the real data was not touched.

## Summary

| Area | Result |
| --- | --- |
| Core (`crates/core`) | 210 tests passing with real phone outputs (anonymised). |
| Interface (`crates/app`) | 4 tests; builds without clippy warnings. |
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

## Ideas for after 1.0

- Features with root and tests on rooted test phones. The main phone is not
  rooted; the current Security page is the read-only baseline without root.
- Android sources not used yet: screen time per app (`usagestats`), data per
  app (`netstats`), memory over time (`procstats`), rendering jank
  (`gfxinfo`), alarms and scheduled jobs, and recent location accesses.
- More interface languages (a `.po` file each).

## Needs installing with `sudo`

- `libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev`: a normal build (without
  them, `scripts/env-no-dev.sh`).
- `fastboot`, `scrcpy`: Flasher and Screen (without them they are shown
  disabled with instructions, or they are downloaded from the assistant).
- `avahi-utils`: QR pairing and finding phones on the network (installed on
  Linux Mint).
- `tshark`: network capture (optional).
