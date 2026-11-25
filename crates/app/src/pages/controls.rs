//! Controls: the phone's quick settings (Wi-Fi, mobile data, Bluetooth,
//! airplane mode, location, brightness, volume, Do Not Disturb…).
//!
//! **They change the phone.** Every change is an explicit action, is checked by
//! reading the state back and is recorded in History. Reading the state is
//! read-only and only happens while the page is in view.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use adbscan_core::controls::{self, Action, ControlState, Dnd, Level, Toggle, Volume};
use adbscan_core::models::now_secs;
use adbscan_core::probe::Probe;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

/// How often the state is read again while the page is in view.
const REFRESH_SECS: f64 = 5.0;

struct ControlsPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    state: RefCell<Option<ControlState>>,
    switches: Vec<(Toggle, adw::SwitchRow)>,
    /// Values that were read are being painted: the changes do not come from
    /// the user.
    updating: Cell<bool>,
    /// Changes in progress: while there are any, the timed re-read is skipped.
    pending: Cell<u32>,
    reading: Cell<bool>,
    last_read: Cell<f64>,
    serial: RefCell<Option<String>>,
    brightness: (adw::ActionRow, gtk::Scale),
    media: (adw::ActionRow, gtk::Scale),
    ring: (adw::ActionRow, gtk::Scale),
    timeout: adw::ComboRow,
    dnd: adw::ComboRow,
    nfc: gtk::Label,
    hotspot: gtk::Label,
    status: gtk::Label,
    refresh: gtk::Button,
    /// Delays before applying a slider once it stops moving.
    debounce: RefCell<HashMap<&'static str, glib::SourceId>>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    ControlsPage::new(ctx)
}

fn toggle_icon(t: Toggle) -> &'static str {
    match t {
        Toggle::Wifi => "adb-wifi-symbolic",
        Toggle::MobileData => "adb-network-symbolic",
        Toggle::Bluetooth => "adb-bluetooth-symbolic",
        Toggle::Airplane => "adb-airplane-mode-symbolic",
        Toggle::Location => "adb-mark-location-symbolic",
        Toggle::DarkMode => "adb-night-light-symbolic",
        Toggle::AutoRotate => "adb-object-rotate-right-symbolic",
        Toggle::AutoBrightness => "adb-display-brightness-symbolic",
        Toggle::BatterySaver => "adb-battery-low-symbolic",
        Toggle::DataSaver => "adb-globe-symbolic",
    }
}

fn slider_row(title: &str, icon: &str) -> (adw::ActionRow, gtk::Scale) {
    let row = adw::ActionRow::builder().title(title).build();
    row.add_prefix(&util::icon(icon));
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    scale.set_hexpand(true);
    scale.set_width_request(260);
    scale.set_draw_value(true);
    scale.set_digits(0);
    scale.set_value_pos(gtk::PositionType::Right);
    scale.set_valign(gtk::Align::Center);
    row.add_suffix(&scale);
    (row, scale)
}

/// Row for something that cannot be changed without root: state and a button to open it on
/// the phone.
fn open_row(title: &str, subtitle: &str, icon: &str) -> (adw::ActionRow, gtk::Label, gtk::Button) {
    let row = adw::ActionRow::builder().title(title).subtitle(subtitle).build();
    row.add_prefix(&util::icon(icon));
    let label = util::label("", &["dim"]);
    let open = gtk::Button::with_label(tr("Open on the phone"));
    open.add_css_class("flat");
    open.set_valign(gtk::Align::Center);
    row.add_suffix(&label);
    row.add_suffix(&open);
    (row, label, open)
}

impl ControlsPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);

        // Notice: this page changes the phone.
        let head = adw::PreferencesGroup::builder()
            .title(tr("Phone controls"))
            .description(tr("These controls change the phone immediately, like its quick settings. Every change is checked on the phone and recorded in History."))
            .build();
        let status = util::label("", &["dim", "caption"]);
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Read the state again"));
        refresh.add_css_class("flat");
        refresh.set_valign(gtk::Align::Center);
        head.set_header_suffix(Some(&refresh));
        head.add(&status);
        content.append(&head);

        let mut switches = Vec::new();
        let mut add_switch = |group: &adw::PreferencesGroup, t: Toggle, subtitle: &str| {
            let row = adw::SwitchRow::builder().title(t.label()).subtitle(subtitle).build();
            let icon = toggle_icon(t);
            let image = util::icon(icon);
            image.set_pixel_size(16);
            row.add_prefix(&image);
            group.add(&row);
            switches.push((t, row));
        };

        // Connections
        let net = adw::PreferencesGroup::builder().title(tr("Connections")).build();
        add_switch(&net, Toggle::Wifi, "");
        add_switch(&net, Toggle::MobileData, tr("Data SIM"));
        add_switch(&net, Toggle::Bluetooth, "");
        add_switch(&net, Toggle::Airplane, tr("Cuts off the mobile network, Wi-Fi and Bluetooth"));
        add_switch(&net, Toggle::Location, "");
        let (hotspot_row, hotspot, hotspot_open) = open_row(tr("Wi-Fi hotspot"), tr("Android only lets you turn it on from the phone (without root)"), "adb-link-symbolic");
        net.add(&hotspot_row);
        let (nfc_row, nfc, nfc_open) = open_row("NFC", tr("Android does not allow changing it over ADB on this phone"), "adb-smartphone-symbolic");
        net.add(&nfc_row);
        content.append(&net);

        // Screen
        let screen = adw::PreferencesGroup::builder().title(tr("Screen")).build();
        let brightness = slider_row(tr("Brightness"), "adb-display-brightness-symbolic");
        brightness.1.set_range(1.0, 255.0);
        screen.add(&brightness.0);
        add_switch(&screen, Toggle::AutoBrightness, tr("Android adjusts the brightness to the light"));
        add_switch(&screen, Toggle::DarkMode, "");
        add_switch(&screen, Toggle::AutoRotate, "");
        let timeout = adw::ComboRow::builder().title(tr("Turn off the screen after")).model(&gtk::StringList::new(&controls::SCREEN_TIMEOUTS.iter().map(|(_, l)| tr(l)).collect::<Vec<_>>())).build();
        timeout.add_prefix(&util::icon("adb-document-open-recent-symbolic"));
        screen.add(&timeout);
        content.append(&screen);

        // Sound
        let sound = adw::PreferencesGroup::builder().title(tr("Sound")).build();
        let media = slider_row(tr("Media volume"), "adb-audio-volume-high-symbolic");
        let ring = slider_row(tr("Ring volume"), "adb-audio-volume-low-symbolic");
        ring.0.set_tooltip_text(Some(tr("Some phones do not allow changing it over ADB: you are then told so and the value goes back to the real one")));
        sound.add(&media.0);
        sound.add(&ring.0);
        let dnd = adw::ComboRow::builder().title(tr("Do Not Disturb")).model(&gtk::StringList::new(&Dnd::ALL.iter().map(|d| d.label()).collect::<Vec<_>>())).build();
        dnd.add_prefix(&util::icon("adb-emblem-important-symbolic"));
        sound.add(&dnd);
        content.append(&sound);

        // Battery and data
        let saving = adw::PreferencesGroup::builder().title(tr("Saving")).build();
        add_switch(&saving, Toggle::BatterySaver, tr("Android does not turn it on while charging"));
        add_switch(&saving, Toggle::DataSaver, tr("Limits background data"));
        content.append(&saving);

        // Flashlight and panels
        let panels = adw::PreferencesGroup::builder()
            .title(tr("Flashlight and panels"))
            .description(tr("Android has no shell command for the flashlight: without root you can only open the quick settings panel and tap it on the phone."))
            .build();
        let flow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(5).column_spacing(8).row_spacing(8).build();
        let panel_buttons: Vec<(gtk::Button, Action, &str)> = vec![
            (util::pill_button("adb-weather-clear-symbolic", tr("Quick settings (flashlight)")), Action::QuickSettings, tr("Quick settings opened on the phone")),
            (util::pill_button("adb-list-symbolic", tr("Notifications")), Action::Notifications, tr("Notifications opened on the phone")),
            (util::pill_button("adb-chevron-down-symbolic", tr("Close panels")), Action::ClosePanels, tr("Panels closed")),
            (util::pill_button("adb-weather-clear-symbolic", tr("Turn screen on")), Action::ScreenOn, tr("Screen turned on")),
            (util::pill_button("adb-system-lock-screen-symbolic", tr("Turn screen off")), Action::ScreenOff, tr("Screen turned off")),
        ];
        for (b, _, _) in &panel_buttons {
            flow.append(b);
        }
        panels.add(&flow);
        content.append(&panels);

        // Power
        let power = adw::PreferencesGroup::builder().title(tr("Power")).description(tr("To turn the phone back on, hold down its power button.")).build();
        let pflow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(5).column_spacing(8).row_spacing(8).build();
        let reboot_button = util::pill_button("adb-system-reboot-symbolic", tr("Restart"));
        let off_button = util::pill_button("adb-system-shutdown-symbolic", tr("Power off"));
        off_button.add_css_class("destructive-action");
        pflow.append(&reboot_button);
        pflow.append(&off_button);
        power.add(&pflow);
        content.append(&power);

        // Open Settings screens
        let screens = adw::PreferencesGroup::builder().title(tr("Open on the phone")).description(tr("Opens that Settings screen on the phone (unlock it to see it)")).build();
        let sflow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(4).column_spacing(8).row_spacing(8).homogeneous(true).build();
        let mut screen_buttons = Vec::new();
        for (label, action) in controls::SETTINGS_SCREENS {
            let b = gtk::Button::with_label(tr(label));
            sflow.append(&b);
            screen_buttons.push((b, action, tr(label)));
        }
        screens.add(&sflow);
        content.append(&screens);

        let root = util::page_scroller(&content, 900);
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            state: RefCell::new(None),
            switches,
            updating: Cell::new(false),
            pending: Cell::new(0),
            reading: Cell::new(false),
            last_read: Cell::new(0.0),
            serial: RefCell::new(None),
            brightness,
            media,
            ring,
            timeout,
            dnd,
            nfc,
            hotspot,
            status,
            refresh,
            debounce: RefCell::new(HashMap::new()),
        });

        {
            let ctx = ctx.clone();
            reboot_button.connect_clicked(move |b| super::device::confirm_reboot(&ctx, b.upcast_ref(), "system"));
        }
        {
            let w = this.me.clone();
            off_button.connect_clicked(move |b| {
                if let Some(p) = w.upgrade() {
                    p.confirm_power_off(b.upcast_ref());
                }
            });
        }
        for (t, row) in &this.switches {
            let (w, t) = (this.me.clone(), *t);
            row.connect_active_notify(move |r| {
                if let Some(p) = w.upgrade() {
                    p.on_switch(t, r);
                }
            });
        }
        for (scale, key) in [(&this.brightness.1, "brightness"), (&this.media.1, "media"), (&this.ring.1, "ring")] {
            let w = this.me.clone();
            scale.connect_value_changed(move |s| {
                if let Some(p) = w.upgrade() {
                    p.on_slider(key, s.value().round() as i64);
                }
            });
        }
        {
            let w = this.me.clone();
            this.timeout.connect_selected_notify(move |c| {
                let Some(p) = w.upgrade() else { return };
                if p.updating.get() {
                    return;
                }
                if let Some((ms, label)) = controls::SCREEN_TIMEOUTS.get(c.selected() as usize) {
                    p.apply_level(Level::ScreenTimeout(*ms), &t!("The screen will turn off after {}", tr(label)));
                }
            });
        }
        {
            let w = this.me.clone();
            this.dnd.connect_selected_notify(move |c| {
                let Some(p) = w.upgrade() else { return };
                if p.updating.get() {
                    return;
                }
                if let Some(d) = Dnd::ALL.get(c.selected() as usize) {
                    p.apply_level(Level::Dnd(*d), &t!("Do Not Disturb: {}", d.label().to_lowercase()));
                }
            });
        }
        {
            let w = this.me.clone();
            this.refresh.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    p.read_state();
                }
            });
        }
        for (b, action, msg) in panel_buttons {
            let w = this.me.clone();
            b.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    p.run_action(action, msg);
                }
            });
        }
        for (b, action, label) in screen_buttons {
            let w = this.me.clone();
            b.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    p.run_action(Action::OpenSettings(action), &t!("“{}” opened on the phone", label));
                }
            });
        }
        for (b, action, label) in [(hotspot_open, "android.settings.TETHER_SETTINGS", tr("Wi-Fi hotspot")), (nfc_open, "android.settings.NFC_SETTINGS", "NFC")] {
            let w = this.me.clone();
            b.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    p.run_action(Action::OpenSettings(action), &t!("{} settings opened on the phone", label));
                }
            });
        }
        this.render();
        this.read_state();
        this
    }

    // -- reading ------------------------------------------------------------------

    fn read_state(&self) {
        if self.reading.get() {
            return;
        }
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        self.reading.set(true);
        crate::anim::set_busy(&self.refresh, true);
        let w = self.me.clone();
        util::run_bg(
            move || (serial.clone(), adb.control_state(&serial)),
            move |(serial, r)| {
                let Some(p) = w.upgrade() else { return };
                p.reading.set(false);
                crate::anim::set_busy(&p.refresh, false);
                p.last_read.set(now_secs());
                // A state from another phone (changed in the meantime) is not
                // valid.
                if p.ctx.ready_serial().as_deref() != Some(serial.as_str()) {
                    return;
                }
                match r {
                    Ok(s) => {
                        *p.state.borrow_mut() = Some(s);
                        p.render();
                    }
                    Err(e) => p.status.set_text(&t!("Could not read the state: {}", e)),
                }
            },
        );
    }

    /// Paints the state that was read without triggering changes.
    fn render(&self) {
        self.updating.set(true);
        let state = self.state.borrow();
        let sdk = state.as_ref().and_then(|s| s.sdk);
        for (t, row) in &self.switches {
            let probe = state.as_ref().map(|s| t.state(s).clone()).unwrap_or(Probe::Failed("leyendo…".into()));
            let supported = t.command(true, sdk).is_some();
            match (&probe, supported) {
                (Probe::Ok(on), true) => {
                    row.set_active(*on);
                    row.set_sensitive(true);
                }
                (Probe::Ok(on), false) => {
                    row.set_active(*on);
                    row.set_sensitive(false);
                    row.set_tooltip_text(Some(tr("Cannot be changed without root on this Android version")));
                }
                (other, _) => {
                    row.set_sensitive(false);
                    row.set_tooltip_text(other.status_text().as_deref());
                }
            }
        }
        let set_slider = |(row, scale): &(adw::ActionRow, gtk::Scale), value: &Probe<i64>, range: Option<(i64, i64)>| match value {
            Probe::Ok(v) => {
                if let Some((lo, hi)) = range {
                    scale.set_range(lo as f64, hi.max(lo + 1) as f64);
                }
                scale.set_value(*v as f64);
                row.set_sensitive(true);
                row.set_subtitle("");
            }
            other => {
                row.set_sensitive(false);
                row.set_subtitle(&other.status_text().unwrap_or_default());
            }
        };
        let vol = |v: &Probe<Volume>| (v.clone().map(|v| v.value), v.ok().map(|v| (v.min, v.max)));
        if let Some(s) = state.as_ref() {
            set_slider(&self.brightness, &s.brightness, None);
            if s.auto_brightness.ok() == Some(&true) {
                self.brightness.0.set_subtitle(tr("With automatic brightness, Android keeps adjusting it"));
            }
            let (v, r) = vol(&s.media_volume);
            set_slider(&self.media, &v, r);
            let (v, r) = vol(&s.ring_volume);
            set_slider(&self.ring, &v, r);
            match &s.screen_timeout_ms {
                Probe::Ok(ms) => {
                    // The closest value in the list.
                    let idx = controls::SCREEN_TIMEOUTS.iter().enumerate().min_by_key(|(_, (v, _))| (v - ms).abs()).map(|(i, _)| i).unwrap_or(0);
                    self.timeout.set_selected(idx as u32);
                    self.timeout.set_sensitive(true);
                }
                _ => self.timeout.set_sensitive(false),
            }
            match &s.dnd {
                Probe::Ok(d) => {
                    self.dnd.set_selected(Dnd::ALL.iter().position(|x| x == d).unwrap_or(0) as u32);
                    self.dnd.set_sensitive(true);
                }
                _ => self.dnd.set_sensitive(false),
            }
            let onoff = |p: &Probe<bool>, on: &str| p.display(|v| if *v { on.to_string() } else { tr("Off").to_string() });
            self.nfc.set_text(&onoff(&s.nfc, tr("On")));
            self.hotspot.set_text(&onoff(&s.hotspot, tr("Sharing")));
            let mut notes = vec![t!("Read at {}", adbscan_core::format::clock(s.read_at))];
            if s.charging == Some(true) {
                notes.push(tr("charging").into());
            }
            self.status.set_text(&notes.join(" · "));
        } else {
            for r in [&self.brightness.0, &self.media.0, &self.ring.0] {
                r.set_sensitive(false);
            }
            self.timeout.set_sensitive(false);
            self.dnd.set_sensitive(false);
            self.status.set_text(tr("Reading the phone's state…"));
        }
        drop(state);
        self.updating.set(false);
    }

    // -- changes --------------------------------------------------------------------

    /// Power off the phone, with confirmation.
    fn confirm_power_off(&self, parent: &gtk::Widget) {
        let Some(device) = self.ctx.selected_device() else {
            self.ctx.toast(tr("No device is selected"));
            return;
        };
        let name = self.ctx.device_label(&device);
        let mut body = t!("{} will shut down cleanly, as with its power button: apps save their state.\n\nADB Scan will lose the connection. To turn it back on, hold down the phone's power button.", name);
        if device.is_wireless() {
            body.push_str(tr("\n\nIt is connected over Wi-Fi: when you turn it on you will have to enable wireless ADB again or connect it over USB."));
        }
        let ctx = self.ctx.clone();
        util::confirm(parent, &t!("Power off {}?", name), &body, tr("Power off"), true, move || {
            ctx.device_op("power_off", &name, tr("Powering off the phone…"), |adb, serial| adb.power_off(&serial).map(|_| "svc power shutdown".to_string()), |_| {});
        });
    }

    fn on_switch(&self, t: Toggle, row: &adw::SwitchRow) {
        if self.updating.get() {
            return;
        }
        let on = row.is_active();
        // Turning off Wi-Fi or turning on airplane mode cuts wireless ADB.
        let wireless = self.ctx.selected_device().is_some_and(|d| d.is_wireless());
        let cuts_adb = wireless && ((t == Toggle::Wifi && !on) || (t == Toggle::Airplane && on));
        if cuts_adb {
            // Until confirmed, the switch goes back to where it was.
            self.updating.set(true);
            row.set_active(!on);
            self.updating.set(false);
            let w = self.me.clone();
            util::confirm(
                row,
                &t!("{} {}?", if on { tr("Turn on") } else { tr("Turn off") }, t.label().to_lowercase()),
                tr("The phone is connected over Wi-Fi: ADB Scan will lose the connection and will not be able to undo the change. You will have to turn it back on from the phone or connect it over USB."),
                if on { tr("Turn on") } else { tr("Turn off") },
                true,
                move || {
                    if let Some(p) = w.upgrade() {
                        p.apply_toggle(t, on);
                    }
                },
            );
            return;
        }
        self.apply_toggle(t, on);
    }

    fn apply_toggle(&self, t: Toggle, on: bool) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let sdk = self.state.borrow().as_ref().and_then(|s| s.sdk);
        if let Some((_, row)) = self.switches.iter().find(|(x, _)| *x == t) {
            row.set_sensitive(false);
        }
        self.pending.set(self.pending.get() + 1);
        let w = self.me.clone();
        let target = format!("{} {}", t.label(), if on { "on" } else { "off" });
        util::run_bg(
            move || adb.set_toggle(&serial, t, on, sdk),
            move |r| {
                let Some(p) = w.upgrade() else { return };
                p.pending.set(p.pending.get().saturating_sub(1));
                match r {
                    Ok((applied, state)) => {
                        let msg = if applied { format!("{} {}", t.label(), if on { tr("on") } else { tr("off") }) } else { controls::not_applied_hint(t, on, &state) };
                        p.ctx.audit("control", Some(&target), applied, Some(&msg));
                        p.ctx.toast(&msg);
                        *p.state.borrow_mut() = Some(state);
                        p.last_read.set(now_secs());
                    }
                    Err(e) => {
                        p.ctx.audit("control", Some(&target), false, Some(&e.to_string()));
                        p.ctx.toast(&format!("{}: {e}", t.label()));
                    }
                }
                // Paints the real state (if it was not applied, the switch goes back).
                p.render();
            },
        );
    }

    /// Sliders are applied once they stop moving.
    fn on_slider(&self, key: &'static str, value: i64) {
        if self.updating.get() {
            return;
        }
        if let Some(id) = self.debounce.borrow_mut().remove(key) {
            id.remove();
        }
        let w = self.me.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(350), move || {
            let Some(p) = w.upgrade() else { return };
            p.debounce.borrow_mut().remove(key);
            let (level, msg) = match key {
                "brightness" => (Level::Brightness(value), t!("Brightness: {}", value)),
                "media" => (Level::MediaVolume(value), t!("Media volume: {}", value)),
                _ => (Level::RingVolume(value), t!("Ring volume: {}", value)),
            };
            p.apply_level(level, &msg);
        });
        self.debounce.borrow_mut().insert(key, id);
    }

    fn apply_level(&self, level: Level, msg: &str) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let Some(current) = self.state.borrow().clone() else {
            return;
        };
        // Locked until finished: volume via keys takes a few seconds and two
        // changes at once would step on each other. `render` enables it again.
        let busy: gtk::Widget = match level {
            Level::Brightness(_) => self.brightness.0.clone().upcast(),
            Level::MediaVolume(_) => self.media.0.clone().upcast(),
            Level::RingVolume(_) => self.ring.0.clone().upcast(),
            Level::ScreenTimeout(_) => self.timeout.clone().upcast(),
            Level::Dnd(_) => self.dnd.clone().upcast(),
        };
        busy.set_sensitive(false);
        self.pending.set(self.pending.get() + 1);
        let (w, msg) = (self.me.clone(), msg.to_string());
        util::run_bg(
            move || adb.set_level(&serial, level, &current),
            move |r| {
                let Some(p) = w.upgrade() else { return };
                p.pending.set(p.pending.get().saturating_sub(1));
                match r {
                    Ok(state) => {
                        p.ctx.audit("control", Some(&msg), true, None);
                        *p.state.borrow_mut() = Some(state);
                        p.last_read.set(now_secs());
                    }
                    Err(e) => {
                        p.ctx.audit("control", Some(&msg), false, Some(&e.to_string()));
                        p.ctx.toast(&t!("Could not change it: {}", e));
                    }
                }
                // No toast on every slider movement: the value is already
                // visible.
                p.render();
            },
        );
    }

    fn run_action(&self, action: Action, success: &str) {
        let target = action.command();
        self.ctx.device_op("control", &target, success, move |adb, serial| adb.control_action(&serial, action).map(|r| r.summary()), |_| {});
    }
}

impl Page for ControlsPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn on_event(&self, ev: &Event) {
        match ev {
            Event::Devices => {
                let serial = self.ctx.ready_serial();
                if *self.serial.borrow() != serial {
                    *self.serial.borrow_mut() = serial;
                    *self.state.borrow_mut() = None;
                    self.render();
                    self.read_state();
                }
            }
            // Telemetry serves as a clock: it only arrives while the page is in
            // view.
            Event::Snapshot if self.pending.get() == 0 && self.debounce.borrow().is_empty() && now_secs() - self.last_read.get() > REFRESH_SECS => {
                self.read_state();
            }
            _ => {}
        }
    }

    fn shown(&self) {
        *self.serial.borrow_mut() = self.ctx.ready_serial();
        if now_secs() - self.last_read.get() > 2.0 {
            self.read_state();
        }
    }
}
