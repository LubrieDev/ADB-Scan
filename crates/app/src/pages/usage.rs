//! Usage: screen time, unlocks, notifications, data per app and apps not
//! opened for months (see `adbscan_core::usage`).
//! Read-only; read when the page is shown and with its refresh button.

use adbscan_core::{format, i18n::tr, t};
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adbscan_core::usage::{Period, Usage, DATA_DAYS};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::widgets::{RowItem, RowList};
use crate::window::Page;

struct UsagePage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::Box,
    status: gtk::Label,
    refresh: gtk::Button,
    week: gtk::ToggleButton,
    screen_total: gtk::Label,
    screen: RowList,
    data_total: gtk::Label,
    data: RowList,
    notif_total: gtk::Label,
    notif: RowList,
    unused_total: gtk::Label,
    unused: RowList,
    usage: RefCell<Option<Usage>>,
    busy: Cell<bool>,
    read_for: RefCell<Option<String>>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    UsagePage::new(ctx)
}

/// Apps not opened in this many days are listed as unused.
const UNUSED_DAYS: u64 = 90;

/// `2 h 05 min`
fn duration(ms: u64) -> String {
    format::human_duration(Some(ms as f64 / 1000.0))
}

fn bytes(b: u64) -> String {
    format::human_bytes(Some(b as f64))
}

/// Area above a list, as wide as the list.
fn clamped(child: &impl IsA<gtk::Widget>, top: i32) -> adw::Clamp {
    let c = adw::Clamp::builder().maximum_size(1000).tightening_threshold(800).child(child).build();
    c.set_margin_top(top);
    c.set_margin_start(18);
    c.set_margin_end(18);
    c
}

impl UsagePage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let intro = adw::PreferencesGroup::builder()
            .title(tr("Usage"))
            .description(tr("Screen time and data of each app, as Android counts them. Nothing is changed: to limit an app, open it in Apps."))
            .build();
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Read again"));
        refresh.add_css_class("flat");
        refresh.set_valign(gtk::Align::Center);
        intro.set_header_suffix(Some(&refresh));
        let status = util::label("", &["dim", "caption"]);
        status.set_wrap(true);
        let head = util::vbox(6);
        head.append(&intro);
        head.append(&status);

        let tabs = adw::ViewStack::new();
        let switcher = adw::ViewSwitcher::builder().stack(&tabs).policy(adw::ViewSwitcherPolicy::Wide).build();
        switcher.set_halign(gtk::Align::Center);

        // Screen time: today or this week.
        let today = gtk::ToggleButton::with_label(tr("Today"));
        let week = gtk::ToggleButton::with_label(tr("This week"));
        week.set_group(Some(&today));
        today.set_active(true);
        let period = util::hbox(0);
        period.add_css_class("linked");
        period.append(&today);
        period.append(&week);
        let screen_total = util::label("", &["dim"]);
        screen_total.set_hexpand(true);
        screen_total.set_wrap(true);
        let bar = util::hbox(12);
        bar.append(&screen_total);
        bar.append(&period);
        let screen = RowList::filling(Some(("go-next-symbolic", tr("Open in Apps"))));
        let sbox = util::vbox(0);
        sbox.append(&clamped(&bar, 12));
        sbox.append(&screen.root);
        tabs.add_titled_with_icon(&sbox, Some("screen"), tr("Screen time"), "adb-monitor-smartphone-symbolic");

        let data_total = util::label("", &["dim"]);
        data_total.set_wrap(true);
        let data = RowList::filling(Some(("go-next-symbolic", tr("Open in Apps"))));
        let dbox = util::vbox(0);
        dbox.append(&clamped(&data_total, 12));
        dbox.append(&data.root);
        tabs.add_titled_with_icon(&dbox, Some("data"), &t!("Data ({} days)", DATA_DAYS), "adb-globe-symbolic");

        let notif_total = util::label("", &["dim"]);
        notif_total.set_wrap(true);
        let notif = RowList::filling(Some(("go-next-symbolic", tr("Open in Apps"))));
        let nbox = util::vbox(0);
        nbox.append(&clamped(&notif_total, 12));
        nbox.append(&notif.root);
        tabs.add_titled_with_icon(&nbox, Some("notifications"), tr("Notifications"), "adb-bell-ring-symbolic");

        let unused_total = util::label("", &["dim"]);
        unused_total.set_wrap(true);
        let unused = RowList::filling(Some(("go-next-symbolic", tr("Open in Apps"))));
        let ubox = util::vbox(0);
        ubox.append(&clamped(&unused_total, 12));
        ubox.append(&unused.root);
        tabs.add_titled_with_icon(&ubox, Some("unused"), tr("Unused apps"), "adb-package-symbolic");

        let root = util::vbox(8);
        root.append(&clamped(&head, 18));
        root.append(&switcher);
        root.append(&tabs);
        tabs.set_vexpand(true);

        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            status,
            refresh: refresh.clone(),
            week: week.clone(),
            screen_total,
            screen,
            data_total,
            data,
            notif_total,
            notif,
            unused_total,
            unused,
            usage: RefCell::new(None),
            busy: Cell::new(false),
            read_for: RefCell::new(None),
        });
        let w = this.me.clone();
        refresh.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.load();
            }
        });
        let w = this.me.clone();
        week.connect_toggled(move |_| {
            if let Some(t) = w.upgrade() {
                t.render_screen();
            }
        });
        for list in [&this.screen, &this.data, &this.notif, &this.unused] {
            let ctx = ctx.clone();
            list.connect_action(move |_, pkg| {
                if !pkg.is_empty() {
                    ctx.emit(Event::ShowApp(pkg.to_string()));
                }
            });
        }
        this
    }

    fn load(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        if self.busy.replace(true) {
            return;
        }
        crate::anim::set_busy(&self.refresh, true);
        self.status.set_text(tr("Reading the usage (a few seconds)…"));
        *self.read_for.borrow_mut() = Some(serial.clone());
        let w = self.me.clone();
        util::run_bg(
            move || adb.usage(&serial),
            move |r| {
                let Some(t) = w.upgrade() else { return };
                t.busy.set(false);
                crate::anim::set_busy(&t.refresh, false);
                match r {
                    Ok(u) => {
                        let wanted = shown_packages(&u);
                        *t.usage.borrow_mut() = Some(u);
                        t.render();
                        t.resolve_names(wanted);
                    }
                    Err(e) => t.status.set_text(&t!("Could not read the usage: {}", e)),
                }
            },
        );
    }

    fn render(&self) {
        let usage = self.usage.borrow();
        let Some(u) = usage.as_ref() else { return };
        let mut notes = Vec::new();
        if u.today.is_none() {
            notes.push(tr("This phone did not give its screen time.").to_string());
        }
        if u.data.is_none() {
            notes.push(tr("This phone did not give its data usage.").to_string());
        }
        if notes.is_empty() {
            notes.push(t!("Read at {}", format::clock(adbscan_core::models::now_secs())));
        }
        self.status.set_text(&notes.join(" "));
        drop(usage);
        self.render_screen();
        self.render_data();
        self.render_notifications();
        self.render_unused();
    }

    fn name(&self, labels: &HashMap<String, String>, package: &str) -> String {
        if package == "android" {
            return adbscan_core::battery_usage::system_uid_name(1000);
        }
        // Its name sits in a resource table of up to 70 MB, not read over
        // Wi-Fi (see `Adb::package_label`).
        if package == "com.android.settings" && !labels.contains_key(package) {
            return tr("Settings").to_string();
        }
        self.ctx.shown(labels.get(package).map(String::as_str).unwrap_or(package))
    }

    fn render_screen(&self) {
        let usage = self.usage.borrow();
        let Some(u) = usage.as_ref() else { return };
        let period: Option<&Period> = if self.week.is_active() { u.week.as_ref() } else { u.today.as_ref() };
        let Some(p) = period else {
            self.screen_total.set_text(tr("No screen time for this period."));
            self.screen.set_items(Vec::new());
            return;
        };
        let since = format::date(Some(p.begin_ms as f64 / 1000.0));
        let mut total = t!("{} on screen since {} · {} apps", duration(p.total_ms()), since, p.apps.len());
        let pickups = if self.week.is_active() { u.pickups_week } else { u.pickups_today };
        if let Some(k) = pickups.filter(|k| k.unlocks > 0 || k.screen_ons > 0) {
            total.push_str(" · ");
            total.push_str(&t!("unlocked {} times, screen turned on {} times", k.unlocks, k.screen_ons));
        }
        self.screen_total.set_text(&total);
        let labels = crate::pages::apps::cached_labels();
        let items = p
            .apps
            .iter()
            .map(|a| {
                let mut sub = vec![duration(a.screen_ms())];
                if a.launches > 0 {
                    sub.push(t!("{} launches", a.launches));
                }
                if a.last_used_ms > 0 {
                    sub.push(t!("last used {}", format::date(Some(a.last_used_ms as f64 / 1000.0))));
                }
                let mut item = RowItem::new(self.name(&labels, &a.package), sub.join(" · "));
                item.action = a.package.clone();
                item
            })
            .collect::<Vec<_>>();
        self.screen.set_items(items);
    }

    fn render_data(&self) {
        let usage = self.usage.borrow();
        let Some(d) = usage.as_ref().and_then(|u| u.data.as_ref()) else {
            self.data_total.set_text(tr("No data usage."));
            self.data.set_items(Vec::new());
            return;
        };
        self.data_total.set_text(&t!("Since {}: {} of mobile data and {} over Wi-Fi.", format::date(Some(d.from as f64)), bytes(d.mobile_total()), bytes(d.wifi_total())));
        let labels = crate::pages::apps::cached_labels();
        let items = d
            .apps
            .iter()
            .map(|a| {
                let title = match a.packages.as_slice() {
                    [one] if a.is_app() => self.name(&labels, one),
                    _ => self.ctx.shown(&a.label()),
                };
                let mut sub = vec![t!("Mobile {}", bytes(a.mobile_bytes)), t!("Wi-Fi {}", bytes(a.wifi_bytes))];
                if a.other_bytes > 0 {
                    sub.push(t!("other {}", bytes(a.other_bytes)));
                }
                let mut item = RowItem::new(title, sub.join(" · "));
                if a.is_app() {
                    item.action = a.packages.first().cloned().unwrap_or_default();
                }
                item
            })
            .collect::<Vec<_>>();
        self.data.set_items(items);
    }
}

/// Packages on screen, to look up their names.
fn shown_packages(u: &Usage) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let periods = [&u.today, &u.week].into_iter().flatten().flat_map(|p| p.apps.iter().take(60).map(|a| &a.package));
    let notif = u.notifications.iter().flatten().map(|(p, _)| p);
    let now = (adbscan_core::models::now_secs() * 1000.0) as u64;
    let unused: Vec<String> = u.unused(now, UNUSED_DAYS).into_iter().map(|a| a.package).collect();
    for p in periods.chain(notif).chain(unused.iter()) {
        if !out.contains(p) {
            out.push(p.clone());
        }
    }
    out
}

impl UsagePage {
    /// Display names (“YouTube”) of the packages shown: painted first with
    /// the package and repainted when they arrive.
    fn resolve_names(&self, packages: Vec<String>) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else { return };
        let w = self.me.clone();
        util::run_bg(
            move || crate::pages::apps::fetch_missing_labels(&adb, &serial, &packages),
            move |found| {
                if let (true, Some(t)) = (found > 0, w.upgrade()) {
                    t.render();
                }
            },
        );
    }

    fn render_notifications(&self) {
        let usage = self.usage.borrow();
        let Some((u, list)) = usage.as_ref().and_then(|u| Some((u, u.notifications.as_ref()?))) else {
            self.notif_total.set_text(tr("This phone did not give its notifications."));
            self.notif.set_items(Vec::new());
            return;
        };
        let total = u.notifications_total();
        self.notif_total.set_text(&t!(
            "{} notifications in the last 24 hours from {} apps: the ones that interrupted (sound, vibration or on screen). Only how many is read, never what they say.",
            total,
            list.len()
        ));
        let labels = crate::pages::apps::cached_labels();
        let items = list
            .iter()
            .map(|(pkg, n)| {
                let pct = if total > 0 { *n as f64 * 100.0 / total as f64 } else { 0.0 };
                let sub = t!("{} notifications · {} % of all", n, format::number(Some(pct), if pct < 10.0 { 1 } else { 0 }));
                // Android records some without the app.
                if pkg == "null" {
                    return RowItem::new(tr("App not recorded by Android"), sub);
                }
                let mut item = RowItem::new(self.name(&labels, pkg), sub);
                item.action = pkg.clone();
                item
            })
            .collect::<Vec<_>>();
        self.notif.set_items(items);
    }

    fn render_unused(&self) {
        let usage = self.usage.borrow();
        let Some(u) = usage.as_ref() else { return };
        let Some(year) = u.year.as_ref() else {
            self.unused_total.set_text(tr("This phone did not give how long ago each app was used."));
            self.unused.set_items(Vec::new());
            return;
        };
        let now = (adbscan_core::models::now_secs() * 1000.0) as u64;
        let list = u.unused(now, UNUSED_DAYS);
        let since = format::date(Some(year.begin_ms as f64 / 1000.0));
        self.unused_total.set_text(&if list.is_empty() {
            t!("Every app you installed was opened in the last {} days.", UNUSED_DAYS)
        } else if list.len() == 1 {
            t!("1 app you installed was not opened in the last {} days. Uninstalling or disabling it frees space and stops it working in the background.", UNUSED_DAYS)
        } else {
            t!("{} apps you installed were not opened in the last {} days. Uninstalling or disabling them frees space and stops them working in the background.", list.len(), UNUSED_DAYS)
        });
        let labels = crate::pages::apps::cached_labels();
        let items = list
            .iter()
            .map(|a| {
                let used = if a.last_used_ms > 0 { t!("last opened {}", format::date(Some(a.last_used_ms as f64 / 1000.0))) } else { t!("not opened since at least {}", since) };
                // Apps that came with the phone have a 1970 date.
                let installed = if a.installed_ms < 946_684_800_000 { tr("came with the phone").to_string() } else { t!("installed {}", format::date(Some(a.installed_ms as f64 / 1000.0))) };
                let mut item = RowItem::new(self.name(&labels, &a.package), format!("{used} · {installed}"));
                item.action = a.package.clone();
                item
            })
            .collect::<Vec<_>>();
        self.unused.set_items(items);
    }
}

impl Page for UsagePage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn shown(&self) {
        if self.usage.borrow().is_none() || *self.read_for.borrow() != self.ctx.ready_serial() {
            self.load();
        }
    }
    fn on_event(&self, ev: &Event) {
        match ev {
            Event::Devices if self.root.is_mapped() && *self.read_for.borrow() != self.ctx.ready_serial() => self.load(),
            Event::Privacy => self.render(),
            _ => {}
        }
    }
}
