//! Quick panel: compact overview from the tray icon.

use adbscan_core::{i18n::{tr, trc}, t};
use adw::prelude::*;
use std::rc::Rc;

use adbscan_core::format;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::widgets::{AnimatedBar, Gauge, Level, StatCard, StatusPill};

pub struct QuickPanel {
    pub window: adw::Window,
    ctx: Rc<Ctx>,
    name: gtk::Label,
    sub: gtk::Label,
    pill: StatusPill,
    speed: gtk::Label,
    gauge: Gauge,
    cpu: StatCard,
    cpu_bar: AnimatedBar,
    ram: StatCard,
    ram_bar: AnimatedBar,
    temp: StatCard,
    storage: StatCard,
    storage_bar: AnimatedBar,
    actions: gtk::Box,
}

/// Compact panel card, with an optional bar.
fn tile(icon: &str, title: &str, bar: bool) -> (StatCard, Option<AnimatedBar>) {
    let card = StatCard::new(icon, title);
    card.root.add_css_class("compact");
    let bar = bar.then(|| {
        let b = AnimatedBar::new();
        card.extra.append(&b.bar);
        b
    });
    (card, bar)
}

impl QuickPanel {
    pub fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let window = adw::Window::builder().application(&ctx.app).title(tr("ADB Scan · Quick panel")).default_width(400).resizable(false).icon_name("adb-scan").build();
        window.add_css_class("quick-panel");
        let content = util::vbox(12);
        content.set_margin_top(4);
        content.set_margin_bottom(16);
        content.set_margin_start(16);
        content.set_margin_end(16);

        // Header: battery and phone, like the Overview.
        let head = util::hbox(16);
        head.add_css_class("hero");
        head.add_css_class("panel-hero");
        let gauge = Gauge::new(88, 8.0);
        let texts = util::vbox(4);
        texts.set_valign(gtk::Align::Center);
        texts.set_hexpand(true);
        let name = util::label(tr("No device"), &["title-3"]);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let sub = util::label("", &["dim", "caption"]);
        sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let pills = util::hbox(6);
        pills.set_margin_top(4);
        let pill = StatusPill::new();
        let speed = util::label("", &["pill"]);
        speed.set_ellipsize(gtk::pango::EllipsizeMode::End);
        speed.set_visible(false);
        pills.append(&pill.root);
        pills.append(&speed);
        texts.append(&name);
        texts.append(&sub);
        texts.append(&pills);
        head.append(&gauge.root);
        head.append(&texts);
        content.append(&head);

        // Four cards in two columns.
        let (cpu, cpu_bar) = tile("adb-cpu-symbolic", "CPU", true);
        let (ram, ram_bar) = tile("adb-memory-stick-symbolic", "RAM", true);
        let (temp, _) = tile("adb-thermometer-symbolic", tr("Temperature"), false);
        let (storage, storage_bar) = tile("adb-hard-drive-symbolic", tr("Storage"), true);
        let grid = gtk::Grid::builder().column_spacing(10).row_spacing(10).column_homogeneous(true).build();
        grid.attach(&cpu.root, 0, 0, 1, 1);
        grid.attach(&ram.root, 1, 0, 1, 1);
        grid.attach(&temp.root, 0, 1, 1, 1);
        grid.attach(&storage.root, 1, 1, 1, 1);
        content.append(&grid);

        // Actions: icon and name.
        let actions = util::hbox(8);
        actions.set_homogeneous(true);
        for (icon, label, action) in [
            ("adb-camera-photo-symbolic", trc("short", "Screenshot"), "win.screenshot"),
            ("adb-video-display-symbolic", tr("Mirror"), "win.mirror"),
            ("adb-folder-symbolic", tr("Files"), "files"),
            ("adb-square-terminal-symbolic", "Terminal", "terminal"),
        ] {
            let inner = util::vbox(4);
            let img = util::icon(icon);
            img.set_pixel_size(20);
            inner.append(&img);
            let l = gtk::Label::new(Some(label));
            inner.append(&l);
            let b = gtk::Button::builder().child(&inner).tooltip_text(label).build();
            b.add_css_class("panel-action");
            let w = window.clone();
            let ctx2 = ctx.clone();
            b.connect_clicked(move |b| match action {
                "win.screenshot" => crate::pages::mirror::quick_screenshot(&ctx2, b.upcast_ref()),
                "win.mirror" => crate::pages::mirror::quick_mirror(&ctx2, b.upcast_ref()),
                page => {
                    crate::app::open_page(page);
                    w.set_visible(false);
                }
            });
            actions.append(&b);
        }
        content.append(&actions);
        let open = gtk::Button::with_label(tr("Open ADB Scan"));
        open.add_css_class("suggested-action");
        open.add_css_class("pill");
        {
            let w = window.clone();
            open.connect_clicked(move |_| {
                crate::app::open_page("dashboard");
                w.set_visible(false);
            });
        }
        content.append(&open);

        let tv = adw::ToolbarView::new();
        let hb = adw::HeaderBar::builder().title_widget(&adw::WindowTitle::new("ADB Scan", "")).build();
        let settings = util::icon_button("adb-settings-symbolic", tr("Preferences"));
        settings.set_action_name(Some("app.settings"));
        hb.pack_start(&settings);
        tv.add_top_bar(&hb);
        tv.set_content(Some(&content));
        window.set_content(Some(&tv));
        window.set_hide_on_close(true);

        let (cpu_bar, ram_bar, storage_bar) = (cpu_bar.unwrap(), ram_bar.unwrap(), storage_bar.unwrap());
        let this = Rc::new(Self { window, ctx: ctx.clone(), name, sub, pill, speed, gauge, cpu, cpu_bar, ram, ram_bar, temp, storage, storage_bar, actions });
        {
            let w = Rc::downgrade(&this);
            ctx.subscribe(move |ev| {
                if let (Some(t), Event::Devices | Event::Snapshot | Event::SettingsChanged | Event::Privacy) = (w.upgrade(), ev) {
                    if t.window.is_visible() {
                        t.refresh();
                    }
                }
            });
        }
        {
            let c = ctx.clone();
            this.window.connect_map(move |_| c.surface_visible(true));
            let c = ctx.clone();
            this.window.connect_unmap(move |_| c.surface_visible(false));
            let w = Rc::downgrade(&this);
            this.window.connect_show(move |_| {
                if let Some(t) = w.upgrade() {
                    t.refresh();
                }
            });
        }
        // Escape closes the panel.
        {
            let keys = gtk::EventControllerKey::new();
            let w = this.window.clone();
            keys.connect_key_pressed(move |_, k, _, _| {
                if k == gtk::gdk::Key::Escape {
                    w.set_visible(false);
                    return gtk::glib::Propagation::Stop;
                }
                gtk::glib::Propagation::Proceed
            });
            this.window.add_controller(keys);
        }
        this.refresh();
        this
    }

    fn refresh(&self) {
        let dev = self.ctx.selected_device();
        let ready = dev.as_ref().is_some_and(|d| d.connected());
        self.actions.set_sensitive(ready);
        let empty = |t: &Self| {
            for c in [&t.cpu, &t.ram, &t.temp, &t.storage] {
                c.set("—", "");
            }
            for b in [&t.cpu_bar, &t.ram_bar, &t.storage_bar] {
                b.set(None, 101.0, 101.0);
            }
            t.speed.set_visible(false);
        };
        match &dev {
            Some(d) => {
                self.name.set_text(&self.ctx.device_label(d));
                let kind = if d.connected() { "ok" } else { "warn" };
                self.pill.set(&d.state_label(), kind);
            }
            None => {
                self.name.set_text(tr("No device"));
                self.pill.set(tr("Disconnected"), "");
                self.sub.set_text(tr("Connect a phone over USB"));
                self.gauge.set(None, "—", "", Level::Normal);
                empty(self);
                return;
            }
        }
        let Some(s) = self.ctx.snapshot() else {
            self.sub.set_text(&dev.map(|d| self.ctx.shown_value(&d.serial)).unwrap_or_default());
            self.gauge.set(None, "—", "", Level::Normal);
            empty(self);
            return;
        };
        self.sub.set_text(&self.ctx.shown(&format!("{} · {}", s.manufacturer(), s.android_version())));
        let pct = s.battery.percentage();
        let level = match pct {
            Some(p) if p <= 15.0 => Level::Critical,
            Some(p) if p <= 30.0 => Level::High,
            _ => Level::Normal,
        };
        self.gauge.set(pct, &format::percent(pct), if s.battery.is_charging() { tr("Charging") } else { tr("Battery") }, level);

        // USB speed (or the connection type if it is not USB).
        let host = self.ctx.usb_devices().first().map(|u| u.speed_label()).unwrap_or_default();
        let link = if host.is_empty() { s.usb.label() } else { host };
        // “USB 3.2 Gen 2 · 10 Gb/s” → “USB 3.2 Gen 2”; the full text in the
        // tooltip.
        self.speed.set_text(link.split(" · ").next().unwrap_or(&link));
        self.speed.set_tooltip_text(Some(&link));
        self.speed.set_visible(!link.is_empty());

        let load = s.cpu.load_percent;
        self.cpu.set(&format::percent(load), &format::frequency_khz(s.cpu.cur_freq_khz()));
        self.cpu_bar.set(load, 75.0, 92.0);
        self.ram.set(&format::human_kib(s.memory.used_kib()), &t!("of {}", format::human_kib(s.memory.total_kib)));
        self.ram_bar.set(s.memory.percent(), 80.0, 92.0);
        self.temp.set(&format::temperature(s.cpu_temperature()), &t!("Battery {}", format::temperature(s.battery_temperature())));
        let st = &s.data_storage;
        self.storage.set(&t!("{} free", format::human_kib(st.available_kib)), &t!("of {}", format::human_kib(st.total_kib)));
        self.storage_bar.set(st.percent(), 85.0, 95.0);
    }
}
