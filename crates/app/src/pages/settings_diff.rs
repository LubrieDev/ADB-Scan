//! Phone changes → Settings: snapshots of the phone's settings kept on the
//! computer and what changed since one of them (see
//! `adbscan_core::settings_snapshot`). Restoring a key changes the phone,
//! after confirming, and is recorded in History.

use adbscan_core::{format, i18n::tr, t};
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adbscan_core::settings_snapshot::{self as snap, Change, Snapshot};

use crate::ctx::Ctx;
use crate::util;
use crate::widgets::{RowItem, RowList};

pub struct SettingsDiff {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    pub root: gtk::Box,
    status: gtk::Label,
    compare: gtk::DropDown,
    save_btn: gtk::Button,
    list: RowList,
    snapshots: RefCell<Vec<Snapshot>>,
    current: RefCell<Option<Snapshot>>,
    changes: RefCell<Vec<Change>>,
    busy: Cell<bool>,
    /// Painting the snapshot list must not compare again.
    painting: Cell<bool>,
    read_for: RefCell<Option<String>>,
}

impl SettingsDiff {
    pub fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let intro = util::label(
            tr("Android keeps no dated history of its settings, so ADB Scan keeps snapshots of them on this computer (without identifiers such as the Android ID). Compare one with now to see what an update, an app or another person changed, and put a value back if you want."),
            &["dim"],
        );
        intro.set_wrap(true);
        let compare = gtk::DropDown::from_strings(&[]);
        compare.set_hexpand(true);
        let save_btn = util::pill_button("adb-save-symbolic", tr("Save a snapshot now"));
        save_btn.add_css_class("pill");
        let bar = util::hbox(8);
        bar.append(&util::label(tr("Compare with"), &[]));
        bar.append(&compare);
        bar.append(&save_btn);
        let status = util::label("", &["dim", "caption"]);
        status.set_wrap(true);
        let head = util::vbox(10);
        head.append(&intro);
        head.append(&bar);
        head.append(&status);
        let clamp = adw::Clamp::builder().maximum_size(1000).tightening_threshold(800).child(&head).build();
        clamp.set_margin_top(12);
        clamp.set_margin_start(18);
        clamp.set_margin_end(18);
        let list = RowList::filling(Some(("adb-history-symbolic", tr("Put back the value of the snapshot"))));
        let root = util::vbox(0);
        root.append(&clamp);
        root.append(&list.root);

        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            status,
            compare: compare.clone(),
            save_btn: save_btn.clone(),
            list,
            snapshots: RefCell::new(Vec::new()),
            current: RefCell::new(None),
            changes: RefCell::new(Vec::new()),
            busy: Cell::new(false),
            painting: Cell::new(false),
            read_for: RefCell::new(None),
        });
        let w = this.me.clone();
        compare.connect_selected_notify(move |_| {
            if let Some(t) = w.upgrade() {
                if !t.painting.get() {
                    t.render();
                }
            }
        });
        let w = this.me.clone();
        save_btn.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.load(true);
            }
        });
        let w = this.me.clone();
        this.list.connect_action(move |b, idx| {
            if let (Some(t), Ok(i)) = (w.upgrade(), idx.parse::<usize>()) {
                t.restore(b, i);
            }
        });
        this
    }

    /// Reads the settings now; `save`: also keeps them as a new snapshot. The
    /// first time for a phone, the first snapshot is saved by itself.
    fn load(&self, save: bool) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        if self.busy.replace(true) {
            return;
        }
        self.save_btn.set_sensitive(false);
        self.status.set_text(tr("Reading the settings…"));
        *self.read_for.borrow_mut() = Some(serial.clone());
        let w = self.me.clone();
        util::run_bg(
            move || {
                let now = adb.settings_snapshot(&serial)?;
                let mut saved = snap::load(&serial);
                let first = saved.is_empty();
                if save || first {
                    saved = snap::save(&serial, now.clone()).map_err(|e| adbscan_core::Error::Invalid(e.to_string()))?;
                }
                Ok::<_, adbscan_core::Error>((now, saved, first, save))
            },
            move |r| {
                let Some(t) = w.upgrade() else { return };
                t.busy.set(false);
                t.save_btn.set_sensitive(true);
                match r {
                    Ok((now, saved, first, saved_now)) => {
                        *t.current.borrow_mut() = Some(now);
                        *t.snapshots.borrow_mut() = saved;
                        t.fill_snapshots();
                        t.render();
                        if first {
                            t.status.set_text(tr("First snapshot saved now: from here on you will see what changes."));
                        } else if saved_now {
                            t.ctx.toast(tr("Snapshot of the settings saved"));
                        }
                    }
                    Err(e) => t.status.set_text(&t!("Could not read the settings: {}", e)),
                }
            },
        );
    }

    fn fill_snapshots(&self) {
        self.painting.set(true);
        let names: Vec<String> = self.snapshots.borrow().iter().rev().map(|s| t!("{} · {} settings", format::date(Some(s.taken_at)), s.key_count())).collect();
        self.compare.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
        // The oldest by default: everything that changed since the first one.
        self.compare.set_selected(names.len().saturating_sub(1) as u32);
        self.painting.set(false);
    }

    /// The snapshot chosen in the list (newest first in the list).
    fn chosen(&self) -> Option<Snapshot> {
        let all = self.snapshots.borrow();
        let i = self.compare.selected() as usize;
        all.len().checked_sub(1 + i).and_then(|j| all.get(j)).cloned()
    }

    pub fn render(&self) {
        let (Some(old), Some(now)) = (self.chosen(), self.current.borrow().clone()) else {
            return;
        };
        let changes = snap::diff(&old, &now);
        let shown = |v: &Option<String>| match v {
            Some(v) if v.is_empty() => tr("(empty)").to_string(),
            Some(v) => self.ctx.shown(v),
            None => tr("(did not exist)").to_string(),
        };
        let mut items = Vec::new();
        let mut ns = String::new();
        for (i, c) in changes.iter().enumerate() {
            if c.namespace != ns {
                items.push(RowItem::heading(snap::namespace_label(&c.namespace)));
                ns = c.namespace.clone();
            }
            let mut item = RowItem::new(c.key.clone(), format!("{} → {}", shown(&c.before), shown(&c.after)));
            if c.restorable() {
                item.action = i.to_string();
            } else {
                item.tooltip = tr("Not restored here: it could cut the connection or undo the phone's setup").into();
            }
            items.push(item);
        }
        if changes.is_empty() {
            items.push(RowItem::new(tr("Nothing changed since that snapshot"), "").with_icon("adb-emblem-ok-symbolic", "severity-ok"));
        }
        self.status.set_text(&t!("{} changes since {}.", changes.len(), format::date(Some(old.taken_at))));
        *self.changes.borrow_mut() = changes;
        self.list.set_items(items);
    }

    fn restore(&self, button: &gtk::Button, index: usize) {
        let Some(change) = self.changes.borrow().get(index).cloned() else { return };
        let before = change.before.clone().map(|v| self.ctx.shown(&v)).unwrap_or_else(|| tr("(did not exist)").into());
        let w = self.me.clone();
        util::confirm(
            button,
            &t!("Put back “{}”?", change.key),
            &t!("{} → {}\n\nThe phone takes the value it had in the snapshot. Some settings only take effect after a restart.", self.ctx.shown(change.after.as_deref().unwrap_or("—")), before),
            tr("Put back"),
            false,
            move || {
                let Some(t) = w.upgrade() else { return };
                let target = format!("{} {}", change.namespace, change.key);
                let w2 = t.me.clone();
                t.ctx.device_op("restore_setting", &target, &t!("“{}” put back", change.key), move |adb, s| adb.restore_setting(&s, &change).map(|r| r.summary()), move |ok| {
                    if let (true, Some(t)) = (ok, w2.upgrade()) {
                        t.load(false);
                    }
                });
            },
        );
    }

    pub fn shown(&self) {
        if self.current.borrow().is_none() || *self.read_for.borrow() != self.ctx.ready_serial() {
            self.load(false);
        }
    }
}
