//! Preinstalled apps: removes the ones in `adbscan_core::debloat` for the
//! phone's user and restores any app removed that way. Everything is
//! reversible: the APKs stay in the system partition.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

use adbscan_core::debloat::{self, Bloat, DebloatState, Risk};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

struct DebloatPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    body: gtk::Box,
    status: gtk::Label,
    state: RefCell<Option<DebloatState>>,
    busy: std::cell::Cell<bool>,
    /// Serial the list was read from (re-read when the phone changes).
    read_for: RefCell<Option<String>>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    DebloatPage::new(ctx)
}

impl DebloatPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);
        let intro = adw::PreferencesGroup::builder()
            .title(tr("Preinstalled apps"))
            .description(tr(
                "Removes apps the manufacturer or carrier preinstalled, only for your user (pm uninstall --user 0). Their APK stays in the system partition: you can restore them here at any time, and a factory reset brings them back. No root needed.",
            ))
            .build();
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Read again"));
        refresh.add_css_class("flat");
        refresh.set_valign(gtk::Align::Center);
        intro.set_header_suffix(Some(&refresh));
        let status = util::label("", &["dim", "caption"]);
        status.set_wrap(true);
        content.append(&intro);
        content.append(&status);
        let body = util::vbox(18);
        content.append(&body);
        let root = util::page_scroller(&content, 900);
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            body,
            status,
            state: RefCell::new(None),
            busy: std::cell::Cell::new(false),
            read_for: RefCell::new(None),
        });
        let w = this.me.clone();
        refresh.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.load();
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
        self.status.set_text(tr("Reading the installed apps…"));
        *self.read_for.borrow_mut() = Some(serial.clone());
        let w = self.me.clone();
        util::run_bg(
            move || adb.debloat_state(&serial),
            move |r| {
                let Some(t) = w.upgrade() else { return };
                t.busy.set(false);
                match r {
                    Ok(s) => {
                        *t.state.borrow_mut() = Some(s);
                        t.render();
                    }
                    Err(e) => t.status.set_text(&t!("Could not read the apps: {}", e)),
                }
            },
        );
    }

    fn render(&self) {
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
        let Some(state) = self.state.borrow().clone() else { return };
        self.status.set_text(&if state.present.is_empty() {
            tr("None of the apps on the list is installed on this phone.").to_string()
        } else {
            t!("{} apps on the list are installed. Removed apps keep their data and can be restored.", state.present.len())
        });
        for risk in Risk::ALL {
            let apps: Vec<&Bloat> = state.present.iter().filter(|b| b.risk == risk).collect();
            if apps.is_empty() {
                continue;
            }
            let group = adw::PreferencesGroup::builder().title(risk.label()).description(risk.description()).build();
            if risk == Risk::Recommended && apps.len() > 1 {
                let all = gtk::Button::with_label(&t!("Remove all ({})", apps.len()));
                all.add_css_class("pill");
                all.set_valign(gtk::Align::Center);
                let packages: Vec<Bloat> = apps.iter().map(|b| (*b).clone()).collect();
                let w = self.me.clone();
                all.connect_clicked(move |b| {
                    if let Some(t) = w.upgrade() {
                        t.remove_many(b, packages.clone());
                    }
                });
                group.set_header_suffix(Some(&all));
            }
            for app in apps {
                group.add(&self.app_row(app));
            }
            self.body.append(&group);
        }
        self.body.append(&self.removed_group(&state.removed));
    }

    fn app_row(&self, app: &Bloat) -> adw::ActionRow {
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(app.name)).subtitle(glib::markup_escape_text(&format!("{} · {} · {}", app.what, app.vendor, app.package))).build();
        row.set_subtitle_lines(3);
        let remove = gtk::Button::with_label(tr("Remove"));
        remove.set_valign(gtk::Align::Center);
        if app.risk == Risk::Expert {
            remove.add_css_class("destructive-action");
        }
        let (w, app) = (self.me.clone(), app.clone());
        remove.connect_clicked(move |b| {
            let Some(t) = w.upgrade() else { return };
            if app.risk == Risk::Expert {
                let (w2, app2) = (w.clone(), app.clone());
                util::confirm(b, &t!("Remove {}?", app.name), app.what, tr("Remove"), true, move || {
                    if let Some(t) = w2.upgrade() {
                        t.remove_one(&app2);
                    }
                });
            } else {
                t.remove_one(&app);
            }
        });
        row.add_suffix(&remove);
        row
    }

    fn removed_group(&self, removed: &[String]) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(tr("Removed for your user"))
            .description(tr("Every app removed with “pm uninstall --user 0”, by ADB Scan or by another tool. Restoring reinstalls the copy kept in the system partition."))
            .build();
        if removed.is_empty() {
            group.add(&adw::ActionRow::builder().title(tr("No app has been removed")).build());
            return group;
        }
        let labels = crate::pages::apps::cached_labels();
        for pkg in removed {
            let known = debloat::lookup(pkg);
            let title = known.as_ref().map(|b| b.name.to_string()).or_else(|| labels.get(pkg).cloned()).unwrap_or_else(|| pkg.clone());
            let row = adw::ActionRow::builder().title(glib::markup_escape_text(&title)).subtitle(glib::markup_escape_text(pkg)).build();
            let restore = gtk::Button::with_label(tr("Restore"));
            restore.set_valign(gtk::Align::Center);
            let (w, pkg) = (self.me.clone(), pkg.clone());
            restore.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    t.restore(&pkg);
                }
            });
            row.add_suffix(&restore);
            group.add(&row);
        }
        group
    }

    fn remove_one(&self, app: &Bloat) {
        let pkg = app.package.to_string();
        let w = self.me.clone();
        self.ctx.device_op("debloat_remove", app.package, &t!("{} removed", app.name), move |adb, s| adb.uninstall_for_user(&s, &pkg).map(|r| r.summary()), move |_| {
            if let Some(t) = w.upgrade() {
                t.load();
            }
        });
    }

    fn remove_many(&self, parent: &gtk::Button, apps: Vec<Bloat>) {
        let names = apps.iter().map(|b| format!("• {}", b.name)).collect::<Vec<_>>().join("\n");
        let w = self.me.clone();
        util::confirm(parent, &t!("Remove {} apps?", apps.len()), &t!("{}\n\nYou can restore them from “Removed for your user”.", names), tr("Remove"), true, move || {
            let Some(t) = w.upgrade() else { return };
            let (Some(adb), Some(serial)) = (t.ctx.adb(), t.ctx.ready_serial()) else { return };
            t.status.set_text(tr("Removing…"));
            let w2 = t.me.clone();
            util::run_bg(
                move || apps.into_iter().map(|b| (b.package, adb.uninstall_for_user(&serial, b.package).map(|r| r.summary()))).collect::<Vec<_>>(),
                move |results| {
                    let Some(t) = w2.upgrade() else { return };
                    let ok = results.iter().filter(|(_, r)| r.is_ok()).count();
                    for (pkg, r) in &results {
                        match r {
                            Ok(d) => t.ctx.audit("debloat_remove", Some(pkg), true, Some(d)),
                            Err(e) => t.ctx.audit("debloat_remove", Some(pkg), false, Some(&e.to_string())),
                        }
                    }
                    t.ctx.toast(&t!("{} of {} apps removed", ok, results.len()));
                    t.load();
                },
            );
        });
    }

    fn restore(&self, pkg: &str) {
        let p = pkg.to_string();
        let w = self.me.clone();
        self.ctx.device_op("debloat_restore", pkg, tr("App restored"), move |adb, s| adb.reinstall_existing(&s, &p).map(|r| r.summary()), move |_| {
            if let Some(t) = w.upgrade() {
                t.load();
            }
        });
    }
}

impl Page for DebloatPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn shown(&self) {
        if self.state.borrow().is_none() || *self.read_for.borrow() != self.ctx.ready_serial() {
            self.load();
        }
    }
    fn on_event(&self, ev: &Event) {
        if matches!(ev, Event::Devices) && self.root.is_mapped() && *self.read_for.borrow() != self.ctx.ready_serial() {
            self.load();
        }
    }
}
