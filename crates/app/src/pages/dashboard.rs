//! Overview: the device's status at a glance, quick actions and the state of
//! the connection (its details are in Connection).

use adbscan_core::{i18n::{tr, trc}, t};
use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

use adbscan_core::format;
use adbscan_core::models::{CapacitySource, ComponentHealth, DeviceState, StorageWear};
use adbscan_core::usb::UsbMode;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::widgets::{self, Chart, Gauge, Level, StatCard};
use crate::window::Page;

pub struct Dashboard {
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    hero_title: gtk::Label,
    hero_sub: gtk::Label,
    chips: gtk::Box,
    gauge: Gauge,
    cpu: StatCard,
    cpu_chart: Chart,
    ram: StatCard,
    ram_bar: gtk::LevelBar,
    ram_chart: Chart,
    storage: StatCard,
    storage_bar: gtk::LevelBar,
    temp: StatCard,
    display: StatCard,
    net: StatCard,
    uptime: StatCard,
    usb_card: StatCard,
    batt_health: StatCard,
    batt_health_bar: gtk::LevelBar,
    storage_health: StatCard,
    storage_health_bar: gtk::LevelBar,
    diag: gtk::ListBox,
    /// Optional invitation to the advanced analysis (without debugging).
    invite: gtk::Widget,
    animated: Cell<bool>,
    last_diag: std::cell::RefCell<String>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    Dashboard::new(ctx)
}

fn chip(icon: &str, text: &str, kind: &str) -> gtk::Box {
    let b = util::hbox(6);
    b.add_css_class("pill");
    if !kind.is_empty() {
        b.add_css_class(kind);
    }
    let i = util::icon(icon);
    i.set_pixel_size(14);
    b.append(&i);
    b.append(&gtk::Label::new(Some(text)));
    b
}

impl Dashboard {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);

        // -- hero --------------------------------------------------------
        let hero = util::hbox(24);
        hero.add_css_class("hero");
        let left = util::vbox(8);
        left.set_hexpand(true);
        left.set_valign(gtk::Align::Center);
        let hero_title = util::label(tr("No device"), &["hero-title"]);
        hero_title.set_wrap(true);
        let hero_sub = util::label(tr("Connect an Android phone over USB"), &["hero-sub"]);
        hero_sub.set_wrap(true);
        let chips = util::hbox(6);
        chips.set_margin_top(6);
        left.append(&hero_title);
        left.append(&hero_sub);
        left.append(&chips);
        let actions = util::hbox(8);
        actions.set_margin_top(10);
        for (icon, text, action) in [
            ("adb-camera-photo-symbolic", trc("short", "Screenshot"), "win.screenshot"),
            ("adb-video-display-symbolic", tr("Mirror"), "win.mirror"),
            ("adb-folder-symbolic", tr("Files"), "win.go::files"),
            ("adb-package-symbolic", "Apps", "win.go::apps"),
            // Without ADB: what the phone shares without debugging.
            ("adb-search-symbolic", tr("Basic analysis"), "win.go::basic"),
        ] {
            let b = util::pill_button(icon, text);
            b.add_css_class("pill");
            if let Some((name, target)) = action.split_once("::") {
                b.set_action_name(Some(name));
                b.set_action_target_value(Some(&target.to_variant()));
            } else {
                b.set_action_name(Some(action));
            }
            actions.append(&b);
        }
        left.append(&actions);
        let gauge = Gauge::new(150, 13.0);
        hero.append(&left);
        hero.append(&gauge.root);
        content.append(&hero);
        let invite = crate::pages::basic::invite_card(ctx);
        content.append(&invite);

        // -- cards -----------------------------------------------------
        let grid = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).homogeneous(true).min_children_per_line(1).max_children_per_line(4).column_spacing(12).row_spacing(12).build();
        let cpu = StatCard::new("adb-cpu-symbolic", tr("Processor"));
        let cpu_chart = Chart::new(46, 60, 0.0, 100.0, " %");
        cpu_chart.add_series("", None, true);
        cpu.extra.append(&cpu_chart.area);
        let ram = StatCard::new("adb-memory-stick-symbolic", tr("RAM"));
        let ram_bar = widgets::usage_bar();
        let ram_chart = Chart::new(30, 60, 0.0, 100.0, " %");
        ram_chart.add_series("", None, true);
        ram.extra.append(&ram_bar);
        ram.extra.append(&ram_chart.area);
        let storage = StatCard::new("adb-hard-drive-symbolic", tr("Storage"));
        let storage_bar = widgets::usage_bar();
        storage.extra.append(&storage_bar);
        let temp = StatCard::new("adb-thermometer-symbolic", tr("Temperature"));
        let display = StatCard::new("adb-monitor-symbolic", tr("Screen"));
        let net = StatCard::new("adb-wifi-symbolic", tr("Network"));
        let uptime = StatCard::new("adb-power-profile-performance-symbolic", tr("Uptime"));
        let usb_card = StatCard::new("adb-link-symbolic", tr("USB connection"));
        for c in [&cpu, &ram, &storage, &temp, &display, &net, &uptime, &usb_card] {
            grid.append(&c.root);
        }
        content.append(&grid);

        // -- health ----------------------------------------------------------
        content.append(&util::label(tr("Component health"), &["section-title"]));
        let health_grid =
            gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).homogeneous(true).min_children_per_line(1).max_children_per_line(2).column_spacing(12).row_spacing(12).build();
        let batt_health = StatCard::new("adb-battery-symbolic", tr("Battery"));
        let batt_health_bar = widgets::usage_bar();
        batt_health.extra.append(&batt_health_bar);
        let storage_health = StatCard::new("adb-hard-drive-symbolic", tr("Storage"));
        let storage_health_bar = widgets::usage_bar();
        storage_health.extra.append(&storage_health_bar);
        health_grid.append(&batt_health.root);
        health_grid.append(&storage_health.root);
        content.append(&health_grid);

        // -- connection: one row with the worst check; details in Connection --
        content.append(&util::label(tr("Connection"), &["section-title"]));
        let diag = gtk::ListBox::new();
        diag.add_css_class("boxed-list");
        diag.set_selection_mode(gtk::SelectionMode::None);
        content.append(&diag);

        let root = util::page_scroller(&content, 1200);
        let this = Rc::new(Self {
            ctx: ctx.clone(),
            root,
            hero_title,
            hero_sub,
            chips,
            gauge,
            cpu,
            cpu_chart,
            ram,
            ram_bar,
            ram_chart,
            storage,
            storage_bar,
            temp,
            display,
            net,
            uptime,
            usb_card,
            batt_health,
            batt_health_bar,
            storage_health,
            storage_health_bar,
            diag,
            invite,
            animated: Cell::new(false),
            last_diag: Default::default(),
        });
        // Preloads charts with the telemetry already received.
        for m in ctx.metrics() {
            this.cpu_chart.push(0, m.cpu_load);
            this.ram_chart.push(0, m.ram_pct);
        }
        this.cpu_chart.set_interval(ctx.live_interval());
        this.ram_chart.set_interval(ctx.live_interval());
        this.refresh_device();
        this.refresh_snapshot();
        this
    }

    fn refresh_device(&self) {
        let ctx = &self.ctx;
        let dev = ctx.selected_device();
        let usb = ctx.usb_devices();
        match &dev {
            Some(d) => {
                self.hero_title.set_text(&ctx.device_label(d));
                if ctx.snapshot().is_none() {
                    self.hero_sub.set_text(&format!("{} · {}", ctx.shown_value(&d.serial), d.state_label()));
                }
            }
            None => {
                let phone = usb.iter().find(|u| matches!(u.mode, UsbMode::Adb | UsbMode::Other));
                self.hero_title.set_text(&match phone {
                    Some(u) => ctx.shown(&u.display_name()),
                    None if usb.is_empty() => tr("No device").into(),
                    None => tr("Phone detected over USB").into(),
                });
                self.hero_sub.set_text(match phone {
                    Some(u) if u.has_adb_interface() => tr("Accept the “Allow USB debugging?” prompt on the phone"),
                    Some(_) => tr("Basic analysis available · USB debugging is optional"),
                    None if usb.is_empty() => tr("Connect an Android phone over USB. Without debugging you get the basic analysis"),
                    None => tr("The phone is not in normal Android mode"),
                });
                self.gauge.set(None, "—", tr("Battery"), Level::Normal);
            }
        }
        self.invite.set_visible(ctx.wants_advanced_invite());
        self.refresh_chips();
        self.refresh_diag();
    }

    fn refresh_chips(&self) {
        while let Some(c) = self.chips.first_child() {
            self.chips.remove(&c);
        }
        let ctx = &self.ctx;
        if let Some(d) = ctx.selected_device() {
            let kind = match d.state {
                DeviceState::Device => "ok",
                DeviceState::Unauthorized => "warn",
                _ => "bad",
            };
            self.chips.append(&chip("adb-smartphone-symbolic", &d.state_label(), kind));
            if d.is_wireless() {
                self.chips.append(&chip("adb-wifi-symbolic", "Wi-Fi", "accent"));
            }
        }
        if let Some(u) = ctx.usb_devices().first() {
            if u.speed_mbps.is_some() {
                self.chips.append(&chip("adb-link-symbolic", &u.speed_label(), ""));
            }
        }
        if let Some(s) = ctx.snapshot() {
            if s.battery.is_charging() {
                let src = s.battery.power_source();
                let text = if src.is_empty() { tr("Charging").to_string() } else { t!("Charging · {}", src) };
                self.chips.append(&chip("adb-battery-charging-symbolic", &text, "ok"));
            }
            if s.thermal_status.is_some_and(|t| t >= 2) {
                self.chips.append(&chip("adb-thermometer-symbolic", &t!("Thermal: {}", s.thermal_status_label()), "warn"));
            }
        }
    }

    fn refresh_snapshot(&self) {
        let Some(s) = self.ctx.snapshot() else {
            for c in [&self.cpu, &self.ram, &self.storage, &self.temp, &self.display, &self.net, &self.uptime, &self.usb_card, &self.batt_health, &self.storage_health] {
                c.set("—", "");
            }
            widgets::set_health(&self.batt_health_bar, None, None);
            widgets::set_health(&self.storage_health_bar, None, None);
            return;
        };
        if !self.animated.replace(true) {
            let cards: Vec<gtk::Widget> = [&self.cpu, &self.ram, &self.storage, &self.temp, &self.display, &self.net, &self.uptime, &self.usb_card].iter().map(|c| c.root.clone().upcast()).collect();
            widgets::reveal_staggered(&cards);
        }
        let d = self.ctx.selected_device();
        if let Some(d) = &d {
            self.hero_title.set_text(&self.ctx.device_label(d));
        }
        let mut sub = vec![s.manufacturer(), s.android_version()];
        if s.security_patch() != "—" {
            sub.push(t!("Patch {}", s.security_patch()));
        }
        self.hero_sub.set_text(&sub.join(" · "));

        let pct = s.battery.percentage();
        let level = match pct {
            Some(p) if p <= 15.0 => Level::Critical,
            Some(p) if p <= 30.0 => Level::High,
            _ => Level::Normal,
        };
        let caption = if s.battery.is_charging() { tr("Charging") } else { s.battery.status() };
        self.gauge.set(pct, &format::percent(pct), caption, level);

        let load = s.cpu.load_percent;
        self.cpu.set(&format::percent(load), &t!("{} cores · {}", s.cpu.core_count.unwrap_or(0), format::frequency_khz(s.cpu.cur_freq_khz())));
        self.cpu_chart.push(0, load);

        let ram_pct = s.memory.percent();
        self.ram.set(&format::human_kib(s.memory.used_kib()), &t!("of {} · {}", format::human_kib(s.memory.total_kib), format::percent(ram_pct)));
        widgets::set_usage(&self.ram_bar, ram_pct, 80.0, 92.0);
        self.ram_chart.push(0, ram_pct);

        let st = &s.data_storage;
        self.storage.set(&format::human_kib(st.used_kib), &t!("of {} · {} free", format::human_kib(st.total_kib), format::human_kib(st.available_kib)));
        widgets::set_usage(&self.storage_bar, st.percent(), 85.0, 95.0);

        let bt = s.battery_temperature();
        let ct = s.cpu_temperature();
        self.temp.set(&format::temperature(bt.or(ct)), &format!("CPU {} · {}", format::temperature(ct), s.thermal_status_label()));

        let hz = s.display.refresh_hz.map(|h| format!(" · {h:.0} Hz")).unwrap_or_default();
        self.display.set(&s.display.resolution(), &format!("{} dpi {}{hz}", s.display.density_dpi.map(|d| d.to_string()).unwrap_or("—".into()), s.display.density_bucket));

        match s.network.wifi_ipv4() {
            Some(ip) => {
                // Live network and signal if telemetry includes Wi-Fi.
                let detail = match s.wifi.as_ref().filter(|w| !w.ssid.is_empty()) {
                    Some(w) => {
                        let mut d = t!("Wi-Fi “{}”", self.ctx.shown_value(&w.ssid));
                        if let Some(r) = w.rssi {
                            d.push_str(&format!(" · {r} dBm"));
                        }
                        if !w.link_speed.is_empty() {
                            d.push_str(&format!(" · {}", w.link_speed));
                        }
                        d
                    }
                    None => tr("Wi-Fi connected").to_string(),
                };
                self.net.set(&self.ctx.shown(&ip), &detail)
            }
            None => {
                let up = s.network.interfaces.iter().filter(|i| i.state == "UP" && i.name != "lo" && !i.addresses.is_empty()).count();
                self.net.set(if up > 0 { tr("Connected") } else { tr("No network") }, &t!("{} active interfaces", up));
            }
        }
        self.uptime.set(&s.uptime_label(), &s.cpu.loadavg.map(|(a, b, c)| t!("Load {} {} {}", format!("{:.2}", a), format!("{:.2}", b), format!("{:.2}", c))).unwrap_or_default());
        let host = self.ctx.usb_devices().first().map(|u| u.speed_label()).unwrap_or_default();
        self.usb_card.set(&s.usb.label(), &host);
        self.refresh_health(&s.health);
        self.refresh_chips();
    }

    fn refresh_health(&self, h: &ComponentHealth) {
        if !h.probed {
            self.batt_health.set("—", "Leyendo…");
            self.storage_health.set("—", "Leyendo…");
            return;
        }
        let pct = h.battery_percent();
        let mut sub = Vec::new();
        if let Some(level) = h.battery_level() {
            sub.push(level.label().to_string());
        }
        if let Some(c) = h.battery_cycles {
            sub.push(t!("{} cycles", c));
        }
        if let (Some(full), Some(design), Some(_)) = (h.battery_full_mah, h.battery_design_mah, pct) {
            sub.push(t!("{} of {} mAh", format::number(Some(full as f64), 0), format::number(Some(design as f64), 0)));
        }
        if sub.is_empty() {
            sub.push(tr("The phone does not expose the capacity").into());
        }
        self.batt_health.set(&format::percent(pct), &sub.join(" · "));
        self.batt_health.root.set_tooltip_text(match h.battery_source {
            CapacitySource::Sysfs => Some(tr("Real capacity measured by the charging chip compared to the design capacity")),
            CapacitySource::BatteryStats => Some(tr("Capacity learned by Android (batterystats) compared to the design capacity")),
            CapacitySource::None => None,
        });
        widgets::set_health(&self.batt_health_bar, pct, h.battery_level());

        let range = h.storage_range();
        let mut detail = h.storage_detail();
        if let Some(level) = h.storage_level() {
            detail = format!("{} · {detail}", level.label());
        }
        self.storage_health.set(&h.storage_label(), &detail);
        self.storage_health.root.set_tooltip_text(Some(match &h.storage {
            StorageWear::Read { .. } => tr("Remaining lifetime according to the chip itself (JEDEC), in 10 % steps"),
            StorageWear::Denied { .. } => tr("The chip reports its wear, but Android only lets you read it with root"),
            StorageWear::Unknown => tr("The storage health descriptor was not found"),
        }));
        // The bar fills up to the top end of the step.
        widgets::set_health(&self.storage_health_bar, range.map(|(_, high)| high as f64), h.storage_level());
    }

    fn refresh_diag(&self) {
        let ctx = &self.ctx;
        let rows = super::connection::diagnosis_rows(ctx);
        let key = format!("{}{rows:?}", ctx.private_mode());
        if *self.last_diag.borrow() == key {
            return;
        }
        *self.last_diag.borrow_mut() = key;
        while let Some(c) = self.diag.first_child() {
            self.diag.remove(&c);
        }
        // The most serious check (2 bad, 1 warning, 3 pending, 0 good) sums it up.
        let rank = |s: u8| match s {
            2 => 3,
            1 => 2,
            3 => 1,
            _ => 0,
        };
        let worst = rows.iter().max_by_key(|r| rank(r.0));
        let (state, title, detail) = match worst {
            Some((0, _, _)) | None => (0, tr("Everything in order").to_string(), rows.iter().find(|r| r.1 == tr("Phone connected over USB")).map(|r| r.2.clone()).unwrap_or_default()),
            Some((s, t, d)) => (*s, t.clone(), d.clone()),
        };
        let row = adw::ActionRow::builder()
            .title(glib_escape(&ctx.shown(&title)))
            .subtitle(glib_escape(&ctx.shown(&detail)))
            .activatable(true)
            .tooltip_text(tr("Cable, ports, stability, wireless ADB and the full diagnosis"))
            .build();
        row.set_action_name(Some("win.go"));
        row.set_action_target_value(Some(&"connection".to_variant()));
        let (icon, class) = match state {
            0 => ("adb-emblem-ok-symbolic", "severity-ok"),
            1 => ("adb-dialog-warning-symbolic", "severity-medium"),
            2 => ("adb-circle-x-symbolic", "severity-high"),
            _ => ("adb-info-symbolic", "dim"),
        };
        let img = util::icon(icon);
        img.add_css_class(class);
        row.add_prefix(&img);
        row.add_suffix(&util::label(tr("Details"), &["dim"]));
        row.add_suffix(&util::icon("go-next-symbolic"));
        self.diag.append(&row);
    }
}

fn glib_escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}

impl Page for Dashboard {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn on_event(&self, ev: &Event) {
        match ev {
            Event::Devices => {
                if self.ctx.snapshot().is_none() {
                    self.cpu_chart.clear();
                    self.ram_chart.clear();
                    self.animated.set(false);
                }
                self.refresh_device();
                self.refresh_snapshot();
            }
            Event::SettingsChanged => self.invite.set_visible(self.ctx.wants_advanced_invite()),
            Event::Privacy => {
                self.last_diag.borrow_mut().clear();
                self.refresh_device();
                self.refresh_snapshot();
            }
            Event::Snapshot => {
                self.cpu_chart.set_interval(self.ctx.live_interval());
                self.ram_chart.set_interval(self.ctx.live_interval());
                self.refresh_snapshot();
                self.refresh_diag();
            }
            _ => {}
        }
    }

    fn shown(&self) {
        // While hidden it receives no telemetry: it catches up when shown.
        let m = self.ctx.metrics();
        self.cpu_chart.refill(0, m.iter().map(|m| m.cpu_load));
        self.ram_chart.refill(0, m.iter().map(|m| m.ram_pct));
        self.refresh_snapshot();
        self.refresh_diag();
    }
}
