//! Shared application state (UI thread).
//!
//! Worker threads (monitors, telemetry) send [`Msg`] through a channel; the
//! state is updated here and pages are notified with [`Event`]. Pages never
//! start polling of their own: they consume the shared stream.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use adbscan_core::adb::Adb;
use adbscan_core::db::{BatterySample, History};
use adbscan_core::fastboot::Fastboot;
use adbscan_core::live::{LiveEvent, LiveStream};
use adbscan_core::models::{now_secs, AdbDevice, CapacitySource, DeviceState, Snapshot};
use adbscan_core::monitor::{DeviceMonitor, MonitorEvent};
use adbscan_core::privacy::{Redactor, MASK};
use adbscan_core::scrcpy::Scrcpy;
use adbscan_core::settings::Settings;
use adbscan_core::tracker::{Action, NoteKind, Tracker, Urgency};
use adbscan_core::usb::{UsbDevice, UsbMode, UsbMonitor};

use crate::tray::{TrayCmd, TrayHandle, TrayIcon};

pub enum Msg {
    Adb(MonitorEvent),
    Usb(Vec<UsbDevice>),
    Fastboot(Vec<AdbDevice>),
    Live { generation: u64, ev: LiveEvent },
    // No tray icon on Windows: nothing sends it there.
    #[cfg_attr(windows, allow(dead_code))]
    Tray(TrayCmd),
}

#[derive(Clone, Debug)]
pub enum Event {
    /// The device list or the selected device changed.
    Devices,
    /// New snapshot of the selected device.
    Snapshot,
    SettingsChanged,
    /// adb/fastboot/scrcpy were detected again.
    Tools,
    /// A new screenshot was saved.
    Screenshot,
    /// Open a specific app on the Apps page.
    ShowApp(String),
    /// Show a page by its identifier.
    ShowPage(&'static str),
    /// Private mode was turned on or off: repaint the texts.
    Privacy,
}

/// Summarised metrics to preload charts when a page opens.
#[derive(Clone, Debug, Default)]
pub struct Metrics {
    pub cpu_load: Option<f64>,
    pub ram_pct: Option<f64>,
    pub cpu_temp: Option<f64>,
    pub batt_temp: Option<f64>,
}

pub const METRICS_CAPACITY: usize = 150;

/// Session log sample (exportable to CSV).
#[derive(Clone, Debug, Default)]
pub struct SessionSample {
    pub t: f64,
    pub cpu: Option<f64>,
    pub ram: Option<f64>,
    pub cpu_temp: Option<f64>,
    pub batt_temp: Option<f64>,
    pub batt_level: Option<f64>,
    pub hottest: Option<f64>,
    pub gpu: Option<f64>,
    pub fps: Option<f64>,
}

/// ~5 h a 500 ms.
const SESSION_CAPACITY: usize = 36_000;

/// Receiver of the app's events (each page).
type Listener = Rc<dyn Fn(&Event)>;

pub struct Ctx {
    pub app: adw::Application,
    pub settings: RefCell<Settings>,
    pub history: Option<Arc<History>>,
    adb: RefCell<Option<Adb>>,
    fastboot: RefCell<Option<Fastboot>>,
    scrcpy: RefCell<Option<Scrcpy>>,
    adb_devices: RefCell<Vec<AdbDevice>>,
    fb_devices: RefCell<Vec<AdbDevice>>,
    usb: RefCell<Vec<UsbDevice>>,
    tracker: RefCell<Tracker>,
    selected: RefCell<Option<String>>,
    snapshot: RefCell<Option<Rc<Snapshot>>>,
    metrics: RefCell<VecDeque<Metrics>>,
    live: RefCell<Option<(String, u32, LiveStream)>>,
    live_generation: Cell<u64>,
    monitor: RefCell<Option<DeviceMonitor>>,
    usb_monitor: RefCell<Option<UsbMonitor>>,
    tx: async_channel::Sender<Msg>,
    listeners: RefCell<Vec<Listener>>,
    toast: RefCell<Option<adw::ToastOverlay>>,
    visible: Cell<u32>,
    tray: RefCell<Option<TrayHandle>>,
    last_battery_sample: Cell<f64>,
    temp_alerted: Cell<bool>,
    full_alerted: Cell<bool>,
    /// Since when a sensor has been above its high threshold, and the last
    /// alert.
    hot_since: Cell<Option<f64>>,
    hot_alerted_at: Cell<f64>,
    /// Since when the phone's CPU has been above 80 %, and the last alert.
    cpu_high_since: Cell<Option<f64>>,
    cpu_alerted_at: Cell<f64>,
    low_alerted: Cell<bool>,
    /// Cuts and speed of the phones on the USB bus (see `adbscan_core::link`).
    link_watch: RefCell<adbscan_core::link::LinkWatch>,
    /// Phone notifications already seen (package, title, text); `None` until
    /// the first read, which only serves as a baseline.
    notif_seen: RefCell<Option<std::collections::HashSet<(String, String, String)>>>,
    notif_busy: Cell<bool>,
    /// Scheduled actions already run today: id → day.
    schedule_runs: RefCell<HashMap<u64, i64>>,
    /// ADB serials whose connection already ran the automations (cleared
    /// when the phone leaves).
    auto_seen: RefCell<std::collections::HashSet<String>>,
    /// Last run of each automation on each phone: (id, hardware serial) → time.
    auto_runs: RefCell<HashMap<(u64, String), f64>>,
    session_log: RefCell<VecDeque<SessionSample>>,
    /// Last GPU/FPS reading (time, GPU %, FPS).
    gpu_fps: Cell<(f64, Option<f64>, Option<f64>)>,
    market_names: RefCell<HashMap<String, String>>,
    monitor_problem: RefCell<Option<String>>,
    fastboot_polling: Cell<bool>,
    /// Battery capacity according to `batterystats` (design, learned), by
    /// serial number. `None`: query in progress or no data (not repeated).
    battery_capacity: RefCell<HashMap<String, Option<(i64, i64)>>>,
    /// Private mode (display only). Learns serial numbers and networks as it
    /// sees them.
    privacy: RefCell<Redactor>,
}

impl Ctx {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let settings = Settings::load();
        let history = match History::open_default() {
            Ok(h) => Some(Arc::new(h)),
            Err(e) => {
                log::error!("Could not open the history: {e}");
                None
            }
        };
        if let Some(h) = &history {
            let _ = h.prune(settings.retain_session_days);
            let _ = h.log_event("app", tr("ADB Scan started"), "info", None);
        }
        let (tx, rx) = async_channel::unbounded::<Msg>();
        let ctx = Rc::new(Self {
            app: app.clone(),
            settings: RefCell::new(settings),
            history,
            adb: RefCell::new(None),
            fastboot: RefCell::new(None),
            scrcpy: RefCell::new(None),
            adb_devices: RefCell::new(Vec::new()),
            fb_devices: RefCell::new(Vec::new()),
            usb: RefCell::new(Vec::new()),
            tracker: RefCell::new(Tracker::new(now_secs())),
            selected: RefCell::new(None),
            snapshot: RefCell::new(None),
            metrics: RefCell::new(VecDeque::new()),
            live: RefCell::new(None),
            live_generation: Cell::new(0),
            monitor: RefCell::new(None),
            usb_monitor: RefCell::new(None),
            tx,
            listeners: RefCell::new(Vec::new()),
            toast: RefCell::new(None),
            visible: Cell::new(0),
            tray: RefCell::new(None),
            last_battery_sample: Cell::new(0.0),
            temp_alerted: Cell::new(false),
            full_alerted: Cell::new(false),
            hot_since: Cell::new(None),
            hot_alerted_at: Cell::new(0.0),
            cpu_high_since: Cell::new(None),
            cpu_alerted_at: Cell::new(0.0),
            low_alerted: Cell::new(false),
            link_watch: RefCell::new(Default::default()),
            schedule_runs: RefCell::new(HashMap::new()),
            auto_seen: RefCell::new(Default::default()),
            auto_runs: RefCell::new(HashMap::new()),
            notif_seen: RefCell::new(None),
            notif_busy: Cell::new(false),
            session_log: RefCell::new(VecDeque::new()),
            gpu_fps: Cell::new((0.0, None, None)),
            market_names: RefCell::new(HashMap::new()),
            monitor_problem: RefCell::new(None),
            fastboot_polling: Cell::new(false),
            battery_capacity: RefCell::new(HashMap::new()),
            privacy: RefCell::new(Redactor::new(false)),
        });
        {
            // Private mode: saved state and already known serial numbers
            // (settings and history), to hide them even when they are not
            // connected.
            let mut r = ctx.privacy.borrow_mut();
            let st = ctx.settings.borrow();
            r.set_enabled(st.privacy_mode);
            for serial in st.nicknames.keys().chain(st.device_ids.keys()).chain(std::iter::once(&st.last_device)) {
                r.learn(serial);
            }
            if let Some(h) = &ctx.history {
                for s in h.sessions(1000).unwrap_or_default() {
                    r.learn(&s.serial);
                }
            }
        }
        ctx.detect_tools();
        // Message loop of the worker threads.
        {
            let weak = Rc::downgrade(&ctx);
            glib::spawn_future_local(async move {
                while let Ok(msg) = rx.recv().await {
                    let Some(ctx) = weak.upgrade() else { break };
                    ctx.handle(msg);
                }
            });
        }
        // Phone notifications on the laptop (only if enabled).
        {
            let weak = Rc::downgrade(&ctx);
            glib::timeout_add_local(Duration::from_secs(5), move || {
                let Some(ctx) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                ctx.mirror_notifications();
                glib::ControlFlow::Continue
            });
        }
        // Scheduled actions: checking two or three times a minute is enough.
        {
            let weak = Rc::downgrade(&ctx);
            glib::timeout_add_local(Duration::from_secs(20), move || {
                let Some(ctx) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                ctx.run_schedules();
                glib::ControlFlow::Continue
            });
        }
        // Clock for deferred alerts.
        {
            let weak = Rc::downgrade(&ctx);
            glib::timeout_add_local(Duration::from_millis(500), move || {
                let Some(ctx) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let actions = ctx.tracker.borrow_mut().tick(now_secs());
                ctx.apply(actions);
                glib::ControlFlow::Continue
            });
        }
        ctx
    }

    // -- tools ---------------------------------------------------------------

    pub fn detect_tools(&self) {
        let s = self.settings.borrow();
        *self.adb.borrow_mut() = Adb::locate(Some(&s.adb_path)).ok();
        *self.fastboot.borrow_mut() = Fastboot::locate(Some(&s.fastboot_path)).ok();
        *self.scrcpy.borrow_mut() = Scrcpy::locate(Some(&s.scrcpy_path)).ok();
    }

    pub fn adb(&self) -> Option<Adb> {
        self.adb.borrow().clone()
    }
    pub fn fastboot(&self) -> Option<Fastboot> {
        self.fastboot.borrow().clone()
    }
    pub fn scrcpy(&self) -> Option<Scrcpy> {
        self.scrcpy.borrow().clone()
    }

    pub fn sender(&self) -> async_channel::Sender<Msg> {
        self.tx.clone()
    }

    /// Starts the ADB and USB monitors. Can be called again after changing the
    /// adb path in Preferences.
    pub fn start_monitors(self: &Rc<Self>) {
        self.monitor.borrow_mut().take();
        if let Some(adb) = self.adb() {
            let tx = self.tx.clone();
            *self.monitor.borrow_mut() = Some(DeviceMonitor::start(adb, move |ev| {
                let _ = tx.send_blocking(Msg::Adb(ev));
            }));
        }
        if self.usb_monitor.borrow().is_none() {
            let tx = self.tx.clone();
            *self.usb_monitor.borrow_mut() = Some(UsbMonitor::start(move |devs| {
                let _ = tx.send_blocking(Msg::Usb(devs));
            }));
        }
    }

    pub fn shutdown(&self) {
        self.live.borrow_mut().take();
        self.monitor.borrow_mut().take();
        self.usb_monitor.borrow_mut().take();
        if let Some(h) = &self.history {
            for d in self.adb_devices.borrow().iter() {
                let _ = h.end_session(&d.serial);
            }
            let _ = h.log_event("app", tr("ADB Scan stopped"), "info", None);
        }
        self.tray.borrow_mut().take();
        let _ = self.settings.borrow().save();
    }

    // -- events ----------------------------------------------------------------

    pub fn subscribe(&self, f: impl Fn(&Event) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn emit(&self, ev: Event) {
        let listeners: Vec<_> = self.listeners.borrow().clone();
        for l in listeners {
            l(&ev);
        }
    }

    pub fn set_toast_overlay(&self, overlay: &adw::ToastOverlay) {
        *self.toast.borrow_mut() = Some(overlay.clone());
    }

    pub fn toast(&self, text: &str) {
        if let Some(o) = self.toast.borrow().as_ref() {
            let t = adw::Toast::new(&glib::markup_escape_text(&self.shown(text)));
            t.set_timeout(4);
            o.add_toast(t);
        }
    }

    /// Toast with an action button (e.g. “Open”).
    pub fn toast_action(&self, text: &str, button: &str, f: impl Fn() + 'static) {
        if let Some(o) = self.toast.borrow().as_ref() {
            let t = adw::Toast::builder().title(glib::markup_escape_text(&self.shown(text))).button_label(button).timeout(6).build();
            t.connect_button_clicked(move |_| f());
            o.add_toast(t);
        }
    }

    pub fn set_tray(&self, tray: TrayHandle) {
        *self.tray.borrow_mut() = Some(tray);
        self.update_tray();
    }

    /// Visible windows (main, quick panel). With none, telemetry drops to the
    /// background rate.
    pub fn surface_visible(self: &Rc<Self>, visible: bool) {
        let n = self.visible.get();
        self.visible.set(if visible { n + 1 } else { n.saturating_sub(1) });
        self.sync_live();
    }

    // -- devices ---------------------------------------------------------------

    /// Combined list: ADB + fastboot (without duplicate serials).
    pub fn devices(&self) -> Vec<AdbDevice> {
        let mut list = self.adb_devices.borrow().clone();
        for f in self.fb_devices.borrow().iter() {
            if !list.iter().any(|d| d.serial == f.serial) {
                list.push(f.clone());
            }
        }
        let names = self.market_names.borrow();
        for d in list.iter_mut() {
            if let Some(n) = names.get(&d.serial) {
                d.model = n.clone();
            }
        }
        list
    }

    pub fn usb_devices(&self) -> Vec<UsbDevice> {
        self.usb.borrow().clone()
    }

    /// There is a phone on USB with debugging turned off (no ADB interface) and
    /// no ADB device ready: this is the basic analysis case.
    pub fn basic_only_phone(&self) -> Option<UsbDevice> {
        if self.ready_serial().is_some() {
            return None;
        }
        self.usb.borrow().iter().find(|u| u.mode == UsbMode::Other).cloned()
    }

    /// Show the optional invitation to the advanced analysis.
    pub fn wants_advanced_invite(&self) -> bool {
        self.settings.borrow().show_advanced_invite && self.basic_only_phone().is_some()
    }

    /// Hides the invitation (“Continue with the basic analysis”). It can be
    /// shown again in Preferences → General → Analysis.
    pub fn dismiss_advanced_invite(&self) {
        self.settings.borrow_mut().show_advanced_invite = false;
        self.save_settings();
        self.emit(Event::SettingsChanged);
    }

    pub fn selected_serial(&self) -> Option<String> {
        self.selected.borrow().clone()
    }

    pub fn selected_device(&self) -> Option<AdbDevice> {
        let serial = self.selected.borrow().clone()?;
        self.devices().into_iter().find(|d| d.serial == serial)
    }

    /// Serial of the selected device if it is ready for ADB.
    pub fn ready_serial(&self) -> Option<String> {
        self.selected_device().filter(|d| d.connected()).map(|d| d.serial)
    }

    pub fn device_title(&self, d: &AdbDevice) -> String {
        self.settings.borrow().nickname(&d.serial).map(String::from).unwrap_or_else(|| d.display_name())
    }

    // -- private mode (display only) --------------------------------------------

    pub fn private_mode(&self) -> bool {
        self.privacy.borrow().enabled()
    }

    pub fn set_private_mode(self: &Rc<Self>, on: bool) {
        if self.private_mode() == on {
            return;
        }
        self.privacy.borrow_mut().set_enabled(on);
        self.settings.borrow_mut().privacy_mode = on;
        self.save_settings();
        self.update_tray();
        self.emit(Event::Privacy);
    }

    /// Remembers a serial number or a network to hide them wherever they
    /// appear.
    pub fn learn_private(&self, value: &str) {
        self.privacy.borrow_mut().learn(value);
    }

    /// Text to display: in private mode it hides serials, networks, IPs and
    /// MACs.
    pub fn shown(&self, text: &str) -> String {
        self.privacy.borrow().text(text).into_owned()
    }

    /// Value that is entirely sensitive data (serial number, Wi-Fi network).
    pub fn shown_value(&self, value: &str) -> String {
        self.privacy.borrow().value(value).into_owned()
    }

    /// Name to display: in private mode, the marketing name instead of the
    /// nickname.
    /// For what is stored or used as data, use `device_title`.
    pub fn device_label(&self, d: &AdbDevice) -> String {
        if !self.private_mode() {
            return self.device_title(d);
        }
        let name = d.display_name();
        if name == d.serial {
            MASK.to_string()
        } else {
            self.shown(&name)
        }
    }

    /// Name saved in a record (may be an old nickname).
    pub fn shown_device_name(&self, stored: &str) -> String {
        if self.private_mode() && self.settings.borrow().nicknames.values().any(|n| n == stored) {
            return MASK.to_string();
        }
        self.shown(stored)
    }

    pub fn snapshot(&self) -> Option<Rc<Snapshot>> {
        self.snapshot.borrow().clone()
    }

    pub fn metrics(&self) -> Vec<Metrics> {
        self.metrics.borrow().iter().cloned().collect()
    }

    pub fn monitor_problem(&self) -> Option<String> {
        self.monitor_problem.borrow().clone()
    }

    pub fn select(self: &Rc<Self>, serial: &str) {
        if self.selected.borrow().as_deref() == Some(serial) {
            return;
        }
        *self.selected.borrow_mut() = Some(serial.to_string());
        self.settings.borrow_mut().last_device = serial.to_string();
        self.reset_device_data();
        self.sync_live();
        self.update_tray();
        self.emit(Event::Devices);
    }

    pub fn refresh_devices(&self) {
        if let Some(m) = self.monitor.borrow().as_ref() {
            m.refresh();
        }
    }

    fn reset_device_data(&self) {
        *self.snapshot.borrow_mut() = None;
        self.metrics.borrow_mut().clear();
        self.session_log.borrow_mut().clear();
        self.gpu_fps.set((0.0, None, None));
        self.temp_alerted.set(false);
        self.full_alerted.set(false);
        self.hot_since.set(None);
        self.cpu_high_since.set(None);
        self.low_alerted.set(false);
        self.notif_seen.borrow_mut().take();
    }

    pub fn note_gpu_fps(&self, gpu: Option<f64>, fps: Option<f64>) {
        self.gpu_fps.set((now_secs(), gpu, fps));
    }

    /// (samples, start) of the session log.
    pub fn session_info(&self) -> (usize, Option<f64>) {
        let log = self.session_log.borrow();
        (log.len(), log.front().map(|s| s.t))
    }

    /// Session log as CSV (separator “,” and no decimal comma: a dot is used so
    /// spreadsheets and scripts read it the same way).
    pub fn session_csv(&self) -> String {
        let f = |v: Option<f64>| v.map(|x| format!("{x:.1}")).unwrap_or_default();
        let mut out = String::from("time,cpu_pct,ram_pct,temp_cpu_c,temp_battery_c,battery_pct,sensor_max_c,gpu_pct,fps\n");
        for s in self.session_log.borrow().iter() {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{}\n",
                adbscan_core::format::timestamp(Some(s.t)),
                f(s.cpu),
                f(s.ram),
                f(s.cpu_temp),
                f(s.batt_temp),
                f(s.batt_level),
                f(s.hottest),
                f(s.gpu),
                f(s.fps)
            ));
        }
        out
    }

    fn update_selection(self: &Rc<Self>) -> bool {
        let devices = self.devices();
        let previous = self.selected.borrow().clone();
        let last = self.settings.borrow().last_device.clone();
        let present = |s: &str| devices.iter().any(|d| d.serial == s);
        let chosen = previous
            .clone()
            .filter(|s| present(s))
            .or_else(|| Some(last).filter(|s| !s.is_empty() && present(s)))
            .or_else(|| devices.iter().find(|d| d.connected()).map(|d| d.serial.clone()))
            .or_else(|| devices.first().map(|d| d.serial.clone()));
        let changed = chosen != previous;
        if changed {
            *self.selected.borrow_mut() = chosen.clone();
            if let Some(s) = &chosen {
                self.settings.borrow_mut().last_device = s.clone();
            }
            self.reset_device_data();
        }
        changed
    }

    fn handle(self: &Rc<Self>, msg: Msg) {
        match msg {
            Msg::Adb(MonitorEvent::Devices(list)) => {
                *self.monitor_problem.borrow_mut() = None;
                *self.adb_devices.borrow_mut() = list;
                self.devices_changed();
            }
            Msg::Adb(MonitorEvent::Problem(p)) => {
                log::warn!("monitor: {p}");
                *self.monitor_problem.borrow_mut() = Some(p);
                self.emit(Event::Devices);
            }
            Msg::Usb(list) => {
                *self.usb.borrow_mut() = list.clone();
                let actions = self.tracker.borrow_mut().update_usb(list.clone(), now_secs());
                self.apply(actions);
                self.watch_usb_link(&list);
                self.ensure_fastboot_polling();
                self.update_tray();
                self.emit(Event::Devices);
            }
            Msg::Fastboot(list) => {
                if *self.fb_devices.borrow() != list {
                    *self.fb_devices.borrow_mut() = list;
                    self.devices_changed();
                }
            }
            Msg::Live { generation, ev } => {
                if generation != self.live_generation.get() {
                    return;
                }
                match ev {
                    LiveEvent::Snapshot(s) => self.on_snapshot(*s),
                    LiveEvent::Notice(n) => log::info!("telemetry: {n}"),
                    LiveEvent::Stopped => {}
                }
            }
            Msg::Tray(cmd) => crate::app::handle_tray(self, cmd),
        }
    }

    fn devices_changed(self: &Rc<Self>) {
        let devices = self.devices();
        for d in &devices {
            self.learn_private(&d.serial);
        }
        // A phone that left runs its automations again when it comes back.
        self.auto_seen.borrow_mut().retain(|s| devices.iter().any(|d| &d.serial == s && d.state == DeviceState::Device));
        let actions = self.tracker.borrow_mut().update_devices(devices, now_secs());
        self.apply(actions);
        self.update_selection();
        self.sync_live();
        self.update_tray();
        self.emit(Event::Devices);
    }

    fn ensure_fastboot_polling(self: &Rc<Self>) {
        let want = self.usb.borrow().iter().any(|d| d.mode == UsbMode::Fastboot) || !self.fb_devices.borrow().is_empty();
        if !want || self.fastboot_polling.get() || self.fastboot().is_none() {
            return;
        }
        self.fastboot_polling.set(true);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(1500), move || {
            let Some(ctx) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(fb) = ctx.fastboot() else {
                ctx.fastboot_polling.set(false);
                return glib::ControlFlow::Break;
            };
            let tx = ctx.tx.clone();
            std::thread::spawn(move || {
                let list = fb.devices().unwrap_or_default();
                let _ = tx.send_blocking(Msg::Fastboot(list));
            });
            let still = ctx.usb.borrow().iter().any(|d| d.mode == UsbMode::Fastboot) || !ctx.fb_devices.borrow().is_empty();
            if still {
                glib::ControlFlow::Continue
            } else {
                ctx.fastboot_polling.set(false);
                glib::ControlFlow::Break
            }
        });
    }

    fn apply(self: &Rc<Self>, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Notify { kind, key, urgency } => self.notify(kind, &key, urgency),
                Action::SessionStart { serial, name } => {
                    if let Some(h) = &self.history {
                        let _ = h.start_session(&serial, &name);
                    }
                }
                Action::SessionTouch { serial } => {
                    if let Some(h) = &self.history {
                        let _ = h.touch_session(&serial);
                    }
                }
                Action::SessionEnd { serial } => {
                    if let Some(h) = &self.history {
                        let _ = h.end_session(&serial);
                    }
                    if self.live.borrow().as_ref().is_some_and(|(s, ..)| *s == serial) {
                        self.live.borrow_mut().take();
                    }
                }
                Action::Log { category, message, level, serial } => {
                    if let Some(h) = &self.history {
                        let _ = h.log_event(category, &message, level, serial.as_deref());
                    }
                }
                Action::DeviceReady { serial } => self.fetch_market_name(&serial),
            }
        }
    }

    fn fetch_market_name(self: &Rc<Self>, serial: &str) {
        let Some(adb) = self.adb() else { return };
        let serial = serial.to_string();
        let weak = Rc::downgrade(self);
        crate::util::run_bg(
            {
                let serial = serial.clone();
                move || (adb.prop(&serial, "ro.product.marketname"), adb.prop(&serial, "ro.product.model"))
            },
            move |(market, model): (String, String)| {
                let Some(ctx) = weak.upgrade() else { return };
                // Without `marketname`, the phone's USB product name (“REDMAGIC
                // 11S Pro”) is better than the model (“NX809J”).
                let usb = || ctx.usb_devices().into_iter().find(|u| u.serial == serial).map(|u| u.product).filter(|p| !p.trim().is_empty());
                let name = if !market.is_empty() { market } else { usb().unwrap_or(model) };
                if !name.is_empty() {
                    ctx.market_names.borrow_mut().insert(serial, name);
                    ctx.update_tray();
                    ctx.emit(Event::Devices);
                }
            },
        );
    }

    // -- notifications -----------------------------------------------------------

    pub fn notify(&self, kind: NoteKind, key: &str, urgency: Urgency) {
        let s = self.settings.borrow();
        if !s.notifications_enabled {
            return;
        }
        let enabled = match kind {
            NoteKind::UsbDetected => s.notify_usb_detected,
            NoteKind::WaitingAuthorization => s.notify_authorization,
            NoteKind::Connected => s.notify_connected,
            NoteKind::Disconnected => s.notify_disconnected,
            NoteKind::Error => s.notify_errors,
        };
        if !enabled {
            return;
        }
        drop(s);
        let mut body = kind.text().to_string();
        if kind == NoteKind::Connected {
            if let Some(d) = self.devices().into_iter().find(|d| d.serial == key) {
                // By cable or by Wi-Fi according to the device's actual
                // transport.
                let transport = if d.is_wireless() { adbscan_core::models::Transport::Tcp } else { d.transport };
                body = format!("{} — {}", self.device_label(&d), NoteKind::connected_text(transport));
            }
        }
        self.send_notification(&format!("{}-{key}", kind.key()), "ADB Scan", &body, kind.icon(), urgency == Urgency::Critical);
        if let Some(h) = &self.history {
            let _ = h.log_event("notification", kind.key(), "info", Some(key));
        }
    }

    /// Counts how often each phone drops off the USB bus and remembers its
    /// fastest link, to warn about a loose connection or a cable that only
    /// carries USB 2.0.
    fn watch_usb_link(&self, list: &[UsbDevice]) {
        use adbscan_core::link::LinkAlert;
        let phones: Vec<(String, Option<u32>)> = list.iter().filter(|u| !u.serial.is_empty()).map(|u| (u.serial.clone(), u.speed_mbps)).collect();
        let best: std::collections::HashMap<String, u32> = self.settings.borrow().usb_best_speed.clone().into_iter().collect();
        let (alerts, records) = self.link_watch.borrow_mut().update(&phones, &best, now_secs());
        if !records.is_empty() {
            {
                let mut s = self.settings.borrow_mut();
                for (k, v) in records {
                    s.usb_best_speed.insert(k, v);
                }
            }
            if let Err(e) = self.settings.borrow().save() {
                log::warn!("Could not save the settings: {e}");
            }
        }
        let name = |key: &str| self.devices().into_iter().find(|d| d.serial == key).map(|d| self.device_label(&d)).unwrap_or_else(|| tr("The phone").to_string());
        for alert in alerts {
            let (enabled, on) = {
                let s = self.settings.borrow();
                (s.notifications_enabled, (s.notify_usb_unstable, s.notify_usb_slow))
            };
            if !enabled {
                continue;
            }
            match alert {
                LinkAlert::Unstable { key, cuts } if on.0 => {
                    let body = t!("{} dropped off USB {} times in a minute. Check the cable, the plug and the port.", name(&key), cuts);
                    self.send_notification(&format!("usb-unstable-{key}"), tr("Unstable USB connection"), &body, "adb-scan-error", false);
                    if let Some(h) = &self.history {
                        let _ = h.log_event("usb", "unstable", "warning", Some(&key));
                    }
                }
                LinkAlert::SlowCable { key, now_mbps, best_mbps } if on.1 => {
                    let body = t!(
                        "{} is connected at {}, but reached {} on this PC before. The cable or an adapter only carries USB 2.0.",
                        name(&key),
                        adbscan_core::link::speed_label(now_mbps),
                        adbscan_core::link::speed_label(best_mbps)
                    );
                    self.send_notification(&format!("usb-slow-{key}"), tr("The cable limits the speed"), &body, "adb-scan-error", false);
                }
                _ => {}
            }
        }
    }

    pub fn send_notification(&self, id: &str, title: &str, body: &str, icon: &str, urgent: bool) {
        // They are visible on the desktop (and when sharing the screen): they
        // are hidden too.
        let n = gio::Notification::new(&self.shown(title));
        n.set_body(Some(&self.shown(body)));
        n.set_icon(&gio::ThemedIcon::new(icon));
        n.set_default_action("app.show");
        if urgent {
            n.set_priority(gio::NotificationPriority::Urgent);
        }
        self.app.send_notification(Some(id), &n);
    }

    // -- telemetry ---------------------------------------------------------------

    pub fn live_interval(&self) -> u32 {
        let s = self.settings.borrow();
        if self.visible.get() > 0 {
            s.live_refresh_ms
        } else {
            s.background_refresh_ms
        }
    }

    pub fn sync_live(self: &Rc<Self>) {
        let want = self.ready_serial();
        let interval = self.live_interval();
        let current = self.live.borrow().as_ref().map(|(s, i, _)| (s.clone(), *i));
        match (want, current) {
            (Some(serial), Some((cur, ci))) if serial == cur && ci == interval => {}
            (Some(serial), _) => {
                self.live.borrow_mut().take();
                let Some(adb) = self.adb() else { return };
                let generation = self.live_generation.get() + 1;
                self.live_generation.set(generation);
                let tx = self.tx.clone();
                let stream = LiveStream::start(adb, serial.clone(), interval, move |ev| {
                    let _ = tx.send_blocking(Msg::Live { generation, ev });
                });
                *self.live.borrow_mut() = Some((serial, interval, stream));
            }
            (None, Some(_)) => {
                self.live_generation.set(self.live_generation.get() + 1);
                // Stopping the thread can take ~100 ms: off the UI thread.
                if let Some(old) = self.live.borrow_mut().take() {
                    std::thread::spawn(move || drop(old));
                }
            }
            (None, None) => {}
        }
    }

    fn on_snapshot(self: &Rc<Self>, mut snap: Snapshot) {
        if self.ready_serial().as_deref() != Some(snap.serial.as_str()) {
            return;
        }
        self.fill_battery_capacity(&mut snap);
        self.run_automations(&snap);
        if let Some(w) = &snap.wifi {
            self.learn_private(&w.ssid);
        }
        let m = Metrics { cpu_load: snap.cpu.load_percent, ram_pct: snap.memory.percent(), cpu_temp: snap.cpu_temperature(), batt_temp: snap.battery_temperature() };
        {
            let mut q = self.metrics.borrow_mut();
            q.push_back(m);
            while q.len() > METRICS_CAPACITY {
                q.pop_front();
            }
        }
        self.record_battery(&snap);
        self.check_alerts(&snap);
        {
            let now = now_secs();
            let (t, gpu, fps) = self.gpu_fps.get();
            let fresh = now - t < 3.0;
            let mut log = self.session_log.borrow_mut();
            log.push_back(SessionSample {
                t: now,
                cpu: snap.cpu.load_percent,
                ram: snap.memory.percent(),
                cpu_temp: snap.cpu_temperature(),
                batt_temp: snap.battery_temperature(),
                batt_level: snap.battery.percentage(),
                hottest: snap.temperatures.iter().filter(|t| t.is_temperature()).map(|t| t.value_c).reduce(f64::max),
                gpu: gpu.filter(|_| fresh),
                fps: fps.filter(|_| fresh),
            });
            while log.len() > SESSION_CAPACITY {
                log.pop_front();
            }
        }
        // Model identity for the Flasher (almost never changes).
        if !snap.props.is_empty() {
            let ids = ["ro.product.model", "ro.product.device", "ro.product.name", "ro.product.vendor.device", "ro.build.product"].iter().map(|k| snap.prop(k).to_string()).collect();
            if self.settings.borrow_mut().remember_device_ids(&snap.serial, ids) {
                if let Err(e) = self.settings.borrow().save() {
                    log::warn!("Could not save the settings: {e}");
                }
            }
        }
        *self.snapshot.borrow_mut() = Some(Rc::new(snap));
        self.update_tray();
        self.emit(Event::Snapshot);
    }

    /// If sysfs does not report the battery capacity, `batterystats` is asked
    /// once per phone (it takes ~10 s) and the result is added to every
    /// snapshot.
    fn fill_battery_capacity(self: &Rc<Self>, snap: &mut Snapshot) {
        if !snap.health.probed || snap.health.battery_percent().is_some() {
            return;
        }
        let known = self.battery_capacity.borrow().get(&snap.serial).copied();
        match known {
            Some(Some((design, learned))) => {
                snap.health.battery_design_mah = Some(design);
                snap.health.battery_full_mah = Some(learned);
                snap.health.battery_source = CapacitySource::BatteryStats;
            }
            Some(None) => {}
            None => {
                let Some(adb) = self.adb() else { return };
                let serial = snap.serial.clone();
                self.battery_capacity.borrow_mut().insert(serial.clone(), None);
                let weak = Rc::downgrade(self);
                crate::util::run_bg(
                    {
                        let serial = serial.clone();
                        move || adb.battery_capacity(&serial)
                    },
                    move |capacity| {
                        if let (Some(ctx), (Some(design), Some(learned))) = (weak.upgrade(), capacity) {
                            ctx.battery_capacity.borrow_mut().insert(serial, Some((design, learned)));
                        }
                    },
                );
            }
        }
    }

    fn record_battery(&self, snap: &Snapshot) {
        let now = now_secs();
        if now - self.last_battery_sample.get() < 60.0 {
            return;
        }
        self.last_battery_sample.set(now);
        if let Some(h) = self.history.clone() {
            let sample =
                BatterySample { timestamp: now, level: snap.battery.percentage(), temperature: snap.battery_temperature(), voltage_mv: snap.battery.voltage_mv, charging: snap.battery.is_charging() };
            let serial = snap.serial.clone();
            std::thread::spawn(move || {
                let _ = h.add_battery_sample(&serial, &sample, 55.0);
            });
        }
    }

    fn check_alerts(self: &Rc<Self>, snap: &Snapshot) {
        let s = self.settings.borrow();
        if !s.notifications_enabled {
            return;
        }
        if s.notify_temperature {
            if let Some(t) = snap.battery_temperature() {
                if t >= s.temperature_alert_c && !self.temp_alerted.get() {
                    self.temp_alerted.set(true);
                    let body = t!("The battery is at {} °C. Unplug the charger or let it cool down.", format!("{:.1}", t));
                    drop(s);
                    self.send_notification("temperature", tr("High temperature"), &body, "adb-scan-error", true);
                    return;
                } else if t < s.temperature_alert_c - 3.0 {
                    self.temp_alerted.set(false);
                }
            }
        }
        if s.notify_temperature {
            // Any sensor above its high threshold for 30 s; at most one alert
            // every 15 minutes.
            let hot = snap.temperatures.iter().filter(|t| t.is_temperature() && t.value_c >= t.thresholds().1).max_by(|a, b| a.value_c.total_cmp(&b.value_c));
            let now = now_secs();
            match hot {
                Some(t) => {
                    let since = *self.hot_since.get().get_or_insert(now);
                    self.hot_since.set(Some(since));
                    if now - since >= 30.0 && now - self.hot_alerted_at.get() >= 900.0 {
                        self.hot_alerted_at.set(now);
                        let body = t!("{} ({}) has been at {} °C for more than 30 s. Close demanding apps or let the phone cool down.", t.name, t.kind(), format!("{:.1}", t.value_c));
                        drop(s);
                        self.send_notification("hot-sensor", tr("Very high temperature"), &body, "adb-scan-error", false);
                        return;
                    }
                }
                None => self.hot_since.set(None),
            }
        }
        if s.notify_battery_low {
            // At the threshold while not charging; re-armed when charging or
            // 5 points above it.
            let pct = snap.battery.percentage();
            let charging = snap.battery.is_charging();
            let low = s.battery_low_percent as f64;
            if pct.is_some_and(|p| p <= low) && !charging && !self.low_alerted.get() {
                self.low_alerted.set(true);
                let body = t!("{} % battery left. Plug in the charger.", format!("{:.0}", pct.unwrap_or(0.0)));
                drop(s);
                self.send_notification("battery-low", tr("Low battery"), &body, "adb-scan-error", false);
                return;
            } else if charging || pct.is_some_and(|p| p > low + 5.0) {
                self.low_alerted.set(false);
            }
        }
        if s.notify_cpu {
            self.check_cpu(snap.cpu.load_percent);
        }
        if s.notify_battery_full {
            let full = snap.battery.status_code == Some(5) || snap.battery.percentage().is_some_and(|p| p >= 100.0);
            if full && snap.battery.is_charging() && !self.full_alerted.get() {
                self.full_alerted.set(true);
                drop(s);
                self.send_notification("battery-full", tr("Battery charged"), tr("The battery is at 100 %."), "adb-scan-connected", false);
            } else if !full {
                self.full_alerted.set(false);
            }
        }
    }

    /// Forwards the phone's new notifications to the desktop (see
    /// `adbscan_core::notifications`). The first read only marks what was
    /// already there; nothing is stored on disk or in History.
    fn mirror_notifications(self: &Rc<Self>) {
        let enabled = {
            let s = self.settings.borrow();
            s.mirror_notifications && s.notifications_enabled
        };
        if !enabled {
            self.notif_seen.borrow_mut().take();
            return;
        }
        if self.notif_busy.get() {
            return;
        }
        let (Some(adb), Some(serial)) = (self.adb(), self.ready_serial()) else {
            return;
        };
        self.notif_busy.set(true);
        let weak = Rc::downgrade(self);
        crate::util::run_bg(
            move || adb.phone_notifications(&serial),
            move |res| {
                let Some(ctx) = weak.upgrade() else { return };
                ctx.notif_busy.set(false);
                let Ok(list) = res else { return };
                let current: std::collections::HashSet<(String, String, String)> = list.iter().filter(|n| n.worth_showing()).map(|n| (n.package.clone(), n.title.clone(), n.text.clone())).collect();
                let previous = ctx.notif_seen.replace(Some(current.clone()));
                let Some(previous) = previous else { return };
                let fresh: Vec<&(String, String, String)> = current.iter().filter(|n| !previous.contains(*n)).collect();
                let labels = crate::pages::apps::cached_labels();
                let app = |p: &str| labels.get(p).cloned().unwrap_or_else(|| p.to_string());
                // At most three individual alerts; if there are more, one that
                // summarises them.
                if fresh.len() > 3 {
                    ctx.send_notification("phone-many", tr("Phone notifications"), &t!("{} new notifications", fresh.len()), "adb-scan-connected", false);
                    return;
                }
                for (pkg, title, text) in fresh {
                    let head = if title.is_empty() { app(pkg) } else { format!("{}: {title}", app(pkg)) };
                    ctx.send_notification(&format!("phone-{pkg}-{title}"), &head, text, "adb-scan-connected", false);
                }
            },
        );
    }

    /// Runs the scheduled actions that are due (see `adbscan_core::schedule`).
    /// If the phone is not connected it retries during its 5-minute window;
    /// after that it is given up.
    fn run_schedules(self: &Rc<Self>) {
        use adbscan_core::schedule::LocalTime;
        let now = LocalTime::now();
        let due: Vec<_> = {
            let runs = self.schedule_runs.borrow();
            self.settings.borrow().scheduled.iter().filter(|s| s.due(now, runs.get(&s.id).copied())).cloned().collect()
        };
        let notify = self.settings.borrow().notifications_enabled;
        for item in due {
            let what = format!("{} ({})", item.action.label(), item.time_text());
            let key = format!("scheduled_{}", item.action.key());
            let (Some(adb), Some(serial)) = (self.adb(), self.ready_serial()) else {
                if now.minute >= item.minute + 4 {
                    self.schedule_runs.borrow_mut().insert(item.id, now.day);
                    self.audit(&key, None, false, Some(tr("the phone was not connected")));
                    if notify {
                        self.send_notification("scheduled", tr("Scheduled action not run"), &t!("{}: the phone was not connected.", what), "adb-scan-error", false);
                    }
                }
                continue;
            };
            self.schedule_runs.borrow_mut().insert(item.id, now.day);
            let weak = Rc::downgrade(self);
            let action = item.action;
            crate::util::run_bg(
                move || adb.run_scheduled(&serial, action),
                move |res| {
                    let Some(ctx) = weak.upgrade() else { return };
                    let (ok, detail) = match &res {
                        Ok(d) => (true, d.clone()),
                        Err(e) => (false, e.to_string()),
                    };
                    ctx.audit(&key, None, ok, Some(&detail));
                    if notify {
                        let title = if ok { tr("Scheduled action") } else { tr("Scheduled action failed") };
                        ctx.send_notification("scheduled", title, &format!("{what}: {detail}"), if ok { "adb-scan-connected" } else { "adb-scan-error" }, false);
                    }
                },
            );
        }
    }

    /// First telemetry of a phone that just connected: runs its automations
    /// (see `adbscan_core::automation`), one after another in the
    /// background, each recorded in History.
    fn run_automations(self: &Rc<Self>, snap: &Snapshot) {
        if !self.auto_seen.borrow_mut().insert(snap.serial.clone()) {
            return;
        }
        let hw = snap.prop("ro.serialno").to_string();
        let hw = if hw.is_empty() { snap.serial.clone() } else { hw };
        let now = now_secs();
        let due: Vec<adbscan_core::automation::Automation> = {
            let runs = self.auto_runs.borrow();
            let list = &self.settings.borrow().automations;
            adbscan_core::automation::due(list, &hw, now, |id| runs.get(&(id, hw.clone())).copied()).into_iter().cloned().collect()
        };
        if due.is_empty() {
            return;
        }
        for a in &due {
            self.auto_runs.borrow_mut().insert((a.id, hw.clone()), now);
        }
        // The mirror opens a window: the interface does it.
        if due.iter().any(|a| a.action == adbscan_core::automation::AutoAction::Mirror) {
            if let Some(w) = self.app.active_window() {
                let _ = gtk::prelude::WidgetExt::activate_action(&w, "win.mirror", None);
            }
            self.audit("auto_mirror", None, true, None);
        }
        let actions: Vec<_> = due.iter().map(|a| a.action).filter(|a| *a != adbscan_core::automation::AutoAction::Mirror).collect();
        let (Some(adb), serial) = (self.adb(), snap.serial.clone()) else { return };
        let photos = std::path::PathBuf::from(self.settings.borrow().photos_dir.clone());
        let notify = self.settings.borrow().notifications_enabled;
        let weak = Rc::downgrade(self);
        crate::util::run_bg(
            move || actions.into_iter().map(|a| (a, adb.run_automation(&serial, a, &photos).map_err(|e| e.to_string()))).collect::<Vec<_>>(),
            move |results| {
                let Some(ctx) = weak.upgrade() else { return };
                let mut lines = Vec::new();
                let mut all_ok = true;
                for (a, r) in &results {
                    let (ok, detail) = match r {
                        Ok(d) => (true, d.clone()),
                        Err(e) => (false, e.clone()),
                    };
                    all_ok &= ok;
                    ctx.audit(&format!("auto_{}", a.key()), None, ok, Some(&detail));
                    lines.push(format!("{}: {}", a.label(), detail));
                }
                if notify && !lines.is_empty() {
                    let title = if all_ok { tr("Automations run") } else { tr("Some automations failed") };
                    ctx.send_notification("automation", title, &lines.join("\n"), if all_ok { "adb-scan-connected" } else { "adb-scan-error" }, false);
                }
            },
        );
    }

    /// CPU above 80 % for more than a minute: alerts with the processes that
    /// use the most (at most one alert every 15 minutes).
    fn check_cpu(self: &Rc<Self>, load: Option<f64>) {
        let now = now_secs();
        let Some(load) = load.filter(|l| *l >= 80.0) else {
            self.cpu_high_since.set(None);
            return;
        };
        let since = *self.cpu_high_since.get().get_or_insert(now);
        self.cpu_high_since.set(Some(since));
        if now - since < 60.0 || now - self.cpu_alerted_at.get() < 900.0 {
            return;
        }
        self.cpu_alerted_at.set(now);
        let (Some(adb), Some(serial)) = (self.adb(), self.ready_serial()) else {
            return;
        };
        let weak = Rc::downgrade(self);
        crate::util::run_bg(
            move || adb.top_cpu(&serial, 3),
            move |top| {
                let Some(ctx) = weak.upgrade() else { return };
                let mut body = t!("The phone's CPU has been at {} % for more than a minute.", format!("{:.0}", load));
                if !top.is_empty() {
                    let list: Vec<String> = top.iter().map(|(name, cpu)| format!("{name} ({cpu:.0} %)")).collect();
                    body.push_str(&t!(" Top consumers: {}.", list.join(", ")));
                }
                ctx.send_notification("cpu-high", tr("Phone CPU at full load"), &body, "adb-scan-error", false);
            },
        );
    }

    // -- system tray -------------------------------------------------------------

    pub fn tray_state(&self) -> (TrayIcon, String) {
        let usb_waiting = self.usb.borrow().iter().any(|u| matches!(u.mode, UsbMode::Adb | UsbMode::Other));
        match self.selected_device() {
            Some(d) => {
                let name = self.device_label(&d);
                let icon = match d.state {
                    DeviceState::Device => TrayIcon::Connected,
                    DeviceState::Unauthorized | DeviceState::Authorizing => TrayIcon::Waiting,
                    DeviceState::Offline | DeviceState::NoPermissions | DeviceState::Unknown => TrayIcon::Error,
                    _ => TrayIcon::Waiting,
                };
                let mut tip = format!("{name} · {}", d.state_label());
                if let Some(s) = self.snapshot() {
                    if let Some(p) = s.battery.percentage() {
                        tip.push_str(&format!(" · {p:.0} %"));
                    }
                }
                (icon, tip)
            }
            None if usb_waiting => {
                let debug = self.usb.borrow().iter().any(|u| u.has_adb_interface());
                (TrayIcon::Waiting, if debug { tr("USB device detected").into() } else { tr("Phone on USB · basic analysis").into() })
            }
            None => (TrayIcon::Disconnected, tr("No device").into()),
        }
    }

    fn update_tray(&self) {
        let (icon, tip) = self.tray_state();
        if let Some(t) = self.tray.borrow().as_ref() {
            t.set_state(icon, tip);
        }
    }

    // -- operations --------------------------------------------------------------

    pub fn audit(&self, action: &str, target: Option<&str>, ok: bool, detail: Option<&str>) {
        if !self.settings.borrow().audit_log {
            return;
        }
        if let Some(h) = &self.history {
            let serial = self.selected_serial();
            let _ = h.audit(serial.as_deref(), action, target, ok, detail);
        }
    }

    /// Runs an ADB operation on the selected device in a thread, shows a toast
    /// with the result and records it in the audit log.
    pub fn device_op<W, D>(self: &Rc<Self>, action: &'static str, target: &str, success: &str, work: W, done: D)
    where
        W: FnOnce(Adb, String) -> adbscan_core::Result<String> + Send + 'static,
        D: FnOnce(bool) + 'static,
    {
        let (Some(adb), Some(serial)) = (self.adb(), self.ready_serial()) else {
            self.toast(tr("No device is ready"));
            done(false);
            return;
        };
        let weak = Rc::downgrade(self);
        let target = target.to_string();
        let success = success.to_string();
        crate::util::run_bg(
            move || work(adb, serial),
            move |res| {
                let Some(ctx) = weak.upgrade() else { return };
                match res {
                    Ok(detail) => {
                        ctx.audit(action, Some(&target), true, Some(&detail));
                        ctx.toast(&success);
                        done(true);
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        ctx.audit(action, Some(&target), false, Some(&msg));
                        ctx.toast(&format!("Error: {msg}"));
                        done(false);
                    }
                }
            },
        );
    }

    pub fn save_settings(&self) {
        if let Err(e) = self.settings.borrow().save() {
            log::error!("Could not save the settings: {e}");
        }
        self.emit(Event::SettingsChanged);
    }
}
