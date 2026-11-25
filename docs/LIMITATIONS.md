# Known limitations

ADB Scan only shows real data: what Android does not expose appears as “—”.

## Permissions

- No root. Some paths, properties and processes cannot be read by Android's
  `shell` user; the interface explains why.
- Without udev rules, the phone may appear as “No permissions”. The `.deb`
  package installs them.

## Basic analysis (without debugging)

- In “Charging only” mode, many phones (the REDMAGIC among them) do not show
  up on the USB bus: there is nothing to read until another mode is chosen.
- Battery, internal model and storage come from MTP: “File transfer” (or
  “photo transfer”) and an unlocked screen are needed. While locked, Android
  does not share the storage.
- Linux Mint mounts the phone when it is connected and the file manager keeps
  the MTP interface. ADB Scan does not take it away: it reads the storage
  through the mount (GVfs) and the battery shows as not available. When it is
  unmounted, some phones reconnect and Mint mounts them again.
- “Declared version” is the one the manufacturer puts in MTP (on the REDMAGIC,
  its own system's, “REDMAGICOS…”), not the Android version: it is shown as
  is. The Android version requires ADB.
- Not every phone publishes the device name (MTP property); the REDMAGIC
  leaves it empty.
- There is no basic analysis over Wi-Fi: without wireless debugging, Android
  does not announce itself on the network in an identifiable way and ADB Scan
  does not scan the network.
- “Seen before” only recognises phones that were connected at some point with
  authorised debugging (that is what the history keeps).

## Wireless debugging (QR and manual)

- Android 11 or later, and phone and computer on the same network. Networks
  that isolate clients or block mDNS (guest or corporate networks) prevent the
  QR code and the automatic search for the connection port; manual pairing
  with both ports typed in works if there is a direct connection.
- Debian/Ubuntu's `adb` (34.0.4) comes without mDNS (`adb mdns check` does not
  respond): ADB Scan uses `avahi-browse` (avahi-utils package, installed on
  Linux Mint). Without either of them, the QR code cannot detect the phone.
- The pairing is saved on the phone and in `~/.android`. The connection is
  not: after restarting the phone or turning off Wi-Fi you have to connect
  again (without pairing), and the connection port changes every time
  wireless debugging is turned on.

## Terminal

- It is a real terminal (VTE, the one in GNOME Terminal): interactive
  `adb shell` and the PC's shell. That is why it does not filter commands:
  whatever is typed runs as is, as in any terminal. fastboot operations with
  safeguards (validation and confirmation) are in the Flasher.
- The phone session closes if the phone disconnects and reopens by itself when
  it is connected again.
- Android's mksh does not save history to disk: ADB Scan saves the commands of
  the phone session and preloads them when opening another one (file
  `/data/local/tmp/.adb_scan_mkshrc`). Only lines typed at the prompt are
  recorded, not what is typed inside `top` or `vi`.

## Apps

- ADB does not provide the apps' display names. They are read from the APK
  itself with `unzip` on the phone, in the interface language when the app
  has it; if there is no `unzip`, the package identifier is shown.
- System apps can only be uninstalled for user 0 (reversible with “reinstall
  existing”).

## Controls

Verified on the REDMAGIC 11S Pro (Android 16, without root). Every change is
checked by reading the state back: if Android does not apply it, the control
returns to its real value and the reason is explained.

- **Flashlight**: Android has no shell command for it. The phone's quick
  settings panel can be opened to tap it there.
- **Wi-Fi hotspot**: `cmd wifi start-softap` only exists in debug builds and the
  tethering service does not accept shell commands. Whether it is sharing is
  shown and its Settings screen is opened. (Detection of the active state
  could not be checked with the hotspot on.)
- **NFC**: `svc nfc enable` is killed by the system on this phone. The state is
  shown and its screen is opened.
- **Volume**: this phone accepts and ignores `cmd media_session volume --set`.
  Media volume is changed with the volume keys (the volume panel appears on
  the phone, as when pressing the buttons) in steps of 10. The ring volume
  cannot be changed over ADB: you are told so.
- **Battery saver**: Android does not turn it on while the phone is charging.
- **Mobile data**: the state is that of the data SIM (`mobile_data<SIM>`), not
  the global setting. With several SIMs, the data one is changed.
- **Wireless ADB**: turning off Wi-Fi or turning on airplane mode cuts the
  connection. With the phone connected over Wi-Fi, confirmation is requested.
- Versions: Wi-Fi and location through `cmd` since Android 11 (before, `svc` and
  `settings`), Bluetooth and mobile data through `cmd` since Android 13
  (before, `svc`), airplane mode only since Android 11.
- Settings screens open behind the lock screen if the phone is locked.

## App technical details

- Components are read from the installed APK's manifest with `unzip -p` on
  the phone itself. Without `unzip`, “Not supported” is shown.
- “Implicitly exported”: without `android:exported`, Android infers it (with
  intent filters → exported; providers → only if the app targets API < 17). A
  value that depends on a resource (`@bool/…`) is treated as not declared.
- Android only publishes the signing certificate (SHA-256) for apps with web
  links (`pm get-app-links`, Android 12+). Computing it for the rest would
  require reading the APK's signing block.
- App Ops records the last access of each operation, not a full history.
  “Denied” means a rejected attempt.
- Exits (`exit-info`, Android 11+) include normal ones: removing the app from
  Recents or the system freeing memory are not failures.
- The process state (on screen, cached…) only exists for app processes;
  native daemons do not have it.

## Platform sheet

- Thermal limits are the thresholds declared by the manufacturer, not
  measurements. Many phones do not publish them.
- GPU frequency, PSI (`/proc/pressure`), zram, `dmesg` and `/proc/partitions`
  are blocked without root on the REDMAGIC.
- The `gpu_model` model only exists on Adreno GPUs (Qualcomm).
- “Hardware features”: “No” means the manufacturer does not declare it to
  Android, which almost always means it does not exist.

## Files

- Progress is measured by querying sizes (adb does not report it without a
  terminal). When uploading a whole folder there is no percentage, only
  activity.

## Flasher

- It uses the system's `fastboot` or the one from the platform-tools the tools
  assistant downloads (Preferences → Tools), always on request.
- `payload.bin` (OTA) is not extracted; you are told so and `adb sideload` is
  offered.

## Performance

- GPU: only on Adreno (Qualcomm), reading `kgsl-3d0/gpubusy`; other chips do
  not expose it without root.
- FPS: frames presented by the foreground app (SurfaceFlinger). An idle screen
  shows 0 FPS: there is nothing new to draw.
- Temperature thresholds depend on the sensor type (CPU/GPU: watch from 85 °C,
  high from 100 °C; battery and skin: 40/45 °C).

## Component health

- Battery: real capacity compared to the design capacity, and cycles, read
  from the charging chip (`/sys/class/power_supply/battery`). If the
  manufacturer blocks it, the capacity learned by Android (`batterystats`) is
  used. Inconsistent readings are discarded (design outside 500–30,000 mAh or
  a ratio outside 30–130 %); the maximum shown is 100 %.
- Storage: the UFS/eMMC chip reports its lifetime in 10 % steps (JEDEC
  standard), which is why it is shown as “90–100 %”. On the REDMAGIC (and on
  most phones) reading it requires root: “—” appears with the reason.
- CPU, GPU, display and the rest have no wear measurement at all in Android: a
  percentage that does not exist is not shown.

## Phone data

- IMEI, IMSI and ICCID are read with `service call iphonesubinfo`, whose
  numbering changes between Android versions. Each value is only accepted if
  it has the right shape (IMEI: 15 digits passing Luhn; IMSI: starts with the
  carrier code; ICCID: starts with 89); otherwise “—” appears. Verified on
  Android 16; on other versions some may be missing.
- The phone number only appears if the SIM stores it; many do not (on the test
  phone, it does not).
- The Wi-Fi MAC is that of the current network: since Android 10 it is random
  per network and Android does not show the factory one without root.
- **Public IP**: together with the tools assistant's downloads, it is ADB
  Scan's only connection to the Internet, and it only happens on request. It
  is made by the phone (with `curl`, to `api.ipify.org`) to give the IP of its
  network, which may not be the PC's (mobile data), and only when pressing
  “Query”. That service sees the phone's IP, like any website it visits.
- The Android ID is different for each app and user since Android 8: the one
  shown is that of ADB's `shell` user.

## Battery usage

- It is Android's estimate (BatteryUsageStats), the same as in its settings: it
  only counts time on battery and attributes the screen to the app in the
  foreground. What cannot be attributed (mobile radio, GPU, idle) is shown as
  “Not attributed to any app”.
- It covers the time since the last full charge: if it has just been charged
  to 100 %, there is no data yet.
- “ADB shell” includes ADB Scan's telemetry; with the phone plugged in it does
  not count.
- Normal and expedited wakelocks of the same job are added up in one row; the
  tooltip shows the original tags.
- ADB Scan never uses the options that clear or write statistics (`--reset`,
  `--write`…).

## Private mode

- The **Terminal** is a real terminal: what the phone prints (for example,
  `ip addr` or `getprop ro.serialno`) appears as is. Its title is hidden.
- On the **Phone data** page, the eye and “Show all” still reveal values: it is
  a deliberate action.
- The **copy** buttons copy the real value, and **exports** (CSV, reports,
  logcat) contain the real data: they are not display.
- What you type into a text field is visible while you type it (except the IP
  fields in the “Ways to connect” dialog, which look like password fields).
  The nickname and the traffic-capture filter are replaced by a “hidden” row
  so the mask cannot be saved by mistake.
- A Wi-Fi network whose name is shorter than 4 characters is hidden in its own
  rows (Overview, Network, Data), but not inside free text such as events or
  logcat: searching for it as text would mask single letters.
- IPs and MACs are recognised by their shape. A version with exactly four
  numbers (“1.2.3.4”) would be hidden as if it were an IP; usual times, dates
  and versions are not touched.

## Screen

- The active refresh rate is read every 2 s (`mActiveRenderFrameRate`); the
  configured one, every 10 s. With the screen off or dozing (AOD) Android drops
  to 60 Hz even if the setting is higher: this is noted next to the value.
- scrcpy (SDL) on X11 stores the window title as Latin-1: it is converted to
  ASCII (`·` → `-`, `ñ` → `n`) so it does not come out garbled.
- Virtual webcam: requires the `v4l2loopback` module
  (`sudo apt install v4l2loopback-dkms`, `sudo modprobe v4l2loopback card_label="ADB Scan" exclusive_caps=1`).

## Language

- The language is chosen at startup: a change in Preferences applies when ADB
  Scan restarts.
- Texts already saved keep the language they were written in: History events
  and security reports saved before a language change are shown as they were
  stored.
- What comes from the phone (app names, Android messages, logcat) is shown as
  the phone gives it.

## Desktop

- The tray icon uses StatusNotifierItem (Cinnamon shows it with
  `xapp-sn-watcher`). It may not appear on other desktops.
- GTK 4 does not allow placing the quick panel next to the icon; the window
  manager positions it.
- Only one ADB Scan runs at a time: starting it again sends the command to the
  running one.
- Linux only: ADB Scan targets Linux Mint Cinnamon.

## Telemetry

Refresh rate while the page is in view:

| Data | Every |
| --- | --- |
| CPU, RAM, temperatures, battery, storage, network, brightness | 0.5 s |
| Wi-Fi (network, signal, speed) | 1 s |
| Active refresh rate and screen state | 2 s |
| GPU and FPS | 1.5 s |
| Processes (with “Live”) · open app running | 3 s |
| Open folder in Files (only if it changed) | 5 s |
| App list (only if an app was installed, removed or disabled) | 15 s |
| Properties, resolution, density, configured rate, battery and storage health | 10 s |
| Battery history | 60 s |

Hidden pages query nothing: they catch up when you return.

- The rate is a target: if the phone takes longer, the block arrives later. If
  the stream drops, it is retried with an increasing backoff.
- The battery history chart groups samples into time bands; periods without a
  connection show up as breaks in the line.
