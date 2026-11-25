//! “Usage” section of the Battery page: which apps use the most, what drains
//! with the screen off and what keeps the phone awake.
//!
//! It is analysed when the button is pressed (about 6 s of `dumpsys` on the
//! phone), not in the background. The result is kept while the same phone stays
//! connected and is repainted without querying when private mode changes.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adbscan_core::battery_usage::{AppUsage, BatteryUsage};
use adbscan_core::format;

use crate::ctx::Ctx;
use crate::util;
use crate::widgets;

const TOP_APPS: usize = 12;
const TOP_SCREEN_OFF: usize = 6;
const TOP_WAKELOCKS: usize = 8;
const TOP_KERNEL: usize = 10;

pub struct UsageView {
    me: Weak<Self>,
    ctx: Rc<Ctx>,
    pub root: gtk::Box,
    analyze: gtk::Button,
    spinner: gtk::Spinner,
    summary: gtk::Label,
    body: gtk::Box,
    data: RefCell<Option<(String, BatteryUsage)>>,
    busy: Cell<bool>,
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn mah(v: f64) -> String {
    let decimals = if v < 10.0 { 1 } else { 0 };
    format!("{} mAh", format::number(Some(v), decimals))
}

fn duration_ms(ms: u64) -> String {
    format::human_duration(Some(ms as f64 / 1000.0))
}

impl UsageView {
    pub fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let root = util::vbox(12);
        let head = util::hbox(10);
        let titles = util::vbox(2);
        titles.set_hexpand(true);
        titles.append(&util::label(tr("Usage since the last full charge"), &["section-title"]));
        let summary = util::label(tr("Which apps use the most, what drains with the screen off and what keeps the phone awake."), &["dim"]);
        summary.set_wrap(true);
        titles.append(&summary);
        head.append(&titles);
        // gtk::Spinner and not adw::Spinner: the latter needs libadwaita 1.6.
        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        spinner.set_valign(gtk::Align::Center);
        head.append(&spinner);
        let analyze = util::pill_button("adb-chart-line-symbolic", tr("Analyse usage"));
        analyze.add_css_class("pill");
        analyze.add_css_class("suggested-action");
        analyze.set_valign(gtk::Align::Center);
        analyze.set_tooltip_text(Some(tr("Reads Android's battery statistics (a few seconds; read-only)")));
        head.append(&analyze);
        root.append(&head);
        let body = util::vbox(18);
        root.append(&body);

        let this = Rc::new_cyclic(|me| Self { me: me.clone(), ctx: ctx.clone(), root, analyze, spinner, summary, body, data: RefCell::new(None), busy: Cell::new(false) });
        let weak = this.me.clone();
        this.analyze.connect_clicked(move |_| {
            if let Some(t) = weak.upgrade() {
                t.run();
            }
        });
        this
    }

    fn run(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            self.ctx.toast(tr("No device is ready"));
            return;
        };
        if self.busy.replace(true) {
            return;
        }
        self.analyze.set_sensitive(false);
        self.spinner.set_visible(true);
        self.spinner.start();
        self.summary.set_text(tr("Reading the phone's battery statistics… (a few seconds)"));
        let weak = self.me.clone();
        let serial2 = serial.clone();
        util::run_bg(
            move || adb.battery_usage(&serial2),
            move |result| {
                let Some(t) = weak.upgrade() else { return };
                t.busy.set(false);
                t.analyze.set_sensitive(true);
                t.spinner.stop();
                t.spinner.set_visible(false);
                // If the phone was changed in the meantime, it is not valid.
                if t.ctx.ready_serial().as_deref() != Some(serial.as_str()) {
                    return;
                }
                match result {
                    Ok(u) => {
                        let wanted = names_to_resolve(&u);
                        *t.data.borrow_mut() = Some((serial.clone(), u));
                        t.analyze.set_child(Some(&adw::ButtonContent::builder().icon_name("adb-refresh-cw-symbolic").label(tr("Analyse again")).build()));
                        t.render();
                        t.resolve_names(serial, wanted);
                    }
                    Err(e) => t.summary.set_text(&t!("Could not read the statistics: {}", e)),
                }
            },
        );
    }

    /// Display names (“YouTube”) of what is shown: painted first with the
    /// package and repainted when they arrive.
    fn resolve_names(&self, serial: String, packages: Vec<String>) {
        let Some(adb) = self.ctx.adb() else { return };
        let weak = self.me.clone();
        let serial2 = serial.clone();
        util::run_bg(
            move || crate::pages::apps::fetch_missing_labels(&adb, &serial2, &packages),
            move |found| {
                let Some(t) = weak.upgrade() else { return };
                let same = t.data.borrow().as_ref().is_some_and(|(s, _)| *s == serial);
                if found > 0 && same {
                    t.render();
                }
            },
        );
    }

    /// Another phone (or none): what was analysed is no longer valid.
    pub fn on_devices(&self) {
        let current = self.ctx.ready_serial();
        let stale = self.data.borrow().as_ref().is_some_and(|(s, _)| Some(s) != current.as_ref());
        if stale {
            *self.data.borrow_mut() = None;
            self.clear();
            self.analyze.set_child(Some(&adw::ButtonContent::builder().icon_name("adb-chart-line-symbolic").label(tr("Analyse usage")).build()));
            self.summary.set_text(tr("Which apps use the most, what drains with the screen off and what keeps the phone awake."));
        }
    }

    /// Same data, another way of showing it (private mode).
    pub fn on_privacy(&self) {
        if self.data.borrow().is_some() {
            self.render();
        }
    }

    fn clear(&self) {
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
    }

    fn render(&self) {
        self.clear();
        let data = self.data.borrow();
        let Some((_, u)) = data.as_ref() else { return };
        let ctx = &self.ctx;
        let labels = crate::pages::apps::cached_labels();
        // The app's display name if known (“YouTube”), otherwise the package.
        let app_name = |a: &AppUsage| -> String {
            match a.packages.as_slice() {
                [one] if a.is_app() => labels.get(one).cloned().unwrap_or_else(|| a.label()),
                [first, rest @ ..] if a.is_app() => format!("{} (+{})", labels.get(first).cloned().unwrap_or_else(|| first.clone()), rest.len()),
                _ => a.label(),
            }
        };
        let owner_name = |uid: u32| -> String {
            let a = AppUsage { uid, packages: u.uid_packages.get(&uid).cloned().unwrap_or_default(), ..Default::default() };
            app_name(&a)
        };

        let mut sum = Vec::new();
        if let Some(ms) = u.on_battery_ms {
            sum.push(t!("On battery {}", duration_ms(ms)));
        }
        if let Some(ms) = u.screen_off_ms {
            sum.push(t!("with the screen off {}", duration_ms(ms)));
        }
        sum.push(t!("{} estimated by Android", mah(u.total_mah)));
        self.summary.set_text(&sum.join(" · "));

        if u.apps.is_empty() {
            let empty = adw::StatusPage::builder()
                .icon_name("adb-battery-symbolic")
                .title(tr("No usage recorded"))
                .description(tr("Android has no data yet since the last full charge: analyse again after using the phone on battery."))
                .build();
            empty.add_css_class("compact");
            self.body.append(&empty);
            return;
        }

        // -- apps that use the most
        // --------------------------------------------
        let top = adw::PreferencesGroup::builder().title(tr("Apps that use the most")).description(tr("Includes the screen while the app was in the foreground, as in Android's settings")).build();
        for a in u.apps.iter().take(TOP_APPS) {
            let pct = u.percent_of(a.mah);
            let mut sub = vec![mah(a.mah)];
            if a.screen_off_mah >= 0.1 {
                sub.push(t!("{} with the screen off", mah(a.screen_off_mah)));
            }
            let row = adw::ActionRow::builder().title(esc(&ctx.shown(&app_name(a)))).subtitle(esc(&sub.join(" · "))).build();
            if a.is_app() && !a.packages.is_empty() {
                row.set_tooltip_text(Some(&a.packages.join("\n")));
            }
            row.add_suffix(&percent_suffix(pct));
            top.add(&row);
        }
        let rest = u.unattributed_mah();
        if rest >= 0.5 {
            let row = adw::ActionRow::builder().title(tr("Not attributed to any app")).subtitle(esc(&t!("{} · mobile radio, GPU and idle system usage", mah(rest)))).build();
            row.add_suffix(&percent_suffix(u.percent_of(rest)));
            top.add(&row);
        }
        if u.apps.len() > TOP_APPS {
            top.set_header_suffix(Some(&util::label(&t!("{} of {}", TOP_APPS, u.apps.len()), &["dim", "caption"])));
        }
        self.body.append(&top);

        // -- with the screen off ------------------------------------------------
        let mut off: Vec<&AppUsage> = u.apps.iter().filter(|a| a.screen_off_mah >= 1.0).collect();
        off.sort_by(|a, b| b.screen_off_mah.total_cmp(&a.screen_off_mah));
        if !off.is_empty() {
            let g =
                adw::PreferencesGroup::builder().title(tr("With the screen off")).description(tr("What drains while you are not using the phone: the apps worth restricting are usually here")).build();
            for a in off.into_iter().take(TOP_SCREEN_OFF) {
                let row = adw::ActionRow::builder().title(esc(&ctx.shown(&app_name(a)))).subtitle(esc(&t!("{} with the screen off", mah(a.screen_off_mah)))).build();
                row.add_prefix(&util::icon("adb-night-light-symbolic"));
                g.add(&row);
            }
            self.body.append(&g);
        }

        // -- wakelocks ------------------------------------------------------------
        if !u.wakelocks.is_empty() {
            let g = adw::PreferencesGroup::builder().title(tr("What keeps the phone awake")).description(tr("Wakelocks: they stop the CPU from sleeping even with the screen off")).build();
            for w in u.grouped_wakelocks().iter().take(TOP_WAKELOCKS) {
                let owner = w.uid.map(&owner_name).unwrap_or_default();
                let row = adw::ActionRow::builder().title(esc(&ctx.shown(&owner))).subtitle(esc(&ctx.shown(&w.short_tag()))).build();
                row.set_tooltip_text(Some(&ctx.shown(&w.tag)));
                row.add_suffix(&util::label(&wakelock_time(w.ms, w.count), &["dim", "numeric"]));
                g.add(&row);
            }
            self.body.append(&g);
        }
        if !u.kernel_wakelocks.is_empty() {
            let g = adw::PreferencesGroup::new();
            let exp =
                adw::ExpanderRow::builder().title(tr("System wakelocks")).subtitle(t!("{} kernel and driver sources (radio, network, sensors)", u.kernel_wakelocks.len())).build();
            for w in u.kernel_wakelocks.iter().take(TOP_KERNEL) {
                let row = adw::ActionRow::builder().title(esc(&ctx.shown(&w.tag))).build();
                row.add_suffix(&util::label(&wakelock_time(w.ms, w.count), &["dim", "numeric"]));
                exp.add_row(&row);
            }
            g.add(&exp);
            self.body.append(&g);
        }

        let note = util::label(
            tr("Android's estimate (BatteryUsageStats), only while the phone runs on battery. “ADB shell” includes ADB Scan's telemetry while it is connected."),
            &["dim", "caption"],
        );
        note.set_wrap(true);
        self.body.append(&note);
    }
}

/// Packages shown in the section (top apps and wakelock owners).
fn names_to_resolve(u: &BatteryUsage) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let apps = u.apps.iter().take(TOP_APPS).chain(u.apps.iter().filter(|a| a.screen_off_mah >= 1.0).take(TOP_SCREEN_OFF));
    let grouped = u.grouped_wakelocks();
    let owners = grouped.iter().take(TOP_WAKELOCKS).filter_map(|w| w.uid).filter_map(|uid| u.uid_packages.get(&uid));
    for pkgs in apps.filter(|a| a.is_app()).map(|a| &a.packages).chain(owners) {
        if let Some(p) = pkgs.first() {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
    }
    out
}

/// “1 h 1 min · 15 times” (Android reports 0 times if it has been active since
/// before).
fn wakelock_time(ms: u64, count: u64) -> String {
    match count {
        0 => duration_ms(ms),
        1 => t!("{} · once", duration_ms(ms)),
        n => t!("{} · {} times", duration_ms(ms), n),
    }
}

/// Percentage with a short bar.
fn percent_suffix(pct: Option<f64>) -> gtk::Box {
    let b = util::hbox(10);
    b.set_valign(gtk::Align::Center);
    let bar = widgets::usage_bar();
    bar.set_value(pct.unwrap_or(0.0));
    bar.set_width_request(90);
    bar.set_valign(gtk::Align::Center);
    let l = util::label(&pct.map(|p| format!("{} %", format::number(Some(p), 1))).unwrap_or_else(|| "—".into()), &["numeric"]);
    l.set_width_chars(6);
    l.set_xalign(1.0);
    b.append(&bar);
    b.append(&l);
    b
}
