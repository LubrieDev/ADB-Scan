//! Stability: apps that closed or froze, native crashes and system failures,
//! and the reason of the last start (see `adbscan_core::stability`).
//! Read-only, apart from saving a bug report to the computer.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adbscan_core::stability::{boot_reason_text, Kind, Stability};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::widgets::{RowItem, RowList};
use crate::window::Page;

struct StabilityPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::Box,
    status: gtk::Label,
    refresh: gtk::Button,
    report_btn: gtk::Button,
    apps: RowList,
    incidents: RowList,
    data: RefCell<Option<Stability>>,
    busy: Cell<bool>,
    read_for: RefCell<Option<String>>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    StabilityPage::new(ctx)
}

fn clamped(child: &impl IsA<gtk::Widget>, top: i32) -> adw::Clamp {
    let c = adw::Clamp::builder().maximum_size(1000).tightening_threshold(800).child(child).build();
    c.set_margin_top(top);
    c.set_margin_start(18);
    c.set_margin_end(18);
    c
}

impl StabilityPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let intro = adw::PreferencesGroup::builder()
            .title(tr("Stability"))
            .description(tr("Apps that closed unexpectedly or stopped responding, and failures of the system itself, from the records Android keeps for a few days. Nothing is changed on the phone."))
            .build();
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Read again"));
        refresh.add_css_class("flat");
        refresh.set_valign(gtk::Align::Center);
        let report_btn = util::pill_button("adb-file-text-symbolic", tr("Bug report…"));
        report_btn.add_css_class("pill");
        report_btn.set_valign(gtk::Align::Center);
        report_btn.set_tooltip_text(Some(tr("Saves Android's full report (a ZIP) to the computer, for the manufacturer's or an app's support")));
        let buttons = util::hbox(6);
        buttons.append(&refresh);
        buttons.append(&report_btn);
        intro.set_header_suffix(Some(&buttons));
        let status = util::label("", &["dim", "caption"]);
        status.set_wrap(true);
        let head = util::vbox(6);
        head.append(&intro);
        head.append(&status);

        let tabs = adw::ViewStack::new();
        let switcher = adw::ViewSwitcher::builder().stack(&tabs).policy(adw::ViewSwitcherPolicy::Wide).build();
        switcher.set_halign(gtk::Align::Center);
        let apps = RowList::filling(Some(("go-next-symbolic", tr("Open in Apps"))));
        tabs.add_titled_with_icon(&apps.root, Some("apps"), tr("By app"), "adb-package-symbolic");
        let incidents = RowList::filling(None);
        tabs.add_titled_with_icon(&incidents.root, Some("all"), tr("Every incident"), "adb-list-symbolic");

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
            report_btn: report_btn.clone(),
            apps,
            incidents,
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
        report_btn.connect_clicked(move |b| {
            if let Some(t) = w.upgrade() {
                t.bug_report(b.upcast_ref());
            }
        });
        let c = ctx.clone();
        this.apps.connect_action(move |_, pkg| {
            if !pkg.is_empty() {
                c.emit(Event::ShowApp(pkg.to_string()));
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
        crate::anim::set_busy(&self.refresh, true);
        self.status.set_text(tr("Reading the records…"));
        *self.read_for.borrow_mut() = Some(serial.clone());
        let w = self.me.clone();
        util::run_bg(
            move || adb.stability(&serial),
            move |r| {
                let Some(t) = w.upgrade() else { return };
                t.busy.set(false);
                crate::anim::set_busy(&t.refresh, false);
                match r {
                    Ok(s) => {
                        *t.data.borrow_mut() = Some(s);
                        t.render();
                    }
                    Err(e) => t.status.set_text(&t!("Could not read the records: {}", e)),
                }
            },
        );
    }

    fn render(&self) {
        let data = self.data.borrow();
        let Some(s) = data.as_ref() else { return };
        let mut notes = vec![t!("Last start: {}", boot_reason_text(&s.last_boot_reason))];
        if !s.readable {
            notes.push(tr("Android did not allow reading its records on this phone.").into());
        } else if let Some(since) = &s.since {
            notes.push(t!("Records since {}", since));
        }
        if s.system_restarts > 0 {
            notes.push(t!("The system interface restarted {} times", s.system_restarts));
        }
        self.status.set_text(&notes.join(" · "));

        let labels = crate::pages::apps::cached_labels();
        let name = |process: &str| {
            // `com.app:remote` belongs to `com.app`.
            let pkg = process.split(':').next().unwrap_or(process);
            self.ctx.shown(labels.get(pkg).map(String::as_str).unwrap_or(process))
        };
        let by_app = s.by_app();
        let mut items: Vec<RowItem> = by_app
            .iter()
            .map(|a| {
                let mut parts = Vec::new();
                if a.crashes > 0 {
                    parts.push(t!("closed {} times", a.crashes));
                }
                if a.freezes > 0 {
                    parts.push(t!("stopped responding {} times", a.freezes));
                }
                parts.push(t!("last {}", a.last));
                let mut item = RowItem::new(name(&a.process), parts.join(" · ")).with_icon("adb-triangle-alert-symbolic", "");
                item.action = a.process.split(':').next().unwrap_or(&a.process).to_string();
                item
            })
            .collect();
        if items.is_empty() {
            items.push(RowItem::new(if s.readable { tr("No app closed or froze in this period") } else { tr("No records") }, "").with_icon("adb-emblem-ok-symbolic", "severity-ok"));
        }
        self.apps.set_items(items);

        let mut all = Vec::new();
        let mut day = String::new();
        for i in &s.incidents {
            let d = i.time.get(..10).unwrap_or(&i.time).to_string();
            if d != day {
                all.push(RowItem::heading(d.clone()));
                day = d;
            }
            let kind = i.kind();
            let who = if i.process.is_empty() {
                match kind {
                    Some(Kind::System) => tr("Android system").to_string(),
                    _ => i.tag.clone(),
                }
            } else {
                name(&i.process)
            };
            let title = format!("{} · {}", who, kind.map(Kind::label).unwrap_or(""));
            let time = i.time.get(11..16).unwrap_or("");
            let sub = if i.detail.is_empty() { time.to_string() } else { format!("{time} · {}", self.ctx.shown(&i.detail)) };
            all.push(RowItem::new(title, sub).with_icon(if kind == Some(Kind::System) { "adb-dialog-warning-symbolic" } else { "adb-triangle-alert-symbolic" }, ""));
        }
        if all.is_empty() {
            all.push(RowItem::new(tr("No incidents"), ""));
        }
        self.incidents.set_items(all);
    }

    /// `adb bugreport` into a folder chosen by the user, after a warning: the
    /// report contains a lot of personal data.
    fn bug_report(&self, parent: &gtk::Widget) {
        if self.ctx.ready_serial().is_none() {
            return;
        }
        let (w, p) = (self.me.clone(), parent.clone());
        util::confirm(
            parent,
            tr("Save a bug report?"),
            tr("Android gathers its full report in one to three minutes: logs, system state and the list of apps. It contains personal data (accounts, networks, app names): share it only with someone you trust, such as the phone's or an app's support."),
            tr("Choose a folder"),
            false,
            move || {
                let w = w.clone();
                util::pick_folder(&p, tr("Folder for the bug report"), move |dir| {
                    let Some(t) = w.upgrade() else { return };
                    let (Some(adb), Some(serial)) = (t.ctx.adb(), t.ctx.ready_serial()) else { return };
                    t.report_btn.set_sensitive(false);
                    crate::anim::set_busy(&t.report_btn, true);
                    t.ctx.toast(tr("Creating the bug report (one to three minutes)…"));
                    let w2 = t.me.clone();
                    util::run_bg(
                        move || adb.bugreport(&serial, &dir),
                        move |r| {
                            let Some(t) = w2.upgrade() else { return };
                            t.report_btn.set_sensitive(true);
                            crate::anim::set_busy(&t.report_btn, false);
                            match r {
                                Ok(path) => {
                                    let shown = path.display().to_string();
                                    t.ctx.audit("bugreport", Some(&shown), true, None);
                                    let folder = path.parent().map(|p| p.to_path_buf()).unwrap_or(path.clone());
                                    t.ctx.toast_action(&t!("Bug report saved: {}", shown), tr("Open folder"), move || util::open_path(&folder));
                                }
                                Err(e) => {
                                    t.ctx.audit("bugreport", None, false, Some(&e.to_string()));
                                    t.ctx.toast(&t!("Could not create the bug report: {}", e));
                                }
                            }
                        },
                    );
                });
            },
        );
    }
}

impl Page for StabilityPage {
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
