//! Streaming logcat: filters by level, tag, text and app, pause, autoscroll,
//! export and session saving.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::io::Write;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adbscan_core::exec::{StreamEvent, StreamHandle};
use adbscan_core::format;
use adbscan_core::models::{now_secs, LogEntry, LogLevel};
use adbscan_core::parsers;
use adbscan_core::paths;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

enum LogMsg {
    Lines(Vec<LogEntry>),
    Exit,
}

struct FilterState {
    min_level: Cell<LogLevel>,
    tag: RefCell<String>,
    text: RefCell<String>,
    pids: RefCell<Option<HashSet<String>>>,
}

struct LogcatPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::Box,
    store: gio::ListStore,
    filter: gtk::CustomFilter,
    fstate: Rc<FilterState>,
    list: gtk::ListView,
    fmodel: gtk::FilterListModel,
    start_btn: gtk::Button,
    pause_btn: gtk::ToggleButton,
    autoscroll: gtk::ToggleButton,
    package: gtk::Entry,
    buffers: gtk::DropDown,
    status: gtk::Label,
    handle: RefCell<Option<StreamHandle>>,
    paused_queue: RefCell<Vec<LogEntry>>,
    session: RefCell<Option<(i64, std::path::PathBuf)>>,
    counter: Arc<Mutex<i64>>,
    total: Cell<u64>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    LogcatPage::new(ctx)
}

const LEVELS: [(LogLevel, &str); 6] =
    [(LogLevel::Verbose, "Verbose"), (LogLevel::Debug, "Debug"), (LogLevel::Info, "Info"), (LogLevel::Warn, "Warning"), (LogLevel::Error, "Error"), (LogLevel::Fatal, "Fatal")];
const BUFFERS: [(&str, &str); 5] = [("default", "Default"), ("all", "All buffers"), ("main,system,crash", "Main + system + crashes"), ("crash", "Crashes only"), ("events", "Events")];

fn entry_of(o: &glib::Object) -> Option<std::cell::Ref<'_, LogEntry>> {
    o.downcast_ref::<glib::BoxedAnyObject>().map(|b| b.borrow::<LogEntry>())
}

impl LogcatPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let fstate = Rc::new(FilterState { min_level: Cell::new(LogLevel::Verbose), tag: RefCell::new(String::new()), text: RefCell::new(String::new()), pids: RefCell::new(None) });
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let filter = {
            let f = fstate.clone();
            gtk::CustomFilter::new(move |o| {
                let Some(b) = o.downcast_ref::<glib::BoxedAnyObject>() else {
                    return false;
                };
                let e = b.borrow::<LogEntry>();
                if e.level != LogLevel::Raw && e.level < f.min_level.get() {
                    return false;
                }
                if let Some(pids) = f.pids.borrow().as_ref() {
                    if !pids.contains(&e.pid) {
                        return false;
                    }
                }
                let tag = f.tag.borrow();
                if !tag.is_empty() && !e.tag.to_lowercase().contains(tag.as_str()) {
                    return false;
                }
                let text = f.text.borrow();
                text.is_empty() || e.raw.to_lowercase().contains(text.as_str())
            })
        };
        let fmodel = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
        fmodel.set_incremental(false);
        let selection = gtk::NoSelection::new(Some(fmodel.clone()));
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let row = util::hbox(8);
            row.add_css_class("log-row");
            let badge = gtk::Label::new(None);
            badge.add_css_class("log-level-badge");
            let time = util::label("", &["dim", "mono"]);
            let tag = util::label("", &["mono"]);
            tag.set_width_chars(18);
            tag.set_max_width_chars(18);
            tag.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let msg = util::label("", &["mono"]);
            msg.set_hexpand(true);
            msg.set_wrap(true);
            msg.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            msg.set_selectable(true);
            for w in [&badge, &time, &tag, &msg] {
                row.append(w);
            }
            item.set_child(Some(&row));
        });
        // The buffer, the export and what is saved on disk are not touched:
        // only what is painted in each visible row is hidden.
        let ctx_bind = ctx.clone();
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(obj) = item.item() else { return };
            let Some(e) = entry_of(&obj) else { return };
            let row = item.child().and_downcast::<gtk::Box>().unwrap();
            let badge = row.first_child().unwrap().downcast::<gtk::Label>().unwrap();
            let time = badge.next_sibling().unwrap().downcast::<gtk::Label>().unwrap();
            let tag = time.next_sibling().unwrap().downcast::<gtk::Label>().unwrap();
            let msg = tag.next_sibling().unwrap().downcast::<gtk::Label>().unwrap();
            for l in ["V", "D", "I", "W", "E", "F"] {
                row.remove_css_class(&format!("log-{l}"));
            }
            if e.level != LogLevel::Raw {
                row.add_css_class(&format!("log-{}", e.level.letter()));
            }
            badge.set_text(e.level.letter());
            time.set_text(e.timestamp.get(6..).unwrap_or(&e.timestamp));
            tag.set_text(&ctx_bind.shown(&e.tag));
            tag.set_tooltip_text(Some(&ctx_bind.shown(&format!("{} · PID {} · TID {}", e.tag, e.pid, e.tid))));
            msg.set_text(&ctx_bind.shown(&e.message));
        });
        let list = gtk::ListView::builder().model(&selection).factory(&factory).build();
        let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).build();

        // Control bar
        let bar = util::hbox(8);
        bar.set_margin_top(10);
        bar.set_margin_start(12);
        bar.set_margin_end(12);
        let start_btn = gtk::Button::with_label(tr("Start"));
        start_btn.add_css_class("suggested-action");
        let pause_btn = gtk::ToggleButton::builder().icon_name("adb-media-playback-stop-symbolic").tooltip_text(tr("Pause view")).build();
        let autoscroll = gtk::ToggleButton::builder().icon_name("adb-arrow-up-symbolic").tooltip_text(tr("Autoscroll")).active(ctx.settings.borrow().logcat_autoscroll).build();
        let level = gtk::DropDown::from_strings(&LEVELS.iter().map(|l| l.1).collect::<Vec<_>>());
        level.set_tooltip_text(Some(tr("Minimum level")));
        let tag_entry = gtk::SearchEntry::builder().placeholder_text(tr("Tag")).width_chars(12).build();
        let text_entry = gtk::SearchEntry::builder().placeholder_text(tr("Search text")).hexpand(true).build();
        let package = gtk::Entry::builder().placeholder_text(tr("Package (com.app…)")).width_chars(18).build();
        package.set_tooltip_text(Some(tr("Shows only the processes of one app. Enter to apply.")));
        let buffers = gtk::DropDown::from_strings(&BUFFERS.iter().map(|b| tr(b.1)).collect::<Vec<_>>());
        let current = ctx.settings.borrow().logcat_buffers.clone();
        buffers.set_selected(BUFFERS.iter().position(|b| b.0 == current).unwrap_or(0) as u32);
        buffers.set_tooltip_text(Some(tr("Logcat buffers")));
        let clear = util::icon_button("adb-edit-clear-symbolic", tr("Clear (on the phone too)"));
        let export = util::icon_button("adb-save-symbolic", tr("Export what is visible"));
        for w in [
            start_btn.upcast_ref::<gtk::Widget>(),
            pause_btn.upcast_ref(),
            autoscroll.upcast_ref(),
            level.upcast_ref(),
            tag_entry.upcast_ref(),
            text_entry.upcast_ref(),
            package.upcast_ref(),
            buffers.upcast_ref(),
            clear.upcast_ref(),
            export.upcast_ref(),
        ] {
            bar.append(w);
        }
        let status = util::label(tr("Stopped"), &["dim", "caption"]);
        status.set_margin_start(14);
        status.set_margin_top(4);
        status.set_margin_bottom(4);
        let root = util::vbox(0);
        root.append(&bar);
        root.append(&status);
        root.append(&scroll);

        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            store,
            filter,
            fstate,
            list,
            fmodel,
            start_btn,
            pause_btn,
            autoscroll,
            package,
            buffers,
            status,
            handle: RefCell::new(None),
            paused_queue: RefCell::new(Vec::new()),
            session: RefCell::new(None),
            counter: Arc::new(Mutex::new(0)),
            total: Cell::new(0),
        });
        {
            let w = this.me.clone();
            this.start_btn.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    if t.handle.borrow().is_some() {
                        t.stop();
                    } else {
                        t.start();
                    }
                }
            });
        }
        {
            let w = this.me.clone();
            level.connect_selected_notify(move |d| {
                if let Some(t) = w.upgrade() {
                    t.fstate.min_level.set(LEVELS[d.selected().min(5) as usize].0);
                    t.refilter();
                }
            });
        }
        {
            let w = this.me.clone();
            tag_entry.connect_search_changed(move |e| {
                if let Some(t) = w.upgrade() {
                    *t.fstate.tag.borrow_mut() = e.text().to_lowercase();
                    t.refilter();
                }
            });
        }
        {
            let w = this.me.clone();
            text_entry.connect_search_changed(move |e| {
                if let Some(t) = w.upgrade() {
                    *t.fstate.text.borrow_mut() = e.text().to_lowercase();
                    t.refilter();
                }
            });
        }
        {
            let w = this.me.clone();
            this.package.connect_activate(move |_| {
                if let Some(t) = w.upgrade() {
                    t.resolve_pids();
                }
            });
        }
        {
            let w = this.me.clone();
            this.pause_btn.connect_toggled(move |b| {
                let Some(t) = w.upgrade() else { return };
                if !b.is_active() {
                    let queued = std::mem::take(&mut *t.paused_queue.borrow_mut());
                    t.ingest(queued);
                }
            });
        }
        {
            let c = ctx.clone();
            this.autoscroll.connect_toggled(move |b| c.settings.borrow_mut().logcat_autoscroll = b.is_active());
        }
        {
            let w = this.me.clone();
            clear.connect_clicked(move |_| {
                let Some(t) = w.upgrade() else { return };
                t.store.remove_all();
                t.total.set(0);
                if let (Some(adb), Some(s)) = (t.ctx.adb(), t.ctx.ready_serial()) {
                    util::run_bg(move || adb.logcat_clear(&s), |_| {});
                }
                t.update_status();
            });
        }
        {
            let w = this.me.clone();
            export.connect_clicked(move |b| {
                let Some(t) = w.upgrade() else { return };
                let lines: Vec<String> = (0..t.fmodel.n_items()).filter_map(|i| t.fmodel.item(i)).filter_map(|o| entry_of(&o).map(|e| e.raw.clone())).collect();
                let ctx = t.ctx.clone();
                let name = format!("logcat-{}.txt", format::file_stamp(now_secs()));
                util::save_file(b, tr("Export logcat"), &name, move |p| match std::fs::write(&p, lines.join("\n")) {
                    Ok(_) => ctx.toast(&t!("{} lines exported", lines.len())),
                    Err(e) => ctx.toast(&format!("Error: {e}")),
                });
            });
        }
        // Keeps the package filter up to date (PIDs change).
        {
            let w = this.me.clone();
            glib::timeout_add_local(Duration::from_secs(5), move || {
                let Some(t) = w.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if t.fstate.pids.borrow().is_some() && t.handle.borrow().is_some() {
                    t.resolve_pids();
                }
                glib::ControlFlow::Continue
            });
        }
        this
    }

    fn refilter(&self) {
        self.filter.changed(gtk::FilterChange::Different);
        self.update_status();
    }

    fn update_status(&self) {
        let running = self.handle.borrow().is_some();
        let shown = self.fmodel.n_items();
        let mut text = t!("{} · {} lines in memory · {} visible", if running { tr("Receiving") } else { tr("Stopped") }, self.store.n_items(), shown);
        if let Some((_, path)) = self.session.borrow().as_ref() {
            text.push_str(&t!(" · saving to {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        }
        if self.pause_btn.is_active() {
            text.push_str(&t!(" · paused ({} pending)", self.paused_queue.borrow().len()));
        }
        self.status.set_text(&text);
    }

    fn resolve_pids(&self) {
        let pkg = self.package.text().trim().to_string();
        if pkg.is_empty() {
            *self.fstate.pids.borrow_mut() = None;
            self.refilter();
            return;
        }
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let w = self.me.clone();
        util::run_bg(
            move || adb.package_pids(&serial, &pkg),
            move |pids| {
                let Some(t) = w.upgrade() else { return };
                let set: HashSet<String> = pids.iter().map(|p| p.to_string()).collect();
                if set.is_empty() {
                    t.status.set_text(tr("The app is not running: open it and press Enter again"));
                }
                *t.fstate.pids.borrow_mut() = Some(set);
                t.refilter();
            },
        );
    }

    fn start(&self) {
        let (Some(adb), Some(dev)) = (self.ctx.adb(), self.ctx.selected_device().filter(|d| d.connected())) else {
            self.ctx.toast(tr("No device is ready"));
            return;
        };
        let buffers = BUFFERS[self.buffers.selected().min(4) as usize].0.to_string();
        self.ctx.settings.borrow_mut().logcat_buffers = buffers.clone();
        // Session file
        let writer: Option<Arc<Mutex<std::io::BufWriter<std::fs::File>>>> = if self.ctx.settings.borrow().logcat_save_sessions {
            let dir = paths::logcat_dir();
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(format!("{}-{}.log", dev.serial, format::file_stamp(now_secs())));
            match std::fs::File::create(&path) {
                Ok(f) => {
                    if let Some(h) = &self.ctx.history {
                        if let Ok(id) = h.start_logcat(&dev.serial, &self.ctx.device_title(&dev), &path.display().to_string(), &buffers) {
                            *self.session.borrow_mut() = Some((id, path));
                        }
                    }
                    Some(Arc::new(Mutex::new(std::io::BufWriter::new(f))))
                }
                Err(_) => None,
            }
        } else {
            None
        };
        *self.counter.lock().unwrap() = 0;
        let (tx, rx) = async_channel::unbounded::<LogMsg>();
        let pending: Arc<Mutex<Vec<LogEntry>>> = Arc::new(Mutex::new(Vec::new()));
        let p2 = pending.clone();
        let counter = self.counter.clone();
        let tx2 = tx.clone();
        let handle = adb.logcat_stream(&dev.serial, &buffers, move |ev| match ev {
            StreamEvent::Stdout(line) => {
                if let Some(w) = &writer {
                    let _ = writeln!(w.lock().unwrap(), "{line}");
                }
                *counter.lock().unwrap() += 1;
                p2.lock().unwrap().push(parsers::parse_logcat_line(&line));
            }
            StreamEvent::Stderr(_) => {}
            StreamEvent::Exit { .. } => {
                if let Some(w) = &writer {
                    let _ = w.lock().unwrap().flush();
                }
                let _ = tx2.send_blocking(LogMsg::Exit);
            }
        });
        match handle {
            Ok(h) => {
                *self.handle.borrow_mut() = Some(h);
                self.start_btn.set_label(tr("Stop"));
                self.start_btn.remove_css_class("suggested-action");
                self.start_btn.add_css_class("destructive-action");
                self.ctx.audit("logcat", Some(&buffers), true, None);
                // Flushes the input buffer every 120 ms.
                let tx3 = tx.clone();
                let p3 = pending.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(120));
                    let batch = std::mem::take(&mut *p3.lock().unwrap());
                    if !batch.is_empty() && tx3.send_blocking(LogMsg::Lines(batch)).is_err() {
                        break;
                    }
                    if tx3.is_closed() {
                        break;
                    }
                });
                let w = self.me.clone();
                glib::spawn_future_local(async move {
                    while let Ok(msg) = rx.recv().await {
                        let Some(t) = w.upgrade() else { break };
                        match msg {
                            LogMsg::Lines(lines) => t.ingest(lines),
                            LogMsg::Exit => {
                                t.finish();
                                break;
                            }
                        }
                    }
                    rx.close();
                });
                self.update_status();
            }
            Err(e) => self.ctx.toast(&t!("Could not start logcat: {}", e)),
        }
    }

    fn ingest(&self, lines: Vec<LogEntry>) {
        if lines.is_empty() {
            return;
        }
        if self.pause_btn.is_active() {
            let mut q = self.paused_queue.borrow_mut();
            q.extend(lines);
            let cap = self.ctx.settings.borrow().logcat_buffer_lines as usize;
            if q.len() > cap {
                let cut = q.len() - cap;
                q.drain(..cut);
            }
            drop(q);
            self.update_status();
            return;
        }
        self.total.set(self.total.get() + lines.len() as u64);
        let objs: Vec<glib::BoxedAnyObject> = lines.into_iter().map(glib::BoxedAnyObject::new).collect();
        self.store.splice(self.store.n_items(), 0, &objs);
        let cap = self.ctx.settings.borrow().logcat_buffer_lines;
        let n = self.store.n_items();
        if n > cap {
            self.store.splice(0, n - cap, &[] as &[glib::Object]);
        }
        if self.autoscroll.is_active() {
            let shown = self.fmodel.n_items();
            if shown > 0 {
                self.list.scroll_to(shown - 1, gtk::ListScrollFlags::NONE, None);
            }
        }
        self.update_status();
    }

    fn stop(&self) {
        if let Some(h) = self.handle.borrow().as_ref() {
            h.cancel();
        }
    }

    fn finish(&self) {
        self.handle.borrow_mut().take();
        self.start_btn.set_label(tr("Start"));
        self.start_btn.remove_css_class("destructive-action");
        self.start_btn.add_css_class("suggested-action");
        if let (Some((id, _)), Some(h)) = (self.session.borrow_mut().take(), &self.ctx.history) {
            let _ = h.update_logcat(id, *self.counter.lock().unwrap(), true);
        }
        self.update_status();
    }
}

impl Page for LogcatPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn on_event(&self, ev: &Event) {
        if matches!(ev, Event::Privacy) {
            // Rebinds the visible rows with the new mode.
            let n = self.fmodel.n_items();
            self.fmodel.items_changed(0, n, n);
        }
        if matches!(ev, Event::Devices) && self.ctx.ready_serial().is_none() && self.handle.borrow().is_some() {
            self.stop();
        }
    }
}
