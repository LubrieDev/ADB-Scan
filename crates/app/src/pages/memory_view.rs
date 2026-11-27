//! Performance → “Memory per app”: which apps stay in memory and how
//! much they hold (see `adbscan_core::procstats`). Read-only, read when the
//! button is pressed.

use adbscan_core::{format, i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adbscan_core::procstats::MemoryReport;

use crate::ctx::{Ctx, Event};
use crate::util;

/// Apps listed.
const TOP: usize = 12;

pub struct MemoryView {
    me: Weak<Self>,
    ctx: Rc<Ctx>,
    pub root: gtk::Box,
    head: adw::ActionRow,
    button: gtk::Button,
    list: gtk::ListBox,
    rows: RefCell<Vec<gtk::Widget>>,
    report: RefCell<Option<MemoryReport>>,
    busy: Cell<bool>,
}

impl MemoryView {
    pub fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let root = util::vbox(8);
        root.append(&util::label(tr("Memory per app"), &["section-title"]));
        let head = adw::ActionRow::builder()
            .title(tr("Apps that stay in memory"))
            .subtitle(tr("How long each app stayed in memory in the last hours and how much it held. The ones always there with a lot push the others out and make them reload."))
            .build();
        head.add_prefix(&util::icon("adb-memory-stick-symbolic"));
        let button = util::pill_button("adb-chart-line-symbolic", tr("Analyse"));
        button.set_valign(gtk::Align::Center);
        head.add_suffix(&button);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        list.append(&head);
        root.append(&list);
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            head,
            button: button.clone(),
            list,
            rows: RefCell::new(Vec::new()),
            report: RefCell::new(None),
            busy: Cell::new(false),
        });
        let w = this.me.clone();
        button.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
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
        crate::anim::set_busy(&self.button, true);
        let w = self.me.clone();
        util::run_bg(
            move || {
                let r = adb.memory_report(&serial);
                if let Ok(rep) = &r {
                    let pkgs: Vec<String> = rep.apps.iter().take(TOP).map(|a| a.package.clone()).collect();
                    crate::pages::apps::fetch_missing_labels(&adb, &serial, &pkgs);
                }
                r
            },
            move |r| {
                let Some(t) = w.upgrade() else { return };
                t.busy.set(false);
                crate::anim::set_busy(&t.button, false);
                match r {
                    Ok(rep) => {
                        *t.report.borrow_mut() = Some(rep);
                        t.render();
                    }
                    Err(e) => t.head.set_subtitle(&t!("Could not read it: {}", e)),
                }
            },
        );
    }

    fn render(&self) {
        for w in self.rows.borrow_mut().drain(..) {
            self.list.remove(&w);
        }
        let rep = self.report.borrow();
        let Some(rep) = rep.as_ref() else { return };
        self.head.set_subtitle(&t!("In the last {} · {} apps were in memory", format::human_duration(Some(rep.period_ms as f64 / 1000.0)), rep.apps.len()));
        let labels = crate::pages::apps::cached_labels();
        let mut rows = self.rows.borrow_mut();
        for a in rep.apps.iter().take(TOP) {
            let name = if a.package == "android" || a.package == "system" {
                adbscan_core::battery_usage::system_uid_name(1000)
            } else {
                labels.get(&a.package).cloned().unwrap_or_else(|| a.package.clone())
            };
            let mut sub = vec![t!("in memory {} % of the time", format::number(Some(a.time_pct), 0))];
            if a.processes > 1 {
                sub.push(t!("{} processes", a.processes));
            }
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&self.ctx.shown(&name)))
                .subtitle(glib::markup_escape_text(&sub.join(" · ")))
                .build();
            row.set_tooltip_text(Some(&a.package));
            let value = util::label(&t!("{} on average · up to {}", format::human_bytes(Some(a.avg_rss as f64)), format::human_bytes(Some(a.max_rss as f64))), &["dim", "numeric"]);
            row.add_suffix(&value);
            self.list.append(&row);
            rows.push(row.upcast());
        }
    }

    pub fn on_event(&self, ev: &Event) {
        match ev {
            Event::Privacy => self.render(),
            Event::Devices if self.report.borrow().is_some() && self.ctx.ready_serial().is_none() => {
                *self.report.borrow_mut() = None;
                self.render();
            }
            _ => {}
        }
    }
}
