//! Storage analyzer: one scan of the shared storage, then the space by
//! type, well-known places, a folder browser, the largest files and
//! possible duplicates (see `adbscan_core::storage`).

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use adbscan_core::format;
use adbscan_core::storage::{self, Node, Report, Scan};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

struct StoragePage {
    me: Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::Box,
    scan_btn: gtk::Button,
    cancel_btn: gtk::Button,
    status: gtk::Label,
    tabs: adw::ViewStack,
    overview: gtk::Box,
    folder_path: gtk::Label,
    folder_up: gtk::Button,
    folder_list: NodeList,
    largest: NodeList,
    dupes_intro: gtk::Label,
    dupes: NodeList,
    /// `None` while a deletion updates it in the background.
    scan: RefCell<Option<Scan>>,
    report: RefCell<Option<Report>>,
    cwd: RefCell<String>,
    running: RefCell<Option<Arc<adbscan_core::exec::StreamHandle>>>,
    scanned_for: RefCell<Option<String>>,
    /// Changes with every new scan or reset: late background results of an
    /// older one are dropped.
    generation: Cell<u64>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    StoragePage::new(ctx)
}

fn size(kib: u64) -> String {
    format::human_kib(Some(kib as i64))
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
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

/// One row of a [`NodeList`].
#[derive(Clone)]
enum Item {
    /// A file or folder. `total`: size the bar is relative to (0: no bar).
    /// `with_path`: shows the folder it is in (lists that mix folders).
    /// `nested`: indented under a [`Item::Group`].
    Node { node: Node, total: u64, with_path: bool, nested: bool },
    /// Heading of a group of rows (a set of duplicates).
    Group { title: String, subtitle: String },
    /// A line of text (empty list, nothing found).
    Note(String),
}

fn item_of(o: &glib::Object) -> Option<Item> {
    o.downcast_ref::<glib::BoxedAnyObject>().map(|b| b.borrow::<Item>().clone())
}

/// Virtualised list of files and folders: only the visible rows have
/// widgets, so a folder with 20 000 photos opens as fast as one with ten
/// (a `GtkListBox` creates and measures every row).
struct NodeList {
    root: gtk::ScrolledWindow,
    store: gio::ListStore,
    /// Rows received while the list had no size (hidden tab): a `GtkListView`
    /// without a height would prepare all of them, so they wait until shown.
    pending: Rc<RefCell<Option<Vec<glib::Object>>>>,
}

/// The widgets of a row, in order: icon, texts (title, subtitle), size,
/// bar, "open" arrow and delete button.
fn row_parts(row: &gtk::Widget) -> Option<(gtk::Image, gtk::Label, gtk::Label, gtk::Label, gtk::LevelBar, gtk::Image, gtk::Button)> {
    let icon = row.first_child()?.downcast::<gtk::Image>().ok()?;
    let texts = icon.next_sibling()?;
    let title = texts.first_child()?.downcast::<gtk::Label>().ok()?;
    let subtitle = texts.last_child()?.downcast::<gtk::Label>().ok()?;
    let size = texts.next_sibling()?.downcast::<gtk::Label>().ok()?;
    let bar = size.next_sibling()?.downcast::<gtk::LevelBar>().ok()?;
    let next = bar.next_sibling()?.downcast::<gtk::Image>().ok()?;
    let del = next.next_sibling()?.downcast::<gtk::Button>().ok()?;
    Some((icon, title, subtitle, size, bar, next, del))
}

impl NodeList {
    fn new(ctx: &Rc<Ctx>, page: Weak<StoragePage>) -> Self {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let factory = gtk::SignalListItemFactory::new();
        {
            let page = page.clone();
            factory.connect_setup(move |_, obj| {
                let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
                let row = util::hbox(10);
                row.add_css_class("rowlist-row");
                let icon = gtk::Image::new();
                icon.set_valign(gtk::Align::Center);
                let texts = util::vbox(2);
                texts.set_hexpand(true);
                texts.set_valign(gtk::Align::Center);
                let title = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).build();
                let subtitle = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Start).build();
                subtitle.add_css_class("dim");
                subtitle.add_css_class("caption");
                texts.append(&title);
                texts.append(&subtitle);
                let size = util::label("", &["numeric"]);
                size.set_xalign(1.0);
                let bar = share_bar(0, 0);
                let next = util::icon("go-next-symbolic");
                let del = util::icon_button("adb-edit-delete-symbolic", tr("Delete this file"));
                del.add_css_class("flat");
                del.set_valign(gtk::Align::Center);
                let (weak_item, page) = (item.downgrade(), page.clone());
                del.connect_clicked(move |b| {
                    let (Some(item), Some(page)) = (weak_item.upgrade(), page.upgrade()) else { return };
                    if let Some(Item::Node { node, .. }) = item.item().and_then(|o| item_of(&o)) {
                        page.delete(b, &node);
                    }
                });
                row.append(&icon);
                row.append(&texts);
                row.append(&size);
                row.append(&bar);
                row.append(&next);
                row.append(&del);
                item.set_child(Some(&row));
            });
        }
        {
            let ctx = ctx.clone();
            factory.connect_bind(move |_, obj| {
                let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
                let (Some(data), Some(row)) = (item.item().and_then(|o| item_of(&o)), item.child()) else { return };
                let Some((icon, title, subtitle, size_l, bar, next, del)) = row_parts(&row) else { return };
                // Rows are reused for other items: every part is set again.
                let (mut is_dir, mut heading) = (false, false);
                match &data {
                    Item::Node { node, total, with_path, nested } => {
                        is_dir = node.is_dir;
                        icon.set_visible(true);
                        icon.set_icon_name(Some(if node.is_dir { "adb-folder-symbolic" } else { "adb-file-text-symbolic" }));
                        icon.set_margin_start(if *nested { 22 } else { 0 });
                        title.set_text(&ctx.shown(&node.name));
                        let sub = if *with_path { ctx.shown(&adbscan_core::adb::remote_parent(&node.path)) } else { String::new() };
                        subtitle.set_text(&sub);
                        subtitle.set_visible(!sub.is_empty());
                        size_l.set_text(&self::size(node.kib));
                        size_l.set_visible(true);
                        bar.set_visible(*total > 0);
                        bar.set_value(if *total > 0 { node.kib as f64 / *total as f64 } else { 0.0 });
                        del.set_visible(!node.is_dir && storage::deletable(&node.path));
                    }
                    Item::Group { title: t, subtitle: st } => {
                        heading = true;
                        icon.set_visible(true);
                        icon.set_icon_name(Some("adb-copy-symbolic"));
                        icon.set_margin_start(0);
                        title.set_text(&ctx.shown(t));
                        subtitle.set_text(st);
                        subtitle.set_visible(true);
                        size_l.set_visible(false);
                        bar.set_visible(false);
                        del.set_visible(false);
                    }
                    Item::Note(text) => {
                        icon.set_visible(false);
                        title.set_text(text);
                        subtitle.set_visible(false);
                        size_l.set_visible(false);
                        bar.set_visible(false);
                        del.set_visible(false);
                    }
                }
                if heading {
                    title.add_css_class("heading");
                } else {
                    title.remove_css_class("heading");
                }
                next.set_visible(is_dir);
                item.set_activatable(is_dir);
            });
        }
        let list = gtk::ListView::builder().model(&gtk::NoSelection::new(Some(store.clone()))).factory(&factory).single_click_activate(true).build();
        list.add_css_class("rowlist");
        list.add_css_class("rowlist-frame");
        {
            let store = store.clone();
            list.connect_activate(move |_, pos| {
                let (Some(page), Some(Item::Node { node, .. })) = (page.upgrade(), store.item(pos).and_then(|o| item_of(&o))) else { return };
                if node.is_dir {
                    page.open_folder(&node.path);
                }
            });
        }
        // Same width and tightening as the page's header.
        let clamp = adw::ClampScrollable::builder().maximum_size(900).tightening_threshold(700).child(&list).build();
        clamp.set_margin_top(6);
        clamp.set_margin_bottom(24);
        clamp.set_margin_start(18);
        clamp.set_margin_end(18);
        let root = gtk::ScrolledWindow::builder().child(&clamp).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let pending: Rc<RefCell<Option<Vec<glib::Object>>>> = Rc::new(RefCell::new(None));
        {
            // When shown: wait for the first frame in which it has a height.
            let (pending, store) = (pending.clone(), store.clone());
            root.connect_map(move |root| {
                let (pending, store) = (pending.clone(), store.clone());
                root.add_tick_callback(move |root, _| {
                    if !root.is_mapped() {
                        return glib::ControlFlow::Break;
                    }
                    if root.height() == 0 {
                        return glib::ControlFlow::Continue;
                    }
                    if let Some(items) = pending.borrow_mut().take() {
                        store.splice(0, store.n_items(), &items);
                    }
                    glib::ControlFlow::Break
                });
            });
        }
        Self { root, store, pending }
    }

    /// Replaces the rows (a single notification to the model). `scroll_top`:
    /// back to the first row (another folder), instead of keeping the place
    /// (the same list after a deletion).
    fn set(&self, items: Vec<Item>, scroll_top: bool) {
        let objs: Vec<glib::Object> = items.into_iter().map(|i| glib::BoxedAnyObject::new(i).upcast()).collect();
        if scroll_top {
            self.root.vadjustment().set_value(0.0);
        }
        if !self.root.is_mapped() || self.root.height() == 0 {
            *self.pending.borrow_mut() = Some(objs);
            return;
        }
        self.pending.borrow_mut().take();
        self.store.splice(0, self.store.n_items(), &objs);
    }
}

/// Area above a list, with the same width as the list.
fn clamped(child: &impl IsA<gtk::Widget>) -> adw::Clamp {
    let c = adw::Clamp::builder().maximum_size(900).tightening_threshold(700).child(child).build();
    c.set_margin_top(12);
    c.set_margin_start(18);
    c.set_margin_end(18);
    c
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

        let nav = util::hbox(8);
        let folder_up = util::icon_button("adb-go-up-symbolic", tr("Parent folder"));
        let folder_path = util::label(storage::ROOT, &["dim"]);
        folder_path.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        folder_path.set_hexpand(true);
        nav.append(&folder_up);
        nav.append(&folder_path);

        let dupes_intro = util::label("", &["dim"]);
        dupes_intro.set_wrap(true);

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

        let this = Rc::new_cyclic(|me: &Weak<Self>| {
            let folder_list = NodeList::new(ctx, me.clone());
            let fbox = util::vbox(0);
            fbox.append(&clamped(&nav));
            fbox.append(&folder_list.root);
            tabs.add_titled_with_icon(&fbox, Some("folders"), tr("Folders"), "adb-folder-symbolic");

            let largest = NodeList::new(ctx, me.clone());
            largest.root.set_margin_top(6);
            tabs.add_titled_with_icon(&largest.root, Some("largest"), tr("Largest files"), "adb-file-text-symbolic");

            let dupes = NodeList::new(ctx, me.clone());
            let dbox = util::vbox(0);
            dbox.append(&clamped(&dupes_intro));
            dbox.append(&dupes.root);
            tabs.add_titled_with_icon(&dbox, Some("dupes"), tr("Duplicates"), "adb-copy-symbolic");

            Self {
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
                dupes_intro,
                dupes,
                scan: RefCell::new(None),
                report: RefCell::new(None),
                cwd: RefCell::new(storage::ROOT.to_string()),
                running: RefCell::new(None),
                scanned_for: RefCell::new(None),
                generation: Cell::new(0),
            }
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
        self.generation.set(self.generation.get() + 1);
        let generation = self.generation.get();
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
            // Parsing and every total in the background: with 100 000+ files
            // they would freeze the window for a moment.
            util::run_bg(
                move || {
                    let scan = Scan::parse(&text);
                    let report = scan.report();
                    (scan, report)
                },
                move |(scan, report)| {
                    let Some(t) = w2.upgrade() else { return };
                    if t.generation.get() != generation {
                        return;
                    }
                    if scan.is_empty() {
                        t.status.set_text(tr("The scan returned nothing: the shared storage may not be readable."));
                        return;
                    }
                    t.status.set_text(&t!("{} in {} files, scanned in {} s.", size(report.total_kib), report.file_count, secs));
                    *t.scan.borrow_mut() = Some(scan);
                    *t.report.borrow_mut() = Some(report);
                    t.tabs.set_sensitive(true);
                    t.render_all(true);
                },
            );
            glib::ControlFlow::Break
        });
    }

    /// `fresh`: a new scan (lists back to the top); otherwise the same one
    /// after a change, keeping each list's place.
    fn render_all(&self, fresh: bool) {
        let started = std::time::Instant::now();
        self.render_overview();
        let cwd = self.cwd.borrow().clone();
        self.show_folder(&cwd, fresh);
        self.render_largest(fresh);
        self.render_dupes(fresh);
        util::log_slow("storage render", started);
    }

    fn render_overview(&self) {
        clear(&self.overview);
        let report = self.report.borrow();
        let Some(report) = report.as_ref() else { return };
        let total = report.total_kib;
        let types = adw::PreferencesGroup::builder().title(tr("By type")).build();
        for (ft, kib, n) in &report.by_type {
            let row = adw::ActionRow::builder().title(ft.label()).subtitle(t!("{} files", n)).build();
            row.add_suffix(&util::label(&size(*kib), &["numeric"]));
            row.add_suffix(&share_bar(*kib, total));
            types.add(&row);
        }
        self.overview.append(&types);
        let places = adw::PreferencesGroup::builder().title(tr("Places that grow")).description(tr("Folders that fill up without anyone noticing. Open one to see what is inside.")).build();
        if report.places.is_empty() {
            places.add(&adw::ActionRow::builder().title(tr("None of these places takes up space")).build());
        }
        for p in &report.places {
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
        self.show_folder(path, true);
    }

    fn show_folder(&self, path: &str, scroll_top: bool) {
        let scan = self.scan.borrow();
        // Being updated after a deletion: the view stays as it is.
        let Some(scan) = scan.as_ref() else { return };
        *self.cwd.borrow_mut() = path.to_string();
        self.folder_path.set_text(&self.ctx.shown(path));
        self.folder_up.set_sensitive(path != storage::ROOT);
        let total = scan.size_of(path).unwrap_or(0);
        let nodes = scan.children(path);
        let items: Vec<Item> = if nodes.is_empty() { vec![Item::Note(tr("Empty folder").to_string())] } else { nodes.into_iter().map(|node| Item::Node { node, total, with_path: false, nested: false }).collect() };
        self.folder_list.set(items, scroll_top);
    }

    fn render_largest(&self, scroll_top: bool) {
        let report = self.report.borrow();
        let Some(report) = report.as_ref() else {
            self.largest.set(Vec::new(), true);
            return;
        };
        let total = report.total_kib;
        let items = report.largest.iter().map(|n| Item::Node { node: n.clone(), total, with_path: true, nested: false }).collect();
        self.largest.set(items, scroll_top);
    }

    fn render_dupes(&self, scroll_top: bool) {
        let report = self.report.borrow();
        let Some(report) = report.as_ref() else {
            self.dupes_intro.set_text("");
            self.dupes.set(Vec::new(), true);
            return;
        };
        self.dupes_intro.set_text(&t!("Files of 1 MB or more with the same name and size in different folders: {} could be freed. Check them before deleting: they are probably, not certainly, the same file.", size(report.freeable_kib())));
        let mut items = Vec::new();
        if report.duplicates.is_empty() {
            items.push(Item::Note(tr("No duplicates found").to_string()));
        }
        for d in &report.duplicates {
            items.push(Item::Group { title: d.name.clone(), subtitle: t!("{} copies of {} · {} could be freed", d.paths.len(), size(d.kib), size(d.reclaimable())) });
            for p in &d.paths {
                let node = Node { path: p.clone(), name: p.rsplit('/').next().unwrap_or(p).to_string(), kib: d.kib, is_dir: false };
                items.push(Item::Node { node, total: 0, with_path: true, nested: true });
            }
        }
        self.dupes.set(items, scroll_top);
    }

    fn delete(&self, button: &gtk::Button, n: &Node) {
        let (w, path) = (self.me.clone(), n.path.clone());
        util::confirm(button, &t!("Delete {}?", self.ctx.shown(&n.name)), &t!("{} · {}\n\nThe file is deleted from the phone for good: the shared storage has no recycle bin.", self.ctx.shown(&n.path), size(n.kib)), tr("Delete"), true, move || {
            let Some(t) = w.upgrade() else { return };
            let (p2, w2, target) = (path.clone(), w.clone(), path.clone());
            t.ctx.device_op("storage_delete", &target, tr("File deleted"), move |adb, s| adb.delete_shared_file(&s, &p2).map(|r| r.summary()), move |ok| {
                let Some(t) = w2.upgrade() else { return };
                if !ok {
                    return;
                }
                // The scan travels to the background, loses the file and comes
                // back with new totals; meanwhile the lists stay as they are.
                let Some(mut scan) = t.scan.borrow_mut().take() else { return };
                let generation = t.generation.get();
                let w3 = t.me.clone();
                util::run_bg(
                    move || {
                        scan.remove_file(&path);
                        let report = scan.report();
                        (scan, report)
                    },
                    move |(scan, report)| {
                        let Some(t) = w3.upgrade() else { return };
                        if t.generation.get() != generation {
                            return;
                        }
                        *t.scan.borrow_mut() = Some(scan);
                        *t.report.borrow_mut() = Some(report);
                        t.render_all(false);
                    },
                );
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
        if matches!(ev, Event::Devices) && self.report.borrow().is_some() && *self.scanned_for.borrow() != self.ctx.ready_serial() {
            *self.scan.borrow_mut() = None;
            *self.report.borrow_mut() = None;
            self.generation.set(self.generation.get() + 1);
            *self.cwd.borrow_mut() = storage::ROOT.to_string();
            self.tabs.set_sensitive(false);
            self.status.set_text(tr("Not scanned yet."));
            clear(&self.overview);
            self.folder_list.set(Vec::new(), true);
            self.render_largest(true);
            self.render_dupes(true);
        }
        if matches!(ev, Event::Privacy) && self.report.borrow().is_some() {
            self.render_all(false);
        }
    }
}
