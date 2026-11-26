//! Storage analyzer: one scan of the shared storage, then the space by
//! type, well-known places, a folder browser, the largest files and
//! possible duplicates (see `adbscan_core::storage`).

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use adbscan_core::format;
use adbscan_core::storage::{self, Node, Scan};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

/// Rows shown at most in a folder (the rest is summed up in one line).
const MAX_ROWS: usize = 300;

struct StoragePage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::Box,
    scan_btn: gtk::Button,
    cancel_btn: gtk::Button,
    status: gtk::Label,
    tabs: adw::ViewStack,
    overview: gtk::Box,
    folder_path: gtk::Label,
    folder_up: gtk::Button,
    folder_list: gtk::ListBox,
    largest: gtk::ListBox,
    dupes: gtk::Box,
    scan: RefCell<Option<Scan>>,
    cwd: RefCell<String>,
    running: RefCell<Option<Arc<adbscan_core::exec::StreamHandle>>>,
    scanned_for: RefCell<Option<String>>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    StoragePage::new(ctx)
}

fn size(kib: u64) -> String {
    format::human_kib(Some(kib as i64))
}

fn boxed_list() -> gtk::ListBox {
    let l = gtk::ListBox::new();
    l.add_css_class("boxed-list");
    l.set_selection_mode(gtk::SelectionMode::None);
    l
}

fn clear(w: &impl IsA<gtk::Widget>) {
    let w = w.as_ref();
    if let Some(l) = w.downcast_ref::<gtk::ListBox>() {
        l.remove_all();
        return;
    }
    if let Some(b) = w.downcast_ref::<gtk::Box>() {
        while let Some(c) = b.first_child() {
            b.remove(&c);
        }
    }
}

/// Bar with the fraction of `total` a size takes.
fn share_bar(kib: u64, total: u64) -> gtk::LevelBar {
    let bar = gtk::LevelBar::builder().min_value(0.0).max_value(1.0).value(if total > 0 { kib as f64 / total as f64 } else { 0.0 }).valign(gtk::Align::Center).build();
    bar.set_width_request(120);
    bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_LOW));
    bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_HIGH));
    bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_FULL));
    bar
}

impl StoragePage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let head = util::vbox(8);
        let intro = adw::PreferencesGroup::builder()
            .title(tr("Shared storage"))
            .description(tr("Finds what takes up space in the phone's shared storage (photos, downloads, chats, app files). The scan only reads, and takes about a minute for every 50 GB. Nothing is deleted without your confirmation, one file at a time."))
            .build();
        let scan_btn = util::pill_button("adb-search-symbolic", tr("Scan"));
        scan_btn.add_css_class("suggested-action");
        scan_btn.add_css_class("pill");
        scan_btn.set_valign(gtk::Align::Center);
        let cancel_btn = gtk::Button::with_label(tr("Cancel"));
        cancel_btn.add_css_class("pill");
        cancel_btn.set_valign(gtk::Align::Center);
        cancel_btn.set_visible(false);
        let buttons = util::hbox(6);
        buttons.append(&cancel_btn);
        buttons.append(&scan_btn);
        intro.set_header_suffix(Some(&buttons));
        let status = util::label(tr("Not scanned yet."), &["dim", "caption"]);
        status.set_wrap(true);
        head.append(&intro);
        head.append(&status);

        let tabs = adw::ViewStack::new();
        let switcher = adw::ViewSwitcher::builder().stack(&tabs).policy(adw::ViewSwitcherPolicy::Wide).build();
        switcher.set_halign(gtk::Align::Center);

        let overview = util::vbox(18);
        tabs.add_titled_with_icon(&util::page_scroller(&overview, 900), Some("overview"), tr("Summary"), "adb-chart-line-symbolic");

        let fbox = util::vbox(10);
        let nav = util::hbox(8);
        let folder_up = util::icon_button("adb-go-up-symbolic", tr("Parent folder"));
        let folder_path = util::label(storage::ROOT, &["dim"]);
        folder_path.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        folder_path.set_hexpand(true);
        nav.append(&folder_up);
        nav.append(&folder_path);
        let folder_list = boxed_list();
        fbox.append(&nav);
        fbox.append(&folder_list);
        tabs.add_titled_with_icon(&util::page_scroller(&fbox, 900), Some("folders"), tr("Folders"), "adb-folder-symbolic");

        let largest = boxed_list();
        tabs.add_titled_with_icon(&util::page_scroller(&largest, 900), Some("largest"), tr("Largest files"), "adb-file-text-symbolic");

        let dupes = util::vbox(12);
        tabs.add_titled_with_icon(&util::page_scroller(&dupes, 900), Some("dupes"), tr("Duplicates"), "adb-copy-symbolic");

        // Same width and tightening as the tabs' content (`page_scroller`).
        let top = adw::Clamp::builder().maximum_size(900).tightening_threshold(700).child(&head).build();
        top.set_margin_top(18);
        top.set_margin_start(18);
        top.set_margin_end(18);
        let root = util::vbox(8);
        root.append(&top);
        root.append(&switcher);
        root.append(&tabs);
        tabs.set_vexpand(true);
        tabs.set_sensitive(false);

        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            scan_btn: scan_btn.clone(),
            cancel_btn: cancel_btn.clone(),
            status,
            tabs,
            overview,
            folder_path,
            folder_up: folder_up.clone(),
            folder_list,
            largest,
            dupes,
            scan: RefCell::new(None),
            cwd: RefCell::new(storage::ROOT.to_string()),
            running: RefCell::new(None),
            scanned_for: RefCell::new(None),
        });
        let w = this.me.clone();
        scan_btn.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.start_scan();
            }
        });
        let w = this.me.clone();
        cancel_btn.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                if let Some(h) = t.running.borrow().as_ref() {
                    h.cancel();
                }
            }
        });
        let w = this.me.clone();
        folder_up.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                let cwd = t.cwd.borrow().clone();
                if cwd != storage::ROOT {
                    t.open_folder(&adbscan_core::adb::remote_parent(&cwd));
                }
            }
        });
        this
    }

    fn start_scan(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        if self.running.borrow().is_some() {
            return;
        }
        let lines = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        let out = Arc::new(Mutex::new(String::new()));
        let (l2, d2, c2, o2) = (lines.clone(), done.clone(), cancelled.clone(), out.clone());
        let handle = adb.shell_stream(&serial, storage::SCAN_SCRIPT, move |ev| match ev {
            adbscan_core::exec::StreamEvent::Stdout(line) => {
                l2.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut o) = o2.lock() {
                    o.push_str(&line);
                    o.push('\n');
                }
            }
            adbscan_core::exec::StreamEvent::Exit { cancelled, .. } => {
                c2.store(cancelled, Ordering::Relaxed);
                d2.store(true, Ordering::Release);
            }
            _ => {}
        });
        let handle = match handle {
            Ok(h) => Arc::new(h),
            Err(e) => {
                self.status.set_text(&t!("Could not start the scan: {}", e));
                return;
            }
        };
        *self.running.borrow_mut() = Some(handle);
        *self.scanned_for.borrow_mut() = Some(serial);
        self.scan_btn.set_sensitive(false);
        self.cancel_btn.set_visible(true);
        let started = std::time::Instant::now();
        self.status.set_text(tr("Scanning…"));
        let w = self.me.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
            let Some(t) = w.upgrade() else { return glib::ControlFlow::Break };
            let n = lines.load(Ordering::Relaxed);
            if !done.load(Ordering::Acquire) {
                t.status.set_text(&t!("Scanning… {} files and folders read ({} s)", n, started.elapsed().as_secs()));
                return glib::ControlFlow::Continue;
            }
            *t.running.borrow_mut() = None;
            t.scan_btn.set_sensitive(true);
            t.cancel_btn.set_visible(false);
            if cancelled.load(Ordering::Relaxed) {
                t.status.set_text(tr("Scan cancelled."));
                return glib::ControlFlow::Break;
            }
            let text = std::mem::take(&mut *out.lock().unwrap());
            let secs = started.elapsed().as_secs();
            let w2 = t.me.clone();
            util::run_bg(
                move || Scan::parse(&text),
                move |scan| {
                    let Some(t) = w2.upgrade() else { return };
                    if scan.is_empty() {
                        t.status.set_text(tr("The scan returned nothing: the shared storage may not be readable."));
                        return;
                    }
                    t.status.set_text(&t!("{} in {} files, scanned in {} s.", size(scan.total_kib()), scan.file_count(), secs));
                    *t.scan.borrow_mut() = Some(scan);
                    t.tabs.set_sensitive(true);
                    t.render_all();
                },
            );
            glib::ControlFlow::Break
        });
    }

    fn render_all(&self) {
        self.render_overview();
        let cwd = self.cwd.borrow().clone();
        self.open_folder(&cwd);
        self.render_largest();
        self.render_dupes();
    }

    fn render_overview(&self) {
        clear(&self.overview);
        let scan = self.scan.borrow();
        let Some(scan) = scan.as_ref() else { return };
        let total = scan.total_kib();
        let types = adw::PreferencesGroup::builder().title(tr("By type")).build();
        for (ft, kib, n) in scan.by_type() {
            let row = adw::ActionRow::builder().title(ft.label()).subtitle(t!("{} files", n)).build();
            row.add_suffix(&util::label(&size(kib), &["numeric"]));
            row.add_suffix(&share_bar(kib, total));
            types.add(&row);
        }
        self.overview.append(&types);
        let places = adw::PreferencesGroup::builder().title(tr("Places that grow")).description(tr("Folders that fill up without anyone noticing. Open one to see what is inside.")).build();
        let found = scan.places();
        if found.is_empty() {
            places.add(&adw::ActionRow::builder().title(tr("None of these places takes up space")).build());
        }
        for p in found {
            let row = adw::ActionRow::builder().title(p.title).subtitle(glib::markup_escape_text(&format!("{} · {}", p.hint, p.path))).activatable(true).build();
            row.set_subtitle_lines(3);
            row.add_suffix(&util::label(&size(p.kib), &["numeric"]));
            row.add_suffix(&util::icon("go-next-symbolic"));
            let (w, path) = (self.me.clone(), p.path.clone());
            row.connect_activated(move |_| {
                if let Some(t) = w.upgrade() {
                    t.open_folder(&path);
                    t.tabs.set_visible_child_name("folders");
                }
            });
            places.add(&row);
        }
        self.overview.append(&places);
    }

    fn open_folder(&self, path: &str) {
        *self.cwd.borrow_mut() = path.to_string();
        self.folder_path.set_text(&self.ctx.shown(path));
        self.folder_up.set_sensitive(path != storage::ROOT);
        self.folder_list.remove_all();
        let scan = self.scan.borrow();
        let Some(scan) = scan.as_ref() else { return };
        let total = scan.size_of(path).unwrap_or(0);
        let nodes = scan.children(path);
        if nodes.is_empty() {
            self.folder_list.append(&adw::ActionRow::builder().title(tr("Empty folder")).build());
        }
        for n in nodes.iter().take(MAX_ROWS) {
            self.folder_list.append(&self.node_row(n, total, false));
        }
        if nodes.len() > MAX_ROWS {
            let rest: u64 = nodes[MAX_ROWS..].iter().map(|n| n.kib).sum();
            self.folder_list.append(&adw::ActionRow::builder().title(t!("{} more items", nodes.len() - MAX_ROWS)).subtitle(size(rest)).build());
        }
    }

    /// A file or folder. `with_path`: shows the folder it is in (lists that
    /// mix folders).
    fn node_row(&self, n: &Node, total: u64, with_path: bool) -> adw::ActionRow {
        let sub = if with_path { self.ctx.shown(&adbscan_core::adb::remote_parent(&n.path)) } else { String::new() };
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(&self.ctx.shown(&n.name))).build();
        if !sub.is_empty() {
            row.set_subtitle(&glib::markup_escape_text(&sub));
        }
        row.add_prefix(&util::icon(if n.is_dir { "adb-folder-symbolic" } else { "adb-file-text-symbolic" }));
        row.add_suffix(&util::label(&size(n.kib), &["numeric"]));
        if total > 0 {
            row.add_suffix(&share_bar(n.kib, total));
        }
        if n.is_dir {
            row.set_activatable(true);
            row.add_suffix(&util::icon("go-next-symbolic"));
            let (w, path) = (self.me.clone(), n.path.clone());
            row.connect_activated(move |_| {
                if let Some(t) = w.upgrade() {
                    t.open_folder(&path);
                }
            });
        } else if storage::deletable(&n.path) {
            let del = util::icon_button("adb-edit-delete-symbolic", tr("Delete this file"));
            del.add_css_class("flat");
            del.set_valign(gtk::Align::Center);
            let (w, node) = (self.me.clone(), n.clone());
            del.connect_clicked(move |b| {
                if let Some(t) = w.upgrade() {
                    t.delete(b, &node);
                }
            });
            row.add_suffix(&del);
        }
        row
    }

    fn render_largest(&self) {
        self.largest.remove_all();
        let scan = self.scan.borrow();
        let Some(scan) = scan.as_ref() else { return };
        let total = scan.total_kib();
        for n in scan.largest_files(100) {
            self.largest.append(&self.node_row(&n, total, true));
        }
    }

    fn render_dupes(&self) {
        clear(&self.dupes);
        let scan = self.scan.borrow();
        let Some(scan) = scan.as_ref() else { return };
        // From 1 MiB: smaller ones are not worth the time.
        let dupes = scan.duplicates(1024);
        let freeable: u64 = dupes.iter().map(|d| d.reclaimable()).sum();
        let group = adw::PreferencesGroup::builder()
            .title(tr("Possible duplicates"))
            .description(t!("Files of 1 MB or more with the same name and size in different folders: {} could be freed. Check them before deleting: they are probably, not certainly, the same file.", size(freeable)))
            .build();
        if dupes.is_empty() {
            group.add(&adw::ActionRow::builder().title(tr("No duplicates found")).build());
        }
        for d in dupes.iter().take(200) {
            let exp = adw::ExpanderRow::builder()
                .title(glib::markup_escape_text(&self.ctx.shown(&d.name)))
                .subtitle(t!("{} copies of {} · {} could be freed", d.paths.len(), size(d.kib), size(d.reclaimable())))
                .build();
            for p in &d.paths {
                let node = Node { path: p.clone(), name: p.rsplit('/').next().unwrap_or(p).to_string(), kib: d.kib, is_dir: false };
                exp.add_row(&self.node_row(&node, 0, true));
            }
            group.add(&exp);
        }
        self.dupes.append(&group);
    }

    fn delete(&self, button: &gtk::Button, n: &Node) {
        let (w, path) = (self.me.clone(), n.path.clone());
        util::confirm(button, &t!("Delete {}?", self.ctx.shown(&n.name)), &t!("{} · {}\n\nThe file is deleted from the phone for good: the shared storage has no recycle bin.", self.ctx.shown(&n.path), size(n.kib)), tr("Delete"), true, move || {
            let Some(t) = w.upgrade() else { return };
            let (p2, w2, target) = (path.clone(), w.clone(), path.clone());
            t.ctx.device_op("storage_delete", &target, tr("File deleted"), move |adb, s| adb.delete_shared_file(&s, &p2).map(|r| r.summary()), move |ok| {
                let Some(t) = w2.upgrade() else { return };
                if ok {
                    if let Some(scan) = t.scan.borrow_mut().as_mut() {
                        scan.remove_file(&path);
                    }
                    t.render_all();
                }
            });
        });
    }
}

impl Page for StoragePage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn on_event(&self, ev: &Event) {
        // Another phone: the scan belongs to the previous one.
        if matches!(ev, Event::Devices) && self.scan.borrow().is_some() && *self.scanned_for.borrow() != self.ctx.ready_serial() {
            *self.scan.borrow_mut() = None;
            *self.cwd.borrow_mut() = storage::ROOT.to_string();
            self.tabs.set_sensitive(false);
            self.status.set_text(tr("Not scanned yet."));
            self.render_all();
        }
        if matches!(ev, Event::Privacy) && self.scan.borrow().is_some() {
            self.render_all();
        }
    }
}
