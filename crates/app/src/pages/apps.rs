//! Apps: virtualised list, details, permissions, batch actions and APK
//! installation with a preview (file chooser or drag and drop).

use adbscan_core::{i18n::{tr, trc}, t};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use adbscan_core::adb::InstallOptions;
use adbscan_core::apk::{self, ApkInfo};
use adbscan_core::format;
use adbscan_core::models::PackageInfo;
use adbscan_core::paths;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    User,
    System,
    Disabled,
    All,
}

/// One step in building the details view (see [`AppsPage::run_stages`]).
type Stage = Box<dyn FnOnce(&AppsPage)>;

/// Rows shown per frame while building the details view.
const ROWS_PER_FRAME: usize = 4;

/// Steps to add `group` to `parent` without blocking the window: first the
/// group with its rows hidden and then a few rows per frame. The expensive part
/// is the first layout of each row (GTK does not lay out what is not visible,
/// and what is already placed is not measured again), so each frame does little
/// work even when the details view has a hundred rows.
fn group_stages(stages: &mut VecDeque<Stage>, parent: &gtk::Box, group: adw::PreferencesGroup) {
    let mut rows = Vec::new();
    collect_rows(group.upcast_ref(), &mut rows);
    for r in &rows {
        r.set_visible(false);
    }
    let parent = parent.clone();
    stages.push_back(Box::new(move |_| parent.append(&group)));
    for chunk in rows.chunks(ROWS_PER_FRAME) {
        let chunk = chunk.to_vec();
        stages.push_back(Box::new(move |_| {
            for r in &chunk {
                r.set_visible(true);
            }
        }));
    }
}

/// Visible rows of a group (without going into them: the rows of an expander
/// are created when it opens).
fn collect_rows(w: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
    let mut child = w.first_child();
    while let Some(c) = child {
        if c.is::<adw::PreferencesRow>() {
            if c.is_visible() {
                out.push(c.clone());
            }
        } else {
            collect_rows(&c, out);
        }
        child = c.next_sibling();
    }
}

struct AppsPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::Overlay,
    store: gio::ListStore,
    filter_model: gtk::FilterListModel,
    selection: gtk::MultiSelection,
    custom_filter: gtk::CustomFilter,
    search: gtk::SearchEntry,
    filter: Rc<Cell<Filter>>,
    filter_dd: gtk::DropDown,
    /// App requested from another page: it is selected when the list loads.
    pending_select: RefCell<Option<String>>,
    labels: Rc<RefCell<HashMap<String, String>>>,
    count_label: gtk::Label,
    /// When Android last measured the app sizes (they are not live).
    sizes_ts: Cell<Option<i64>>,
    details_stack: gtk::Stack,
    details_box: gtk::Box,
    /// App name on the loading screen.
    loading_title: gtk::Label,
    /// Changes with every app opened: pending steps of the previous one are
    /// discarded.
    /// Details generation: changes with every app chosen. Shared with the threads so
    /// that a read for an app that is no longer open never runs (otherwise several
    /// clicks in a row queue up the ADB reads of the previous apps and the last one
    /// takes a while to appear).
    details_gen: Arc<AtomicU64>,
    batch_bar: gtk::Revealer,
    batch_label: gtk::Label,
    /// Reload button: its arrows spin while the list is read.
    refresh: gtk::Button,
    /// Last technical details read (reused for a few seconds).
    inspection: RefCell<Option<adbscan_core::app_inspect::AppInspection>>,
    loaded_for: RefCell<Option<String>>,
    current: RefCell<Option<PackageInfo>>,
    label_job: Cell<u64>,
    /// Signature of the package list, to reload only if it changed.
    list_sig: RefCell<String>,
    list_check: Cell<f64>,
    /// “Running” label of the open details view and its last query.
    running_label: RefCell<Option<gtk::Label>>,
    running_check: Cell<f64>,
    busy_bg: Cell<bool>,
    /// Real icons already read (package → icon; `None`: the app has no image
    /// icon and keeps the box).
    icons: RefCell<HashMap<String, Option<Rc<adbscan_core::models::AppIcon>>>>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    AppsPage::new(ctx)
}

fn pkg_of(obj: &glib::Object) -> Option<PackageInfo> {
    obj.downcast_ref::<glib::BoxedAnyObject>().map(|b| b.borrow::<PackageInfo>().clone())
}

fn labels_cache_file() -> PathBuf {
    paths::cache_dir().join("labels.json")
}

/// Display names already read (cache shared with other pages).
pub fn cached_labels() -> HashMap<String, String> {
    std::fs::read_to_string(labels_cache_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Reads the display name of packages not yet in the cache and updates it.
/// Blocking (call from a worker thread). Returns how many new names it found.
pub fn fetch_missing_labels(adb: &adbscan_core::adb::Adb, serial: &str, packages: &[String]) -> usize {
    let known = cached_labels();
    let missing: Vec<&String> = packages.iter().filter(|p| !known.contains_key(p.as_str())).collect();
    if missing.is_empty() || adb.shell_text(serial, &["command", "-v", "unzip"]).trim().is_empty() {
        return 0;
    }
    let queue = std::sync::Mutex::new(missing.into_iter());
    let found = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                while let Some(pkg) = queue.lock().ok().and_then(|mut q| q.next()) {
                    let paths = adb.package_apk_paths(serial, pkg).unwrap_or_default();
                    // The base APK has the name; split APKs do not.
                    let Some(path) = paths.iter().find(|p| p.ends_with("base.apk")).or(paths.first()) else {
                        continue;
                    };
                    if let Some(label) = adb.package_label(serial, path) {
                        if let Ok(mut f) = found.lock() {
                            f.push((pkg.clone(), label));
                        }
                    }
                }
            });
        }
    });
    let found = found.into_inner().unwrap_or_default();
    if !found.is_empty() {
        // Read again: another page may have saved names in the meantime.
        let mut all = cached_labels();
        all.extend(found.iter().cloned());
        if let Ok(text) = serde_json::to_string(&all) {
            let _ = std::fs::create_dir_all(paths::cache_dir());
            let _ = std::fs::write(labels_cache_file(), text);
        }
    }
    found.len()
}

impl AppsPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let filter = Rc::new(Cell::new(Filter::User));
        // 0 name · 1 size · 2 recently updated · 3 cache
        let sort_mode = Rc::new(Cell::new(0u32));
        // 0 any · 1 Play Store · 2 other stores · 3 no store (ADB/APK)
        let origin = Rc::new(Cell::new(0u32));
        let labels: Rc<RefCell<HashMap<String, String>>> = Rc::new(RefCell::new(std::fs::read_to_string(labels_cache_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()));
        let search = gtk::SearchEntry::builder().placeholder_text(tr("Search apps…")).tooltip_text(tr("Search by name or package")).hexpand(true).build();

        let custom_filter = {
            let (filter, labels, search, origin) = (filter.clone(), labels.clone(), search.clone(), origin.clone());
            gtk::CustomFilter::new(move |obj| {
                let Some(p) = obj.downcast_ref::<glib::BoxedAnyObject>() else {
                    return false;
                };
                let p = p.borrow::<PackageInfo>();
                let store = p.installer == "com.android.vending";
                let sideload = p.installer.is_empty() || matches!(p.installer.as_str(), "com.android.shell" | "com.google.android.packageinstaller" | "com.android.packageinstaller");
                let origin_ok = match origin.get() {
                    1 => store,
                    2 => !store && !sideload,
                    3 => sideload,
                    _ => true,
                };
                if !origin_ok {
                    return false;
                }
                let ok = match filter.get() {
                    Filter::User => !p.is_system,
                    Filter::System => p.is_system,
                    Filter::Disabled => p.enabled == Some(false),
                    Filter::All => true,
                };
                if !ok {
                    return false;
                }
                let q = search.text().to_lowercase();
                if q.is_empty() {
                    return true;
                }
                p.package.to_lowercase().contains(&q) || labels.borrow().get(&p.package).is_some_and(|l| l.to_lowercase().contains(&q))
            })
        };
        let sorter = {
            let (labels, sort_mode) = (labels.clone(), sort_mode.clone());
            gtk::CustomSorter::new(move |a, b| {
                let get = |o: &glib::Object| o.downcast_ref::<glib::BoxedAnyObject>().unwrap().borrow::<PackageInfo>().clone();
                let (pa, pb) = (get(a), get(b));
                let name = |p: &PackageInfo| labels.borrow().get(&p.package).cloned().unwrap_or_else(|| p.display_name()).to_lowercase();
                let by_name = name(&pa).cmp(&name(&pb));
                match sort_mode.get() {
                    1 => pb.total_bytes.unwrap_or(0).cmp(&pa.total_bytes.unwrap_or(0)).then(by_name),
                    2 => pb.updated_ts.unwrap_or(0).cmp(&pa.updated_ts.unwrap_or(0)).then(by_name),
                    3 => pb.disk.map_or(0, |d| d.cache).cmp(&pa.disk.map_or(0, |d| d.cache)).then(by_name),
                    _ => by_name,
                }
                .into()
            })
        };
        let sort_model = gtk::SortListModel::new(Some(store.clone()), Some(sorter.clone()));
        let filter_model = gtk::FilterListModel::new(Some(sort_model), Some(custom_filter.clone()));
        let selection = gtk::MultiSelection::new(Some(filter_model.clone()));

        // -- row -----------------------------------------------------------
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let row = util::hbox(12);
            row.set_margin_top(6);
            row.set_margin_bottom(6);
            row.set_margin_start(6);
            // The box opens with the selection. A row recycled for another app
            // changes state without animating: only a selection change of the
            // same app is animated.
            let pkg_icon = crate::anim::PackageIcon::new(28);
            let icon = pkg_icon.widget.clone();
            let last_item: Rc<RefCell<Option<glib::Object>>> = Rc::default();
            {
                let (pkg_icon, last_item) = (pkg_icon.clone(), last_item.clone());
                item.connect_item_notify(move |it| {
                    *last_item.borrow_mut() = it.item();
                    pkg_icon.set_open(it.is_selected(), false);
                });
            }
            item.connect_selected_notify(move |it| {
                let same = *last_item.borrow() == it.item();
                pkg_icon.set_open(it.is_selected(), same);
            });
            let texts = util::vbox(2);
            texts.set_hexpand(true);
            let title = util::label("", &["heading"]);
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let sub = util::label("", &["dim", "caption"]);
            sub.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            texts.append(&title);
            texts.append(&sub);
            let tag = gtk::Label::new(None);
            tag.add_css_class("pill");
            tag.set_valign(gtk::Align::Center);
            row.append(&icon);
            row.append(&texts);
            row.append(&tag);
            item.set_child(Some(&row));
        });
        {
            let (labels, sort_mode) = (labels.clone(), sort_mode.clone());
            factory.connect_bind(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().unwrap();
                let Some(p) = item.item().and_then(|o| pkg_of(&o)) else {
                    return;
                };
                let row = item.child().and_downcast::<gtk::Box>().unwrap();
                let texts = row.first_child().unwrap().next_sibling().unwrap().downcast::<gtk::Box>().unwrap();
                let title = texts.first_child().unwrap().downcast::<gtk::Label>().unwrap();
                let sub = title.next_sibling().unwrap().downcast::<gtk::Label>().unwrap();
                let tag = texts.next_sibling().unwrap().downcast::<gtk::Label>().unwrap();
                let name = labels.borrow().get(&p.package).cloned().unwrap_or_else(|| p.display_name());
                title.set_text(&name);
                let mut parts = vec![p.package.clone()];
                if let Some(b) = p.total_bytes {
                    parts.push(format::human_bytes(Some(b as f64)));
                }
                // Sorted by cache: how much of it is cache.
                if let Some(c) = p.disk.map(|d| d.cache).filter(|c| *c > 0 && sort_mode.get() == 3) {
                    parts.push(t!("cache {}", format::human_bytes(Some(c as f64))));
                }
                if let Some(t) = p.updated_ts {
                    parts.push(format::date(Some(t as f64)));
                }
                if parts.len() == 1 && !p.version_code.is_empty() {
                    parts.push(format!("v{}", p.version_code));
                }
                sub.set_text(&parts.join(" · "));
                for c in ["warn", "accent"] {
                    tag.remove_css_class(c);
                }
                if p.enabled == Some(false) {
                    tag.set_text(tr("Disabled"));
                    tag.add_css_class("warn");
                    tag.set_visible(true);
                } else if p.is_system {
                    tag.set_text(tr("System"));
                    tag.add_css_class("accent");
                    tag.set_visible(true);
                } else {
                    tag.set_visible(false);
                }
            });
        }
        let list = gtk::ListView::builder().model(&selection).factory(&factory).build();
        list.add_css_class("navigation-sidebar");
        let list_scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();

        // -- top bar ----------------------------------------------------------
        let bar = util::hbox(8);
        bar.set_margin_top(10);
        bar.set_margin_bottom(6);
        bar.set_margin_start(10);
        bar.set_margin_end(10);
        bar.append(&search);
        let filter_dd = gtk::DropDown::from_strings(&[tr("User"), tr("System"), trc("plural", "Disabled"), tr("All")]);
        filter_dd.set_tooltip_text(Some(tr("Filter")));
        bar.append(&filter_dd);
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Reload"));
        bar.append(&refresh);
        let install = util::pill_button("adb-list-add-symbolic", tr("Install APK"));
        install.add_css_class("suggested-action");
        bar.append(&install);
        bar.append(&super::bulk::menu_button(ctx));
        let count_label = util::label("", &["dim", "caption"]);
        // Long with the cache total: it must not widen the list pane.
        count_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        count_label.set_hexpand(true);
        count_label.set_margin_start(12);
        count_label.set_margin_bottom(4);

        let left = util::vbox(0);
        left.append(&bar);
        // Sort and source on one line, the count (with the cache total) on its
        // own line underneath so it is not cut.
        let count_row = util::hbox(6);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        count_row.append(&spacer);
        let sort_dd = gtk::DropDown::from_strings(&[tr("Name"), tr("Size"), tr("Recent"), tr("Cache")]);
        sort_dd.set_tooltip_text(Some(tr("Sort by name, by space used (app + data + cache), by update or by cache")));
        let origin_dd = gtk::DropDown::from_strings(&[tr("Any source"), "Play Store", tr("Other stores"), tr("No store (APK/ADB)")]);
        origin_dd.set_tooltip_text(Some(tr("Filter by who installed the app")));
        for d in [&sort_dd, &origin_dd] {
            d.add_css_class("flat");
            count_row.append(d);
        }
        count_row.set_margin_end(10);
        left.append(&count_row);
        left.append(&count_label);
        {
            let (sort_mode, sorter) = (sort_mode.clone(), sorter.clone());
            sort_dd.connect_selected_notify(move |d| {
                sort_mode.set(d.selected());
                sorter.changed(gtk::SorterChange::Different);
            });
        }
        left.append(&list_scroll);

        // Batch actions
        let batch_label = util::label("", &["heading"]);
        let batch_box = util::hbox(6);
        batch_box.add_css_class("transfer-bar");
        batch_box.append(&batch_label);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        batch_box.append(&spacer);
        let batch_buttons: Vec<(gtk::Button, &str)> = vec![
            (util::icon_button("adb-media-playback-stop-symbolic", tr("Force stop")), "stop"),
            (util::icon_button("adb-edit-clear-symbolic", tr("Clear cache")), "cache"),
            (util::icon_button("adb-eye-off-symbolic", tr("Disable")), "disable"),
            (util::icon_button("adb-emblem-ok-symbolic", tr("Enable")), "enable"),
            (util::icon_button("adb-save-symbolic", tr("Extract APK")), "pull"),
            (util::icon_button("adb-edit-delete-symbolic", tr("Uninstall")), "uninstall"),
        ];
        for (b, _) in &batch_buttons {
            batch_box.append(b);
        }
        let batch_bar = gtk::Revealer::builder().transition_type(gtk::RevealerTransitionType::SlideUp).child(&batch_box).build();
        left.append(&batch_bar);

        // -- details ---------------------------------------------------------
        let details_box = util::vbox(14);
        let details_scroll = util::page_scroller(&details_box, 620);
        let empty = adw::StatusPage::builder().icon_name("adb-package-symbolic").title(tr("Choose an app")).description(tr("You can also drag .apk files here to install them")).build();
        // Loading screen: shown as soon as an app is chosen, while its data is
        // read and the details view is built.
        let loading = util::vbox(12);
        loading.set_valign(gtk::Align::Center);
        loading.set_halign(gtk::Align::Center);
        let spinner = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).build();
        let loading_title = util::label("", &["title-2"]);
        loading_title.set_xalign(0.5);
        loading_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let loading_sub = util::label(tr("Reading data from the phone…"), &["dim"]);
        loading_sub.set_xalign(0.5);
        loading.append(&spinner);
        loading.append(&loading_title);
        loading.append(&loading_sub);
        let details_stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).transition_duration(120).build();
        details_stack.add_named(&empty, Some("empty"));
        details_stack.add_named(&loading, Some("loading"));
        details_stack.add_named(&details_scroll, Some("details"));

        let paned =
            gtk::Paned::builder().orientation(gtk::Orientation::Horizontal).start_child(&left).end_child(&details_stack).position(440).shrink_start_child(false).shrink_end_child(false).build();
        let root = gtk::Overlay::builder().child(&paned).build();

        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            store,
            filter_model,
            selection,
            custom_filter,
            search,
            filter,
            filter_dd: filter_dd.clone(),
            pending_select: RefCell::new(None),
            labels,
            count_label,
            sizes_ts: Cell::new(None),
            details_stack,
            details_box,
            loading_title,
            details_gen: Arc::default(),
            batch_bar,
            batch_label,
            refresh: refresh.clone(),
            inspection: RefCell::new(None),
            loaded_for: RefCell::new(None),
            current: RefCell::new(None),
            label_job: Cell::new(0),
            list_sig: RefCell::new(String::new()),
            list_check: Cell::new(adbscan_core::models::now_secs()),
            running_label: RefCell::new(None),
            running_check: Cell::new(0.0),
            busy_bg: Cell::new(false),
            icons: RefCell::new(HashMap::new()),
        });

        {
            let w = this.me.clone();
            this.search.connect_search_changed(move |_| {
                if let Some(t) = w.upgrade() {
                    t.custom_filter.changed(gtk::FilterChange::Different);
                    t.update_count();
                }
            });
        }
        {
            let w = this.me.clone();
            let origin = origin.clone();
            origin_dd.connect_selected_notify(move |dd| {
                origin.set(dd.selected());
                if let Some(t) = w.upgrade() {
                    t.custom_filter.changed(gtk::FilterChange::Different);
                    t.update_count();
                }
            });
        }
        {
            let w = this.me.clone();
            filter_dd.connect_selected_notify(move |dd| {
                if let Some(t) = w.upgrade() {
                    t.filter.set(match dd.selected() {
                        0 => Filter::User,
                        1 => Filter::System,
                        2 => Filter::Disabled,
                        _ => Filter::All,
                    });
                    t.custom_filter.changed(gtk::FilterChange::Different);
                    t.update_count();
                }
            });
        }
        {
            let w = this.me.clone();
            refresh.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    t.reload();
                }
            });
        }
        {
            let w = this.me.clone();
            install.connect_clicked(move |b| {
                let w = w.clone();
                util::pick_files(b, tr("Choose APK"), &[(tr("Android apps"), &["*.apk", "*.APK"])], move |paths| {
                    if let Some(t) = w.upgrade() {
                        t.install_dialog(paths);
                    }
                });
            });
        }
        {
            let w = this.me.clone();
            this.selection.connect_selection_changed(move |_, _, _| {
                if let Some(t) = w.upgrade() {
                    t.on_selection();
                }
            });
        }
        for (b, action) in batch_buttons {
            let w = this.me.clone();
            b.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    t.batch(action);
                }
            });
        }
        this.setup_drop();
        this.reload();
        this
    }

    fn selected(&self) -> Vec<PackageInfo> {
        let set = self.selection.selection();
        (0..set.size()).filter_map(|i| self.selection.item(set.nth(i as u32))).filter_map(|o| pkg_of(&o)).collect()
    }

    fn update_count(&self) {
        let shown = self.filter_model.n_items();
        let total = self.store.n_items();
        // Cache of the apps in view: what “Clear cache” on them would free.
        let cache: i64 = (0..shown).filter_map(|i| self.filter_model.item(i)).filter_map(|o| pkg_of(&o)).filter_map(|p| p.disk.map(|d| d.cache)).sum();
        let mut text = t!("{} of {} apps", shown, total);
        if cache > 0 {
            text.push_str(&t!(" · {} of cache", format::human_bytes(Some(cache as f64))));
            if let Some(ts) = self.sizes_ts.get() {
                text.push_str(&t!(" ({})", format::date(Some(ts as f64))));
            }
        }
        self.count_label.set_text(&text);
        // The whole text (it may be cut) and, since Android measures the
        // sizes now and then, when it did.
        let when = self.sizes_ts.get().map(|ts| t!("Sizes measured by Android on {}; it updates them about once a day", format::date(Some(ts as f64))));
        self.count_label.set_tooltip_text(Some(&match when {
            Some(w) => format!("{text}\n{w}"),
            None => text.clone(),
        }));
    }

    fn reload(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        crate::anim::set_busy(&self.refresh, true);
        *self.loaded_for.borrow_mut() = Some(serial.clone());
        let w = self.me.clone();
        util::run_bg(
            move || (adb.packages(&serial), adb.disk_stats_time(&serial)),
            move |(res, sizes_ts)| {
                let Some(t) = w.upgrade() else { return };
                crate::anim::set_busy(&t.refresh, false);
                t.sizes_ts.set(sizes_ts);
                match res {
                    Ok(list) => {
                        let objs: Vec<glib::BoxedAnyObject> = list.into_iter().map(glib::BoxedAnyObject::new).collect();
                        t.store.splice(0, t.store.n_items(), &objs);
                        t.update_count();
                        t.resolve_labels();
                        t.select_pending();
                    }
                    Err(e) => t.ctx.toast(&t!("Could not read the app list: {}", e)),
                }
            },
        );
    }

    /// Display names of user apps, in the background and cached.
    fn resolve_labels(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let job = self.label_job.get() + 1;
        self.label_job.set(job);
        let known = self.labels.borrow().clone();
        let pending: Vec<(String, String)> = (0..self.store.n_items())
            .filter_map(|i| self.store.item(i).and_then(|o| pkg_of(&o)))
            .filter(|p| !p.is_system && !known.contains_key(&p.package) && p.apk_path.ends_with(".apk"))
            .map(|p| (p.package, p.apk_path))
            .collect();
        if pending.is_empty() {
            return;
        }
        let (tx, rx) = async_channel::unbounded::<(String, String)>();
        std::thread::spawn(move || {
            // Without unzip on the phone there is nothing to do.
            if adb.shell_text(&serial, &["command", "-v", "unzip"]).trim().is_empty() {
                return;
            }
            // Four reads in parallel: each one is two short `unzip -p` calls.
            let queue = std::sync::Mutex::new(pending.into_iter());
            std::thread::scope(|scope| {
                for _ in 0..4 {
                    scope.spawn(|| {
                        while let Some((pkg, path)) = queue.lock().ok().and_then(|mut q| q.next()) {
                            if let Some(label) = adb.package_label(&serial, &path) {
                                if tx.send_blocking((pkg, label)).is_err() {
                                    break;
                                }
                            }
                        }
                    });
                }
            });
        });
        let w = self.me.clone();
        glib::spawn_future_local(async move {
            let mut batch = 0;
            while let Ok((pkg, label)) = rx.recv().await {
                let Some(t) = w.upgrade() else { break };
                if t.label_job.get() != job {
                    break;
                }
                t.labels.borrow_mut().insert(pkg, label);
                batch += 1;
                if batch % 8 == 0 {
                    t.refresh_rows();
                }
            }
            if let Some(t) = w.upgrade() {
                t.refresh_rows();
                if let Ok(text) = serde_json::to_string(&*t.labels.borrow()) {
                    let _ = std::fs::create_dir_all(paths::cache_dir());
                    let _ = std::fs::write(labels_cache_file(), text);
                }
            }
        });
    }

    /// While the page is visible (only then does it receive telemetry): every 3
    /// s, if the open app is running; every 15 s, if the package list changed
    /// (installs, uninstalls, enabled).
    fn live_tick(&self) {
        let now = adbscan_core::models::now_secs();
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        if self.busy_bg.get() {
            return;
        }
        let current = self.current.borrow().as_ref().map(|p| p.package.clone());
        if let (Some(pkg), true) = (current, now - self.running_check.get() >= 3.0) {
            self.running_check.set(now);
            self.busy_bg.set(true);
            let w = self.me.clone();
            let (adb2, serial2) = (adb.clone(), serial.clone());
            util::run_bg(
                move || adb2.package_pids(&serial2, &pkg),
                move |pids: Vec<i64>| {
                    let Some(t) = w.upgrade() else { return };
                    t.busy_bg.set(false);
                    let label = t.running_label.borrow().clone();
                    if let Some(l) = label {
                        l.set_text(&if pids.is_empty() { tr("No").to_string() } else { t!("Yes · PID {}", pids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")) });
                    }
                },
            );
            return;
        }
        if now - self.list_check.get() >= 15.0 && self.store.n_items() > 0 {
            self.list_check.set(now);
            self.busy_bg.set(true);
            let w = self.me.clone();
            util::run_bg(
                move || format!("{}\n--\n{}", adb.shell_text(&serial, &["pm", "list", "packages"]), adb.shell_text(&serial, &["pm", "list", "packages", "-d"])),
                move |sig: String| {
                    let Some(t) = w.upgrade() else { return };
                    t.busy_bg.set(false);
                    let changed = { !t.list_sig.borrow().is_empty() && *t.list_sig.borrow() != sig };
                    *t.list_sig.borrow_mut() = sig;
                    // Do not reload in the middle of a multiple selection.
                    if changed && t.selected().len() <= 1 {
                        if let Some(cur) = t.current.borrow().as_ref() {
                            *t.pending_select.borrow_mut() = Some(cur.package.clone());
                        }
                        t.reload();
                    }
                },
            );
        }
    }

    fn refresh_rows(&self) {
        let n = self.store.n_items();
        self.store.items_changed(0, n, n);
    }

    /// Selects the app requested from another page if it is already in the
    /// list.
    fn select_pending(&self) {
        let Some(pkg) = self.pending_select.borrow().clone() else {
            return;
        };
        for i in 0..self.filter_model.n_items() {
            if self.filter_model.item(i).and_then(|o| pkg_of(&o)).is_some_and(|p| p.package == pkg) {
                self.pending_select.borrow_mut().take();
                self.selection.select_item(i, true);
                return;
            }
        }
    }

    fn on_selection(&self) {
        let t0 = std::time::Instant::now();
        let _slow = util::SlowGuard("apps: selection", t0);
        let sel = self.selected();
        self.batch_bar.set_reveal_child(sel.len() > 1);
        self.batch_label.set_text(&t!("{} selected", sel.len()));
        match sel.as_slice() {
            [one] => self.load_details(one.clone()),
            [] => {
                self.details_gen.fetch_add(1, Ordering::Relaxed);
                self.details_stack.set_visible_child_name("empty");
            }
            _ => {}
        }
    }

    fn load_details(&self, p: PackageInfo) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        *self.current.borrow_mut() = Some(p.clone());
        let gen = self.details_gen.fetch_add(1, Ordering::Relaxed) + 1;
        // Immediate response to the click: the loading animation, instead of
        // the previous details sitting still while the new one is read and
        // built.
        self.loading_title.set_text(&self.labels.borrow().get(&p.package).cloned().unwrap_or_else(|| p.display_name()));
        self.details_stack.set_visible_child_name("loading");
        let (w, live) = (self.me.clone(), self.details_gen.clone());
        util::run_bg(
            move || {
                if live.load(Ordering::Relaxed) != gen {
                    return None;
                }
                let mut info = adb.package_details(&serial, &p.package, &p.apk_path).unwrap_or(p.clone());
                // The sizes come from the list (one bulk read), not from the details.
                info.total_bytes = info.total_bytes.or(p.total_bytes);
                info.disk = info.disk.or(p.disk);
                info.updated_ts = info.updated_ts.or(p.updated_ts);
                let pids = adb.package_pids(&serial, &p.package);
                Some((info, pids))
            },
            move |res| {
                if let (Some(t), Some((info, pids))) = (w.upgrade(), res) {
                    if t.details_gen.load(Ordering::Relaxed) == gen {
                        t.show_details(info, pids, gen);
                    }
                }
            },
        );
    }

    /// Runs the steps one at a time, each in a different frame, while the same
    /// app stays open. Laying out the whole details view at once (hundreds of
    /// text rows) blocked the window for up to a second; in parts, each frame
    /// does little work and the interface stays responsive.
    fn run_stages(&self, gen: u64, mut stages: VecDeque<Stage>) {
        let Some(stage) = stages.pop_front() else {
            return;
        };
        let w = self.me.clone();
        let next = RefCell::new(Some((stage, stages)));
        // A “tick” added during another one runs on the next frame.
        self.details_box.add_tick_callback(move |_, _| {
            if let (Some(t), Some((stage, rest))) = (w.upgrade(), next.take()) {
                if t.details_gen.load(Ordering::Relaxed) == gen {
                    let _slow = util::SlowGuard("apps: details step", std::time::Instant::now());
                    stage(&t);
                    util::cap_wrapping(&t.details_box);
                    t.run_stages(gen, rest);
                }
            }
            glib::ControlFlow::Break
        });
    }

    fn show_details(&self, info: PackageInfo, pids: Vec<i64>, gen: u64) {
        let _slow = util::SlowGuard("apps: details header", std::time::Instant::now());
        while let Some(c) = self.details_box.first_child() {
            self.details_box.remove(&c);
        }
        self.running_label.borrow_mut().take();
        *self.current.borrow_mut() = Some(info.clone());
        let name = self.labels.borrow().get(&info.package).cloned().unwrap_or_else(|| info.display_name());
        // Header: the app's real icon (the box while it is read, or if it has
        // no image icon), name and package.
        let head = util::hbox(16);
        let icon_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icon_slot.set_size_request(56, 56);
        icon_slot.set_valign(gtk::Align::Center);
        let placeholder = crate::anim::PackageIcon::new(48);
        placeholder.set_open(true, false);
        placeholder.widget.set_halign(gtk::Align::Center);
        icon_slot.append(&placeholder.widget);
        head.append(&icon_slot);
        let texts = util::vbox(4);
        texts.set_valign(gtk::Align::Center);
        let title = util::label(&name, &["title-1"]);
        title.set_wrap(true);
        texts.append(&title);
        let sub = util::label(&info.package, &["dim"]);
        sub.set_selectable(true);
        texts.append(&sub);
        head.append(&texts);
        self.details_box.append(&head);
        self.load_icon(&info, &icon_slot, gen);

        // Actions
        let actions = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(4).column_spacing(6).row_spacing(6).build();
        let add = |icon: &str, text: &str, action: &'static str, destructive: bool| {
            let b = util::pill_button(icon, text);
            if destructive {
                b.add_css_class("destructive-action");
            }
            let w = self.me.clone();
            b.connect_clicked(move |b| {
                if let Some(t) = w.upgrade() {
                    t.single_action(b.upcast_ref(), action);
                }
            });
            actions.append(&b);
        };
        add("adb-media-playback-start-symbolic", tr("Open"), "launch", false);
        add("adb-media-playback-stop-symbolic", tr("Stop"), "stop", false);
        add("adb-edit-clear-symbolic", tr("Clear cache"), "cache", false);
        add("adb-eraser-symbolic", tr("Clear data"), "clear", true);
        if info.enabled == Some(false) {
            add("adb-emblem-ok-symbolic", tr("Enable"), "enable", false);
        } else {
            add("adb-eye-off-symbolic", tr("Disable"), "disable", false);
        }
        add("adb-save-symbolic", tr("Extract APK"), "pull", false);
        if info.is_system {
            add("adb-edit-delete-symbolic", tr("Remove (user 0)"), "uninstall-user", true);
        } else {
            add("adb-edit-delete-symbolic", tr("Uninstall"), "uninstall", true);
        }
        self.details_box.append(&actions);

        let group = adw::PreferencesGroup::builder().title(tr("Information")).build();
        let rows: Vec<(&str, String)> = vec![
            (tr("Version"), format!("{} ({})", if info.version_name.is_empty() { "—" } else { &info.version_name }, info.version_code)),
            ("SDK", t!("minimum {} · target {}", info.min_sdk, info.target_sdk)),
            (tr("Type"), info.type_label().into()),
            (tr("Status"), if info.enabled == Some(false) { tr("Disabled").into() } else { tr("Enabled").into() }),
            (tr("Running"), if pids.is_empty() { tr("No").into() } else { t!("Yes · PID {}", pids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")) }),
            (tr("APK size"), info.size_bytes.map(|b| format::human_bytes(Some(b as f64))).unwrap_or("—".into())),
            (
                tr("Space used"),
                info.disk
                    .map(|d| {
                        let h = |b: i64| format::human_bytes(Some(b as f64));
                        t!("{} · app {} · data {} · cache {}", h(d.total()), h(d.app), h(d.data), h(d.cache))
                    })
                    .unwrap_or_default(),
            ),
            (tr("Installed"), info.first_install.clone()),
            (tr("Updated"), info.last_update.clone()),
            (tr("Installer"), if info.installer.is_empty() || info.installer == "null" { tr("Unknown (ADB or sideload)").into() } else { info.installer.clone() }),
            ("UID", info.uid.clone()),
            (tr("Path"), info.apk_path.clone()),
        ];
        for (t, v) in rows {
            let (row, l) = util::info_row(t);
            l.set_text(if v.is_empty() { "—" } else { &v });
            if t == tr("Running") {
                *self.running_label.borrow_mut() = Some(l.clone());
            }
            group.add(&row);
        }
        if info.is_debuggable() || info.allows_backup() {
            let mut flags = Vec::new();
            if info.is_debuggable() {
                flags.push(tr("Debuggable"));
            }
            if info.allows_backup() {
                flags.push(tr("Allows backup"));
            }
            let (row, l) = util::info_row(tr("Flags"));
            l.set_text(&flags.join(" · "));
            group.add(&row);
        }
        self.details_box.append(&group);
        util::cap_wrapping(&self.details_box);
        self.details_stack.set_visible_child_name("details");

        // The rest, in parts (see `run_stages`).
        let mut stages: VecDeque<Stage> = VecDeque::new();
        group_stages(&mut stages, &self.details_box, self.permissions_group(&info));
        // Technical details: read separately so as not to delay the above.
        let security = self.security_group(&info);
        let pkg = info.package.clone();
        stages.push_back(Box::new(move |t: &AppsPage| {
            let tech = util::vbox(14);
            t.details_box.append(&tech);
            t.load_inspection(&pkg, &tech, gen);
        }));
        group_stages(&mut stages, &self.details_box, security);
        self.run_stages(gen, stages);
    }

    /// Puts the real icon into `slot` (from the cache or reading it from the
    /// phone).
    fn load_icon(&self, info: &PackageInfo, slot: &gtk::Box, gen: u64) {
        let put = |slot: &gtk::Box, icon: &adbscan_core::models::AppIcon| {
            if let Some(w) = crate::widgets::app_icon(icon, 56) {
                while let Some(c) = slot.first_child() {
                    slot.remove(&c);
                }
                slot.append(&w);
            }
        };
        if let Some(cached) = self.icons.borrow().get(&info.package) {
            if let Some(icon) = cached {
                put(slot, icon);
            }
            return;
        }
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let (pkg, apk, slot, w) = (info.package.clone(), info.apk_path.clone(), slot.clone(), self.me.clone());
        util::run_bg(
            move || adb.app_icon(&serial, &apk),
            move |res| {
                let Some(t) = w.upgrade() else { return };
                // A connection failure is not remembered: it is retried when returning
                // to the app.
                let icon = match res {
                    Ok(icon) => icon.map(Rc::new),
                    Err(e) => {
                        log::debug!("icon of {pkg}: {e}");
                        return;
                    }
                };
                t.icons.borrow_mut().insert(pkg, icon.clone());
                if let (true, Some(icon)) = (t.details_gen.load(Ordering::Relaxed) == gen, icon) {
                    let _slow = util::SlowGuard("apps: icon", std::time::Instant::now());
                    put(&slot, &icon);
                }
            },
        );
    }

    /// Sensitive permissions (toggleable) and the list of all granted ones.
    fn permissions_group(&self, info: &PackageInfo) -> adw::PreferencesGroup {
        // Dangerous permissions: toggleable
        let dangerous: Vec<&String> = info.requested_permissions.iter().filter(|p| apk::is_dangerous_permission(p)).collect();
        let perms = adw::PreferencesGroup::builder()
            .title(tr("Sensitive permissions"))
            .description(t!("{} requested in total. You can grant or revoke runtime permissions.", info.requested_permissions.len()))
            .build();
        if dangerous.is_empty() {
            perms.add(&adw::ActionRow::builder().title(tr("Requests no sensitive permissions")).build());
        }
        for p in dangerous {
            let short = p.rsplit('.').next().unwrap_or(p);
            let row = adw::SwitchRow::builder().title(short).subtitle(p.as_str()).active(info.granted_permissions.contains(p)).build();
            let w = self.me.clone();
            let perm = p.clone();
            let pkg = info.package.clone();
            row.connect_active_notify(move |r| {
                let Some(t) = w.upgrade() else { return };
                let grant = r.is_active();
                let (perm, pkg) = (perm.clone(), pkg.clone());
                let msg = if grant { tr("Permission granted") } else { tr("Permission revoked") };
                t.ctx.device_op("permission", &format!("{pkg} {perm}"), msg, move |adb, serial| adb.grant_permission(&serial, &pkg, &perm, grant).map(|r| r.summary()), |_| {});
            });
            perms.add(&row);
        }
        // All granted ones (including normal ones, which cannot be revoked).
        if !info.granted_permissions.is_empty() {
            let all = adw::ExpanderRow::builder()
                .title(t!("All granted permissions ({})", info.granted_permissions.len()))
                .subtitle(tr("Includes “normal” ones, which Android grants at install time"))
                .build();
            let mut granted = info.granted_permissions.clone();
            granted.sort();
            // Rows are created when the expander opens (see `lazy_rows`).
            super::facts::lazy_rows(&all, move |all| {
                for p in &granted {
                    let short = p.rsplit('.').next().unwrap_or(p);
                    let r = adw::ActionRow::builder().title(glib::markup_escape_text(short)).subtitle(glib::markup_escape_text(p)).build();
                    r.set_subtitle_selectable(true);
                    if apk::is_dangerous_permission(p) {
                        r.add_prefix(&util::icon("adb-dialog-warning-symbolic"));
                    }
                    all.add_row(&r);
                }
            });
            perms.add(&all);
        }
        perms
    }

    /// Paints the technical details into `slot`, in parts (see `group_stages`).
    fn render_inspection(&self, slot: &gtk::Box, a: &adbscan_core::app_inspect::AppInspection, gen: u64) {
        let t0 = std::time::Instant::now();
        let groups = super::facts::inspection_groups(&self.ctx, a);
        util::log_slow("apps: build technical details", t0);
        let mut stages: VecDeque<Stage> = VecDeque::new();
        for (i, g) in groups.into_iter().enumerate() {
            // The first group replaces the “Inspecting…” notice.
            if i == 0 {
                let slot = slot.clone();
                stages.push_back(Box::new(move |_: &AppsPage| {
                    while let Some(c) = slot.first_child() {
                        slot.remove(&c);
                    }
                }));
            }
            group_stages(&mut stages, slot, g);
        }
        self.run_stages(gen, stages);
    }

    fn load_inspection(&self, package: &str, slot: &gtk::Box, gen: u64) {
        // Reuse a recent read (e.g. when repainting after an action).
        let cached = self.inspection.borrow().clone().filter(|a| a.package == package && adbscan_core::models::now_secs() - a.read_at < 20.0);
        if let Some(a) = cached {
            self.render_inspection(slot, &a, gen);
            return;
        }
        let wait = adw::PreferencesGroup::builder().title(tr("Technical details")).build();
        let row = adw::ActionRow::builder().title(tr("Inspecting the app…")).subtitle(tr("dumpsys package, App Ops, exits, processes and manifest")).build();
        let spinner = gtk::Spinner::builder().spinning(true).build();
        row.add_suffix(&spinner);
        wait.add(&row);
        slot.append(&wait);
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let (w, pkg, slot, live) = (self.me.clone(), package.to_string(), slot.clone(), self.details_gen.clone());
        util::run_bg(
            move || (live.load(Ordering::Relaxed) == gen).then(|| adb.inspect_package(&serial, &pkg)),
            move |res| {
                let Some(t) = w.upgrade() else { return };
                // If the user already chose another app, the details are not
                // painted.
                let Some(res) = res.filter(|_| t.details_gen.load(Ordering::Relaxed) == gen) else {
                    return;
                };
                match res {
                    Ok(a) => {
                        t.render_inspection(&slot, &a, gen);
                        *t.inspection.borrow_mut() = Some(a);
                    }
                    Err(e) => {
                        row.set_title(tr("Could not inspect the app"));
                        row.set_subtitle(&glib::markup_escape_text(&e.to_string()));
                        spinner.set_visible(false);
                    }
                }
            },
        );
    }

    /// What the last Security audit says about this app.
    fn security_group(&self, info: &PackageInfo) -> adw::PreferencesGroup {
        let _slow = util::SlowGuard("apps: security group", std::time::Instant::now());
        let group = adw::PreferencesGroup::builder().title(tr("Security")).build();
        let report = match (self.ctx.history.clone(), self.ctx.ready_serial()) {
            (Some(h), Some(serial)) => {
                h.security_reports(&serial, 1).ok().and_then(|r| r.into_iter().next()).and_then(|(_, _, _, json)| serde_json::from_str::<adbscan_core::security::Report>(&json).ok())
            }
            _ => None,
        };
        let open = gtk::Button::with_label(tr("Open Security"));
        open.add_css_class("flat");
        let ctx = self.ctx.clone();
        open.connect_clicked(move |_| ctx.emit(Event::ShowPage("security")));
        group.set_header_suffix(Some(&open));
        match report {
            None => group.set_description(Some(tr("There is no audit of this phone yet. Scan it in Security to see each app's risks here."))),
            Some(r) => {
                let hits: Vec<_> = r.findings.iter().filter(|f| f.packages.contains(&info.package)).collect();
                group.set_description(Some(&t!("According to the audit of {}.", format::timestamp(Some(r.timestamp as f64)))));
                if hits.is_empty() {
                    let row = adw::ActionRow::builder().title(tr("No warnings for this app")).build();
                    row.add_prefix(&util::icon("adb-emblem-ok-symbolic"));
                    group.add(&row);
                }
                for f in hits {
                    let row = adw::ActionRow::builder().title(glib::markup_escape_text(&f.title)).subtitle(glib::markup_escape_text(&f.recommendation)).build();
                    row.add_prefix(&util::icon("adb-dialog-warning-symbolic"));
                    group.add(&row);
                }
            }
        }
        group
    }

    fn single_action(&self, parent: &gtk::Widget, action: &'static str) {
        let Some(info) = self.current.borrow().clone() else {
            return;
        };
        let pkg = info.package.clone();
        let target = info.package.clone();
        let name = self.labels.borrow().get(&pkg).cloned().unwrap_or_else(|| info.display_name());
        let ctx = self.ctx.clone();
        let w = self.me.clone();
        let after = move |ok: bool| {
            if let (true, Some(t)) = (ok, w.upgrade()) {
                t.reload();
            }
        };
        match action {
            "launch" => ctx.device_op("launch", &target, &t!("{} opened", name), move |adb, s| adb.launch(&s, &pkg).map(|r| r.summary()), |_| {}),
            "stop" => ctx.device_op("force_stop", &target, &t!("{} stopped", name), move |adb, s| adb.force_stop(&s, &pkg).map(|r| r.summary()), |_| {}),
            "cache" => {
                let Some(adb) = ctx.adb() else { return };
                let Some(serial) = ctx.ready_serial() else {
                    ctx.toast(tr("No device is ready"));
                    return;
                };
                util::run_bg(
                    move || adb.clear_cache(&serial, &pkg),
                    move |res| match res {
                        Ok(kib) => {
                            let msg = match kib.filter(|k| *k > 0) {
                                Some(k) => t!("Cache of {} cleared · {} freed", name, format::human_bytes(Some(k as f64 * 1024.0))),
                                None => t!("Cache of {} cleared", name),
                            };
                            ctx.audit("clear_cache", Some(&target), true, Some(&msg));
                            ctx.toast(&msg);
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            ctx.audit("clear_cache", Some(&target), false, Some(&msg));
                            ctx.toast(&format!("Error: {msg}"));
                        }
                    },
                );
            }
            "enable" => ctx.device_op("enable", &target, &t!("{} enabled", name), move |adb, s| adb.set_enabled(&s, &pkg, true).map(|r| r.summary()), after),
            "disable" => util::confirm(parent, &t!("Disable {}?", name), tr("The app will stop working and disappear from the app drawer until you enable it again. Its data is not deleted."), tr("Disable"), false, move || {
                ctx.device_op("disable", &target, &t!("{} disabled", name), move |adb, s| adb.set_enabled(&s, &pkg, false).map(|r| r.summary()), after)
            }),
            "clear" => util::confirm(parent, &t!("Clear the data of {}?", name), &t!("{} ({})\n\nIts files, accounts, settings and cache will be deleted, as if freshly installed. This cannot be undone.", name, target), tr("Clear data"), true, move || {
                ctx.device_op("clear_data", &target, &t!("{} data cleared", name), move |adb, s| adb.clear_data(&s, &pkg).map(|r| r.summary()), |_| {})
            }),
            "uninstall" => util::confirm(parent, &t!("Uninstall {}?", name), &t!("{} ({})\n\nThe app and all its data will be deleted. This cannot be undone.", name, target), tr("Uninstall"), true, move || {
                ctx.device_op("uninstall", &target, &t!("{} uninstalled", name), move |adb, s| adb.uninstall(&s, &pkg, false).map(|r| r.summary()), after)
            }),
            "uninstall-user" => util::confirm(
                parent,
                &t!("Remove {} from the main user?", name),
                &t!("{} ({})\n\nThis is a system app: it is uninstalled only for user 0 and kept on the system partition. It may affect how the phone works. It can be restored with “cmd package install-existing” or with a factory reset.", name, target),
                tr("Remove"),
                true,
                move || ctx.device_op("uninstall_user0", &target, &t!("{} removed", name), move |adb, s| adb.uninstall_for_user(&s, &pkg).map(|r| r.summary()), after),
            ),
            "pull" => self.pull_apks(vec![info]),
            _ => {}
        }
    }

    fn batch(&self, action: &str) {
        let sel = self.selected();
        if sel.is_empty() {
            return;
        }
        let n = sel.len();
        let ctx = self.ctx.clone();
        let w = self.me.clone();
        let parent: gtk::Widget = self.root.clone().upcast();
        let run = move |action: &'static str, label: String, sel: Vec<PackageInfo>| {
            let w2 = w.clone();
            ctx.device_op(
                action,
                &format!("{} apps", sel.len()),
                &label,
                move |adb, s| {
                    let mut failed = Vec::new();
                    for p in &sel {
                        let ok = match action {
                            "stop" => adb.force_stop(&s, &p.package).is_ok(),
                            "clear_cache" => match adb.clear_cache(&s, &p.package) {
                                Ok(_) => true,
                                // The same for every app: no point in trying the rest.
                                Err(e) if !adb.supports_clear_cache(&s) => return Err(e),
                                Err(_) => false,
                            },
                            "disable" => adb.set_enabled(&s, &p.package, false).is_ok(),
                            "enable" => adb.set_enabled(&s, &p.package, true).is_ok(),
                            _ if p.is_system => adb.uninstall_for_user(&s, &p.package).is_ok(),
                            _ => adb.uninstall(&s, &p.package, false).is_ok(),
                        };
                        if !ok {
                            failed.push(p.package.clone());
                        }
                    }
                    if failed.is_empty() {
                        Ok(t!("{} succeeded", sel.len()))
                    } else {
                        Err(adbscan_core::Error::Command { summary: t!("{} failed: {}", failed.len(), failed.join(", ")), code: None })
                    }
                },
                move |_| {
                    if let Some(t) = w2.upgrade() {
                        t.reload();
                    }
                },
            );
        };
        match action {
            "stop" => run("stop", t!("{} apps stopped", n), sel),
            "cache" => run("clear_cache", t!("Cache of {} apps cleared", n), sel),
            "enable" => run("enable", t!("{} apps enabled", n), sel),
            "disable" => {
                let list = self.names_list(&sel);
                util::confirm(&parent, &t!("Disable {} apps?", n), &t!("{}\n\nThey will stop working until you enable them again.", list), tr("Disable"), false, move || {
                    run("disable", t!("{} apps disabled", n), sel)
                })
            }
            "uninstall" => {
                let list = self.names_list(&sel);
                util::confirm(
                    &parent,
                    &t!("Uninstall {} apps?", n),
                    &t!("{}\n\nUser apps are deleted with their data; system apps are only removed from user 0.", list),
                    tr("Uninstall"),
                    true,
                    move || run("uninstall", t!("{} apps uninstalled", n), sel),
                )
            }
            "pull" => self.pull_apks(sel),
            _ => {}
        }
    }

    /// “• Name (package)” of each app, up to 10, for the confirmations.
    fn names_list(&self, sel: &[PackageInfo]) -> String {
        let labels = self.labels.borrow();
        let mut lines: Vec<String> = sel.iter().take(10).map(|p| format!("• {} ({})", labels.get(&p.package).cloned().unwrap_or_else(|| p.display_name()), p.package)).collect();
        if sel.len() > 10 {
            lines.push(t!("… and {} more", sel.len() - 10));
        }
        lines.join("\n")
    }

    fn pull_apks(&self, list: Vec<PackageInfo>) {
        let w = self.me.clone();
        util::pick_folder(&self.root, tr("Destination folder for the APKs"), move |dir| {
            let Some(t) = w.upgrade() else { return };
            let list = list.clone();
            let d2 = dir.clone();
            t.ctx.device_op(
                "pull_apk",
                &format!("{} apps", list.len()),
                tr("APKs extracted"),
                move |adb, s| {
                    let mut n = 0;
                    for p in &list {
                        n += adb.pull_apks(&s, &p.package, &d2)?.len();
                    }
                    Ok(t!("{} files", n))
                },
                move |ok| {
                    if ok {
                        util::open_path(&dir);
                    }
                },
            );
        });
    }

    fn setup_drop(&self) {
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let w = self.me.clone();
        target.connect_drop(move |_, value, _, _| {
            let Ok(list) = value.get::<gdk::FileList>() else {
                return false;
            };
            let apks: Vec<PathBuf> = list.files().iter().filter_map(|f| f.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("apk"))).collect();
            if apks.is_empty() {
                return false;
            }
            if let Some(t) = w.upgrade() {
                t.install_dialog(apks);
            }
            true
        });
        let root = self.root.clone();
        target.connect_enter(move |_, _, _| {
            root.add_css_class("drop-target");
            gdk::DragAction::COPY
        });
        let root = self.root.clone();
        target.connect_leave(move |_| root.remove_css_class("drop-target"));
        let root = self.root.clone();
        target.connect_drop(move |_, _, _, _| {
            root.remove_css_class("drop-target");
            false
        });
        self.root.add_controller(target);
    }

    fn install_dialog(&self, apks: Vec<PathBuf>) {
        let w = self.me.clone();
        let paths = apks.clone();
        util::run_bg(
            move || paths.iter().map(|p| (p.clone(), apk::inspect(p))).collect::<Vec<_>>(),
            move |infos| {
                if let Some(t) = w.upgrade() {
                    t.show_install(infos);
                }
            },
        );
    }

    fn show_install(&self, infos: Vec<(PathBuf, adbscan_core::Result<ApkInfo>)>) {
        let snap = self.ctx.snapshot();
        let sdk = snap.as_ref().and_then(|s| s.sdk_level());
        let abis = snap.as_ref().map(|s| s.prop("ro.product.cpu.abilist").to_string()).unwrap_or_default();
        let body = util::vbox(10);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        let mut ok_paths = Vec::new();
        let mut problems = 0;
        let split_set = infos.iter().filter_map(|(_, r)| r.as_ref().ok()).any(|i| i.split);
        for (path, res) in &infos {
            match res {
                Ok(i) => {
                    let mut notes = vec![format!("{} · v{} ({})", i.package, i.version_name, i.version_code), format::human_bytes(Some(i.size_bytes as f64))];
                    let danger = i.dangerous_permissions().len();
                    if danger > 0 {
                        notes.push(t!("{} sensitive permissions", danger));
                    }
                    let mut warn = Vec::new();
                    if let (Some(min), Some(dev)) = (i.min_sdk, sdk) {
                        if min as i64 > dev {
                            warn.push(t!("requires API {} and the phone has {}", min, dev));
                        }
                    }
                    if !i.native_abis.is_empty() && !abis.is_empty() && !i.native_abis.iter().any(|a| abis.split(',').any(|d| d == a)) {
                        warn.push(t!("no code for {}", abis));
                    }
                    if i.debuggable {
                        notes.push(tr("debuggable").into());
                    }
                    let title = if i.label.is_empty() { i.file_name.clone() } else { i.label.clone() };
                    let row = adw::ActionRow::builder()
                        .title(glib::markup_escape_text(&title))
                        .subtitle(glib::markup_escape_text(&if warn.is_empty() { notes.join(" · ") } else { format!("⚠ {}", warn.join("; ")) }))
                        .build();
                    row.add_prefix(&util::icon(if warn.is_empty() { "adb-package-symbolic" } else { "adb-dialog-warning-symbolic" }));
                    if !i.permissions.is_empty() {
                        row.set_tooltip_text(Some(&i.permissions.join("\n")));
                    }
                    if !warn.is_empty() {
                        problems += 1;
                    }
                    list.append(&row);
                    ok_paths.push(path.clone());
                }
                Err(e) => {
                    problems += 1;
                    let row = adw::ActionRow::builder()
                        .title(glib::markup_escape_text(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))
                        .subtitle(glib::markup_escape_text(&t!("Cannot be read: {}", e)))
                        .build();
                    row.add_prefix(&util::icon("adb-circle-x-symbolic"));
                    list.append(&row);
                }
            }
        }
        body.append(&list);
        let opts = adw::PreferencesGroup::new();
        let replace = adw::SwitchRow::builder().title(tr("Replace if already installed")).subtitle(tr("Keeps the data (-r)")).active(true).build();
        let downgrade = adw::SwitchRow::builder().title(tr("Allow downgrade")).subtitle("-d").build();
        let grant = adw::SwitchRow::builder().title(tr("Grant all permissions")).subtitle("-g").build();
        let test = adw::SwitchRow::builder().title(tr("Allow test APKs")).subtitle("-t").build();
        for r in [&replace, &downgrade, &grant, &test] {
            opts.add(r);
        }
        body.append(&opts);
        let heading = if ok_paths.len() > 1 && split_set {
            tr("Install split APKs").to_string()
        } else if ok_paths.len() == 1 {
            tr("Install app").to_string()
        } else {
            t!("Install {} apps", ok_paths.len())
        };
        let dialog = adw::AlertDialog::new(Some(&heading), Some(if problems > 0 { tr("Review the warnings before continuing.") } else { tr("It will be installed on the selected device.") }));
        dialog.set_extra_child(Some(&body));
        dialog.add_responses(&[("cancel", tr("Cancel")), ("install", tr("Install"))]);
        dialog.set_response_appearance("install", adw::ResponseAppearance::Suggested);
        dialog.set_response_enabled("install", !ok_paths.is_empty());
        dialog.set_default_response(Some("install"));
        let w = self.me.clone();
        dialog.connect_response(None, move |_, resp| {
            if resp != "install" {
                return;
            }
            let Some(t) = w.upgrade() else { return };
            let opts = InstallOptions { replace: replace.is_active(), downgrade: downgrade.is_active(), grant_all: grant.is_active(), allow_test: test.is_active() };
            let paths = ok_paths.clone();
            let together = split_set && paths.len() > 1;
            let names = paths.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ");
            t.ctx.toast(tr("Installing…"));
            let w2 = t.me.clone();
            t.ctx.device_op(
                "install",
                &names,
                tr("Installation complete"),
                move |adb, s| {
                    if together {
                        return adb.install(&s, &paths, &opts).map(|r| r.summary());
                    }
                    let mut errors = Vec::new();
                    for p in &paths {
                        if let Err(e) = adb.install(&s, std::slice::from_ref(p), &opts) {
                            errors.push(format!("{}: {e}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                        }
                    }
                    if errors.is_empty() {
                        Ok("Success".into())
                    } else {
                        Err(adbscan_core::Error::Command { summary: errors.join(" · "), code: None })
                    }
                },
                move |_| {
                    if let Some(t) = w2.upgrade() {
                        t.reload();
                    }
                },
            );
        });
        dialog.present(Some(&self.root));
    }
}

impl Page for AppsPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn on_event(&self, ev: &Event) {
        if matches!(ev, Event::Snapshot) {
            self.live_tick();
        }
        if let Event::ShowApp(pkg) = ev {
            *self.pending_select.borrow_mut() = Some(pkg.clone());
            self.filter_dd.set_selected(3);
            self.search.set_text(pkg);
            self.select_pending();
            return;
        }
        if matches!(ev, Event::Devices) && self.ctx.ready_serial() != *self.loaded_for.borrow() {
            self.store.remove_all();
            self.details_stack.set_visible_child_name("empty");
            self.update_count();
            if self.ctx.ready_serial().is_some() {
                self.reload();
            }
        }
    }
    fn shown(&self) {
        if self.store.n_items() == 0 && self.ctx.ready_serial().is_some() {
            self.reload();
        }
    }
}
