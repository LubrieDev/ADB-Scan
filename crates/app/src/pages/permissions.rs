//! Permissions: which apps can use each sensitive permission and special
//! access, and taking it away from one of them (see
//! `adbscan_core::permissions`).

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use adbscan_core::permissions::{Access, Holder, Kind, Overview};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

struct PermissionsPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    body: gtk::Box,
    status: gtk::Label,
    system: gtk::CheckButton,
    data: RefCell<Option<Overview>>,
    /// Accesses left open, kept open after a reload.
    expanded: RefCell<HashSet<&'static str>>,
    busy: std::cell::Cell<bool>,
    read_for: RefCell<Option<String>>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    PermissionsPage::new(ctx)
}

impl PermissionsPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);
        let intro = adw::PreferencesGroup::builder()
            .title(tr("Who can do what"))
            .description(tr("Each sensitive permission and special access, with the apps that have it. Taking one away is the same as turning it off in Settings: the app can ask for it again, and you can grant it again from Settings or from Apps."))
            .build();
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Read again"));
        refresh.add_css_class("flat");
        refresh.set_valign(gtk::Align::Center);
        intro.set_header_suffix(Some(&refresh));
        let system = gtk::CheckButton::with_label(tr("Include preinstalled apps"));
        system.set_tooltip_text(Some(tr("System apps usually need their permissions: change them only if you know what they are for.")));
        let status = util::label("", &["dim", "caption"]);
        status.set_wrap(true);
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
            expanded: RefCell::new(HashSet::new()),
            busy: std::cell::Cell::new(false),
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
        self.status.set_text(tr("Reading the permissions of every app…"));
        *self.read_for.borrow_mut() = Some(serial.clone());
        let w = self.me.clone();
        util::run_bg(
            move || adb.permission_overview(&serial),
            move |r| {
                let Some(t) = w.upgrade() else { return };
                t.busy.set(false);
                match r {
                    Ok(o) => {
                        *t.data.borrow_mut() = Some(o);
                        t.render();
                    }
                    Err(e) => t.status.set_text(&t!("Could not read the permissions: {}", e)),
                }
            },
        );
    }

    fn render(&self) {
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
        let data = self.data.borrow();
        let Some(data) = data.as_ref() else { return };
        let with_system = self.system.is_active();
        let labels = crate::pages::apps::cached_labels();
        let runtime = adw::PreferencesGroup::builder().title(tr("Permissions")).build();
        let special = adw::PreferencesGroup::builder().title(tr("Special access")).description(tr("Accesses granted from Settings → Apps → Special app access. They are more powerful than ordinary permissions.")).build();
        let mut apps = HashSet::new();
        for (access, holders) in &data.accesses {
            let shown: Vec<&Holder> = holders.iter().filter(|h| with_system || !h.system).collect();
            apps.extend(shown.iter().map(|h| h.package.clone()));
            let exp = adw::ExpanderRow::builder()
                .title(access.title)
                .subtitle(glib::markup_escape_text(&format!("{} · {}", match shown.len() {
                    0 => tr("No app").to_string(),
                    1 => tr("1 app").to_string(),
                    n => t!("{} apps", n),
                }, access.description)))
                .build();
            exp.set_subtitle_lines(2);
            exp.add_prefix(&util::icon(access.icon));
            exp.set_enable_expansion(!shown.is_empty());
            let id = access.id;
            if self.expanded.borrow().contains(id) && !shown.is_empty() {
                exp.set_expanded(true);
            }
            let rows: Vec<(String, Holder)> = shown.iter().map(|h| (labels.get(&h.package).cloned().unwrap_or_else(|| h.package.clone()), (*h).clone())).collect();
            for (label, h) in rows {
                exp.add_row(&self.holder_row(access, &label, &h));
            }
            let w = self.me.clone();
            exp.connect_expanded_notify(move |e| {
                if let Some(t) = w.upgrade() {
                    if e.is_expanded() {
                        t.expanded.borrow_mut().insert(id);
                    } else {
                        t.expanded.borrow_mut().remove(id);
                    }
                }
            });
            if access.is_special() { &special } else { &runtime }.add(&exp);
        }
        self.status.set_text(&t!("{} apps have at least one of these permissions{}.", apps.len(), if with_system { "" } else { tr(" (preinstalled apps hidden)") }));
        self.body.append(&special);
        self.body.append(&runtime);
    }

    fn holder_row(&self, access: &Access, label: &str, h: &Holder) -> adw::ActionRow {
        let what = match access.kind {
            Kind::Runtime(_) => h.grants.join(", "),
            Kind::AppOp(_) => h.package.clone(),
            Kind::Accessibility | Kind::NotificationListener => h.grants.iter().map(|c| c.split_once('/').map(|(_, cls)| cls).unwrap_or(c)).collect::<Vec<_>>().join(", "),
        };
        let mut sub = if label == h.package { what } else { format!("{} · {}", h.package, what) };
        if h.system {
            sub.push_str(&format!(" · {}", tr("preinstalled")));
        }
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(label)).subtitle(glib::markup_escape_text(&sub)).build();
        row.set_subtitle_lines(2);
        if h.fixed {
            let l = util::label(tr("Fixed by the system"), &["dim", "caption"]);
            l.set_tooltip_text(Some(tr("Android or a device policy grants it and does not let it be taken away.")));
            row.add_suffix(&l);
            return row;
        }
        let b = gtk::Button::with_label(tr("Take away"));
        b.set_valign(gtk::Align::Center);
        let (w, access, h, label) = (self.me.clone(), access.clone(), h.clone(), label.to_string());
        b.connect_clicked(move |b| {
            let Some(t) = w.upgrade() else { return };
            if h.system || matches!(access.kind, Kind::Accessibility) {
                let (w2, access2, h2) = (w.clone(), access.clone(), h.clone());
                let body = if h.system { tr("It is a preinstalled app: a system feature may stop working until you grant it again.") } else { tr("The app will stop working as an accessibility service until you turn it on again in Settings → Accessibility.") };
                util::confirm(b, &t!("Take “{}” away from {}?", access.title, label), body, tr("Take away"), true, move || {
                    if let Some(t) = w2.upgrade() {
                        t.revoke(&access2, &h2);
                    }
                });
            } else {
                t.revoke(&access, &h);
            }
        });
        row.add_suffix(&b);
        row
    }

    fn revoke(&self, access: &Access, h: &Holder) {
        let (kind, pkg, grants) = (access.kind, h.package.clone(), h.grants.clone());
        let w = self.me.clone();
        let target = format!("{} {}", h.package, access.id);
        self.ctx.device_op(
            "permission_revoke",
            &target,
            &t!("“{}” taken away", access.title),
            move |adb, s| {
                let mut done = Vec::new();
                match kind {
                    Kind::Runtime(_) => {
                        let mut last_err = None;
                        for g in &grants {
                            match adb.grant_permission(&s, &pkg, &format!("android.permission.{g}"), false) {
                                Ok(_) => done.push(g.clone()),
                                Err(e) => last_err = Some(e),
                            }
                        }
                        if done.is_empty() {
                            if let Some(e) = last_err {
                                return Err(e);
                            }
                        }
                    }
                    Kind::AppOp(op) => {
                        adb.set_app_op(&s, &pkg, op, false)?;
                        done.push(op.to_string());
                    }
                    Kind::Accessibility => {
                        for c in &grants {
                            adb.disable_accessibility_service(&s, c)?;
                            done.push(c.clone());
                        }
                    }
                    Kind::NotificationListener => {
                        for c in &grants {
                            adb.disallow_notification_listener(&s, c)?;
                            done.push(c.clone());
                        }
                    }
                }
                Ok(done.join(", "))
            },
            move |_| {
                if let Some(t) = w.upgrade() {
                    t.load();
                }
            },
        );
    }
}

impl Page for PermissionsPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn shown(&self) {
        if self.data.borrow().is_none() || *self.read_for.borrow() != self.ctx.ready_serial() {
            self.load();
        }
    }
    fn on_event(&self, ev: &Event) {
        if matches!(ev, Event::Devices) && self.root.is_mapped() && *self.read_for.borrow() != self.ctx.ready_serial() {
            self.load();
        }
    }
}
