# Usage guide

## Getting started

ADB Scan has three independent modes: **basic analysis** (nothing to turn
on), **USB debugging** and **wireless debugging** (QR code or manual). The
last two are the advanced analysis and are **optional**. The **Ways to
connect** dialog (device selector, top right) explains each one and takes
you to it.

**Basic analysis (nothing to turn on in the phone)**

1. Plug in the cable. ADB Scan recognises the phone over USB: manufacturer,
   model, serial number, speed, USB mode and whether it was connected before.
2. To also see the battery and free space, unlock the phone and choose
   **File transfer** in its USB notification. In “Charging only” mode many
   phones share no data at all.
3. Open **Basic analysis** (DATA section, or the button on the Overview).

**Advanced analysis over USB (voluntary)**

1. On the phone: Settings → About phone → tap “Build number” seven times.
   Then, in Developer options, turn on **USB debugging**.
2. Plug in the cable and accept the “Allow USB debugging?” prompt on the
   phone.
3. Open ADB Scan from the menu or with `adb-scan`. The phone appears on the
   sidebar card; if something fails, the **Connection diagnosis** on the
   Overview shows which step is the problem.

**Advanced analysis without a cable (wireless debugging, Android 11+, voluntary)**

Phone and computer on the same Wi-Fi network. On the phone, turn on
Developer options → **Wireless debugging**. In ADB Scan: Ways to connect →
Wireless debugging, and choose an option:

- **With a QR code**: on the phone, “Pair device with QR code”, and scan the
  code on the screen. ADB Scan pairs and connects by itself. Each code works
  for one pairing and expires after 3 minutes or when leaving the screen.
- **Manually**: on the phone, “Pair device with pairing code”. Type the IP
  address, the pairing port and the 6-digit code. The connection port is a
  different one; if you leave it empty, ADB Scan looks for it on the network
  after pairing.
- **Connect to an already paired phone**: IP:port, the “On the network” list
  (a phone waiting for pairing fills in the manual pairing) and “Recent”.

With a phone without debugging, the Overview and Basic analysis show the
invitation “Want a more complete analysis?”. **Continue with the basic
analysis** hides it (it can be shown again in Preferences → General →
Analysis), **How to turn it on** explains the steps for the cable and
**Without a cable (QR or code)** opens wireless debugging. ADB Scan never
turns anything on for you.

If `adb` is missing (or, the first time, `fastboot` or `scrcpy`), the
**tools assistant** opens: it says what is on the computer and downloads
whatever is missing from the official sources (Google's platform-tools and
scrcpy from GitHub), with pinned versions and SHA-256 sums, into
`~/.local/share/adb-scan/tools`, without root. It can also be opened from
Preferences → Tools.

When the window is closed, ADB Scan stays in the tray icon. To quit
completely: tray icon → Quit, or **Ctrl+Q**.

## Pages

The sidebar groups the pages in four sections: **Status**, **Manage**,
**Data** and **Advanced**. Click a section's title to fold or unfold it;
right-click a page (or press and hold) to pin it to the top, under
**Pinned**. Both are remembered.

The search box at the top of the sidebar filters the pages by name or by
what is in them (“calls” leaves Control); Enter opens the first one, and when
no page matches it searches the actions. **Ctrl+K** opens that search of
pages and actions directly: “cache” finds the clean-up, “restart” the
reboot.

### Status

- **Overview**: phone status, battery, CPU, RAM, storage, temperature,
  display, network and USB connection, live. Quick actions for screenshots,
  mirroring, files, apps and basic analysis. **Component health**: battery
  capacity compared to the original, charge cycles and storage lifetime
  (UFS/eMMC) when the phone exposes it. **Health report** saves one page
  (HTML, printable to PDF from the browser) with the battery, storage life,
  bootloader, verified boot, the latest security audit and the public sheet;
  it has no serial number or IMEI, so it can be shared when selling or
  repairing the phone. It is also in the ⋮ menu and in the search.
  **Connection** sums up the connection checks in one row that leads to the
  Connection page.
- **Device**: identity, software, hardware and display (active and
  configured refresh rate), the phone's nickname, reboots (system, recovery,
  bootloader, fastbootd, sideload, EDL) and all properties with a search box.
- **Performance**: total and per-core CPU, GPU, FPS of the foreground app,
  memory, temperatures and thermal sensors grouped by type. **Export CSV**
  saves everything measured during the session.
- **Battery**: status, health, capacity compared to the original, cycles,
  voltage, power and its history (24 h or more). **Analyse usage** (a few
  seconds, read-only) shows, since the last full charge: the apps that use
  the most with their percentage, what drains with the screen off (the apps
  worth restricting) and what keeps the phone awake (app and system
  wakelocks).
- **Connection**: everything about how the phone is connected, in three tabs.
  It also works without USB debugging (the PC's side).
  - **USB**: a summary with what was found, then:
    - *Cable*: USB-C on both ends or not, and whether it has USB 3 wires or
      only USB 2.0. What is read from the hardware is told apart from what is
      deduced (“probable”): 3 A at 5 V or two lanes mean USB-C on both ends;
      a phone that reached USB 3 on this PC and now links at 480 Mb/s means a
      USB 2.0 cable or adapter. Copper or fibre, rated speed and current come
      from the cable's chip (e-marker), which Linux only shows on PCs with a
      readable USB-C controller (UCSI); Android hides it without root. Hubs
      and docks in between are shown; a simple adapter leaves no trace.
    - *PC port*: negotiated speed and lanes, bus and port, the port's limit,
      its location according to the firmware, the USB controller, the power
      the phone asks for, link power saving and over-current events; on
      USB-C ports with a readable controller, the current offered and which
      way round the plug is.
    - *Phone's port* (with debugging): controller, port modes, current role,
      moisture and dirt detection, Android's cable warnings, video output
      over USB-C, how long it has been plugged in.
    - *Power*: how Android sees the cable, the volts, amps and watts the port
      offers and what the battery is taking right now (or that charging is
      paused at the phone's charge limit, which is not a fault).
    - *Stability*: a line of time with each cut recorded by the phone, the
      worst minute and the last cut. Many cuts in a row point to a loose
      plug, a worn cable or a dirty port; a burst more than an hour old is
      shown as a note, not a warning.
    - *Real speed*: sends 128 MB to the phone and reads them back
      (`/data/local/tmp`, deleted afterwards) and compares the result with
      what the link allows. The last 30 measurements are kept with the port
      and the link speed, the best one marked: measure with each cable or
      port to compare them.
  - **Wi-Fi**: the current connection and **wireless ADB** (switch the
    connected phone to Wi-Fi with `adb tcpip`, or pair with a QR code or
    manually).
  - **Diagnosis**: the connection checks (ADB, USB, permissions,
    authorisation, telemetry), the phone's USB mode and the ADB server.
- **Network**: live Wi-Fi, interfaces and routes, and traffic capture with
  TShark (only on your own networks).

### Manage

- **Apps**: list with real names, size and date. Sort by name, size,
  recent or cache; filter by type and by source (Play Store, other stores,
  APK/ADB). Under the list, the cache of the apps in view; in the details,
  the space split into app, data and cache. Android measures these sizes
  about once a day, so they show the date of that measurement.
  In the details: open, stop, clear cache, clear data, disable, extract the
  APK, uninstall, permissions and what the last audit said about that app.
  Select several apps to stop them, clear their cache, disable, enable,
  extract or uninstall them together. Drag an `.apk` to install it.
  The **⋮** button next to *Install APK* acts on all apps, always after
  showing what it will do:
  - **Close open apps**: the same as “Clear all” in Recents. Apps that were
    already closed may keep their card in Recents: Android does not let ADB
    remove it.
  - **Stop all apps**: also stops the apps running in the background and asks
    Android to free the processes that are not needed. It keeps system apps,
    the home screen, the active keyboard, the calls and SMS apps,
    accessibility services and the live wallpaper. Stopped apps do not
    receive notifications or alarms until you open them again.
  - **Clear all caches**: deletes the temporary files of every app; no data,
    accounts or settings. Clearing the cache of a single app needs an Android
    that supports it (`pm clear --cache-only`); on older versions ADB Scan
    refuses instead of risking the app's data.
  - **Export apps…**: copies the APKs of your apps (splits included) to a new
    folder with a manifest, to install them on another phone. Only the apps:
    their data comes from each app's own backup (WhatsApp, Telegram, the
    Google account…), because without root Android does not let ADB read it.
  - **Install from an export…**: choose an exported folder, check the apps
    (those the phone already has start unchecked) and install them.
- **Files**: browser of the phone's storage with uploads and downloads with
  progress. The folder refreshes by itself if it changes. The **photos**
  button copies the new photos and videos from `DCIM`, `Pictures` and
  `Movies` to the laptop (it keeps the subfolders and skips what was already
  copied); it remembers the destination folder and changes nothing on the
  phone.
- **Screen**: mirroring with scrcpy (recording, the phone's camera, a virtual
  webcam on the PC, profiles), screenshots and remote keys (power, volume,
  home, back…).
- **Control**, in three tabs:
  - **Quick settings**: the phone's quick settings (Wi-Fi, mobile data,
    Bluetooth, airplane mode, brightness, volume, Do Not Disturb…) and
    **Power**: restart or power off (clean shutdown; it is turned back on with
    its button).
  - **Remote**: control the phone from the laptop.
    - **Media**: what is playing, play/pause, previous, next and volume.
    - **Calls**: call a number (if Android does not allow calling over ADB, it
      opens the dialer with the number), answer and hang up.
    - **Send to the phone**: open a link on the phone, or leave an SMS written
      in the messaging app for you to send.
    - **Find the phone**: makes it ring with a 1-second timer on its clock
      (alarm volume, even when silenced).
    - **Phone notifications** on the desktop. Off by default: the text is
      private, it is not stored and private mode hides it.
    - **Scheduled actions**: power off, restart, stop all, Do Not Disturb or
      turn off the screen at a time and on some days. ADB Scan runs them while
      it is open (also when hidden in the tray) and the phone is connected; it
      notifies each time.
  - **Automations**: actions that run by themselves when a phone connects
    (by cable or Wi-Fi) or when ADB Scan starts with it connected: copy the
    new photos (to the folder chosen in Files), turn Do Not Disturb on or
    off, keep the screen on while charging or let it turn off, stop all
    apps, clear all caches or open the mirror. For any phone or only for
    the one connected when you add it. Each one runs at most once every
    10 minutes per phone and is recorded in History.
- **Preinstalled apps**: the manufacturer's, the carrier's and Google's
  apps that can be removed, from a curated list, grouped as *Recommended*
  (ads, telemetry, promotional apps), *Optional* (a feature some people use)
  and *Expert* (part of a system feature; it asks first). **Remove** takes
  the app away for your user only (`pm uninstall --user 0`): its APK stays
  in the system partition. **Removed for your user** lists every app removed
  that way, by ADB Scan or another tool, with **Restore**; a factory reset
  also brings them back.
- **Permissions**: each sensitive permission (location, location in the
  background, camera, microphone, contacts, SMS, call log, phone, calendar,
  photos and files, body sensors, nearby devices) and special access
  (accessibility, notification access, display over other apps, install
  unknown apps, all files access, usage access, modify system settings) with
  the apps that have it. **Take away** turns it off as Settings would; the
  app can ask again. Preinstalled apps are hidden unless you include them,
  and taking a permission from one asks first. Permissions fixed by the
  system or a device policy are shown as such.
- **Storage**: **Scan** reads the shared storage once (about a minute for
  every 50 GB; it can be cancelled). Then: space by file type, *places that
  grow* (WhatsApp media, Telegram, downloads, camera, screenshots,
  recordings, app files, game data, thumbnail caches), a folder browser
  sorted by size, the 100 largest files and possible duplicates (same name
  and size in different folders). Files are deleted one at a time, after
  confirming; folders are never deleted from here.

### Data

- **Basic analysis**: what is known about the phone without developer options
  or debugging, read-only. Each row carries its status: ✓ available,
  ⊗ not available (with the reason) or 🔑 requires USB debugging. Each row's
  tooltip says where the data comes from. Groups: identification (USB
  descriptor), USB connection (mode chosen on the phone, debugging on or
  off, speed, permissions), file transfer (MTP: model, the version the phone
  declares, battery and the space of each storage; files are never read),
  recognised on this computer (local history, by serial number) and what only
  the advanced analysis provides. If the file manager has the phone mounted,
  the space is read through it and the battery is not available: ADB Scan
  does not take the phone away from it. Refresh reads again. It also works
  with ADB authorised, in parallel.
- **Phone data**: two views chosen with the cards at the top.
  - **Public information**: model, system, hardware and battery. It is what
    identifies the model, not the person, so it is useful to ask for help on
    a forum or from technical support. **Copy sheet** copies it as text
    (without the serial number, IMEI, accounts or the phone's nickname).
  - **Private information**: IMEI (one per slot), serial number, Android ID,
    device name, SIM (IMSI, ICCID, number if the SIM stores it, carrier of
    each slot), network (public IP, local IP, Wi-Fi network, Wi-Fi and
    Bluetooth MAC) and the phone's accounts. Values start hidden: the eye
    shows one and “Show all” shows every one. When copied, the value is
    cleared from the clipboard after 30 s. Nothing is stored: when leaving the
    page it returns to the public view and the data is wiped from memory. The
    **public IP** is only queried when pressing “Query” (see Limitations).
- **Security**: read-only audit with a score. From the findings you can remove
  the “Install unknown apps” permission, open each app in Apps or turn off
  USB debugging. The report is exported as Markdown. **Delete reports**
  removes the saved ones for that phone (those exported to files are kept).
- **History**: connections, events and operations performed.

### Advanced

- **Flasher**: with the phone in fastboot, flashes a partition or a full ROM.
  It always asks you to type the serial number and checks that the model
  matches the one ADB Scan saw in Android and the one the ROM requires. Also:
  A/B slots, booting an image without flashing, sideload, and unlocking and
  locking the bootloader.
- **Terminal**: a real terminal. **Phone** opens `adb shell`; **PC** opens your
  shell with `adb` and `fastboot`. The buttons at the top type common
  commands. The up arrow brings back commands, also from previous sessions.
- **Logcat**: live system log with filters by level, tag, text and package;
  it can be saved.
- **Tools**: live processes (stop user apps), sensors, system services and
  **Developer**:
  - Switches: show taps, pointer location, layout bounds and stay awake while
    charging; the animation speed (the three scales at once).
  - **Screen**: resolution and density. After **Apply** a dialog asks to keep
    the change and undoes it by itself after 15 seconds without an answer;
    **Restore the original** goes back to the phone's own values.
  - **Status bar demo mode**: full battery and signal, 12:00 and no
    notification icons, for clean screenshots. It changes nothing real.
  - **Port forwarding**: *Phone → PC* (`adb reverse`) makes a server on the PC
    reachable on the phone as `localhost`; *PC → phone* (`adb forward`) the
    opposite. Rules last until ADB or the phone restarts.

## Private mode

The crossed-out eye button in the top bar (or **Ctrl+Shift+P**, or
Preferences → Privacy and data) hides, across the whole app, serial numbers, IP
and MAC addresses, Wi-Fi network names and phone nicknames (the marketing
name is shown instead). It is useful for screenshots, screen recordings or
screen sharing. While it is on, the button is highlighted.

It is **display only**: queries to the phone, the saved history, logcat
captures, file names, exports and what the copy buttons copy keep the real
data. It applies to the sidebar, the title, the quick panel, the tray icon,
notifications, toasts, tooltips and every page.

In the Flasher, the dialogs that ask you to type the serial number hide it
and offer “Show serial number” only inside the dialog. In the “Ways to
connect” dialog, the IP fields look like password fields.

## Shortcuts

| Shortcut | Action |
| --- | --- |
| Ctrl+K | Search pages and actions |
| Alt+1 … Alt+9 | Go to the first nine pages |
| F5 | Refresh devices |
| Ctrl+M | Mirror the screen |
| Ctrl+Shift+S | Screenshot of the phone |
| Ctrl+Shift+P | Turn private mode on or off |
| Ctrl+, | Preferences |
| Ctrl+W | Close the window (it stays in the tray) |
| Ctrl+Q | Quit |

In the terminal: Ctrl+Shift+C / V copy and paste, Ctrl+Shift+K clear,
Ctrl + / Ctrl − / Ctrl+0 font size, right click for the menu.

## Command line

```bash
adb-scan              # main window
adb-scan --hidden     # tray icon only
adb-scan --panel      # quick panel
adb-scan --settings   # preferences
adb-scan --page apps  # open a page: dashboard, device, performance,
                      # battery, connection, network, apps, files, mirror,
                      # control, basic, data, security, history, terminal,
                      # logcat, tools, flasher
adb-scan --version    # show the version
```

`ADB_SCAN_LANG=es adb-scan` starts it in Spanish (or `en`, English) for that
run only, without changing Preferences.

## Preferences

- **General**: language (English or Spanish; applied when ADB Scan
  restarts), light or dark, accent colour, animations; close to the tray,
  start hidden, start with the session; the optional invitation to the
  advanced analysis.
- **Monitoring**: the telemetry interval with the window open and in the
  background, and the alerts grouped by subject, each threshold next to its
  alert:
  - *Connection*: phone detected, waiting for authorisation, connected,
    disconnected, errors, **unstable USB connection** (the phone drops off
    USB several times in a minute) and **the cable limits the speed** (back
    at USB 2.0 after reaching USB 3 on this PC).
  - *Battery and temperature*: battery full, low battery and its threshold
    (15 % by default), high temperature and its threshold.
  - *Performance*: phone CPU at full load for more than a minute (it says
    what is using it).
- **Connection**: custom paths for `adb`, `fastboot`, `scrcpy` and `tshark`,
  the detected ones and the assistant that downloads the missing ones; what
  is remembered (wireless addresses, the fastest USB link of each phone) with
  a button to forget it.
- **Privacy and data**: private mode, the check for new versions, operations
  log, logcat on disk, logcat and terminal lines, days of history.

## Updates

At startup ADB Scan asks GitHub once, in the background, for the latest
release of `LubrieDev/ADB-Scan`. If it is newer than the running version, a
banner under the header bar says so, and its **View release** button opens
the release page in the browser. Nothing is downloaded or installed: the
new `.deb` is installed by hand. Without Internet, without releases or with
any other problem nothing is shown. It can be turned off in Preferences →
Privacy and data → Updates.

## Virtual webcam

To use the phone's screen or camera as the PC's webcam:

```bash
sudo apt install v4l2loopback-dkms
sudo modprobe v4l2loopback card_label="ADB Scan" exclusive_caps=1
```

Then, in Screen, turn on **Send to a virtual webcam on the PC** and press
Start. The “ADB Scan” camera will appear in Meet, Zoom or OBS.
