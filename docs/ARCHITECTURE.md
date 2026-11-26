# Architecture

ADB Scan is a Rust workspace with two crates:

```text
crates/core   adbscan-core   Logic without an interface. Does not depend on GTK.
crates/app    adb-scan       GTK 4 + libadwaita + VTE interface.
```

The split makes it possible to test almost the whole core without a phone or a
screen: the tests use real outputs captured from a REDMAGIC 11S Pro
(`crates/core/tests/fixtures/`).

## Core (`crates/core/src`)

| Module | Responsibility |
| --- | --- |
| `exec.rs` | Starts processes with a predictable environment (`LC_ALL=C`), a timeout, cancellation by process group and line-by-line reading (`stream`). |
| `adb.rs` | `adb` client: devices, `shell`, apps, files with progress, permissions, reboots, GPU/FPS, processes. Validates packages, paths and arguments before running anything. |
| `fastboot.rs` | Variables (`getvar all`), flashing with confirmation levels, ROM plans read from `flash_all.sh` without running it, model check. |
| `scrcpy.rs` | Version detection, per-profile arguments, recording, virtual webcam (`--v4l2-sink`), ASCII title. |
| `live.rs` | Live telemetry: a single `adb shell` with a loop that emits `@@@SECTION@@@` blocks. 4 s watchdog and retries with an increasing backoff (1–8 s). |
| `parsers.rs` | All the parsers of Android output (`dumpsys`, `getprop`, `ps`, `top`, `ip`, `df`, `cmd wifi`…). Pure functions. |
| `models.rs` | Data types (`Snapshot`, `PackageInfo`, `Temperature`…) and their presentation logic (thresholds, texts). |
| `monitor.rs` | `adb track-devices -l` (frames with a hexadecimal length) with fallback polling. |
| `usb.rs` | Kernel USB events over netlink and sysfs reading. Tells ADB, fastboot and EDL apart by their USB interfaces; reads interfaces and endpoints and deduces the phone's USB mode (`UsbFunction`: MTP, PTP, tethering, MIDI, webcam…). |
| `mtp.rs` | **Basic analysis**: read-only MTP/PTP client over usbfs (device information, storages and three properties; never files). It does not claim an interface that has an owner; if GVfs has the phone mounted, it reads the storage with `gio`. |
| `wireless.rs` | Wireless debugging: QR credentials and content, the code's matrix (`qrcode`), discovery through `adb mdns` and `avahi-browse` (announcements only). Pairing and connecting are `Adb::pair`, `Adb::pair_with_password` and `Adb::connect`. |
| `basic.rs` | **Basic analysis**: report without ADB from USB, MTP and the history, with every item available, not available (and why) or requiring debugging. See `docs/BASIC_ANALYSIS.md`. |
| `tracker.rs` | Notification state machine with settling times so blips are not reported. |
| `identity.rs` | Phone data: public sheet and on-demand reading of private identifiers (`service call iphonesubinfo`, `settings`, `dumpsys account`). Each value is accepted by its shape (Luhn, carrier prefix, length), not by the code that returned it, because the numbering changes between Android versions. |
| `privacy.rs` | Private mode: `Redactor` hides, in text already prepared for display, the learned serial numbers and networks (literals of 4+ characters) and the IPs/MACs recognised by their shape (without confusing times, dates or versions). Display only. |
| `security.rs` | Audit script (read-only) and its evaluation: findings, severity, packages involved and score. |
| `battery_usage.rs` | Per-app usage (`dumpsys batterystats --usage`, only the periods on battery) and wakelocks (summary of `--charged` filtered on the phone). Two read-only queries in parallel; UID → package with `pm list packages -U`. |
| `apk.rs` | APK reader: binary manifest and `resources.arsc` for the display name (in the interface language when available), version, SDK and ABIs. `manifest_summary` reads the components (activities, services, receivers, providers) with their effective export and permission. |
| `link.rs` | **The physical link** (read-only). PC side from sysfs: speed, lanes, the port and its USB 3 companion (`peer`), hubs, controller (PCI name from `pci.ids`), ACPI location, over-current, LPM and, when Linux has a `typec` class for the port, the cable's e-marker (USB PD cable VDOs: passive/active, copper/optical, rated speed, current, length). Phone side from `dumpsys usb`/`dumpsys battery`/properties: roles, modes, moisture detection, compliance warnings, current offered and the USB event log (`stability`). `assess` turns both into deductions marked as read or probable and a list of findings; `LinkWatch` counts drops off the bus and compares each phone's speed with the best it reached. The speed test (`Adb::speed_test`) sends 128 MB of incompressible data to `/data/local/tmp` and back; `SpeedRecord` keeps the latest ones in the settings. `charge_now` reads the battery's live current and voltage and the charge limit (`charge_control_end_threshold`) to tell charging, paused at the limit, full and on battery apart. |
| `controls.rs` | **Controls that change the phone** (Wi-Fi, mobile data, Bluetooth, airplane mode, location, brightness, volume, Do Not Disturb…), kept apart from everything else. Read-only state reading (~0.2 s); fixed commands chosen by Android version, without user text; after each change the state is read again to confirm Android applied it. |
| `cleanup.rs` | **Actions on all apps** (they change the phone). *Close open apps*: removes each app window with `am stack remove`, as swiping it away in Recents. *Stop all*: closes the user's running apps with `am force-stop` and frees the expendable ones with `am kill-all`; it never closes system apps or those the phone needs (home screen, active keyboard, calls, SMS, accessibility, live wallpaper). *Caches*: `pm trim-caches` for every app and `pm clear --cache-only` for one, only after checking that `pm` knows the option (an older `pm` ignores it and would clear all the data). |
| `remote.rs` | **Remote** (changes the phone): media keys, call/answer/hang up, open links, prepare SMS and find the phone (clock timer). What the user types (numbers, links, messages) is validated and quoted. Read-only reading of the call state (`telephony.registry`) and of what is playing (`media_session`). |
| `schedule.rs` | Scheduled actions: time, days and action; when each one is due (once per day, with a 5-minute window). Local time with `localtime_r` (respects daylight saving time). The app runs them (`ctx.rs`). |
| `notifications.rs` | Phone notifications (`dumpsys notification --noredact`, read-only): title, text, ongoing ones and group summaries. Optional and off by default; nothing is stored. |
| `changes.rs` | **Phone changes** (read-only): one script for the boot reason history (`persist.sys.boot.reason.history`), the uninstalls in the battery history (`pkgunin`, years from its `TIME:` lines), and each package's `firstInstallTime` / `lastUpdateTime` in the phone's time zone (`date +%z`). Factory dates (before 2010) and an install's own “update” are left out. |
| `debloat.rs` | **Preinstalled apps** (they change the phone): the curated list (package, what it does, risk level, vendor), packages never offered (`is_protected`) and the state read in one script (listed apps still installed, every app removed for the user). Removing is `pm uninstall -k --user 0`; restoring is `cmd package install-existing`. |
| `permissions.rs` | **Who can do what**: the accesses (runtime permission groups, App Ops, accessibility services, notification listeners) and their holders, from one read-only script (`dumpsys package packages`, user 0 only, up to “Hidden system packages”; `appops query-op`; two secure settings). Taking one away: `pm revoke`, `appops set … deny`, the accessibility list rewritten without the component, `cmd notification disallow_listener`. Permissions fixed by the system or a policy are shown as such. |
| `storage.rs` | **Storage analyzer**: parses one `du -a -k /sdcard/` walk into a tree; largest files, space by type, places that grow, possible duplicates (same name and size, not caches) and `remove_file` to keep the view right after a deletion. `deletable` limits deletions to files inside `/sdcard/` (`Adb::delete_shared_file` uses `rm -f` without `-r` and checks the file is gone). |
| `devtools.rs` | **Developer tools**: the switches and their fixed scripts, the state script, animation scales, demo mode, port rules (`adb forward/reverse --list`) and the checks on every number that reaches the phone (ports, size, density). |
| `health_report.rs` | **Health report**: self-contained HTML from the snapshot and the latest audit, with no serial number, IMEI or accounts. Nothing is read from the phone. |
| `migration.rs` | **App migration**: the manifest (`adb-scan-apps.json`), the user's apps with their version, and the checks on the paths read back from it (`safe_relative`: nothing outside the export folder). |
| `automation.rs` | **Automations on connect**: actions, which phone (hardware serial `ro.serialno`, the same over USB and Wi-Fi) and `due` with the 10-minute cooldown. `Ctx::run_automations` runs them on a phone's first telemetry; `Adb::run_automation` does each one. |
| `photos.rs` | Photo backup: read-only listing of `DCIM`, `Pictures` and `Movies`, and a plan of what is new (same name and size = already copied). It never writes outside the destination folder. |
| `probe.rs` | Availability of each item (`Probe`): available, not published, restricted by Android, not supported or an error. Classifies the error messages of `toybox`, `cmd` and `dumpsys`. |
| `platform.rs` | Platform sheet: a read-only script (~0.4 s) with SELinux, kernel, memory page size, GPU (GLES and Vulkan), CPU clusters, the manufacturer's thermal limits and hardware features; plus what is derived from `getprop` (Treble, A/B, verified boot, encryption…). |
| `app_inspect.rs` | Technical details of an app: a read-only script (~0.3 s) with `dumpsys package`, `pm get-app-links`, standby bucket, App Ops, battery exemption, `exit-info` and `activity lru`. `parse_lru` also gives the Android state to the process list. |
| `db.rs` | SQLite: sessions, events, audit log, battery samples, security reports. |
| `settings.rs` | Settings in JSON. Keeps unknown keys. Interface language. History of the phone terminal. |
| `tools_setup.rs` | Tools assistant: downloads platform-tools and scrcpy from their official sources on request, with a pinned SHA-256 sum (see “Managed tools”). |
| `updater.rs` | Updates. The check asks GitHub once per start for the latest release (5 s timeout, any failure means “no update”) and compares its tag with the running version using semver. It also picks this system's package among the release's files (`adb-scan_*_amd64.deb` or `ADB-Scan-*-setup-x64.exe`), only from this repository's downloads and only with the SHA-256 sum GitHub publishes. `download` fetches it with progress and checks size and sum (nothing is kept otherwise); `install_deb` runs `pkexec apt-get install` and `start_installer` runs the Windows installer silently with `/RUN`. `can_install` says whether this copy came from those packages. |
| `i18n.rs` | Interface language: `tr`, `trc`, `t!` and `tc!`, the `.po` catalog reader and `Language`. See “Translations”. |
| `paths.rs`, `format.rs`, `error.rs` | XDG paths, number and date formatting (following the language), errors. |

### On-demand data and its status

What does not change every half second is not part of the telemetry: the
platform sheet is read when Device opens (and with its refresh button) and an
app's details when it is selected (reused for 20 s). The order is always the
same:

```text
fixed script (adb.rs) ─▶ @@@X@@@ sections ─▶ pure parser ─▶ Probe<T> ─▶ pages/facts.rs
 read-only               split_sections       platform.rs /    status per    one row per item
                                              app_inspect.rs   item          with its source
```

A failing section does not invalidate the others, and an error inside a line
(e.g. the frequencies of a cluster) does not invalidate the section. The
`scripts_only_read` tests (and `read_scripts_only_read`, for the Preinstalled
apps, Permissions, Storage and Developer scripts) check that the scripts
contain no verb that modifies the phone.

### Live telemetry

A single `adb shell` process per phone runs a loop every 500 ms (less often if
no window is visible). Each round prints sections; `live.rs` splits them,
parses them and keeps in an accumulator what is not sent every round:

| Sections | Rate |
| --- | --- |
| Battery, memory, storage, temperatures, CPU, load, network, USB, brightness | every round (0.5 s) |
| `WIFI` (`cmd wifi status`) | every 2 rounds |
| `REFRESH` (active rate, screen state) | every 4 rounds |
| `PROPS`, `CPUINFO`, `SIZE`, `DENSITY`, `DISPLAY`, `REFRESHCFG`, `WEAR` | every 20 rounds |

The script only interpolates numbers: it never contains user text.

`WEAR` reads wear from sysfs: the battery's cycles and real/design capacity,
and the UFS health descriptor (`health_descriptor`) or eMMC (`life_time`,
`pre_eol_info`). If the capacity is not readable, `Ctx` asks
`dumpsys batterystats` for it once per phone (~10 s, in the background) and
adds it to the following snapshots.

## Interface (`crates/app/src`)

| File | Responsibility |
| --- | --- |
| `main.rs`, `app.rs` | Startup (the language is fixed first), single instance (`com.adbscan.AdbScan`), command-line options, capture mode for tests. |
| `ctx.rs` | Shared state (`Rc<Ctx>`) on the GTK thread: devices, snapshot, metrics, session log, alerts (including the USB link ones from `LinkWatch`), settings. Receives `Msg` from the worker threads and emits `Event` to the pages. |
| `window.rs` | Main window, sidebar, lazy page construction, event dispatch. |
| `pages/*.rs` | One page per section (21). Renamed or merged ids keep working through `window::page_alias` (`controls`, `remote` → `control`). |
| `pages/connection.rs` | Connection page: tabs USB (summary, cable, PC port, phone's port, power, stability, speed test), Wi-Fi (wireless ADB) and Diagnosis (`diagnosis_rows`, also summed up on the Overview; USB mode; ADB server). Reads both sides in the background when shown, on device changes and every 10 s while in view. |
| `pages/control.rs` | Control page: the Controls, Remote and Automations pages as three tabs; telemetry, their clock, only reaches the tab in view. |
| `pages/bulk.rs` | Actions on all apps (the ⋮ menu in Apps, `win.bulk::close|stop|caches|export|import`, the search). |
| `pages/migration.rs` | Export and install apps (from `bulk.rs`): folder choice, a progress dialog that can be cancelled, and the list of apps to install with those the phone already has unchecked. |
| `pages/debloat.rs`, `pages/permissions.rs`, `pages/storage.rs` | Preinstalled apps, Permissions and Storage pages. They read when shown and again when the phone changes; the storage scan only runs from its button (it streams `du` and shows the count while it reads). |
| `pages/dashboard.rs` | Overview. `save_health_report` makes the health report (`win.health-report`: the button in Component health, the ⋮ menu and the search), saves it where the user chooses and opens it. |
| `pages/developer.rs` | Developer tab in Tools. A new resolution or density opens a dialog that undoes it by itself after 15 s without an answer. |
| `pages/automations.rs` | Automations tab in Control (stored in `Settings::automations`). |
| `sidebar.rs` | Sidebar list: sections fold from their title, pages go to the favorites at the top with the star on each row (shown on hover, filled on a favorite) or a right click / long press, the search box filters; folded sections and favorites (`sidebar_pinned`) are saved. Rows are found by their page id, never by position. |
| `pages/changes.rs` | Phone changes: the timeline by day, read when shown and again when the phone changes. |
| `search.rs` | Search everywhere (Ctrl+K, `win.search`): pages and common actions, matched by title and keywords without accents. |
| `widgets.rs` | Circular gauge, charts, cards, animated bars and `RowList` (virtualised list). |
| `anim.rs` | Icons with meaningful motion: an app box that opens (Cairo), the terminal cursor (two SVG layers) and arrows that spin during a reload. |
| `pages/controls.rs` | Quick settings tab: switches, sliders, shortcuts to Settings screens and Power (restart; power off with `svc power shutdown`, a clean shutdown). Every change is an explicit action, locks its control until it is confirmed and is recorded in History. |
| `pages/remote.rs` | Remote tab: media, calls, send to the phone, find the phone, phone notifications and scheduled actions. The state is read again every 3 s only while the page is in view. |
| `pages/basic.rs` | Basic analysis: MTP reads in the background (only while the page is in view, 4 s after the phone appears, or with Refresh) and the optional invitation to the advanced analysis (`invite_card`, also on the Overview). |
| `pages/facts.rs` | Presentation of the platform and app sheets (`probe_row`, expanders that create their rows when opened). |
| `tray.rs`, `panel.rs` | Tray icon (StatusNotifierItem) and quick panel. |
| `wireless.rs` | **Ways to connect** dialog (`win.connect::<page>`; `win.wireless` opens the wireless one): basic analysis, USB debugging (live status) and wireless debugging with QR, manual pairing and connection to already paired phones. Waits stop when leaving the page or closing. |
| `prefs.rs`, `help.rs` | Preferences (including the language and the restart), “About” and screenshots. |
| `update.rs` | **Update now** (from the update banner): download dialog with the percentage, then the install. On Linux it offers to restart; on Windows ADB Scan quits and the installer opens it again. |
| `theme.rs`, `style.css` | Light/dark theme, accent colours, styles. The accent is a palette colour or `accent_custom`, its saturation scaled by `accent_intensity` (HSL, 40–160 %); the text on it is white or near-black, whichever contrasts more. |
| `util.rs` | Background work (`run_bg`), confirmation dialogs, file choosers, stall diagnosis. |

### Threads and events

```text
worker threads ──Msg──▶ async_channel ──▶ Ctx (GTK thread) ──Event──▶ pages
 (adb, USB, telemetry)                     single state              (only the visible one
                                                                      receives Snapshot)
```

- Nothing that talks to the phone runs on the GTK thread: `util::run_bg`
  starts the work and returns the result to the main thread.
- `Event::Snapshot` (every 0.5 s) only reaches the visible page and only if the
  window is visible. When returning to a page, its `shown()` brings it up to
  date with the last snapshot and `Ctx`'s metric history.
- Pages that query something else (processes, folders, app list, GPU/FPS) use
  those same events as a clock: that way they only work while they are in
  view.

### Interface performance

- Large lists (properties, processes, services, history) use `RowList`, a
  `GtkListView` that only creates the rows near the visible ones (GTK prepares
  up to 200). If the list has no size yet (hidden tab or page not drawn),
  `set_items` keeps the rows and applies them on the first frame with a
  height: a `GtkListView` without a size would prepare all of them.
- History queries in the background and only rebuilds the lists if the data
  changed; several requests in a row are resolved with a single query.
- Labels that wrap text have a line cap and an ellipsis (`util::cap_wrapping`;
  `info_row` already does it). Without a cap, GTK measures each one again
  dozens of times on every layout: in an app's details (a hundred libadwaita
  rows with subtitles) that was about 5,000 Pango measurements per selection
  and frames of up to 1.2 s. With a cap, about ten times less.
- An app's details are built in parts (`group_stages`): each group goes in
  with its rows hidden and four are shown per frame. The expensive part is the
  first layout of each row; GTK does not lay out what is hidden and does not
  measure again what is already placed, so no frame goes beyond ~60 ms. ADB
  reads for an app that is no longer open are discarded before they start
  (`details_gen`, shared with the threads).
- With `ADB_SCAN_LOG=debug`, operations longer than 16 ms, main-loop stalls
  longer than 150 ms and frames longer than 50 ms (separating layout and
  paint) are logged.

### Dark palette

Linux Mint's libadwaita applies the desktop's GTK theme (e.g. Orchis, with
neutral greys) on top of the app's. If ADB Scan is going to open in dark mode,
`theme::prefer_mint_palette` sets `ADW_DEBUG_THEME_NAME=Mint-Y-Aqua` at
startup (before GTK starts, only if Mint-Y is installed and neither
`ADW_DEBUG_THEME_NAME` nor `GTK_THEME` is set): greys with a bluish tint
(#222226). In dark mode the sidebar is slightly lighter (`sidebar_bg_color`
#2b2b30, next to the accent in `apply_accent`). It is decided at startup:
switching to light without restarting shows light Mint-Y. Beware when testing
with an isolated `XDG_CONFIG_HOME`: GTK does not read the user's dconf and
loads Mint's default theme (Mint-Y), not theirs.

### Private mode

`Ctx` keeps the `Redactor` and offers `shown(text)` (free text),
`shown_value(value)` (a value that is entirely sensitive) and
`device_label(device)` (the marketing name instead of the nickname). Pages use
them **only when painting**: data, ADB queries, the history and file names
keep using the real values (`device_title`, `serial`). When the mode changes,
`Event::Privacy` is emitted and each page repaints what it already had,
without querying again. The `Redactor` learns serial numbers when a device
connects, at startup (settings and history) and when painting the history,
and Wi-Fi networks from the telemetry.

### Terminal

Each session is a `vte::Terminal` with its own PTY. The process is started
with `std::process::Command`; between `fork` and `exec` only
`vte_pty_child_setup` is called. The phone session runs `adb shell -t` with a
startup file that preloads the history.

## Translations

The code is written in English. Every interface string goes through:

- `tr("Text")` → `&'static str`, for plain text;
- `t!("{} apps stopped", n)` → `String`, for text with `{}` placeholders (no
  format specs: arguments that need one are formatted first, e.g.
  `format!("{:.1}", x)`);
- `trc("context", "Text")` / `tc!(…)` when English has one word for what
  another language says in several ways (gender, plural, verb or state).

`crates/core/i18n/es.po` is the Spanish catalog (gettext format, embedded in
the binary with `include_str!`): `msgid` is the English text in the code and
`msgstr` its translation; `{0}`, `{1}`… let a translation reorder arguments.
`i18n::set_language` is called once at the start of `main`, from Preferences
or `ADB_SCAN_LANG`; a text missing from the catalog is shown in English.
Strings stored in constants are translated where they are displayed
(`tr(label)`).

Two tests keep the catalog in step with the code: `catalog_placeholders_match`
(each translation keeps the placeholders of its English text) and
`i18n_catalog` (every literal passed to `tr`, `trc`, `t!` or `tc!` has a
Spanish translation). Adding a language means adding a variant to `Language`,
its `.po` file and its entry in `catalog_source`.

Numbers and dates also follow the language (`format::number`,
`format::civil_date`, `format::month_name`): `1,234.5` and `2026-11-24` in
English, `1.234,5` and `24/11/2026` in Spanish.

## Tests

```bash
cargo test --workspace                # 214 tests, no phone needed
```

Examples against a real phone in `crates/core/examples/`: `basic` (basic
analysis without ADB: USB, MTP and GVfs), `probe` (snapshot and telemetry),
`identity` (which identifiers can be read, without showing them),
`battery_usage` (usage and wakelocks), `watch` (monitors), `audit`, `gpufps`,
`labels`, `inspect` (platform sheet and inspection of every user app, with
timings), `icons` (icon of each user app; with a folder, it saves the images),
`remote` (remote, notifications and photos: counts only), `controls` (reads
the state; with `--apply` it changes harmless controls and restores them) and
`tools_setup` (downloads the tools into a temporary folder).

The real outputs in `tests/fixtures/` are anonymised: serial number, Wi-Fi
network, MACs, public IPv6 addresses and the apps installed by the user were
replaced with made-up values of the same shape.

Capture mode (development), with isolated data so the real data is not
touched:

```bash
S=$(mktemp -d)
# Screenshots and recordings of the phone go to the user folders; without
# this file they would fall back to ~/Pictures and ~/Videos.
mkdir -p $S/c && printf 'XDG_PICTURES_DIR="%s"\nXDG_VIDEOS_DIR="%s"\n' $S/p $S/v > $S/c/user-dirs.dirs
XDG_CONFIG_HOME=$S/c XDG_DATA_HOME=$S/d XDG_CACHE_HOME=$S/k XDG_STATE_HOME=$S/s \
ADB_SCAN_LANG=en ADB_SCAN_SCREENSHOT=$S/shots ADB_SCAN_PAGES=dashboard,apps,action:app.about \
target/debug/adb-scan
```

It runs as a separate instance without a tray icon, so it can coexist with the
installed ADB Scan. Its window ignores the real keyboard and mouse and has no
shortcuts: it takes the focus when it opens, and a key meant for another
window used to press the focused button (the dashboard's Screenshot). Besides pages and `action:` (also with a parameter:
`action:win.connect::qr`), `ADB_SCAN_PAGES` accepts `app:<package>` (opens that
app's details), `row:<n>` (selects row `n` of the visible list, like a click),
`click:<text>` (clicks the first visible button with that text, also in
dialogs) and `scroll:<percentage>` (scrolls the largest scrollable area). With
`ADB_SCAN_NO_PNG=1` it goes through the steps without saving screenshots (to
measure timings without the cost of saving them). Capture mode turns off
Mesa's shader cache (`MESA_SHADER_CACHE_DISABLE`): once it is full, the window
stops delivering its drawing and screenshots come out empty.

### Managed tools

`tools_setup` downloads Google's platform-tools and scrcpy with `curl` (HTTPS
only), checks the SHA-256 sum pinned in the code, extracts without following
links or paths that leave the folder (and without setuid) and installs
atomically into `~/.local/share/adb-scan/tools/<package>`. `find_tool` looks
there after the configured path and the PATH; when downloading from the
assistant (`setup.rs`) the path is set to the downloaded copy. To change
versions: update `version`, `url`, `size` and `sha256` (computed over the
official archive). The `tools_setup` example installs both packages (with a
temporary `XDG_DATA_HOME`, without touching the real ones).

### App icons

`Adb::app_icon` reads the manifest and `resources.arsc` with `unzip -p` on the
phone and `apk::resolve_icon` chooses: the sharpest image (also in the
`split_config.*dpi.apk` splits); if there is none, the XML: `<adaptive-icon>`
with its layers (`drawable=`, or `<bitmap>`/`<inset>` inside), solid-colour
`<shape>` and `<vector>`, which is converted to SVG (paths, groups with their
transformations, clips, linear and radial gradients). Images and XML are told
apart by their content: with obfuscated names (`res/QtC`) images lose their
extension. Tables are indexed by full identifier: a table can bring several
packages (0x7f and 0x80…). `Ok(None)` is “no icon” and is remembered; `Err` is
a connection failure that is not remembered (it is retried when returning to
the app).

### Animations

They only move when the state changes, never permanently:

| Icon | State | Technique |
| --- | --- | --- |
| Box (Apps) | Selected → open | `PackageIcon`: Lucide `package` geometry drawn with Cairo; the flaps rotate on their hinges (`AdwTimedAnimation`, 320/220 ms). Closed is identical to the icon. |
| Terminal | Section open → the `_` blinks | `TerminalIcon`: frame and cursor SVGs stacked; opacity `@keyframes` only on the cursor layer. |
| Refresh | Reload in progress → they spin | `busy` class and `-gtk-icon-transform`. |
| Sections with an active Lucide variant | Section open → switches to it and plays its gesture | `SectionIcon`: the two stacked SVGs crossfade (opacity, 220 ms) and the active one plays a `@keyframes` (class `play`). Overview → grid, Device → vibrating, Performance → beating heart, Battery → charging, Files → open folder (jump), Screen → monitor with “play” (flicker), Logcat → scroll with text (rises), Security → shield with a check mark (stamped). |
| Network | Section open → the arcs light up from the inside out | Wi-Fi in four layers (`adb-wifi-dot/arc1/arc2/arc3`) with `animation-delay`. |
| Other sections | Section open → a gesture | Controls slide, Remote and Data get pressed, History turns back once, Flasher (USB) plugs in, Tools (wrench) turns. |
| Preferences | Hover → the gear turns 60° | `transition` of `-gtk-icon-transform`. |

GTK paints symbolic SVGs as textures, so CSS cannot animate an inner stroke:
that is why the cursor and the Wi-Fi arcs are separate SVGs, the box is drawn
and state changes crossfade two icons. The new icons come from
`lucide-static@1.77.0` converted with `picosvg` (stroke to outline, `#bebebe`
fill), like the others. With “Animations” turned off or the system's reduced
motion (`gtk-enable-animations`) libadwaita jumps to the final state and GTK
does not run CSS animations. GTK 4.14 does not support the
`prefers-reduced-motion` CSS query; that system setting arrives through
`gtk-enable-animations`.

## Building without `-dev` packages

`scripts/env-no-dev.sh` downloads the GTK 4, libadwaita and VTE `-dev`
packages with `apt-get download` and extracts them into
`~/.cache/adb-scan-build/sysroot`; then it points `pkg-config` there. It does
not modify the system.
