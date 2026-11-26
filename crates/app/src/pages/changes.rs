//! Phone changes: a timeline of apps installed, updated and removed, and of
//! restarts and shutdowns, grouped by day (see `adbscan_core::changes`).

use adbscan_core::{format, i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adbscan_core::changes::{Change, Entry, Timeline};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

/// Rows shown at most (the newest).
const MAX_ROWS: usize = 400;

struct ChangesPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    body: gtk::Box,
    status: gtk::Label,
    system: gtk::CheckButton,
    data: RefCell<Option<Timeline>>,
    busy: Cell<bool>,
    read_for: RefCell<Option<String>>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    ChangesPage::new(ctx)
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

/// `HH:MM` in the computer's time zone.
fn clock(ts: i64) -> String {
    let (_, _, _, h, m, _) = format::civil_from_unix(ts, format::local_offset_secs());
    format!("{h:02}:{m:02}")
}

/// Android's reason for a restart or shutdown, in words (empty: none given).
fn reason_text(reason: &str) -> String {
    let parts: Vec<&str> = reason.split(',').filter(|p| !p.is_empty()).collect();
    let detail = parts.get(1).copied().unwrap_or("");
    match detail {
        "" => String::new(),
        "userrequested" | "shell" => tr("by the user").into(),
        d if d.contains("ota") || d.contains("update") => tr("to install a system update").into(),
        d if d.contains("panic") || d.contains("watchdog") || d.contains("crash") => t!("after an error ({})", d),
        d if d.contains("battery") || d.contains("thermal") => t!("for safety ({})", d),
        d => d.to_string(),
    }
}

impl ChangesPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);
        let intro = adw::PreferencesGroup::builder()
            .title(tr("What changed on the phone"))
            .description(tr("Apps installed, updated and removed, and restarts, from what Android itself keeps. Android does not keep a dated history of settings or permissions; what ADB Scan changed is in History → Operations."))
            .build();
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Read again"));
        refresh.add_css_class("flat");
        refresh.set_valign(gtk::Align::Center);
        intro.set_header_suffix(Some(&refresh));
        let system = gtk::CheckButton::with_label(tr("Include preinstalled apps"));
        system.set_tooltip_text(Some(tr("Updates of the apps that came with the phone (Google, the manufacturer…).")));
        let status = util::label("", &["dim", "caption"]);
        status.set_wrap(true);
        status.set_xalign(0.0);
        status.set_hexpand(true);
        let bar = util::hbox(12);
        bar.append(&status);
        bar.append(&system);
        content.append(&intro);
        content.append(&bar);
        let body = util::vbox(18);
        content.append(&body);
        let root = util::page_scroller(&content, 900);
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            body,
            status,
            system: system.clone(),
            data: RefCell::new(None),
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
        system.connect_toggled(move |_| {
            if let Some(t) = w.upgrade() {
                t.render();
            }
        });
        this
    }

    fn load(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        if self.busy.replace(true) {
            return;
        }
        self.status.set_text(tr("Reading what changed on the phone (about 15 seconds)…"));
        *self.read_for.borrow_mut() = Some(serial.clone());
        let w = self.me.clone();
        util::run_bg(
            move || adb.phone_changes(&serial),
            move |r| {
                let Some(t) = w.upgrade() else { return };
                t.busy.set(false);
                match r {
                    Ok(timeline) => {
                        *t.data.borrow_mut() = Some(timeline);
                        t.render();
                    }
                    Err(e) => t.status.set_text(&t!("Could not read the changes: {}", e)),
                }
            },
        );
    }

    fn row(&self, e: &Entry, labels: &std::collections::HashMap<String, String>) -> adw::ActionRow {
        let name = |p: &str| labels.get(p).cloned().unwrap_or_else(|| p.to_string());
        // The package goes under the name only when the name is known (the
        // Apps section learns the names).
        let pkg_note = |p: &str| if labels.contains_key(p) { format!(" · {p}") } else { String::new() };
        let with_version = |p: &str, v: &str| if v.is_empty() { format!("{}{}", clock(e.time), pkg_note(p)) } else { format!("{} · {}{}", clock(e.time), t!("version {}", v), pkg_note(p)) };
        let (icon, title, subtitle) = match &e.change {
            Change::Installed { package, version } => ("adb-system-software-install-symbolic", t!("{} installed", name(package)), with_version(package, version)),
            Change::Updated { package, version } => ("adb-software-update-available-symbolic", t!("{} updated", name(package)), with_version(package, version)),
            Change::Removed { package } => ("adb-edit-delete-symbolic", t!("{} uninstalled", name(package)), format!("{}{}", clock(e.time), pkg_note(package))),
            Change::Restarted { reason } => {
                let why = reason_text(reason);
                ("adb-system-reboot-symbolic", tr("Restarted").to_string(), if why.is_empty() { clock(e.time) } else { format!("{} · {}", clock(e.time), why) })
            }
            Change::TurnedOff { reason } => {
                let why = reason_text(reason);
                ("adb-system-shutdown-symbolic", tr("Turned off").to_string(), if why.is_empty() { clock(e.time) } else { format!("{} · {}", clock(e.time), why) })
            }
        };
        let row = adw::ActionRow::builder().title(esc(&title)).subtitle(esc(&subtitle)).build();
        row.add_prefix(&util::icon(icon));
        row
    }

    fn render(&self) {
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
        let data = self.data.borrow();
        let Some(data) = data.as_ref() else { return };
        let with_system = self.system.is_active();
        let shown: Vec<&Entry> = data.entries.iter().filter(|e| with_system || !e.system).take(MAX_ROWS).collect();
        let mut status = match shown.len() {
            0 => tr("Nothing to show.").to_string(),
            1 => tr("1 change.").to_string(),
            n => t!("{} changes.", n),
        };
        match data.removals_since {
            Some(since) => status.push_str(&format!(" {}", t!("Uninstalled apps are known since {}.", format::date(Some(since as f64))))),
            None => status.push_str(&format!(" {}", tr("This phone does not say which apps were uninstalled."))),
        }
        self.status.set_text(&status);
        let labels = crate::pages::apps::cached_labels();
        let mut day = String::new();
        let mut group: Option<adw::PreferencesGroup> = None;
        for e in shown {
            let d = format::date(Some(e.time as f64));
            if d != day || group.is_none() {
                let g = adw::PreferencesGroup::builder().title(esc(&d)).build();
                self.body.append(&g);
                group = Some(g);
                day = d;
            }
            if let Some(g) = &group {
                g.add(&self.row(e, &labels));
            }
        }
    }
}

impl Page for ChangesPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn shown(&self) {
        if self.data.borrow().is_none() || *self.read_for.borrow() != self.ctx.ready_serial() {
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
