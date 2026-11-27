//! Connection: everything about how the phone is connected. USB (the cable,
//! the PC's port, the phone's port, the power offered, cuts and a real speed
//! test), wireless ADB, and the connection diagnosis with the phone's USB mode
//! and the ADB server. The data and deductions come from
//! `adbscan_core::link`.

use adbscan_core::{
    i18n::{tr, trc},
    t,
};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adbscan_core::format;
use adbscan_core::link::{self, CableEnds, Certainty, Level, PcLink, PhoneLink};
use adbscan_core::models::{now_secs, DeviceState};
use adbscan_core::usb::{self, UsbDevice, UsbMode};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

/// A group whose rows are rebuilt on every refresh.
struct Section {
    group: adw::PreferencesGroup,
    rows: RefCell<Vec<gtk::Widget>>,
}

impl Section {
    fn new(title: &str, description: Option<&str>) -> Self {
        let group = adw::PreferencesGroup::builder().title(title).build();
        if let Some(d) = description {
            group.set_description(Some(d));
        }
        Self { group, rows: RefCell::new(Vec::new()) }
    }

    fn clear(&self) {
        for r in self.rows.borrow_mut().drain(..) {
            self.group.remove(&r);
        }
    }

    fn add(&self, row: &impl IsA<gtk::Widget>) {
        self.group.add(row);
        self.rows.borrow_mut().push(row.clone().upcast());
    }

    /// Title, value and an optional explanation underneath.
    fn value(&self, title: &str, value: &str, sub: Option<&str>) {
        let (row, l) = util::info_row(title);
        l.set_text(if value.is_empty() { "—" } else { value });
        if let Some(s) = sub.filter(|s| !s.is_empty()) {
            row.set_subtitle(&glib::markup_escape_text(s));
        }
        self.add(&row);
    }

    /// A finding with its severity icon.
    fn status(&self, level: Level, title: &str, detail: &str) {
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(title)).subtitle(glib::markup_escape_text(detail)).build();
        let (icon, class) = level_icon(level);
        let img = util::icon(icon);
        img.add_css_class(class);
        row.add_prefix(&img);
        self.add(&row);
    }
}

fn level_icon(level: Level) -> (&'static str, &'static str) {
    match level {
        Level::Ok => ("adb-emblem-ok-symbolic", "severity-ok"),
        Level::Info => ("adb-info-symbolic", "dim"),
        Level::Warning => ("adb-dialog-warning-symbolic", "severity-medium"),
        Level::Problem => ("adb-circle-x-symbolic", "severity-high"),
    }
}

/// Connection checks (ADB, USB, permissions, authorisation, telemetry): state
/// 0 good, 1 warning, 2 bad, 3 pending. Also used by Overview.
pub fn diagnosis_rows(ctx: &Rc<Ctx>) -> Vec<(u8, String, String)> {
    let dev = ctx.selected_device();
    let mut rows: Vec<(u8, String, String)> = Vec::new();
    match ctx.adb() {
        Some(a) => rows.push((0, tr("ADB available").into(), a.path().display().to_string())),
        None if cfg!(windows) => rows.push((2, tr("ADB not found").into(), tr("Reinstall ADB Scan: adb.exe comes in its platform-tools folder").into())),
        None => rows.push((2, tr("ADB not found").into(), tr("Install adb: sudo apt install adb").into())),
    }
    if let Some(p) = ctx.monitor_problem() {
        rows.push((1, tr("ADB server").into(), p));
    }
    let selected_usb = ctx.selected_usb();
    let phone = selected_usb.as_ref();
    if !usb::HOST_USB {
        rows.push((3, tr("Phone over USB").into(), tr("Not available on Windows: the phone is seen only through adb").into()));
    }
    match phone {
        Some(u) => {
            rows.push((0, tr("Phone connected over USB").into(), format!("{} · {}", u.display_name(), u.speed_label())));
            match u.mode {
                UsbMode::Adb => rows.push((0, tr("USB debugging interface").into(), tr("Present (ADB)").into())),
                UsbMode::Fastboot => rows.push((1, tr("Fastboot mode").into(), tr("The phone is in the bootloader").into())),
                UsbMode::Edl => rows.push((1, tr("EDL mode (Qualcomm)").into(), tr("Emergency download mode").into())),
                // No debugging is not a failure: the basic analysis works.
                UsbMode::Other => rows.push((
                    3,
                    tr("USB debugging turned off").into(),
                    tr("It is optional: the basic analysis works without it. Turn it on only if you want the advanced analysis").into(),
                )),
            }
            match usb::node_accessible(u) {
                Some(true) => rows.push((0, tr("USB permissions").into(), tr("The user can access the device").into())),
                Some(false) => rows.push((2, tr("USB permissions").into(), tr("No access to the USB node: install the udev rules or add your user to plugdev").into())),
                None => {}
            }
        }
        None if dev.as_ref().is_some_and(|d| d.is_wireless()) || !usb::HOST_USB => {}
        None => rows.push((3, tr("Phone over USB").into(), tr("No connected Android phone detected").into())),
    }
    match dev.as_ref().map(|d| d.state) {
        Some(DeviceState::Device) => rows.push((0, tr("Debugging authorised").into(), tr("ADB has full access").into())),
        Some(DeviceState::Unauthorized) => rows.push((1, tr("Waiting for authorisation").into(), tr("Accept the “Allow USB debugging?” prompt on the phone").into())),
        Some(s) => rows.push((1, t!("State: {}", s.label()), String::new())),
        None if phone.is_some_and(|u| u.mode == UsbMode::Adb) => rows.push((3, tr("Authorisation").into(), tr("ADB does not see the device yet").into())),
        None => {}
    }
    if dev.as_ref().is_some_and(|d| d.connected()) {
        match ctx.snapshot() {
            Some(_) => rows.push((0, tr("Live telemetry").into(), t!("A single ADB stream every {} ms", ctx.live_interval()))),
            None => rows.push((3, tr("Live telemetry").into(), tr("Starting…").into())),
        }
    }
    rows
}

/// The phone on the PC's USB bus that matches the selected device (see
/// [`Ctx::selected_usb`]).
fn usb_phone(ctx: &Rc<Ctx>) -> Option<UsbDevice> {
    ctx.selected_usb()
}

/// Seconds since the last read after which a visible page reads again.
const REFRESH_SECS: f64 = 10.0;

/// The last USB device, PC link and phone link read.
type LastRead = (Option<UsbDevice>, Option<PcLink>, Option<Result<PhoneLink, String>>);

struct ConnectionPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::Box,
    summary: Section,
    cable: Section,
    pc: Section,
    phone: Section,
    power: Section,
    stability: Section,
    speed_group: adw::PreferencesGroup,
    /// Previous measurements (rebuilt after each one).
    speed_history: RefCell<Option<adw::ExpanderRow>>,
    speed_button: gtk::Button,
    speed_up: (adw::ActionRow, gtk::Label),
    speed_down: (adw::ActionRow, gtk::Label),
    current: (adw::ActionRow, gtk::Label),
    diag: Section,
    diag_key: RefCell<String>,
    server_info: gtk::Label,
    busy: Cell<bool>,
    last_read: Cell<f64>,
    /// Last data read, to repaint on a privacy change without reading again.
    last: RefCell<LastRead>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    ConnectionPage::new(ctx)
}

fn tab(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    util::page_scroller(child, 980)
}

impl ConnectionPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let stack = adw::ViewStack::new();
        let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();
        switcher.set_halign(gtk::Align::Center);
        switcher.set_margin_top(10);

        // -- USB ----------------------------------------------------------------
        let usb_box = util::vbox(18);
        let summary = Section::new(tr("Summary"), None);
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Read again"));
        refresh.add_css_class("flat");
        summary.group.set_header_suffix(Some(&refresh));
        let cable = Section::new(tr("Cable"), Some(tr("What is read from the hardware and what is deduced (marked as probable).")));
        let pc = Section::new(tr("PC port"), None);
        let phone = Section::new(tr("Phone's port"), None);
        let power = Section::new(tr("Power"), None);
        let stability = Section::new(tr("Stability"), None);
        let speed = adw::PreferencesGroup::builder().title(tr("Real speed")).build();
        let speed_row = adw::ActionRow::builder()
            .title(tr("Measure the transfer speed"))
            .subtitle(tr("Sends 128 MB to the phone and reads them back. Nothing is left on the phone."))
            .build();
        let speed_button = util::pill_button("adb-activity-symbolic", tr("Measure"));
        speed_button.set_valign(gtk::Align::Center);
        speed_row.add_suffix(&speed_button);
        speed.add(&speed_row);
        let speed_up = util::info_row(tr("PC → phone"));
        let speed_down = util::info_row(tr("Phone → PC"));
        for (r, _) in [&speed_up, &speed_down] {
            r.set_visible(false);
            speed.add(r);
        }
        for g in [&summary.group, &cable.group, &pc.group, &phone.group, &power.group, &stability.group] {
            usb_box.append(g);
        }
        usb_box.append(&speed);
        stack.add_titled_with_icon(&tab(&usb_box), Some("usb"), "USB", "adb-usb-symbolic");

        // -- Wi-Fi ---------------------------------------------------------------
        let wifi_box = util::vbox(18);
        let now_group = adw::PreferencesGroup::builder().title(tr("Current connection")).build();
        let current = util::info_row(tr("Selected device"));
        now_group.add(&current.0);
        wifi_box.append(&now_group);
        let wireless = adw::PreferencesGroup::builder()
            .title(tr("Wireless ADB"))
            .description(tr("Turns on ADB over TCP (port 5555) and connects over Wi-Fi so you can unplug the cable. The phone and the PC must be on the same network."))
            .build();
        let enable_row = adw::ActionRow::builder().title(tr("Switch this device to Wi-Fi")).subtitle("adb tcpip 5555 + adb connect IP:5555").build();
        let enable_btn = gtk::Button::with_label(tr("Turn on"));
        enable_btn.add_css_class("suggested-action");
        enable_btn.set_valign(gtk::Align::Center);
        enable_row.add_suffix(&enable_btn);
        wireless.add(&enable_row);
        // Wireless debugging (Android 11+): QR or manual, in the Connect dialog.
        for (title, sub, target) in [
            (tr("Pair with a QR code (Android 11+)"), tr("Wireless debugging → Pair device with QR code"), "qr"),
            (tr("Pair manually (Android 11+)"), tr("IP address, pairing port and 6-digit code"), "manual"),
            (tr("All the ways to connect"), tr("USB, basic analysis without debugging, QR or manual pairing"), "root"),
        ] {
            let row = adw::ActionRow::builder().title(title).subtitle(sub).activatable(true).build();
            row.add_suffix(&util::icon("go-next-symbolic"));
            row.set_action_name(Some("win.connect"));
            row.set_action_target_value(Some(&target.to_variant()));
            wireless.add(&row);
        }
        wifi_box.append(&wireless);
        stack.add_titled_with_icon(&tab(&wifi_box), Some("wifi"), "Wi-Fi", "adb-wifi-symbolic");

        // -- Diagnosis -----------------------------------------------------------
        let diag_box = util::vbox(18);
        let diag = Section::new(tr("Connection diagnosis"), None);
        diag_box.append(&diag.group);
        let usb_mode = adw::PreferencesGroup::builder().title(tr("Phone USB mode")).description(tr("Switching to MTP or PTP without ADB cuts USB debugging until the cable is reconnected.")).build();
        for (func, title, sub) in [
            ("mtp", tr("File transfer (MTP)"), tr("Turns off ADB over USB")),
            ("ptp", tr("Photos (PTP)"), tr("Turns off ADB over USB")),
            ("rndis", tr("USB tethering"), tr("The PC uses the phone's data")),
            ("none", tr("Charging only"), tr("Turns off ADB over USB")),
        ] {
            let row = adw::ActionRow::builder().title(title).subtitle(sub).activatable(true).build();
            row.add_suffix(&util::icon("adb-link-symbolic"));
            let c = ctx.clone();
            row.connect_activated(move |r| {
                let c2 = c.clone();
                let f = func.to_string();
                util::confirm(
                    r,
                    &t!("Switch to “{}”?", title),
                    tr("The ADB connection over USB may close. To get it back, unplug and replug the cable or choose “File transfer” on the phone."),
                    tr("Switch"),
                    false,
                    move || {
                        let f2 = f.clone();
                        c2.device_op("usb_function", &f, tr("USB mode changed"), move |adb, s| adb.set_usb_function(&s, &f2).map(|r| r.summary()), |_| {});
                    },
                );
            });
            usb_mode.add(&row);
        }
        diag_box.append(&usb_mode);
        let server = adw::PreferencesGroup::builder().title(tr("ADB server")).build();
        let server_info = util::label("", &["dim", "caption"]);
        let restart = adw::ActionRow::builder().title(tr("Restart the ADB server")).subtitle(tr("adb kill-server + start-server: fixes “offline” devices")).activatable(true).build();
        let reconnect = adw::ActionRow::builder().title(tr("Reconnect the device")).subtitle("adb reconnect device").activatable(true).build();
        restart.add_suffix(&util::icon("adb-refresh-cw-symbolic"));
        reconnect.add_suffix(&util::icon("adb-link-symbolic"));
        server.add(&restart);
        server.add(&reconnect);
        server.add(&server_info);
        diag_box.append(&server);
        stack.add_titled_with_icon(&tab(&diag_box), Some("diagnosis"), tr("Diagnosis"), "adb-heart-pulse-symbolic");

        let root = util::vbox(0);
        root.append(&switcher);
        root.append(&stack);
        stack.set_vexpand(true);

        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            summary,
            cable,
            pc,
            phone,
            power,
            stability,
            speed_group: speed.clone(),
            speed_history: RefCell::new(None),
            speed_button: speed_button.clone(),
            speed_up,
            speed_down,
            current,
            diag,
            diag_key: RefCell::new(String::new()),
            server_info,
            busy: Cell::new(false),
            last_read: Cell::new(0.0),
            last: RefCell::new((None, None, None)),
        });
        macro_rules! on {
            ($w:expr, $sig:ident, |$t:ident, $a:pat_param| $body:expr) => {{
                let w = this.me.clone();
                $w.$sig(move |$a| {
                    if let Some($t) = w.upgrade() {
                        $body;
                    }
                });
            }};
        }
        on!(refresh, connect_clicked, |t, _| t.read());
        on!(speed_button, connect_clicked, |t, _| t.speed_test());
        on!(enable_btn, connect_clicked, |t, b| t.enable_wireless(b));
        on!(restart, connect_activated, |t, _| t.restart_server());
        {
            let c = ctx.clone();
            reconnect.connect_activated(move |_| c.device_op("reconnect", "", tr("Reconnecting…"), |adb, s| adb.reconnect(&s).map(|r| r.summary()), |_| {}));
        }
        this.render_history();
        this.read();
        this.refresh_diag();
        this.load_server_info();
        this
    }

    /// The previous measurements, newest first, with the best one marked: to
    /// compare cables and ports.
    fn render_history(&self) {
        if let Some(old) = self.speed_history.borrow_mut().take() {
            self.speed_group.remove(&old);
        }
        let history = self.ctx.settings.borrow().speed_tests.clone();
        if history.is_empty() {
            return;
        }
        let best = history.iter().map(|r| r.score()).fold(0.0, f64::max);
        let names = self.ctx.devices();
        let exp = adw::ExpanderRow::builder()
            .title(t!("Previous measurements ({})", history.len()))
            .subtitle(tr("Measure with each cable or port to compare them; the best one is marked"))
            .build();
        for r in &history {
            let who = names.iter().find(|d| d.serial == r.serial).map(|d| self.ctx.device_label(d)).unwrap_or_else(|| self.ctx.shown_value(&r.serial));
            let link = r.link_mbps.map(|m| format!(" · {}", link::speed_label(m))).unwrap_or_default();
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&format!("{} · {}", format::timestamp(Some(r.ts)), who)))
                .subtitle(glib::markup_escape_text(&format!("{}{link}", r.via)))
                .build();
            let value = util::label(&format!("↑ {} · ↓ {} MB/s", format::number(Some(r.upload_mbs), 0), format::number(Some(r.download_mbs), 0)), &["numeric"]);
            if (r.score() - best).abs() < f64::EPSILON {
                let star = util::icon("adb-emblem-ok-symbolic");
                star.add_css_class("severity-ok");
                star.set_tooltip_text(Some(tr("The best so far")));
                row.add_suffix(&star);
            }
            row.add_suffix(&value);
            exp.add_row(&row);
        }
        let forget = gtk::Button::with_label(tr("Forget"));
        forget.add_css_class("flat");
        forget.set_valign(gtk::Align::Center);
        let w = self.me.clone();
        forget.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.ctx.settings.borrow_mut().speed_tests.clear();
                t.ctx.save_settings();
                t.render_history();
            }
        });
        exp.add_suffix(&forget);
        self.speed_group.add(&exp);
        *self.speed_history.borrow_mut() = Some(exp);
    }

    /// Reads both sides of the link in the background and repaints.
    fn read(&self) {
        if self.busy.get() {
            return;
        }
        self.busy.set(true);
        self.last_read.set(now_secs());
        let dev = usb_phone(&self.ctx);
        let adb_phone = match (self.ctx.adb(), self.ctx.ready_serial()) {
            (Some(adb), Some(serial)) => Some((adb, serial)),
            _ => None,
        };
        let w = self.me.clone();
        let d = dev.clone();
        util::run_bg(
            move || {
                let pc = d.as_ref().map(link::read_pc_link);
                let phone = adb_phone.map(|(adb, serial)| adb.phone_link(&serial).map_err(|e| e.to_string()));
                (pc, phone)
            },
            move |(pc, phone)| {
                let Some(t) = w.upgrade() else { return };
                t.busy.set(false);
                *t.last.borrow_mut() = (dev, pc, phone);
                t.render();
            },
        );
    }

    fn render(&self) {
        let (dev, pc, phone) = self.last.borrow().clone();
        let phone_ok = phone.as_ref().and_then(|p| p.as_ref().ok());
        let best = dev.as_ref().and_then(|d| self.ctx.settings.borrow().usb_best_speed.get(&d.serial).copied());
        let a = link::assess(pc.as_ref(), phone_ok, best);
        for s in [&self.summary, &self.cable, &self.pc, &self.phone, &self.power, &self.stability] {
            s.clear();
        }

        // -- summary ---------------------------------------------------------
        if !usb::HOST_USB {
            self.summary.status(
                Level::Info,
                tr("PC side: not available on Windows"),
                tr("The cable and the PC's port are read from Linux's USB system. The phone's port, the power, the cuts and the speed test come through adb and do work."),
            );
        } else if dev.is_none() {
            let wireless = self.ctx.selected_device().is_some_and(|d| d.is_wireless());
            self.summary.status(
                Level::Info,
                tr("No phone on the USB cable"),
                if wireless { tr("The selected phone is connected over Wi-Fi: see the Wi-Fi tab.") } else { tr("Connect the phone with a cable to see the link, the cable and the ports.") },
            );
        }
        for f in &a.findings {
            self.summary.status(f.level, &f.title, &f.detail);
        }

        // -- cable -------------------------------------------------------------
        let probable = |c: Certainty, text: &str| if c == Certainty::Probable { t!("{} (probable)", text) } else { text.to_string() };
        if let Some(p) = &pc {
            let ends = match a.cable.ends {
                CableEnds::UsbCToC => probable(a.cable.ends_certainty, tr("USB-C to USB-C")),
                CableEnds::Unknown => tr("Cannot be told").to_string(),
            };
            self.cable.value(tr("Plugs"), &ends, Some(&a.cable.ends_reason));
            let wires = match a.cable.superspeed {
                Some(true) => tr("USB 3 (SuperSpeed)"),
                Some(false) => tr("USB 2.0 only"),
                None => tr("Cannot be told"),
            };
            self.cable.value(tr("Data wires"), wires, Some(&a.cable.superspeed_reason));
            match p.typec.as_ref().and_then(|t| t.cable.as_ref()) {
                Some(c) => {
                    let kind = if c.kind == "active" { tr("Active (with electronics inside)") } else { tr("Passive") };
                    self.cable.value(tr("Type"), kind, Some(tr("According to the cable's chip (e-marker)")));
                    if let Some(o) = c.optical {
                        self.cable.value(tr("Conductor"), if o { tr("Optical fibre") } else { tr("Copper") }, None);
                    }
                    if let Some(s) = c.speed {
                        self.cable.value(tr("Rated speed"), s, None);
                    }
                    if let Some(a) = c.current_a {
                        let v = c.max_voltage_v.map(|v| format!(" · {v} V")).unwrap_or_default();
                        self.cable.value(tr("Rated current"), &format!("{a} A{v}"), Some(if a >= 5 { tr("Up to 240 W with USB PD") } else { tr("Up to 60 W with USB PD") }));
                    }
                    if let Some(l) = c.length {
                        self.cable.value(tr("Length (approx.)"), l, Some(tr("Estimated from the signal delay the cable declares")));
                    }
                    if let Some(v) = c.vendor_id {
                        self.cable.value(tr("Maker (USB ID)"), &format!("{v:04x}"), None);
                    }
                }
                None => self.cable.value(
                    tr("Cable chip (e-marker)"),
                    tr("Not readable"),
                    Some(tr("Copper or fibre, rated speed and current are only known from the cable's own chip. Linux shows it when the PC has a USB-C controller it can read (UCSI); this PC does not expose one, and Android hides it without root.")),
                ),
            }
            let hubs = if p.hubs.is_empty() { tr("None detected").to_string() } else { p.hubs.join(" → ") };
            self.cable.value(tr("Hubs and docks in between"), &hubs, Some(tr("Hubs and docks show up; a simple adapter (USB-A to USB-C) leaves no trace.")));
        }

        // -- PC port --------------------------------------------------------------
        if let Some(p) = &pc {
            let lanes = if p.lanes() >= 2 { tr("Two lanes each way") } else { tr("One lane each way") };
            let speed = p.speed_mbps.map(link::speed_label).unwrap_or_else(|| "—".into());
            self.pc.value(tr("Link"), &format!("{speed} · USB {}", p.version), Some(lanes));
            let kind = match p.connect_type.as_str() {
                "hotplug" => tr("external port"),
                "hardwired" => tr("internal"),
                _ => "",
            };
            let port = match p.root_port {
                Some(n) => t!("Bus {} · port {}", p.bus, n),
                None => t!("Bus {}", p.bus),
            };
            self.pc.value(tr("Port"), &if kind.is_empty() { port } else { format!("{port} ({kind})") }, None);
            if let Some(m) = p.port_max_mbps {
                self.pc.value(tr("Port maximum"), &link::speed_label(m), Some(if p.port_superspeed { tr("USB 3 port") } else { tr("USB 2.0 only port") }));
            }
            if let Some(l) = p.location.as_ref().map(|l| l.label()).filter(|l| !l.is_empty()) {
                self.pc.value(tr("Location"), &l, Some(tr("As the PC's firmware (ACPI) describes it; it is not always exact")));
            }
            if let Some(c) = &p.controller {
                let sub = [c.address.as_str(), c.driver.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                self.pc.value(tr("Controller"), &c.name, Some(&sub));
            }
            if let Some(ma) = p.max_power_ma {
                self.pc.value(tr("Power the phone asks for"), &format!("{ma} mA"), Some(tr("What its USB configuration declares; charging is negotiated separately")));
            }
            let lpm = match p.lpm.as_str() {
                "u1_u2" => tr("U1 and U2"),
                "u1" => "U1",
                "u2" => "U2",
                "0" => tr("Off"),
                _ => "",
            };
            if !lpm.is_empty() {
                self.pc.value(tr("Link power saving"), lpm, Some(tr("Low-power states the port allows between transfers")));
            }
            if let Some(n) = p.over_current {
                self.pc.value(tr("Over-current events"), &n.to_string(), Some(tr("Since the PC started")));
            }
            if let Some(tc) = &p.typec {
                let mode = match tc.power_mode.as_str() {
                    "default" => tr("Standard USB current"),
                    "1.5A" => "1.5 A",
                    "3.0A" => "3 A",
                    "usb_power_delivery" => "USB Power Delivery",
                    other => other,
                };
                let sub = [
                    (!tc.pd_revision.is_empty() && tc.pd_revision != "0.0").then(|| format!("PD {}", tc.pd_revision)),
                    match tc.orientation.as_str() {
                        "normal" => Some(tr("plug face up").to_string()),
                        "reverse" => Some(tr("plug turned over").to_string()),
                        _ => None,
                    },
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                self.pc.value(tr("USB-C port"), mode, Some(&sub));
            }
        }

        // -- phone's port, power, stability ------------------------------------
        // Only the groups with something to show.
        self.cable.group.set_visible(pc.is_some());
        self.pc.group.set_visible(pc.is_some());
        for s in [&self.phone, &self.power, &self.stability] {
            s.group.set_visible(phone.is_some());
        }
        self.speed_group.set_visible(phone.is_some());
        if pc.is_some() && phone.is_none() {
            self.summary.status(Level::Info, tr("Needs ADB"), tr("The phone's side (its port, power and cuts) is read with USB or wireless debugging turned on."));
        }
        match &phone {
            None => {}
            Some(Err(e)) => {
                for s in [&self.phone, &self.power, &self.stability] {
                    s.status(Level::Warning, tr("Could not read the phone"), e);
                }
            }
            Some(Ok(ph)) => self.render_phone(ph),
        }
        let selected = self.ctx.selected_device();
        let text = match &selected {
            Some(d) if d.is_wireless() => t!("Wi-Fi · {}", self.ctx.shown_value(&d.serial)),
            Some(_) => tr("USB cable").into(),
            None => tr("No device").into(),
        };
        self.current.1.set_text(&text);
        util::cap_wrapping(&self.root);
    }

    fn render_phone(&self, ph: &PhoneLink) {
        let s = &self.phone;
        if !ph.controller.is_empty() {
            let kind = if ph.controller.contains("dwc3") { tr("Synopsys DesignWare USB 3 (dwc3)") } else { "" };
            s.value(tr("Controller"), &ph.controller, Some(kind));
        }
        if !ph.port_modes.is_empty() {
            let modes: Vec<String> = ph
                .port_modes
                .iter()
                .map(|m| match m.as_str() {
                    "dual" => tr("host and device (OTG)").to_string(),
                    "ufp" => tr("device").to_string(),
                    "dfp" => tr("host").to_string(),
                    "audio_acc" => tr("analogue audio adapter").to_string(),
                    "debug_acc" => tr("debug accessory").to_string(),
                    other => other.to_string(),
                })
                .collect();
            s.value(tr("Port modes"), &modes.join(", "), None);
        }
        if !ph.data_role.is_empty() {
            let data = match ph.data_role.as_str() {
                "device" => tr("Device: the PC is in charge"),
                "host" => tr("Host: the phone is in charge"),
                other => other,
            };
            let power = match ph.power_role.as_str() {
                "sink" => tr("receives power"),
                "source" => tr("gives power"),
                _ => "",
            };
            s.value(tr("Current role"), data, Some(power));
        }
        let (connector, level) = match ph.contaminant.as_str() {
            "not detected" => (tr("Clean"), None),
            "not supported" => (tr("This phone does not check it"), None),
            "disabled" => (tr("Check turned off"), None),
            "detected" => (tr("Moisture or dirt detected"), Some(Level::Problem)),
            "" => ("", None),
            other => (other, None),
        };
        if !connector.is_empty() {
            match level {
                Some(l) => s.status(l, tr("Connector"), connector),
                None => s.value(tr("Connector"), connector, Some(tr("Moisture and dirt detection in the USB-C port"))),
            }
        }
        if !ph.data_status.is_empty() {
            let v = if ph.data_status == "enabled" { trc("plural", "Enabled") } else { ph.data_status.as_str() };
            s.value(tr("Data over USB"), v, None);
        }
        let warnings = match ph.supports_compliance {
            Some(false) => tr("This phone does not report them").to_string(),
            _ if ph.compliance.is_empty() => tr("None").to_string(),
            _ => ph.compliance.iter().map(|w| link::compliance_label(w)).collect::<Vec<_>>().join(" · "),
        };
        s.value(tr("Cable warnings"), &warnings, Some(tr("Android 15+ warns about cables without data wires, weak chargers and unstable connections")));
        if ph.displayport == Some(true) {
            s.value(tr("Video output"), tr("DisplayPort over USB-C"), Some(tr("It can show its screen on a monitor with a USB-C video cable")));
        }
        if let Some(secs) = ph.connected_for_s {
            s.value(tr("Connected for"), &format::human_duration(Some(secs)), None);
        }
        if !ph.usb_config.is_empty() {
            s.value(tr("USB functions"), &ph.usb_config.replace(',', " + "), None);
        }
        if let Ok(v) = ph.hal_version.parse::<f64>() {
            s.value(tr("USB HAL version"), &format!("{:.1}", v / 10.0), None);
        }

        // Power
        let p = &self.power;
        let source = match (ph.ac_powered, ph.usb_powered) {
            (Some(true), _) => tr("As from a charger"),
            (_, Some(true)) => tr("As from a computer port"),
            (Some(false), Some(false)) => tr("Not charging from the cable"),
            _ => "—",
        };
        p.value(tr("Android sees the cable"), source, Some(tr("A USB-C port that offers 1.5 A or 3 A counts as a charger")));
        if let (Some(ua), Some(uv)) = (ph.max_current_ua, ph.max_voltage_uv) {
            let (a, v) = (ua as f64 / 1e6, uv as f64 / 1e6);
            p.value(tr("Offered"), &format!("{} V · {} A · {} W", format::number(Some(v), 1), format::number(Some(a), 1), format::number(Some(a * v), 1)), Some(tr("What the port announces; the phone may take less")));
        }
        let kind = match ph.charge_type.as_str() {
            "Fast" => tr("fast charging"),
            "Standard" => tr("standard charging"),
            "Trickle" => tr("trickle charging"),
            "Adaptive" => tr("adaptive charging"),
            _ => "",
        };
        match link::charge_now(ph) {
            Some(link::ChargeNow::Charging { watts, amps, volts }) => {
                let offered = ph.max_current_ua.zip(ph.max_voltage_uv).map(|(a, v)| a as f64 / 1e6 * v as f64 / 1e6).filter(|w| *w > 0.0);
                let mut sub = match offered {
                    Some(o) => t!("Measured at the battery, of the {} W the port offers", format::number(Some(o), 1)),
                    None => tr("Measured at the battery").to_string(),
                };
                if !kind.is_empty() {
                    sub = format!("{sub} · {kind}");
                }
                let value = format!("{} W · {} A · {} V", format::number(Some(watts), 1), format::number(Some(amps), 2), format::number(Some(volts), 2));
                p.value(tr("Charging now"), &value, Some(&sub));
            }
            Some(link::ChargeNow::PausedAtLimit(limit)) => {
                p.status(Level::Info, &t!("Charging paused at {} %", limit), tr("The phone stops charging there to protect the battery; it is not a problem with the cable or the port."));
            }
            Some(link::ChargeNow::Full) => p.value(tr("Charging now"), tr("Battery full"), None),
            Some(link::ChargeNow::Discharging(w)) => {
                let sub = w.map(|w| t!("The phone is using {} W from its battery", format::number(Some(w), 1)));
                p.value(tr("Charging now"), tr("Not charging"), sub.as_deref());
            }
            None => {}
        }

        // Stability
        let st = link::stability(&ph.events, ph.now);
        let st_section = &self.stability;
        if st.disconnects == 0 {
            st_section.status(Level::Ok, tr("No cuts recorded"), tr("The phone keeps its latest USB events: none is a cut."));
        } else {
            st_section.add(&cuts_timeline(&ph.events, ph.now));
            st_section.value(tr("Cuts in the phone's log"), &st.disconnects.to_string(), Some(tr("Includes unplugging it and changing the USB mode")));
            let worst = t!("{} · {}", st.worst_minute, st.worst_at);
            if st.worst_minute >= link::UNSTABLE_CUTS_PER_MINUTE {
                // An old burst stays as a note: the connection may be fine now.
                let level = if st.is_recent() { Level::Warning } else { Level::Info };
                st_section.status(level, &t!("Worst minute: {}", worst), tr("Many cuts in a row point to a loose plug, a worn cable or a dirty port."));
            } else {
                st_section.value(tr("Worst minute"), &worst, None);
            }
            if let Some((at, ago)) = &st.last_cut {
                let v = match ago {
                    Some(secs) => t!("{} ({} ago)", at, format::human_duration(Some(*secs))),
                    None => at.clone(),
                };
                st_section.value(tr("Last cut"), &v, None);
            }
        }
    }

    fn speed_test(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            self.ctx.toast(tr("No device is ready"));
            return;
        };
        self.speed_button.set_sensitive(false);
        self.ctx.toast(tr("Measuring… it takes a few seconds"));
        let w = self.me.clone();
        let who = serial.clone();
        util::run_bg(
            move || adb.speed_test(&serial),
            move |res| {
                let Some(t) = w.upgrade() else { return };
                t.speed_button.set_sensitive(true);
                match res {
                    Ok(r) => {
                        let ceiling = t.last.borrow().1.as_ref().and_then(|p| p.speed_mbps).map(link::link_ceiling_mbs);
                        let sub = ceiling.map(|c| t!("The link alone allows up to {} MB/s; ADB and the phone's storage are usually the limit.", format::number(Some(c), 0)));
                        for ((row, label), v) in [(&t.speed_up, r.upload_mbs), (&t.speed_down, r.download_mbs)] {
                            label.set_text(&format!("{} MB/s", format::number(Some(v), 1)));
                            if let Some(s) = &sub {
                                row.set_subtitle(s);
                            }
                            row.set_visible(true);
                        }
                        t.ctx.audit("speed_test", None, true, Some(&format!("{:.1} / {:.1} MB/s", r.upload_mbs, r.download_mbs)));
                        let wireless = t.ctx.selected_device().is_some_and(|d| d.is_wireless());
                        let pc = t.last.borrow().1.clone().filter(|_| !wireless);
                        let via = match &pc {
                            Some(p) => {
                                let port = p.root_port.map(|n| t!("Bus {} · port {}", p.bus, n)).unwrap_or_else(|| t!("Bus {}", p.bus));
                                if p.hubs.is_empty() { port } else { t!("{} (through a hub)", port) }
                            }
                            None => "Wi-Fi".to_string(),
                        };
                        let record = link::SpeedRecord { ts: now_secs(), serial: who, via, link_mbps: pc.and_then(|p| p.speed_mbps), upload_mbs: r.upload_mbs, download_mbs: r.download_mbs };
                        link::remember_speed(&mut t.ctx.settings.borrow_mut().speed_tests, record);
                        t.ctx.save_settings();
                        t.render_history();
                    }
                    Err(e) => t.ctx.toast(&t!("Could not measure the speed: {}", e)),
                }
            },
        );
    }

    fn refresh_diag(&self) {
        let rows = diagnosis_rows(&self.ctx);
        let key = format!("{}{rows:?}", self.ctx.private_mode());
        if *self.diag_key.borrow() == key {
            return;
        }
        *self.diag_key.borrow_mut() = key;
        self.diag.clear();
        for (state, title, detail) in rows {
            let level = match state {
                0 => Level::Ok,
                1 => Level::Warning,
                2 => Level::Problem,
                _ => Level::Info,
            };
            self.diag.status(level, &self.ctx.shown(&title), &self.ctx.shown(&detail));
        }
    }

    fn load_server_info(&self) {
        let Some(adb) = self.ctx.adb() else {
            self.server_info.set_text(tr("adb not found"));
            return;
        };
        let w = self.me.clone();
        let path = adb.path().to_path_buf();
        util::run_bg(
            move || adbscan_core::adb::tool_version(&path, adbscan_core::adb::Tool::Adb),
            move |v| {
                if let Some(t) = w.upgrade() {
                    t.server_info.set_text(&v.unwrap_or_default());
                }
            },
        );
    }

    fn restart_server(&self) {
        let Some(adb) = self.ctx.adb() else { return };
        let ctx = self.ctx.clone();
        ctx.toast(tr("Restarting the ADB server…"));
        util::run_bg(
            move || {
                let _ = adb.kill_server();
                adb.start_server()
            },
            move |r| {
                ctx.audit("adb_restart", None, r.is_ok(), None);
                ctx.start_monitors();
                ctx.toast(if r.is_ok() { tr("ADB server restarted") } else { tr("Could not restart the server") });
            },
        );
    }

    fn enable_wireless(&self, button: &gtk::Button) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            self.ctx.toast(tr("No device is ready"));
            return;
        };
        let Some(ip) = self.ctx.snapshot().and_then(|s| s.network.wifi_ipv4()) else {
            self.ctx.toast(tr("The phone has no Wi-Fi IP. Connect it to a network first."));
            return;
        };
        button.set_sensitive(false);
        let b = button.clone();
        let ctx = self.ctx.clone();
        let addr = format!("{ip}:5555");
        util::run_bg(
            move || -> adbscan_core::Result<String> {
                adb.tcpip(&serial, 5555)?;
                // The adbd daemon takes a moment to restart in TCP mode.
                std::thread::sleep(Duration::from_millis(1800));
                let mut last = None;
                for _ in 0..4 {
                    match adb.connect(&addr) {
                        Ok(_) => return Ok(addr),
                        Err(e) => last = Some(e),
                    }
                    std::thread::sleep(Duration::from_millis(1000));
                }
                Err(last.unwrap())
            },
            move |r| {
                b.set_sensitive(true);
                match r {
                    Ok(addr) => {
                        {
                            let mut s = ctx.settings.borrow_mut();
                            s.wireless_history.retain(|a| *a != addr);
                            s.wireless_history.insert(0, addr.clone());
                            s.wireless_history.truncate(8);
                        }
                        ctx.save_settings();
                        ctx.audit("adb_wifi", Some(&addr), true, None);
                        ctx.toast(&t!("Connected over Wi-Fi to {}. You can now unplug the cable.", addr));
                        ctx.refresh_devices();
                    }
                    Err(e) => ctx.toast(&t!("Could not connect over Wi-Fi: {}", e)),
                }
            },
        );
    }
}

impl Page for ConnectionPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn on_event(&self, ev: &Event) {
        match ev {
            Event::Devices => {
                self.refresh_diag();
                self.read();
            }
            Event::Tools => self.load_server_info(),
            Event::Privacy => {
                self.refresh_diag();
                self.render();
            }
            // Telemetry only reaches the visible page: it serves as a clock.
            Event::Snapshot => {
                self.refresh_diag();
                if now_secs() - self.last_read.get() > REFRESH_SECS {
                    self.read();
                }
            }
            _ => {}
        }
    }

    fn shown(&self) {
        self.refresh_diag();
        if now_secs() - self.last_read.get() > 2.0 {
            self.read();
        }
    }
}

/// Line of time from the first event in the phone's USB log to now, with a
/// mark at each cut: a burst shows up as a thick band.
fn cuts_timeline(events: &[link::UsbEvent], now: Option<f64>) -> adw::PreferencesRow {
    let cuts: Vec<f64> = events.iter().filter(|e| !e.connected).map(|e| e.t).collect();
    let start = events.first().map(|e| e.t).unwrap_or(0.0);
    let end = now.unwrap_or_else(|| events.last().map(|e| e.t).unwrap_or(start)).max(start + 1.0);
    let area = gtk::DrawingArea::builder().content_height(34).hexpand(true).build();
    area.set_draw_func(move |w, cr, width, height| {
        let (width, height) = (width as f64, height as f64);
        let fg = w.color();
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.12);
        cr.rectangle(0.0, height / 2.0 - 2.0, width, 4.0);
        let _ = cr.fill();
        cr.set_source_rgba(0.90, 0.60, 0.04, 0.9);
        for t in &cuts {
            let x = ((t - start) / (end - start)).clamp(0.0, 1.0) * (width - 2.0) + 1.0;
            cr.rectangle(x - 1.0, 4.0, 2.0, height - 8.0);
        }
        let _ = cr.fill();
    });
    let first = events.first().map(|e| e.at.clone()).unwrap_or_default();
    let labels = util::hbox(6);
    labels.append(&util::label(&first, &["dim", "caption", "numeric"]));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    labels.append(&spacer);
    labels.append(&util::label(if now.is_some() { tr("now") } else { "" }, &["dim", "caption"]));
    let content = util::vbox(4);
    content.set_margin_top(10);
    content.set_margin_bottom(10);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&util::label(tr("Cuts over time"), &[]));
    content.append(&area);
    content.append(&labels);
    let row = adw::PreferencesRow::new();
    row.set_activatable(false);
    row.set_child(Some(&content));
    row.set_tooltip_text(Some(tr("Each mark is a cut recorded by the phone; a thick band is a burst of cuts")));
    row
}
