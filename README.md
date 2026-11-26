# ADB Scan

Native desktop app for Linux Mint Cinnamon that detects, monitors,
diagnoses and manages Android devices over ADB and fastboot. Written in
**Rust + GTK 4 + libadwaita**. It also runs on Windows 10 and 11, with its
own installer (see [Windows](#windows)).

## Features

- **Resident**: Cinnamon tray icon with four states, a quick panel, native
  notifications and optional autostart.
- **Basic analysis without debugging**: recognises the phone without
  developer options. Over USB it reports the manufacturer, model, serial
  number, USB mode and whether it was connected before; with “File transfer”
  (MTP), the battery, storage and confirmation that it is Android. Every item
  says whether it is available, whether it is not and why, or whether it
  requires debugging. USB debugging is an optional upgrade (advanced
  analysis), never a requirement.
- **Three ways to connect**: basic analysis, USB debugging and wireless
  debugging (Android 11+) **pairing with a QR code** or **manually** (IP
  address, port and code). After pairing it finds the connection port by
  itself.
- **Instant detection**: `adb track-devices` + kernel USB events (netlink).
  It reports the phone before ADB authorises it and tells ADB, fastboot and
  EDL apart by their USB interfaces.
- **Live telemetry every 500 ms** with a single `adb shell` per device; the
  rate drops when no window is visible.
- **Pages**: Overview, Device, Performance, Battery (with history),
  Connection, Network, Apps, Files, Screen (scrcpy), Control, Preinstalled
  apps, Permissions, Storage, Basic analysis, Phone data, Security, History,
  Terminal, Logcat, Tools and Flasher. **Ctrl+K** searches pages and actions.
- **Connection**: the cable (USB-C on both ends, USB 3 or USB 2.0 wires, its
  e-marker chip when the PC can read it), the PC's port (speed, lanes, limit,
  location, controller, hubs), the phone's port (roles, moisture detection,
  cable warnings), the power offered, cuts and a real speed test; alerts for
  an unstable connection or a cable that limits the speed.
- **Phone data**: a public sheet to share (model, system, hardware) and
  private information hidden until requested (IMEI, SIM, accounts, MAC,
  public IP), which is never stored.
- **Flasher with safeguards**: validated partitions, confirmation by typing
  the serial number for critical or destructive operations, ROM plans read
  from `flash_all.sh` without running it.
- **Security audit without root** with a score, an exportable report, the
  name of every app involved and a button to remove “Install unknown apps”.
- **Performance**: CPU per core, GPU (Adreno), FPS of the foreground app,
  thermal sensors grouped with per-type thresholds, temperature alerts and a
  session log exportable to CSV.
- **Apps**: per-app actions (open, stop, clear cache or data, disable,
  extract, uninstall) and actions on all apps: close the open ones (as “Clear
  all” in Recents), stop all except the ones the phone needs, and clear every
  app's cache.
- **Component health**: battery capacity compared to the original and charge
  cycles; storage lifetime (UFS/eMMC) when the phone exposes it.
- **Real embedded terminal** (VTE): interactive `adb shell` on the phone and
  the PC's shell with `adb` and `fastboot`; `top`, `vi`, colours, selection,
  the shell's own history and quick commands.
- **Interface** in English or Spanish, light/dark theme, eight accent colours
  and animations (can be turned off).
- **Technical details of each app**: state according to Android, standby
  bucket, components and their exposure (read from the manifest), signature,
  ART compilation, recorded accesses (camera, microphone, location…) and the
  reason for each exit (crash, ANR, memory). Read-only.
- **Platform sheet**: SELinux, verified boot, Treble, A/B, GPU and Vulkan, CPU
  clusters, the manufacturer's thermal limits and hardware features (NFC,
  eSIM, UWB, StrongBox…). Every item says whether it is missing, restricted
  or not supported.
- **Phone controls**: Wi-Fi, mobile data, Bluetooth, airplane mode, location,
  brightness, volume, Do Not Disturb, dark theme, rotation, battery and data
  saver, shortcuts to Settings screens, and restarting or powering off the
  phone. Every change is checked on the phone and recorded in History.
- **Remote**: media, calls, sending links and SMS to the phone, finding the
  phone, its notifications on the desktop (optional) and scheduled actions
  (power off at a given time, Do Not Disturb…).
- **Photo backup**: new photos and videos from the phone to the laptop.
- **Preinstalled apps (debloat)**: a curated list of manufacturer, carrier
  and Google apps (Facebook services, ads, telemetry, duplicates…) with what
  each one does and a risk level. They are removed only for the user
  (`pm uninstall --user 0`): the APK stays in the system partition and every
  removed app, by ADB Scan or another tool, can be restored from the same page.
- **Permissions**: who can do what, seen from the permission's side: which
  apps can use the location, camera, microphone, contacts, SMS…, and the
  special accesses (accessibility, notification access, display over other
  apps, install unknown apps, all files, usage access). Each one can be taken
  away from an app.
- **Storage analyzer**: one read-only scan of the shared storage, then the
  space by file type, places that grow (WhatsApp media, downloads,
  thumbnails…), a folder browser, the largest files and possible duplicates.
  Files are deleted one at a time and only after confirming.
- **Developer tools** (Tools → Developer): port forwarding (`adb reverse` /
  `adb forward`), the status bar's demo mode for clean screenshots,
  resolution and density (undone by themselves if not confirmed in 15 s),
  animation speed, show taps, pointer location, layout bounds and stay awake.
- **Health report**: one HTML page, printable to PDF, with battery wear,
  storage life, bootloader and verified boot, the latest security audit and
  the public sheet. It has no serial number or IMEI, so it can be shared
  when selling or repairing the phone.
- **App migration** (Apps → ⋮): exports the user's apps (APKs and splits)
  to a folder with a manifest and installs them on another phone. Only the
  apps travel: their data comes from each app's own backup.
- **Automations on connect** (Control → Automations): copy the new photos,
  Do Not Disturb, keep the screen on while charging, stop all apps, clear
  caches or open the mirror when a phone (or a specific one) connects. At
  most once every 10 minutes per phone, recorded in History.
- **Watcher**: alerts for a maxed-out CPU (with the culprit process), low
  battery and high temperature.
- **Battery usage per app**: what uses the most, what drains with the screen
  off and which wakelocks keep the phone awake.
- **Private mode** (Ctrl+Shift+P): hides serial numbers, IP and MAC
  addresses, Wi-Fi networks and nicknames on screen, for screenshots or
  screen sharing. Display only: it does not change data, queries or logs.
- **Privacy**: everything is local. No telemetry, no automatic downloads, no
  root. It only connects to the Internet when you ask: the public IP (queried
  by the phone) and downloading platform-tools or scrcpy from the tools
  assistant. The one exception, which can be turned off in Preferences →
  Privacy and data, is a single check for a newer release on GitHub at
  startup. It only shows a notice: nothing is downloaded until you press
  **Update now** (see [Updates](#updates)).

## Requirements

- Linux Mint 22.x / Ubuntu 24.04 or compatible (GTK ≥ 4.14, libadwaita ≥ 1.5,
  VTE for GTK 4 ≥ 0.76: embedded terminal).
- `adb`. Optional: `fastboot` (Flasher), `scrcpy` (Screen), `tshark` (network
  capture), `avahi-utils` (QR pairing and finding phones on the network) and
  `libglib2.0-bin` (`gio`: phone storage while the file manager has it
  mounted). The last two come installed with Linux Mint.

```bash
sudo apt install adb fastboot scrcpy avahi-utils
```

## Building

```bash
# Build dependencies (once)
sudo apt install libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

cargo build --release -p adb-scan
target/release/adb-scan
```

It can also be built without administrator rights:
`. scripts/env-no-dev.sh` downloads the `-dev` packages with
`apt-get download` and extracts them into `~/.cache/adb-scan-build` (only for
building; it does not modify the system).

## Installing

Download the `.deb` of the latest version from
[Releases](https://github.com/LubrieDev/ADB-Scan/releases/latest) and install
it (Linux Mint 22 / Ubuntu 24.04 or newer, amd64):

```bash
sudo apt install ./adb-scan_1.2.0-1_amd64.deb
```

To build the package yourself, use `cargo deb -p adb-scan` (see
[RELEASING.md](RELEASING.md)) or `scripts/build-deb.sh`, which writes
`dist/adb-scan_<version>_amd64.deb`.

The package also installs the udev rules needed to access USB without root.

## Updates

When a newer version is published, a notice appears under the header bar.
Its **Update now** button downloads the new version (with its progress),
checks it and installs it:

- **Linux**: it downloads the release's `.deb` and installs it with apt. A
  window of the desktop asks for your password (`pkexec`). Then ADB Scan
  offers to restart.
- **Windows**: it downloads the installer and runs it. ADB Scan closes and
  opens again by itself a few seconds later, already updated.

The file is downloaded only from this repository's releases, and it is
installed only if its SHA-256 sum is the one GitHub publishes for it.
Otherwise nothing is installed, and the notice links to the release page. A
copy that was not installed from those packages (a build of your own) gets
only that link. The check can be turned off in Preferences → Privacy and
data.

## Windows

ADB Scan also runs on **Windows 10 and 11 (64-bit)**. It installs for your
user only: it does not need administrator rights.

### Installing

1. Download `ADB-Scan-<version>-setup-x64.exe` from
   [Releases](https://github.com/LubrieDev/ADB-Scan/releases/latest) (or, for
   a build that is not released yet, from the **Windows** workflow run in the
   Actions tab: the installer is in its *Artifacts*).
2. Double-click it and follow the steps. It installs into
   `%LOCALAPPDATA%\Programs\ADB Scan`, adds ADB Scan to the Start menu (and,
   if you tick it on the last page, to the desktop) and to **Settings →
   Apps**, from where it is uninstalled.

To update, use **Update now** in ADB Scan (see [Updates](#updates)), or run
the new installer over the installed copy. Uninstalling keeps your settings
and history.

The installed folder holds `adb-scan.exe`, the GTK and libadwaita DLLs it uses,
`platform-tools\` (Google's `adb.exe` and `fastboot.exe`) and `scrcpy\`
(screen mirroring). ADB Scan uses these copies first; a different `adb` can be
chosen in Preferences → Tools.

### “Windows protected your PC” (SmartScreen)

The installer is not signed with a paid certificate, so Windows may show a
blue **“Windows protected your PC”** window. Click **More info** → **Run
anyway**.

### Turning on USB debugging on the phone

1. **Settings → About phone** → tap **Build number** seven times (on some
   phones it is under *Software information*) until it says you are a
   developer.
2. **Settings → System → Developer options** → turn on **USB debugging**.
3. Connect the phone with a data cable, unlock it and accept **“Allow USB
   debugging?”** (tick *Always allow from this computer*).

Wireless debugging (Android 11+) works too, from the **Connect** dialog. If the
Windows firewall asks about `adb.exe`, allow it on **private networks**: that
is how the phone is found on the Wi-Fi.

### If ADB Scan does not see the phone

Windows needs a USB driver for ADB. Pixel phones and many others work with
Google's [USB driver](https://developer.android.com/studio/run/win-usb); other
brands publish their own (Samsung, Xiaomi, OnePlus…: search for “<brand> USB
driver”). After installing it, unplug and replug the cable. Also check that the
cable carries data (some only charge) and that the phone is unlocked.

### What is different on Windows

- **Not available on Windows** (each section says so): the basic analysis
  without debugging, the PC side of the cable (port, hubs, cable chip), the
  embedded terminal (the Terminal section opens `adb shell` or a command
  prompt in a window of their own instead), the virtual webcam
  (v4l2loopback) and avahi (adb's own mDNS is used).
- There is no tray icon yet: closing the window quits ADB Scan.
- Data lives in `%APPDATA%\adb-scan` (settings, history, logcat) and
  `%LOCALAPPDATA%\adb-scan` (cache). The log is
  `%LOCALAPPDATA%\adb-scan\adb-scan.log`: attach it when reporting a problem.
- If the window stays blank or flickers (some virtual machines or old
  graphics drivers), start it with the software renderer: in the installed
  folder, type `cmd` in the address bar and run
  `set GSK_RENDERER=cairo && adb-scan.exe`.

### Building for Windows

[`.github/workflows/windows.yml`](.github/workflows/windows.yml) builds
`adb-scan.exe` on `windows-latest` with MSYS2 (UCRT64: GTK 4, libadwaita) and
the `stable-x86_64-pc-windows-gnu` Rust toolchain, downloads platform-tools
and scrcpy pinned by version and SHA-256, and checks that `adb-scan.exe`
starts with only the bundled DLLs. Then it builds the installer with NSIS from
[`packaging/windows/adb-scan.nsi`](packaging/windows/adb-scan.nsi), installs
it silently, starts the installed copy and uninstalls it. It runs on pushes to
`main`, on version tags (the installer is attached to the release) and by hand
from the Actions tab.

## Usage

```bash
adb-scan              # main window
adb-scan --hidden     # tray icon only
adb-scan --panel      # quick panel of the running instance
adb-scan --settings   # preferences
adb-scan --page apps  # open a specific page
adb-scan --version    # show the version
```

Shortcuts: F5 refresh devices · Ctrl+M mirror the screen · Ctrl+Shift+S
screenshot · Ctrl+Shift+P private mode · Alt+1…9 pages · Ctrl+, preferences ·
Ctrl+Q quit.

## Language

The interface is in **English** by default. **Spanish**, the language ADB Scan
was first written in, is available in Preferences → General → Language; the
change applies when ADB Scan restarts (the toast offers to restart it).
`ADB_SCAN_LANG=es adb-scan` forces a language for a single run.

Translations live in `crates/core/i18n/<code>.po`. The code is written in
English and every interface string goes through `tr()` or `t!()`; a test
checks that each of them has a Spanish translation.

## Data

| Data | Path |
| --- | --- |
| Configuration | `~/.config/adb-scan/config.json` |
| SQLite history | `~/.local/share/adb-scan/adb-scan.db` |
| Logcat sessions | `~/.local/share/adb-scan/logcat/` |
| Screenshots | `ADB Scan/` in your Pictures folder |
| Recordings | `ADB Scan/` in your Videos folder |
| Cache (icons, app names) | `~/.cache/adb-scan/` |

Uninstalling does not delete this data.

## Layout

```text
crates/core   Core without an interface: ADB, fastboot, scrcpy, parsers,
              telemetry, USB, SQLite, settings, audit, translations. Tested
              with real outputs captured from a device.
crates/app    GTK 4 + libadwaita interface, tray (StatusNotifierItem).
assets/       Icons (embedded in the binary at build time).
packaging/    .desktop, metainfo, udev rules, Debian scripts.
scripts/      Building, .deb package, per-user installation.
docs/         Usage, basic analysis, architecture, status, limitations and
              research on ADB without root.
```

## Documentation

- [Usage guide](docs/USAGE.md): ways to connect, pages, shortcuts, preferences and the virtual webcam.
- [Basic analysis and ways to connect](docs/BASIC_ANALYSIS.md): what can be known without debugging and how QR and manual pairing work.
- [Architecture](docs/ARCHITECTURE.md): modules, telemetry, threads, translations and tests.
- [Limitations](docs/LIMITATIONS.md): what cannot be done without root and refresh rates.
- [Status](docs/STATUS.md): what is verified and what is pending.
- [Research on ADB without root](docs/ADB_RESEARCH.md): which commands Android allows the `shell` user, measured on a real phone.

## Tests

```bash
cargo test --workspace                           # no phone needed
cargo run -p adbscan-core --example probe 5       # against a real phone
cargo run -p adbscan-core --example watch 3       # ADB and USB monitors
cargo run -p adbscan-core --example audit         # security audit (read-only)
cargo run -p adbscan-core --example gpufps        # GPU and FPS of the foreground app
cargo run -p adbscan-core --example labels        # display names of the apps
cargo run -p adbscan-core --example identity      # which private data can be read (without showing it)
cargo run -p adbscan-core --example battery_usage # per-app usage and wakelocks
cargo run -p adbscan-core --example basic         # basic analysis without ADB (USB and MTP)
cargo run -p adbscan-core --example inspect       # platform sheet and app inspection
```

All of them are read-only except `controls --apply`, which changes harmless
phone controls and restores them.

## Acknowledgements

Some features were inspired by ADBKit; ADB Scan's
code is its own.

## License

GPL-3.0-or-later (see [LICENSE](LICENSE)). Lucide icons under the ISC license (`assets/icons/LICENSE-lucide`).
