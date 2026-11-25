//! Device files: browsing, transfers with progress and cancellation, drag and
//! drop, rename, delete and preview.

use adbscan_core::{i18n::tr, t, tc};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adbscan_core::adb::{self, CancelToken, Progress};
use adbscan_core::format;
use adbscan_core::models::FileEntry;
use adbscan_core::parsers;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

const HOME: &str = "/storage/emulated/0";

enum Transfer {
    Push(Vec<PathBuf>, String),
    Pull(Vec<(String, u64)>, PathBuf),
    /// Photo backup: (path on the phone, size, destination) and root folder.
    Photos(Vec<(String, u64, PathBuf)>, PathBuf),
}

enum TransferMsg {
    Progress { name: String, index: usize, total: usize, p: Progress, speed: f64 },
    Done { ok: usize, errors: Vec<String>, cancelled: bool },
}

struct FilesPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::Overlay,
    path: RefCell<String>,
    crumbs: gtk::Box,
    store: gio::ListStore,
    selection: gtk::MultiSelection,
    filter: gtk::CustomFilter,
    show_hidden: Rc<Cell<bool>>,
    status: gtk::Label,
    stack: gtk::Stack,
    error_page: adw::StatusPage,
    places: gtk::DropDown,
    places_paths: RefCell<Vec<String>>,
    sel_bar: gtk::Revealer,
    sel_label: gtk::Label,
    rename_btn: gtk::Button,
    xfer_bar: gtk::Revealer,
    xfer_label: gtk::Label,
    xfer_progress: gtk::ProgressBar,
    xfer_cancel: RefCell<Option<CancelToken>>,
    history: RefCell<Vec<String>>,
    loaded_for: RefCell<Option<String>>,
    /// Periodic check of the folder while it is shown.
    watch_last: std::cell::Cell<f64>,
    watch_busy: std::cell::Cell<bool>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    FilesPage::new(ctx)
}

fn entry_of(o: &glib::Object) -> Option<FileEntry> {
    o.downcast_ref::<glib::BoxedAnyObject>().map(|b| b.borrow::<FileEntry>().clone())
}

pub fn icon_for(e: &FileEntry) -> &'static str {
    if e.is_dir || (e.is_link && !e.name.contains('.')) {
        return "adb-folder-symbolic";
    }
    let ext = e.name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "heic" | "bmp" => "adb-camera-photo-symbolic",
        "mp4" | "mkv" | "webm" | "3gp" | "mov" => "adb-camera-video-symbolic",
        "mp3" | "m4a" | "ogg" | "opus" | "flac" | "wav" | "aac" => "adb-audio-volume-high-symbolic",
        "apk" | "apks" | "xapk" => "adb-package-symbolic",
        "zip" | "rar" | "7z" | "tar" | "gz" | "xz" => "adb-hard-drive-symbolic",
        "txt" | "log" | "json" | "xml" | "csv" | "md" | "prop" | "conf" | "ini" => "adb-file-text-symbolic",
        _ => "adb-file-text-symbolic",
    }
}

fn is_text(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    ["txt", "log", "json", "xml", "csv", "md", "prop", "conf", "ini", "sh", "rc", "cfg", "html", "js", "kt", "java", "py"].contains(&ext.as_str())
}

fn is_image(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    ["jpg", "jpeg", "png", "webp", "gif", "bmp"].contains(&ext.as_str())
}

impl FilesPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let show_hidden = Rc::new(Cell::new(ctx.settings.borrow().show_hidden_files));
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let filter = {
            let sh = show_hidden.clone();
            gtk::CustomFilter::new(move |o| sh.get() || entry_of(o).is_some_and(|e| !e.is_hidden()))
        };
        let fmodel = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
        let selection = gtk::MultiSelection::new(Some(fmodel));

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let row = util::hbox(12);
            row.set_margin_top(5);
            row.set_margin_bottom(5);
            row.set_margin_start(6);
            row.set_margin_end(6);
            let icon = gtk::Image::new();
            icon.set_pixel_size(22);
            let name = util::label("", &[]);
            name.set_hexpand(true);
            name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            let size = util::label("", &["dim", "numeric", "caption"]);
            size.set_width_chars(10);
            size.set_xalign(1.0);
            let date = util::label("", &["dim", "numeric", "caption"]);
            date.set_width_chars(16);
            date.set_xalign(1.0);
            row.append(&icon);
            row.append(&name);
            row.append(&size);
            row.append(&date);
            item.set_child(Some(&row));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(e) = item.item().and_then(|o| entry_of(&o)) else {
                return;
            };
            let row = item.child().and_downcast::<gtk::Box>().unwrap();
            let icon = row.first_child().unwrap().downcast::<gtk::Image>().unwrap();
            let name = icon.next_sibling().unwrap().downcast::<gtk::Label>().unwrap();
            let size = name.next_sibling().unwrap().downcast::<gtk::Label>().unwrap();
            let date = size.next_sibling().unwrap().downcast::<gtk::Label>().unwrap();
            icon.set_icon_name(Some(icon_for(&e)));
            name.set_text(&if e.is_link { format!("{} → {}", e.name, e.link_target) } else { e.name.clone() });
            name.set_opacity(if e.is_hidden() { 0.6 } else { 1.0 });
            size.set_text(&if e.is_dir { String::new() } else { format::human_bytes(Some(e.size as f64)) });
            date.set_text(&e.date);
        });
        let list = gtk::ListView::builder().model(&selection).factory(&factory).build();
        list.add_css_class("navigation-sidebar");
        let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).build();

        // Navigation bar
        let nav = util::hbox(6);
        nav.set_margin_top(10);
        nav.set_margin_start(10);
        nav.set_margin_end(10);
        let back = util::icon_button("adb-arrow-left-symbolic", tr("Back"));
        let up = util::icon_button("adb-arrow-up-symbolic", tr("Up"));
        let crumbs = util::hbox(0);
        crumbs.add_css_class("linked");
        crumbs.add_css_class("breadcrumb");
        let crumbs_scroll = gtk::ScrolledWindow::builder().child(&crumbs).hexpand(true).vscrollbar_policy(gtk::PolicyType::Never).build();
        let places = gtk::DropDown::from_strings(&[tr("Internal")]);
        places.set_tooltip_text(Some(tr("Places")));
        let hidden_btn = gtk::ToggleButton::builder().icon_name("adb-view-reveal-symbolic").tooltip_text(tr("Show hidden files")).active(show_hidden.get()).build();
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Refresh"));
        let mkdir = util::icon_button("adb-list-add-symbolic", tr("New folder"));
        let photos = util::icon_button("adb-images-symbolic", tr("Copy new photos and videos to the laptop"));
        let upload = util::pill_button("adb-send-to-symbolic", tr("Up"));
        upload.add_css_class("suggested-action");
        for w in [
            back.upcast_ref::<gtk::Widget>(),
            up.upcast_ref(),
            crumbs_scroll.upcast_ref(),
            places.upcast_ref(),
            hidden_btn.upcast_ref(),
            refresh.upcast_ref(),
            mkdir.upcast_ref(),
            photos.upcast_ref(),
            upload.upcast_ref(),
        ] {
            nav.append(w);
        }
        let status = util::label("", &["dim", "caption"]);
        status.set_margin_start(14);
        status.set_margin_top(4);
        status.set_margin_bottom(2);

        let error_page = adw::StatusPage::builder().icon_name("adb-triangle-alert-symbolic").title(tr("Cannot open")).build();
        let empty = adw::StatusPage::builder().icon_name("adb-folder-symbolic").title(tr("Empty folder")).description(tr("Drag files here to upload them")).build();
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).transition_duration(150).build();
        stack.add_named(&scroll, Some("list"));
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&error_page, Some("error"));
        let loading = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        stack.add_named(&loading, Some("loading"));

        // Selection bar
        let sel_label = util::label("", &["heading"]);
        let sel_box = util::hbox(6);
        sel_box.add_css_class("transfer-bar");
        sel_box.append(&sel_label);
        let sp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sp.set_hexpand(true);
        sel_box.append(&sp);
        let dl = util::pill_button("adb-save-symbolic", tr("Download"));
        let rename_btn = util::pill_button("adb-document-edit-symbolic", tr("Rename"));
        let copy_path = util::icon_button("adb-copy-symbolic", tr("Copy path"));
        let del = util::pill_button("adb-edit-delete-symbolic", tr("Delete"));
        del.add_css_class("destructive-action");
        for b in [&dl, &rename_btn, &copy_path, &del] {
            sel_box.append(b);
        }
        let sel_bar = gtk::Revealer::builder().transition_type(gtk::RevealerTransitionType::SlideUp).child(&sel_box).build();

        // Transfer bar
        let xfer_label = util::label("", &[]);
        xfer_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let xfer_progress = gtk::ProgressBar::new();
        xfer_progress.set_hexpand(true);
        xfer_progress.set_valign(gtk::Align::Center);
        let xfer_cancel_btn = gtk::Button::with_label(tr("Cancel"));
        let xfer_box = util::vbox(6);
        xfer_box.add_css_class("transfer-bar");
        let xrow = util::hbox(10);
        xrow.append(&xfer_progress);
        xrow.append(&xfer_cancel_btn);
        xfer_box.append(&xfer_label);
        xfer_box.append(&xrow);
        let xfer_bar = gtk::Revealer::builder().transition_type(gtk::RevealerTransitionType::SlideUp).child(&xfer_box).build();

        let body = util::vbox(0);
        body.append(&nav);
        body.append(&status);
        body.append(&stack);
        body.append(&sel_bar);
        body.append(&xfer_bar);
        let root = gtk::Overlay::builder().child(&body).build();

        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            path: RefCell::new(HOME.into()),
            crumbs,
            store,
            selection,
            filter,
            show_hidden,
            status,
            stack,
            error_page,
            places,
            places_paths: RefCell::new(vec![HOME.into()]),
            sel_bar,
            sel_label,
            rename_btn,
            xfer_bar,
            xfer_label,
            xfer_progress,
            xfer_cancel: RefCell::new(None),
            history: RefCell::new(Vec::new()),
            loaded_for: RefCell::new(None),
            watch_last: std::cell::Cell::new(0.0),
            watch_busy: std::cell::Cell::new(false),
        });

        macro_rules! on_click {
            ($btn:expr, |$t:ident, $b:ident| $body:expr) => {{
                let w = this.me.clone();
                $btn.connect_clicked(move |$b| {
                    if let Some($t) = w.upgrade() {
                        $body;
                    }
                });
            }};
        }
        on_click!(back, |t, _b| t.go_back());
        on_click!(up, |t, _b| {
            let p = adb::remote_parent(&t.path.borrow());
            t.navigate(&p, true)
        });
        on_click!(refresh, |t, _b| t.load());
        on_click!(mkdir, |t, _b| t.new_folder());
        on_click!(photos, |t, _b| t.copy_photos());
        on_click!(upload, |t, b| {
            let w = t.me.clone();
            util::pick_files(b, tr("Upload to the device"), &[], move |paths| {
                if let Some(t) = w.upgrade() {
                    let dest = t.path.borrow().clone();
                    t.run_transfer(Transfer::Push(paths, dest));
                }
            })
        });
        on_click!(dl, |t, _b| t.download_selected());
        on_click!(this.rename_btn, |t, _b| t.rename_selected());
        on_click!(copy_path, |t, b| {
            let paths: Vec<String> = t.selected().iter().map(|e| adb::remote_join(&t.path.borrow(), &e.name)).collect();
            util::copy_to_clipboard(b, &paths.join("\n"));
            t.ctx.toast(tr("Path copied"))
        });
        on_click!(del, |t, _b| t.delete_selected());
        on_click!(xfer_cancel_btn, |t, _b| {
            if let Some(c) = t.xfer_cancel.borrow().as_ref() {
                c.cancel();
            }
        });
        {
            let w = this.me.clone();
            hidden_btn.connect_toggled(move |b| {
                if let Some(t) = w.upgrade() {
                    t.show_hidden.set(b.is_active());
                    t.ctx.settings.borrow_mut().show_hidden_files = b.is_active();
                    t.filter.changed(gtk::FilterChange::Different);
                    t.update_status();
                }
            });
        }
        {
            let w = this.me.clone();
            list.connect_activate(move |_, pos| {
                let Some(t) = w.upgrade() else { return };
                let Some(e) = t.selection.item(pos).and_then(|o| entry_of(&o)) else {
                    return;
                };
                t.open_entry(&e);
            });
        }
        {
            let w = this.me.clone();
            this.selection.connect_selection_changed(move |_, _, _| {
                if let Some(t) = w.upgrade() {
                    let n = t.selected().len();
                    t.sel_bar.set_reveal_child(n > 0);
                    t.sel_label.set_text(&tc!("items", "{} selected", n));
                    t.rename_btn.set_sensitive(n == 1);
                }
            });
        }
        {
            let w = this.me.clone();
            this.places.connect_selected_notify(move |dd| {
                let Some(t) = w.upgrade() else { return };
                let p = t.places_paths.borrow().get(dd.selected() as usize).cloned();
                if let Some(p) = p {
                    if p != *t.path.borrow() {
                        t.navigate(&p, true);
                    }
                }
            });
        }
        this.setup_drop();
        this.render_crumbs();
        this
    }

    fn selected(&self) -> Vec<FileEntry> {
        let set = self.selection.selection();
        (0..set.size()).filter_map(|i| self.selection.item(set.nth(i as u32))).filter_map(|o| entry_of(&o)).collect()
    }

    fn navigate(&self, path: &str, push_history: bool) {
        let old = self.path.replace(path.to_string());
        if push_history && old != path {
            self.history.borrow_mut().push(old);
        }
        self.render_crumbs();
        self.load();
    }

    fn go_back(&self) {
        let prev = self.history.borrow_mut().pop();
        if let Some(p) = prev {
            self.navigate(&p, false);
        }
    }

    fn render_crumbs(&self) {
        while let Some(c) = self.crumbs.first_child() {
            self.crumbs.remove(&c);
        }
        let path = self.path.borrow().clone();
        let mut acc = String::new();
        let mut parts: Vec<(String, String)> = vec![("/".into(), "/".into())];
        for seg in path.split('/').filter(|s| !s.is_empty()) {
            acc.push('/');
            acc.push_str(seg);
            let label = if acc == HOME { tr("Internal").to_string() } else { seg.to_string() };
            parts.push((label, acc.clone()));
        }
        // Simplifies /storage/emulated/0 → “Internal”.
        if path.starts_with(HOME) {
            parts.retain(|(_, p)| p == "/" || p.len() >= HOME.len());
            parts.remove(0);
        }
        for (label, target) in parts {
            let b = gtk::Button::with_label(&label);
            b.add_css_class("flat");
            let w = self.me.clone();
            b.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    t.navigate(&target, true);
                }
            });
            self.crumbs.append(&b);
        }
    }

    fn load(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        *self.loaded_for.borrow_mut() = Some(serial.clone());
        let path = self.path.borrow().clone();
        self.stack.set_visible_child_name("loading");
        let w = self.me.clone();
        util::run_bg(
            move || adb.list_dir(&serial, &path).map(|e| (path, e)),
            move |res| {
                let Some(t) = w.upgrade() else { return };
                match res {
                    Ok((path, entries)) => {
                        if path != *t.path.borrow() {
                            return;
                        }
                        let objs: Vec<glib::BoxedAnyObject> = entries.into_iter().map(glib::BoxedAnyObject::new).collect();
                        t.store.splice(0, t.store.n_items(), &objs);
                        t.update_status();
                    }
                    Err(e) => {
                        t.store.remove_all();
                        t.error_page.set_description(Some(&e.to_string()));
                        t.stack.set_visible_child_name("error");
                        t.status.set_text("");
                    }
                }
            },
        );
        self.refresh_places();
    }

    /// Every 5 s, while the page is visible: if the folder's content changed on
    /// the phone, it is refreshed without losing the view.
    fn watch_dir(&self) {
        let now = adbscan_core::models::now_secs();
        if self.watch_busy.get() || now - self.watch_last.get() < 5.0 || self.stack.visible_child_name().as_deref() == Some("loading") {
            return;
        }
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        self.watch_last.set(now);
        self.watch_busy.set(true);
        let path = self.path.borrow().clone();
        let w = self.me.clone();
        util::run_bg(
            move || adb.list_dir(&serial, &path).map(|e| (path, e)),
            move |res| {
                let Some(t) = w.upgrade() else { return };
                t.watch_busy.set(false);
                let Ok((path, entries)) = res else { return };
                if path != *t.path.borrow() {
                    return;
                }
                let current: Vec<FileEntry> = (0..t.store.n_items()).filter_map(|i| t.store.item(i)).filter_map(|o| entry_of(&o)).collect();
                if current != entries {
                    let objs: Vec<glib::BoxedAnyObject> = entries.into_iter().map(glib::BoxedAnyObject::new).collect();
                    t.store.splice(0, t.store.n_items(), &objs);
                    t.update_status();
                }
            },
        );
    }

    fn update_status(&self) {
        let total = self.store.n_items();
        let entries: Vec<FileEntry> = (0..total).filter_map(|i| self.store.item(i)).filter_map(|o| entry_of(&o)).collect();
        let visible: Vec<&FileEntry> = entries.iter().filter(|e| self.show_hidden.get() || !e.is_hidden()).collect();
        let dirs = visible.iter().filter(|e| e.is_dir).count();
        let bytes: i64 = visible.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
        let hidden = entries.len() - visible.len();
        let mut text = t!("{} folders · {} files · {}", dirs, visible.len() - dirs, format::human_bytes(Some(bytes as f64)));
        if hidden > 0 {
            text.push_str(&t!(" · {} hidden", hidden));
        }
        if let Some(s) = self.ctx.snapshot() {
            if self.path.borrow().starts_with("/storage/emulated") || self.path.borrow().starts_with("/sdcard") {
                text.push_str(&t!(" · {} free of {}", format::human_kib(s.external_storage.available_kib), format::human_kib(s.external_storage.total_kib)));
            }
        }
        self.status.set_text(&text);
        self.stack.set_visible_child_name(if visible.is_empty() { "empty" } else { "list" });
    }

    fn refresh_places(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let w = self.me.clone();
        util::run_bg(
            move || {
                let df = adb.shell(&serial, &["df", "-k"], std::time::Duration::from_secs(10)).map(|r| r.stdout).unwrap_or_default();
                parsers::external_sd_mounts(&parsers::parse_df(&df))
            },
            move |sd| {
                let Some(t) = w.upgrade() else { return };
                let mut names = vec![tr("Internal").to_string(), tr("Downloads").into(), "DCIM".into(), tr("Documents").into(), tr("Pictures").into(), tr("ADB temp").into()];
                let mut paths = vec![HOME.to_string(), format!("{HOME}/Download"), format!("{HOME}/DCIM"), format!("{HOME}/Documents"), format!("{HOME}/Pictures"), "/data/local/tmp".into()];
                for s in sd {
                    names.push(t!("SD card ({})", s.mount.trim_start_matches("/storage/")));
                    paths.push(s.mount);
                }
                if *t.places_paths.borrow() != paths {
                    *t.places_paths.borrow_mut() = paths.clone();
                    let model = gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
                    t.places.set_model(Some(&model));
                }
                let cur = t.path.borrow().clone();
                let idx = paths.iter().position(|p| *p == cur).unwrap_or(gtk::INVALID_LIST_POSITION as usize);
                t.places.set_selected(idx as u32);
            },
        );
    }

    fn open_entry(&self, e: &FileEntry) {
        let full = adb::remote_join(&self.path.borrow(), &e.name);
        if e.is_dir || (e.is_link && !e.link_target.contains('.')) {
            let target = if e.is_link && e.link_target.starts_with('/') { e.link_target.clone() } else { full };
            self.navigate(&target, true);
            return;
        }
        if is_text(&e.name) && e.size < 512 * 1024 {
            self.preview_text(&full, &e.name);
        } else if is_image(&e.name) && e.size < 40 * 1024 * 1024 {
            self.preview_image(&full, e);
        } else {
            self.run_transfer(Transfer::Pull(vec![(full, e.size as u64)], adbscan_core::paths::downloads_dir()));
        }
    }

    fn preview_text(&self, full: &str, name: &str) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let path = full.to_string();
        let name = name.to_string();
        let root = self.root.clone();
        let ctx = self.ctx.clone();
        util::run_bg(
            move || adb.read_text(&serial, &path, 512 * 1024),
            move |r| match r {
                Ok(text) => {
                    let view = gtk::TextView::builder().editable(false).monospace(true).wrap_mode(gtk::WrapMode::WordChar).build();
                    view.buffer().set_text(&text);
                    view.add_css_class("console");
                    let sw = gtk::ScrolledWindow::builder().child(&view).min_content_height(420).min_content_width(640).build();
                    let d = adw::Dialog::builder().title(&name).content_width(760).content_height(560).build();
                    let tv = adw::ToolbarView::new();
                    tv.add_top_bar(&adw::HeaderBar::new());
                    tv.set_content(Some(&sw));
                    d.set_child(Some(&tv));
                    d.present(Some(&root));
                }
                Err(e) => ctx.toast(&t!("Could not read: {}", e)),
            },
        );
    }

    fn preview_image(&self, full: &str, e: &FileEntry) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let tmp = adbscan_core::paths::cache_dir().join("preview");
        let _ = std::fs::create_dir_all(&tmp);
        let local = tmp.join(&e.name);
        let (remote, l2, size, name) = (full.to_string(), local.clone(), e.size as u64, e.name.clone());
        let root = self.root.clone();
        let ctx = self.ctx.clone();
        util::run_bg(
            move || adb.pull(&serial, &remote, &l2, size, &CancelToken::new(), |_| {}),
            move |r| match r {
                Ok(_) => {
                    let pic = gtk::Picture::for_filename(&local);
                    pic.set_can_shrink(true);
                    pic.set_content_fit(gtk::ContentFit::Contain);
                    let d = adw::Dialog::builder().title(&name).content_width(900).content_height(700).build();
                    let tv = adw::ToolbarView::new();
                    let hb = adw::HeaderBar::new();
                    let save = util::icon_button("adb-save-symbolic", tr("Save to Downloads"));
                    let l3 = local.clone();
                    let c2 = ctx.clone();
                    save.connect_clicked(move |_| {
                        let dest = adbscan_core::paths::downloads_dir().join(l3.file_name().unwrap());
                        match std::fs::copy(&l3, &dest) {
                            Ok(_) => c2.toast_action(tr("Saved to Downloads"), tr("Open"), move || util::open_path(&dest)),
                            Err(e) => c2.toast(&format!("Error: {e}")),
                        }
                    });
                    hb.pack_start(&save);
                    tv.add_top_bar(&hb);
                    tv.set_content(Some(&pic));
                    d.set_child(Some(&tv));
                    d.present(Some(&root));
                }
                Err(e) => ctx.toast(&t!("Could not open: {}", e)),
            },
        );
    }

    fn new_folder(&self) {
        let entry = gtk::Entry::builder().placeholder_text(tr("Folder name")).activates_default(true).build();
        let d = adw::AlertDialog::new(Some(tr("New folder")), None);
        d.set_extra_child(Some(&entry));
        d.add_responses(&[("cancel", tr("Cancel")), ("ok", tr("Create"))]);
        d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        d.set_default_response(Some("ok"));
        let w = self.me.clone();
        d.connect_response(None, move |_, r| {
            let name = entry.text().trim().to_string();
            if r != "ok" || name.is_empty() || name.contains('/') {
                return;
            }
            let Some(t) = w.upgrade() else { return };
            let full = adb::remote_join(&t.path.borrow(), &name);
            let w2 = t.me.clone();
            t.ctx.device_op(
                "mkdir",
                &full.clone(),
                tr("Folder created"),
                move |adb, s| adb.make_dir(&s, &full).map(|r| r.summary()),
                move |_| {
                    if let Some(t) = w2.upgrade() {
                        t.load();
                    }
                },
            );
        });
        d.present(Some(&self.root));
    }

    fn rename_selected(&self) {
        let sel = self.selected();
        let [e] = sel.as_slice() else { return };
        let entry = gtk::Entry::builder().text(&e.name).activates_default(true).build();
        let d = adw::AlertDialog::new(Some(tr("Rename")), None);
        d.set_extra_child(Some(&entry));
        d.add_responses(&[("cancel", tr("Cancel")), ("ok", tr("Rename"))]);
        d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        d.set_default_response(Some("ok"));
        let w = self.me.clone();
        let old = e.name.clone();
        d.connect_response(None, move |_, r| {
            let name = entry.text().trim().to_string();
            if r != "ok" || name.is_empty() || name.contains('/') || name == old {
                return;
            }
            let Some(t) = w.upgrade() else { return };
            let dir = t.path.borrow().clone();
            let (from, to) = (adb::remote_join(&dir, &old), adb::remote_join(&dir, &name));
            let w2 = t.me.clone();
            t.ctx.device_op(
                "rename",
                &format!("{from} → {to}"),
                tr("Renamed"),
                move |adb, s| adb.rename(&s, &from, &to).map(|r| r.summary()),
                move |_| {
                    if let Some(t) = w2.upgrade() {
                        t.load();
                    }
                },
            );
        });
        d.present(Some(&self.root));
    }

    fn delete_selected(&self) {
        let sel = self.selected();
        if sel.is_empty() {
            return;
        }
        let dir = self.path.borrow().clone();
        let targets: Vec<String> = sel.iter().map(|e| adb::remote_join(&dir, &e.name)).collect();
        let names = sel.iter().take(5).map(|e| e.name.clone()).collect::<Vec<_>>().join(", ");
        let more = if sel.len() > 5 { t!(" and {} more", sel.len() - 5) } else { String::new() };
        let ctx = self.ctx.clone();
        let w = self.me.clone();
        util::confirm(
            &self.root,
            &t!("Delete {} items?", sel.len()),
            &t!("{}{}\n\nThey will be permanently deleted from the phone (there is no trash).", names, more),
            tr("Delete"),
            true,
            move || {
                let w2 = w.clone();
                ctx.device_op(
                    "delete",
                    &targets.join(", "),
                    tr("Deleted"),
                    move |adb, s| {
                        for t in &targets {
                            adb.delete(&s, t)?;
                        }
                        Ok(t!("{} items", targets.len()))
                    },
                    move |_| {
                        if let Some(t) = w2.upgrade() {
                            t.load();
                        }
                    },
                );
            },
        );
    }

    fn download_selected(&self) {
        let sel = self.selected();
        let dir = self.path.borrow().clone();
        let items: Vec<(String, u64)> = sel.iter().map(|e| (adb::remote_join(&dir, &e.name), e.size as u64)).collect();
        let w = self.me.clone();
        util::pick_folder(&self.root, tr("Save to…"), move |dest| {
            if let Some(t) = w.upgrade() {
                t.run_transfer(Transfer::Pull(items.clone(), dest));
            }
        });
    }

    /// Backup of new photos and videos (see `adbscan_core::photos`). The first
    /// time it asks for the folder; afterwards it remembers it and offers to
    /// change it.
    fn copy_photos(&self) {
        let dir = self.ctx.settings.borrow().photos_dir.clone();
        let w = self.me.clone();
        let pick = move || {
            let w = w.clone();
            let Some(t) = w.upgrade() else { return };
            util::pick_folder(&t.root, tr("Folder for the phone's photos"), move |dest| {
                if let Some(t) = w.upgrade() {
                    t.ctx.settings.borrow_mut().photos_dir = dest.display().to_string();
                    t.ctx.save_settings();
                    t.start_photo_copy(dest);
                }
            });
        };
        if dir.is_empty() || !std::path::Path::new(&dir).is_dir() {
            pick();
            return;
        }
        let dialog = adw::AlertDialog::new(Some(tr("Copy new photos")), Some(&t!("The photos and videos not yet in this folder will be copied:\n{}\n\nNothing on the phone is deleted or changed.", dir)));
        dialog.add_responses(&[("cancel", tr("Cancel")), ("other", tr("Another folder…")), ("copy", tr("Copy"))]);
        dialog.set_response_appearance("copy", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("copy"));
        dialog.set_close_response("cancel");
        let w = self.me.clone();
        dialog.connect_response(None, move |_, resp| match resp {
            "copy" => {
                if let Some(t) = w.upgrade() {
                    t.start_photo_copy(PathBuf::from(&dir));
                }
            }
            "other" => pick(),
            _ => {}
        });
        dialog.present(Some(&self.root));
    }

    fn start_photo_copy(&self, dest: PathBuf) {
        if self.xfer_cancel.borrow().is_some() {
            self.ctx.toast(tr("A transfer is already in progress"));
            return;
        }
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        self.ctx.toast(tr("Looking for new photos…"));
        let w = self.me.clone();
        let d = dest.clone();
        util::run_bg(
            move || adb.list_media(&serial).map(|list| adbscan_core::photos::plan(&list, &d, |p| std::fs::metadata(p).ok().map(|m| m.len()))),
            move |res| {
                let Some(t) = w.upgrade() else { return };
                match res {
                    Ok(items) if items.is_empty() => {
                        let d = dest.clone();
                        t.ctx.toast_action(tr("No new photos: everything is copied"), tr("Open folder"), move || util::open_path(&d));
                    }
                    Ok(items) => t.run_transfer(Transfer::Photos(items, dest)),
                    Err(e) => t.ctx.toast(&t!("Could not read the photos: {}", e)),
                }
            },
        );
    }

    fn setup_drop(&self) {
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let w = self.me.clone();
        target.connect_drop(move |_, value, _, _| {
            let Some(t) = w.upgrade() else { return false };
            t.root.remove_css_class("drop-target");
            let Ok(list) = value.get::<gdk::FileList>() else {
                return false;
            };
            let paths: Vec<PathBuf> = list.files().iter().filter_map(|f| f.path()).collect();
            if paths.is_empty() || t.ctx.ready_serial().is_none() {
                return false;
            }
            let dest = t.path.borrow().clone();
            t.run_transfer(Transfer::Push(paths, dest));
            true
        });
        let r = self.root.clone();
        target.connect_enter(move |_, _, _| {
            r.add_css_class("drop-target");
            gdk::DragAction::COPY
        });
        let r = self.root.clone();
        target.connect_leave(move |_| r.remove_css_class("drop-target"));
        self.root.add_controller(target);
    }

    fn run_transfer(&self, job: Transfer) {
        if self.xfer_cancel.borrow().is_some() {
            self.ctx.toast(tr("A transfer is already in progress"));
            return;
        }
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let cancel = CancelToken::new();
        *self.xfer_cancel.borrow_mut() = Some(cancel.clone());
        self.xfer_progress.set_fraction(0.0);
        self.xfer_label.set_text(tr("Preparing…"));
        self.xfer_bar.set_reveal_child(true);
        let (tx, rx) = async_channel::unbounded::<TransferMsg>();
        let is_push = matches!(job, Transfer::Push(..));
        let pull_dest = match &job {
            Transfer::Pull(_, d) | Transfer::Photos(_, d) => Some(d.clone()),
            Transfer::Push(..) => None,
        };
        let verb = match &job {
            Transfer::Push(..) => tr("Uploading"),
            Transfer::Pull(..) => tr("Downloading"),
            Transfer::Photos(..) => tr("Copying"),
        };
        let action = match &job {
            Transfer::Push(..) => "push",
            Transfer::Pull(..) => "pull",
            Transfer::Photos(..) => "copy_photos",
        };
        std::thread::spawn(move || {
            let mut ok = 0;
            let mut errors = Vec::new();
            let items: Vec<(String, String, u64)> = match &job {
                Transfer::Push(paths, dest) => paths
                    .iter()
                    .map(|p| {
                        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let size = dir_size(p);
                        (p.display().to_string(), adb::remote_join(dest, &name), size)
                    })
                    .collect(),
                Transfer::Pull(list, dest) => list
                    .iter()
                    .map(|(remote, size)| {
                        let name = remote.rsplit('/').next().unwrap_or(tr("file")).to_string();
                        (remote.clone(), dest.join(name).display().to_string(), *size)
                    })
                    .collect(),
                Transfer::Photos(list, _) => list.iter().map(|(remote, size, local)| (remote.clone(), local.display().to_string(), *size)).collect(),
            };
            let total = items.len();
            for (i, (from, to, size)) in items.into_iter().enumerate() {
                if cancel.is_cancelled() {
                    break;
                }
                let name = from.rsplit('/').next().unwrap_or("").to_string();
                let start = std::time::Instant::now();
                let tx2 = tx.clone();
                let n2 = name.clone();
                let cb = move |p: Progress| {
                    let secs = start.elapsed().as_secs_f64().max(0.001);
                    let _ = tx2.send_blocking(TransferMsg::Progress { name: n2.clone(), index: i, total, p, speed: p.done as f64 / secs });
                };
                let r = if is_push {
                    adb.push(&serial, std::path::Path::new(&from), &to, &cancel, cb)
                } else {
                    // The photo backup keeps the subfolders (DCIM/Camera…).
                    if let Some(parent) = std::path::Path::new(&to).parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    adb.pull(&serial, &from, std::path::Path::new(&to), size, &cancel, cb)
                };
                match r {
                    Ok(_) => ok += 1,
                    Err(e) => errors.push(format!("{name}: {e}")),
                }
            }
            let _ = tx.send_blocking(TransferMsg::Done { ok, errors, cancelled: cancel.is_cancelled() });
        });
        let w = self.me.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                let Some(t) = w.upgrade() else { break };
                match msg {
                    TransferMsg::Progress { name, index, total, p, speed } => {
                        t.xfer_progress.set_fraction(p.fraction());
                        t.xfer_label.set_text(&t!("{} {} ({}/{}) · {} of {} · {}/s", verb, name, index + 1, total, format::human_bytes(Some(p.done as f64)), format::human_bytes(Some(p.total as f64)), format::human_bytes(Some(speed))));
                    }
                    TransferMsg::Done { ok, errors, cancelled } => {
                        t.xfer_cancel.borrow_mut().take();
                        t.xfer_bar.set_reveal_child(false);
                        t.ctx.audit(action, None, errors.is_empty() && !cancelled, Some(&errors.join("; ")));
                        if cancelled {
                            t.ctx.toast(tr("Transfer cancelled"));
                        } else if errors.is_empty() {
                            match (&pull_dest, is_push) {
                                (Some(d), false) => {
                                    let d = d.clone();
                                    let what = if action == "copy_photos" { tr("photos and videos copied") } else { tr("items downloaded") };
                                    t.ctx.toast_action(&format!("{ok} {what}"), tr("Open folder"), move || util::open_path(&d));
                                }
                                _ => t.ctx.toast(&t!("{} items uploaded", ok)),
                            }
                        } else {
                            util::show_error(&t.root, tr("Some transfers failed"), &errors.join("\n"));
                        }
                        if is_push {
                            t.load();
                        }
                        break;
                    }
                }
            }
        });
    }
}

fn dir_size(p: &std::path::Path) -> u64 {
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| dir_size(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

impl Page for FilesPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn on_event(&self, ev: &Event) {
        if matches!(ev, Event::Snapshot) {
            self.watch_dir();
        }
        if matches!(ev, Event::Devices) && self.ctx.ready_serial() != *self.loaded_for.borrow() {
            self.store.remove_all();
            self.history.borrow_mut().clear();
            *self.path.borrow_mut() = HOME.into();
            self.render_crumbs();
            if self.ctx.ready_serial().is_some() {
                self.load();
            }
        }
    }
    fn shown(&self) {
        if self.loaded_for.borrow().is_none() || *self.loaded_for.borrow() != self.ctx.ready_serial() {
            self.load();
        } else {
            // Back to the page: check right away whether the folder changed.
            self.watch_last.set(0.0);
            self.watch_dir();
        }
    }
}
