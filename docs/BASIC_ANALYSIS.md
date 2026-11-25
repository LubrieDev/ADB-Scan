# Basic and advanced analysis

ADB Scan offers three independent modes: basic analysis, USB debugging and
wireless debugging (QR code or manual). The last two make up the advanced
analysis. The **Ways to connect** dialog (`app/src/wireless.rs`) presents them
and explains how they differ.

| | Basic analysis | Advanced analysis |
| --- | --- | --- |
| Requirements on the phone | None. Optional: “File transfer” | Developer options, USB or wireless debugging and authorisation |
| How it talks to the phone | USB descriptors (sysfs) and MTP/PTP | ADB |
| Where it is shown | **Basic analysis** section, Overview | The other sections |
| Code | `core/src/basic.rs`, `core/src/mtp.rs`, `app/src/pages/basic.rs` | Everything else |

The basic analysis never uses ADB and works even if `adb` is not installed.
The advanced analysis does not depend on the basic one.

## 1. How ADB Scan recognises a phone

- **Detection**: two sources in parallel.
  - `monitor.rs`: `adb track-devices -l`, with `adb devices` polling as a
    fallback. It gives the serial number, the state (`device`,
    `unauthorized`, `offline`, `no permissions`…), the model and the
    transport.
  - `usb.rs`: kernel USB events (netlink) and sysfs reading. It sees the phone
    before ADB does and tells ADB (`ff/42/01`), fastboot (`ff/42/03`) and EDL
    (`05c6:9008`) apart by their interfaces. It accepts vendors on a list,
    phones with a debugging interface, and phones from any vendor that
    expose Android's MTP interface.
- **Identification with ADB**: `getprop`, `dumpsys`, telemetry through
  `adb shell`, `identity.rs` with `service call`…
- **Without ADB**: `tracker.rs` reports “USB detected”, the **Basic analysis**
  section shows what the phone shares without debugging, and the sections
  that need ADB (`Need::Adb` in `window.rs`) explain that debugging is
  optional and offer **View basic analysis**.

## 2. What can be known without debugging

Identifying a phone is not accessing its content. Android isolates apps and
their data: without ADB (or without an app installed on the phone with its own
permissions), the computer only sees what the phone deliberately publishes
when it connects. ADB Scan installs nothing on the phone, does not change its
settings and does not try to bypass any restriction.

### Phone connected over USB

| Data | USB mode needed | Source |
| --- | --- | --- |
| Manufacturer, product, serial number, VID:PID | Any mode with data | USB descriptor (sysfs) |
| USB speed and version | Any mode with data | sysfs |
| USB mode chosen on the phone (files, photos, tethering, MIDI, webcam) and whether debugging is on | Any mode with data | USB interfaces |
| “Already seen on this computer” | Any mode with data | Local history, by serial number |
| Confirmation that it is Android | File or photo transfer | MTP: `android.com` extension |
| Internal model, declared version, device type | File or photo transfer | MTP `GetDeviceInfo` and properties |
| Battery level | File or photo transfer | MTP, property `0x5001` |
| Storages with capacity and free space | File transfer (unlocked) | MTP `GetStorageInfo`, or GVfs if mounted |
| Nothing | Charging only | In “Charging only” mode many phones do not even appear on the USB bus |

Checked with the REDMAGIC 11S Pro (Android 16) without developer options:

- In “File transfer” mode it presents itself as `19d2:0307` with a single MTP
  interface (`06/01/01`, named “MTP”). With debugging on, as `19d2:0306` with
  MTP and ADB, and in “Charging only” with debugging, as `19d2:1352` with ADB
  only.
- The USB serial number matches ADB's: that is how a phone seen before is
  recognised even if debugging is off today.
- MTP answered in 5 ms: extension `microsoft.com: 1.0; android.com: 1.0;`,
  model “REDMAGIC 11S Pro”, type “Mobile phone”, battery, the internal storage
  and the version “REDMAGICOS11.5.9_GB”. That is the version of the
  manufacturer's system, not Android's, and it is shown as is, without
  interpreting it.
- After a restart with the screen locked, the phone stays in “Charging only”
  and does not appear on USB: there is nothing to read until the user chooses
  another mode.
- The “image size” property (`0x5003`) is **not** shown: Android returns the
  largest dimension of the screen repeated (“W×W”), not the real resolution.

### Phone on the same network

Without wireless debugging, Android announces nothing that identifies it as
the user's phone. Its Wi-Fi MAC is random per network (Android 10+), only the
router sees the DHCP name and there is no mDNS service of its own. Companion
apps (KDE Connect, Quick Share…) do announce themselves, but they are other
protocols with their own pairing. ADB Scan **does not scan the network**: over
Wi-Fi there is only the advanced analysis, through wireless debugging with a
QR code or manual pairing (see below).

### Without any connection

There is no live data. The only legitimate thing is what is already stored
locally from previous connections (History), presented as such.

### What requires authorised ADB

Android version and patch, CPU, RAM and temperatures, battery health
(temperature, voltage, cycles, capacity), network (IP, Wi-Fi, carrier), apps,
file browser, screenshots and screen mirroring, the security audit, the
platform sheet and identifiers (IMEI, SIM). The basic analysis lists them as
“Requires USB debugging”.

## 3. Design

### Core

- `usb.rs`: reads the interfaces and their endpoints (`ep_XX`) and deduces the
  functions (`UsbFunction`). It also accepts a phone from any vendor if it has
  Android's MTP interface (`ff/ff` named “MTP”). Media players and cameras
  (`06/01/01` without an Android vendor) are still ignored. The driver that
  has claimed each interface is not kept in the state: it changes when the
  interface is claimed and would cause rescans in a loop. It is read when the
  interface is used (`interface_driver`).
- `mtp.rs`: a minimal, **read-only** PTP/MTP client over usbfs
  (`USBDEVFS_CLAIMINTERFACE`/`BULK`/`RELEASEINTERFACE`), with no new libraries.
  It only asks for `GetDeviceInfo`, `OpenSession`, `GetStorageIDs`,
  `GetStorageInfo`, three properties and `CloseSession`. It never lists, reads
  or writes objects. The parsers are pure functions with tests.
- `basic.rs`: turns what was read into rows with three states, `Available`,
  `Unavailable` (with the reason) and `NeedsAdb`, plus `Pending` while reading.
  It never fills in an item that was not obtained.
- `db.rs`: `known_device(serial)` for recognition through the history.

### Coexisting with the file manager

Linux Mint mounts the phone as soon as it appears in “File transfer” mode
(GVfs), and `gvfsd-mtp` keeps the MTP interface. ADB Scan **does not fight
over it**:

- If the interface has an owner (`driver` = `usbfs` in sysfs), it is not
  opened. The storages are read with `gio info -f` on the existing mount (it
  mounts nothing) and the battery shows as not available, with the reason.
- The automatic read waits 4 s after the phone appears, so GVfs mounts it
  first, and only happens with the section in view. The Refresh button reads
  immediately.

### Interface

- **Basic analysis** section (DATA, always available). It contains a legend of
  the states, five groups (Identification, USB connection, File transfer,
  Recognised on this computer, Advanced analysis) and help on what to do
  without a cable. It sits outside the first nine sections so the Alt+1…9
  shortcuts do not move.
- **Optional invitation** “Want a more complete analysis?” on the Overview and
  in Basic analysis, only with a phone without debugging. “Continue with the
  basic analysis” hides it and remembers it; “How to turn it on” shows the
  steps without doing anything on the phone. It can be shown again in
  Preferences → General → Analysis.
- The advanced sections do not just say “turn on debugging”: they explain
  that it is optional and offer **View basic analysis**.
- Overview: it names the phone detected over USB and its diagnosis shows “USB
  debugging turned off · it is optional” as information, not as a warning. The
  sidebar card says “Basic analysis” instead of “Not authorised”, and the
  “USB detected” desktop notification does not talk about waiting for
  authorisation.

## 4. Wireless debugging: QR and manual

- `core/src/wireless.rs`: new QR credentials on every attempt (name
  `adbscan-XXXXXXXX` and a 12-character password, from `/dev/urandom`), the
  `WIFI:T:ADB;S:…;P:…;;` content and the QR matrix (`qrcode` crate, encoder
  only). Also discovery: `adb mdns services` and `avahi-browse -tpr` (IPv4,
  without duplicates), because Debian's `adb` comes without mDNS.
- `Adb::pair` (6-digit code) and `Adb::pair_with_password` (QR) share the
  `adb pair` call; the connection is `Adb::connect`.
- QR flow: the code is shown and every 2 s the `_adb-tls-pairing._tcp` service
  with that name is looked for; when it appears, `adb pair`. Then the
  `_adb-tls-connect._tcp` service at the same IP is looked for and connected
  to.
- Manual flow: IP address, pairing port and code; the connection port is
  optional (if missing, it is looked for as in the QR flow and, if it does not
  appear, it is asked for).
- The waits carry a generation number: leaving the page, pressing “New code”
  or closing the dialog stops them. Every query to avahi or adb has a timeout.
  Nothing is disconnected when switching modes: the user decides about an
  existing connection (device selector → disconnect).

## 5. Tests

- `cargo test -p adbscan-core`: MTP parsers (DeviceInfo, StorageInfo,
  truncated data or impossible counts, `ioctl` numbers), GVfs mounts, a fake
  sysfs with an Android without debugging from an unknown vendor, USB
  functions, the basic report in every situation (charging only, busy because
  of GVfs, locked, debugging pending) and `known_device`.
- `cargo run -p adbscan-core --example basic`: report against the real phone
  without using ADB, with private data hidden.
- With the REDMAGIC 11S Pro: with ADB authorised (advanced active and basic in
  parallel), without developer options in “File transfer” (mounted by GVfs and
  free) and in “Charging only” (does not appear).
- Wireless debugging with the same phone and Debian's `adb` 34.0.4 (without
  mDNS): **QR** scanned → paired and connected to `IP:34363`, with the port
  found by avahi; **manual** with the connection port empty → paired and
  connected. The code expires after 3 minutes; when closing, no `avahi-browse`
  or `adb pair` is left running.
