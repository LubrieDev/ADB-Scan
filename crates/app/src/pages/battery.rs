//! Battery: live status and local history (one sample per minute).

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

use adbscan_core::format;
use adbscan_core::models::now_secs;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::widgets::{Chart, Gauge, Level};
use crate::window::Page;

struct BatteryPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    gauge: Gauge,
    headline: gtk::Label,
    subline: gtk::Label,
    rows: Vec<(&'static str, gtk::Label)>,
    live: Chart,
    history: Chart,
    history_info: gtk::Label,
    range_hours: Cell<u32>,
    usage: Rc<crate::pages::battery_usage::UsageView>,
    last_history: Cell<f64>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    BatteryPage::new(ctx)
}

const FIELDS: [(&str, &str); 16] = [
    ("status", "Status"),
    ("health", "Health"),
    ("wear", "Capacity compared to the original"),
    ("cycles", "Charge cycles"),
    ("capacity_mah", "Current / design capacity"),
    ("first_use", "Battery in use since"),
    ("source", "Power source"),
    ("voltage", "Voltage"),
    ("temp", "Temperature"),
    ("tech", "Technology"),
    ("counter", "Remaining charge"),
    ("power", "Max. charging power"),
    ("current", "Max. charging current"),
    ("present", "Present"),
    ("charging_state", "Charging state (manufacturer)"),
    ("capacity", "Capacity level"),
];

impl BatteryPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);
        let hero = util::hbox(28);
        hero.add_css_class("hero");
        let gauge = Gauge::new(190, 16.0);
        let text = util::vbox(6);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let headline = util::label("—", &["hero-title"]);
        let subline = util::label("", &["hero-sub"]);
        subline.set_wrap(true);
        text.append(&headline);
        text.append(&subline);
        hero.append(&gauge.root);
        hero.append(&text);
        content.append(&hero);

        let group = adw::PreferencesGroup::builder().title(tr("Details")).build();
        let mut rows = Vec::new();
        for (k, t) in FIELDS {
            let (row, v) = util::info_row(tr(t));
            group.add(&row);
            rows.push((k, v));
        }
        content.append(&group);

        // Per-app usage and wakelocks (on demand).
        let usage = crate::pages::battery_usage::UsageView::new(ctx);
        content.append(&usage.root);

        let live = Chart::new(120, 120, 15.0, 50.0, " °C");
        live.set_auto_max(true);
        live.add_series(tr("Temperature"), Some("#e66100"), true);
        let live_card = util::vbox(8);
        live_card.add_css_class("card");
        live_card.add_css_class("stat-card");
        live_card.append(&util::label(tr("Live temperature"), &["section-title"]));
        live_card.append(&live.area);
        content.append(&live_card);

        let history = Chart::new(170, 1440, 0.0, 100.0, "");
        history.add_series(tr("Level (%)"), None, true);
        history.add_series("Temp. (°C)", Some("#e01b24"), false);
        let hist_card = util::vbox(8);
        hist_card.add_css_class("card");
        hist_card.add_css_class("stat-card");
        let head = util::hbox(8);
        head.append(&util::label(tr("History"), &["section-title"]));
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        head.append(&spacer);
        let ranges = util::hbox(0);
        ranges.add_css_class("linked");
        let mut first: Option<gtk::ToggleButton> = None;
        let mut buttons = Vec::new();
        for (label, hours) in [("1 h", 1u32), ("6 h", 6), ("24 h", 24), (tr("7 days"), 168)] {
            let b = gtk::ToggleButton::with_label(label);
            if let Some(f) = &first {
                b.set_group(Some(f));
            } else {
                first = Some(b.clone());
            }
            if hours == 24 {
                b.set_active(true);
            }
            ranges.append(&b);
            buttons.push((b, hours));
        }
        head.append(&ranges);
        hist_card.append(&head);
        hist_card.append(&history.area);
        let history_info = util::label("", &["dim", "caption"]);
        hist_card.append(&history_info);
        content.append(&hist_card);

        let root = util::page_scroller(&content, 1000);
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            gauge,
            headline,
            subline,
            rows,
            live,
            history,
            history_info,
            range_hours: Cell::new(24),
            usage,
            last_history: Cell::new(0.0),
        });
        for (b, hours) in buttons {
            let weak = Rc::downgrade(&this);
            b.connect_toggled(move |b| {
                if let (true, Some(t)) = (b.is_active(), weak.upgrade()) {
                    t.range_hours.set(hours);
                    t.load_history();
                }
            });
        }
        for m in ctx.metrics() {
            this.live.push(0, m.batt_temp);
        }
        this.update(false);
        this.load_history();
        this
    }

    fn set(&self, key: &str, value: String) {
        if let Some((_, l)) = self.rows.iter().find(|(k, _)| *k == key) {
            l.set_text(if value.is_empty() { "—" } else { &value });
        }
    }

    fn update(&self, push: bool) {
        let Some(s) = self.ctx.snapshot() else { return };
        let b = &s.battery;
        let pct = b.percentage();
        let level = match pct {
            Some(p) if p <= 15.0 => Level::Critical,
            Some(p) if p <= 30.0 => Level::High,
            _ => Level::Normal,
        };
        self.gauge.set(pct, &format::percent(pct), if b.is_charging() { tr("Charging") } else { "" }, level);
        self.headline.set_text(&format!("{} · {}", format::percent(pct), b.status()));
        let h = &s.health;
        let mut sub = vec![t!("Health {}", b.health().to_lowercase())];
        if let Some(p) = h.battery_percent() {
            sub.push(t!("{} of capacity", format::percent(Some(p))));
        }
        sub.push(format::temperature(s.battery_temperature()));
        if let Some(w) = b.max_charging_watts() {
            sub.push(t!("up to {} W", format!("{:.1}", w)));
        }
        self.subline.set_text(&sub.join(" · "));
        self.set("status", b.status().into());
        self.set("health", b.health().into());
        self.set(
            "wear",
            match (h.battery_percent(), h.battery_level()) {
                (Some(p), Some(l)) => {
                    format!("{} · {}", format::number(Some(p), 1) + " %", l.label())
                }
                _ if h.probed => tr("Not available on this phone").into(),
                _ => String::new(),
            },
        );
        self.set("cycles", h.battery_cycles.map(|c| c.to_string()).unwrap_or_default());
        self.set("first_use", h.battery_first_use.clone().unwrap_or_default());
        self.set(
            "capacity_mah",
            match (h.battery_full_mah, h.battery_design_mah, h.battery_percent()) {
                (Some(f), Some(d), Some(_)) => format!("{} / {} mAh", format::number(Some(f as f64), 0), format::number(Some(d as f64), 0)),
                _ => String::new(),
            },
        );
        self.set("source", if b.power_source().is_empty() { tr("Battery").into() } else { b.power_source() });
        self.set("voltage", format::voltage_mv(b.voltage_mv));
        self.set("temp", format::temperature(s.battery_temperature()));
        self.set("tech", b.technology.clone());
        self.set("counter", b.charge_counter_mah.map(|m| format!("{m} mAh")).unwrap_or_default());
        self.set("power", b.max_charging_watts().map(|w| format!("{w:.1} W")).unwrap_or_default());
        self.set("current", b.max_charging_current_ua.filter(|c| *c > 0).map(|c| format!("{:.2} A", c as f64 / 1e6)).unwrap_or_default());
        self.set("present", b.present.map(|p| if p { tr("Yes") } else { tr("No") }.to_string()).unwrap_or_default());
        self.set("charging_state", b.charging_state.clone());
        self.set("capacity", b.capacity_level.map(|c| c.to_string()).unwrap_or_default());
        if push {
            self.live.set_interval(self.ctx.live_interval());
            self.live.push(0, s.battery_temperature());
        }
        // The history reloads every minute (a sample is saved every 60 s).
        if now_secs() - self.last_history.get() > 61.0 {
            self.load_history();
        }
    }

    fn load_history(&self) {
        self.last_history.set(now_secs());
        let (Some(h), Some(serial)) = (self.ctx.history.clone(), self.ctx.ready_serial()) else {
            return;
        };
        let until = now_secs();
        let since = until - self.range_hours.get() as f64 * 3600.0;
        // Bands of ≥2 min (samples are one per minute), max. 240.
        let buckets = ((until - since) / 120.0).clamp(2.0, 240.0) as usize;
        let weak = self.me.clone();
        util::run_bg(
            move || h.battery_samples(&serial, since).unwrap_or_default(),
            move |samples| {
                let Some(t) = weak.upgrade() else { return };
                let (levels, temps) = adbscan_core::db::bucket_samples(&samples, since, until, buckets);
                t.history.replace(0, &levels);
                t.history.replace(1, &temps);
                t.history_info.set_text(&if samples.is_empty() {
                    tr("There are no saved samples yet. ADB Scan saves one per minute while the phone is connected.").to_string()
                } else {
                    t!("{} samples since {} · hover over the chart to see values", samples.len(), format::timestamp(samples.first().map(|s| s.timestamp)))
                });
            },
        );
    }
}

impl Page for BatteryPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn on_event(&self, ev: &Event) {
        match ev {
            Event::Snapshot => self.update(true),
            Event::Devices => {
                if self.ctx.snapshot().is_none() {
                    self.live.clear();
                }
                self.load_history();
                self.usage.on_devices();
            }
            Event::Privacy => self.usage.on_privacy(),
            _ => {}
        }
    }
    fn shown(&self) {
        self.live.refill(0, self.ctx.metrics().iter().map(|m| m.batt_temp));
        self.update(false);
        self.load_history();
    }
}
