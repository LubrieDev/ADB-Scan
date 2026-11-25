//! Local history: sessions, events, logcat captures and audit log.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adbscan_core::db::{AuditRecord, EventRecord, LogcatSessionRecord, SessionRecord};
use adbscan_core::format;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::widgets::{RowItem, RowList};
use crate::window::Page;

/// What was shown last time: if it does not change, nothing is rebuilt.
type Snapshot = (Vec<SessionRecord>, Vec<EventRecord>, Vec<(LogcatSessionRecord, bool)>, Vec<AuditRecord>);

struct HistoryPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::Box,
    // Virtualised lists: only the widgets of the visible rows exist, so opening
    // the page costs the same with 20 rows as with 1,000.
    sessions: RowList,
    events: RowList,
    audit: RowList,
    // Few rows, each with its own buttons (open, delete).
    logcat: gtk::ListBox,
    shown_data: RefCell<Option<Snapshot>>,
    loading: Cell<bool>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    HistoryPage::new(ctx)
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

impl HistoryPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let stack = adw::ViewStack::new();
        let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();
        let (sessions, events, audit) = (RowList::filling(None), RowList::filling(None), RowList::filling(None));
        let logcat = gtk::ListBox::new();
        logcat.add_css_class("boxed-list");
        logcat.set_selection_mode(gtk::SelectionMode::None);
        stack.add_titled_with_icon(&sessions.root, Some("sessions"), tr("Connections"), "adb-smartphone-symbolic");
        stack.add_titled_with_icon(&util::page_scroller(&logcat, 1000), Some("logcat"), tr("Logcat captures"), "adb-scroll-text-symbolic");
        stack.add_titled_with_icon(&audit.root, Some("audit"), tr("Operations"), "adb-wrench-symbolic");
        stack.add_titled_with_icon(&events.root, Some("events"), tr("Events"), "adb-list-symbolic");
        let top = util::hbox(8);
        top.set_margin_top(10);
        top.set_margin_start(12);
        top.set_margin_end(12);
        switcher.set_hexpand(true);
        top.append(&switcher);
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Refresh"));
        let wipe = util::pill_button("adb-edit-delete-symbolic", tr("Clear history"));
        // Red but not filled: it is destructive (and asks for confirmation),
        // not the page's main action.
        wipe.add_css_class("destructive-action");
        wipe.add_css_class("flat");
        top.append(&refresh);
        top.append(&wipe);
        let root = util::vbox(0);
        root.append(&top);
        stack.set_vexpand(true);
        root.append(&stack);

        let this = Rc::new_cyclic(|me| Self { me: me.clone(), ctx: ctx.clone(), root, sessions, events, audit, logcat, shown_data: RefCell::new(None), loading: Cell::new(false) });
        {
            let w = this.me.clone();
            refresh.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    t.load();
                }
            });
        }
        {
            let w = this.me.clone();
            wipe.connect_clicked(move |b| {
                let w = w.clone();
                util::confirm(
                    b,
                    tr("Clear the history?"),
                    tr("Sessions, events, battery samples and the operations log will be deleted. Logcat captures saved on disk are kept."),
                    tr("Clear"),
                    true,
                    move || {
                        let Some(t) = w.upgrade() else { return };
                        if let Some(h) = &t.ctx.history {
                            let _ = h.clear(false);
                        }
                        t.ctx.toast(tr("History cleared"));
                        t.load();
                    },
                );
            });
        }
        // No loading here: `shown()` arrives right after the page is built.
        this
    }

    /// Query in the background; it only touches the interface if something
    /// changed.
    fn load(&self) {
        let Some(h) = self.ctx.history.clone() else {
            return;
        };
        // Several requests in a row (e.g. device notices at startup) are
        // resolved with a single query.
        if self.loading.replace(true) {
            return;
        }
        let w = self.me.clone();
        util::run_bg(
            move || -> Snapshot {
                let logcat = h
                    .logcat_sessions(200)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|l| {
                        // Check the disk here, not on the UI thread.
                        let exists = std::path::Path::new(&l.filename).exists();
                        (l, exists)
                    })
                    .collect();
                (h.sessions(300).unwrap_or_default(), h.events(400).unwrap_or_default(), logcat, h.audit_log(400).unwrap_or_default())
            },
            move |data| {
                let Some(t) = w.upgrade() else { return };
                t.loading.set(false);
                if t.shown_data.borrow().as_ref() == Some(&data) {
                    return;
                }
                let t0 = std::time::Instant::now();
                t.render(&data);
                util::log_slow("history: show", t0);
                *t.shown_data.borrow_mut() = Some(data);
            },
        );
    }

    fn render(&self, (sessions, events, logcat, audit): &Snapshot) {
        let ctx = &self.ctx;
        // Serials of old phones: they are hidden inside the texts too.
        for serial in sessions.iter().map(|s| s.serial.as_str()).chain(events.iter().filter_map(|e| e.serial.as_deref())).chain(audit.iter().filter_map(|a| a.serial.as_deref())) {
            ctx.learn_private(serial);
        }
        let empty = |title: &str, subtitle: &str| RowItem::new(title, subtitle).with_icon("adb-info-symbolic", "dim");

        let mut rows: Vec<RowItem> = sessions
            .iter()
            .map(|s| {
                // Connected now: “ongoing”, not the duration until the last
                // heartbeat.
                let dur = match s.disconnected_at {
                    Some(end) => format::human_duration(Some(end - s.connected_at)),
                    None => tr("ongoing").into(),
                };
                let serial = ctx.shown_value(&s.serial);
                let title = if s.model.is_empty() { serial } else { format!("{} · {serial}", ctx.shown_device_name(&s.model)) };
                let (icon, class) = if s.disconnected_at.is_none() { ("adb-link-symbolic", "severity-ok") } else { ("adb-smartphone-symbolic", "dim") };
                RowItem::new(title, format!("{} · {}", format::timestamp(Some(s.connected_at)), dur)).with_icon(icon, class)
            })
            .collect();
        if rows.is_empty() {
            rows.push(empty(tr("No recorded connections"), ""));
        }
        self.sessions.set_items(rows);

        let mut rows: Vec<RowItem> = audit
            .iter()
            .map(|a| {
                let title = ctx.shown(&format!("{}{}", a.action, a.target.as_ref().map(|t| format!(" · {t}")).unwrap_or_default()));
                let detail = a.detail.as_ref().filter(|d| !d.is_empty()).map(|d| format!(" · {}", format::truncate(&ctx.shown(d), 140))).unwrap_or_default();
                let subtitle = format!("{} · {}{detail}", format::timestamp(Some(a.timestamp)), ctx.shown_value(a.serial.as_deref().unwrap_or_default()));
                let mut item = RowItem::new(title, subtitle);
                // The full detail, in case the subtitle is cut off.
                item.tooltip = ctx.shown(a.detail.as_deref().unwrap_or_default());
                if a.ok {
                    item.with_icon("adb-emblem-ok-symbolic", "severity-ok")
                } else {
                    item.with_icon("adb-circle-x-symbolic", "severity-high")
                }
            })
            .collect();
        if rows.is_empty() {
            rows.push(empty(tr("No recorded operations"), tr("Installs, deletions, flashing and other actions on the phone appear here")));
        }
        self.audit.set_items(rows);

        let mut rows: Vec<RowItem> = events
            .iter()
            .map(|e| {
                let subtitle = format!("{} · {}{}", format::timestamp(Some(e.timestamp)), e.category, e.serial.as_ref().map(|s| format!(" · {}", ctx.shown_value(s))).unwrap_or_default());
                let (icon, class) = match e.level.as_str() {
                    "error" => ("adb-circle-x-symbolic", "severity-high"),
                    "warning" => ("adb-dialog-warning-symbolic", "severity-medium"),
                    _ => ("adb-info-symbolic", "dim"),
                };
                RowItem::new(ctx.shown(&e.message), subtitle).with_icon(icon, class)
            })
            .collect();
        if rows.is_empty() {
            rows.push(empty(tr("No recorded events"), ""));
        }
        self.events.set_items(rows);

        while let Some(c) = self.logcat.first_child() {
            self.logcat.remove(&c);
        }
        if logcat.is_empty() {
            self.logcat.append(&adw::ActionRow::builder().title(tr("No saved captures")).subtitle(tr("They are saved automatically when using Logcat (configurable in Preferences)")).build());
        }
        for (l, exists) in logcat {
            let exists = *exists;
            let path = std::path::PathBuf::from(&l.filename);
            let row = adw::ActionRow::builder()
                .title(esc(&ctx.shown(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())))
                .subtitle(esc(&t!("{} · {} lines · {}{}", format::timestamp(Some(l.started_at)), format::number(Some(l.lines as f64), 0), ctx.shown_device_name(&l.model), if exists { "" } else { tr(" · file deleted") })))
                .activatable(exists)
                .build();
            if exists {
                let p = path.clone();
                row.connect_activated(move |_| util::open_path(&p));
            }
            let del = util::icon_button("adb-edit-delete-symbolic", tr("Delete capture"));
            del.add_css_class("flat");
            del.set_valign(gtk::Align::Center);
            let w2 = self.me.clone();
            let id = l.id;
            del.connect_clicked(move |b| {
                let (w3, p2) = (w2.clone(), path.clone());
                util::confirm(b, tr("Delete the capture?"), tr("The file will be deleted from the disk."), tr("Delete"), true, move || {
                    let Some(t) = w3.upgrade() else { return };
                    let _ = std::fs::remove_file(&p2);
                    if let Some(h) = &t.ctx.history {
                        let _ = h.delete_logcat(id);
                    }
                    t.load();
                });
            });
            row.add_suffix(&del);
            self.logcat.append(&row);
        }
    }
}

impl Page for HistoryPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn shown(&self) {
        self.load();
    }
    fn on_event(&self, ev: &Event) {
        // Same data, another way of showing it: repainted without querying.
        if matches!(ev, Event::Privacy) {
            let data = self.shown_data.borrow().clone();
            if let Some(data) = data {
                self.render(&data);
            }
        }
        // Connections and disconnections add entries to the history.
        if matches!(ev, Event::Devices) && self.root.is_mapped() {
            self.load();
        }
    }
}
