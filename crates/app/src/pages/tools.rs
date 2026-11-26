//! Tools: processes, sensors, services and developer tools (see
//! `developer`). (The USB mode and the ADB server are in Connection.)

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

use adbscan_core::format;
use adbscan_core::models::{ProcessInfo, SensorInfo, ServiceInfo};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

struct ToolsPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::Box,
    procs: RefCell<Vec<ProcessInfo>>,
    user_pkgs: RefCell<std::collections::HashSet<String>>,
    proc_sort: gtk::DropDown,
    proc_live: gtk::CheckButton,
    proc_busy: std::cell::Cell<bool>,
    proc_last: std::cell::Cell<f64>,
    tabs: adw::ViewStack,
    show_kernel: gtk::CheckButton,
    proc_list: crate::widgets::RowList,
    proc_search: gtk::SearchEntry,
    proc_info: gtk::Label,
    sensors: gtk::ListBox,
    services: RefCell<Vec<ServiceInfo>>,
    service_list: crate::widgets::RowList,
    service_search: gtk::SearchEntry,
    developer: Rc<dyn Page>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    ToolsPage::new(ctx)
}

fn tab(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    util::page_scroller(child, 1000)
}

impl ToolsPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let stack = adw::ViewStack::new();
        let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();
        switcher.set_halign(gtk::Align::Center);
        switcher.set_margin_top(10);

        // Processes
        let pbox = util::vbox(10);
        let ptop = util::hbox(8);
        let proc_search = gtk::SearchEntry::builder().placeholder_text(tr("Filter by name, user or PID")).hexpand(true).build();
        let prefresh = util::pill_button("adb-refresh-cw-symbolic", tr("Refresh"));
        let proc_sort = gtk::DropDown::from_strings(&[tr("Memory"), "CPU", tr("Name")]);
        proc_sort.set_tooltip_text(Some(tr("Sort by")));
        let proc_live = gtk::CheckButton::with_label(tr("Live"));
        proc_live.set_active(true);
        proc_live.set_tooltip_text(Some(tr("Refresh the list every 3 s while it is in view")));
        let show_kernel = gtk::CheckButton::with_label(tr("Kernel threads"));
        show_kernel.set_tooltip_text(Some(tr("Also show the kernel's internal threads ([kworker], [ksoftirqd]…)")));
        ptop.append(&proc_search);
        ptop.append(&proc_live);
        ptop.append(&show_kernel);
        ptop.append(&proc_sort);
        ptop.append(&prefresh);
        let proc_info = util::label("", &["dim", "caption"]);
        let proc_list = crate::widgets::RowList::new(Some(("adb-media-playback-stop-symbolic", tr("Force stop the app"))));
        pbox.append(&ptop);
        pbox.append(&proc_info);
        pbox.append(&proc_list.root);
        stack.add_titled_with_icon(&tab(&pbox), Some("procs"), tr("Processes"), "adb-activity-symbolic");

        // Sensors
        let sbox = util::vbox(10);
        let srefresh = util::pill_button("adb-refresh-cw-symbolic", tr("Read sensors"));
        srefresh.set_halign(gtk::Align::End);
        let sensors = gtk::ListBox::new();
        sensors.add_css_class("boxed-list");
        sensors.set_selection_mode(gtk::SelectionMode::None);
        sbox.append(&srefresh);
        sbox.append(&sensors);
        stack.add_titled_with_icon(&tab(&sbox), Some("sensors"), tr("Sensors"), "adb-thermometer-symbolic");

        // Services
        let svbox = util::vbox(10);
        let service_search = gtk::SearchEntry::builder().placeholder_text(tr("Filter services")).build();
        let service_list = crate::widgets::RowList::new(None);
        svbox.append(&service_search);
        svbox.append(&service_list.root);
        stack.add_titled_with_icon(&tab(&svbox), Some("services"), tr("Services"), "adb-list-symbolic");

        // Developer
        let developer = super::developer::build(ctx);
        stack.add_titled_with_icon(&developer.widget(), Some("developer"), tr("Developer"), "adb-square-terminal-symbolic");

        let root = util::vbox(0);
        root.append(&switcher);
        root.append(&stack);
        stack.set_vexpand(true);

        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            procs: RefCell::new(Vec::new()),
            user_pkgs: RefCell::new(Default::default()),
            proc_sort: proc_sort.clone(),
            proc_live: proc_live.clone(),
            proc_busy: std::cell::Cell::new(false),
            proc_last: std::cell::Cell::new(0.0),
            tabs: stack.clone(),
            show_kernel: show_kernel.clone(),
            proc_list,
            proc_search,
            proc_info,
            sensors,
            services: RefCell::new(Vec::new()),
            service_list,
            service_search,
            developer,
        });
        macro_rules! on {
            ($w:expr, $sig:ident, |$t:ident| $body:expr) => {{
                let w = this.me.clone();
                $w.$sig(move |_| {
                    if let Some($t) = w.upgrade() {
                        $body;
                    }
                });
            }};
        }
        on!(prefresh, connect_clicked, |t| t.load_procs());
        {
            let w = this.me.clone();
            this.proc_list.connect_action(move |b, pkg| {
                if let Some(t) = w.upgrade() {
                    t.stop_app(b, pkg);
                }
            });
        }
        on!(this.proc_search, connect_search_changed, |t| t.render_procs());
        on!(proc_sort, connect_selected_notify, |t| t.render_procs());
        on!(show_kernel, connect_toggled, |t| t.render_procs());
        on!(srefresh, connect_clicked, |t| t.load_sensors());
        on!(this.service_search, connect_search_changed, |t| t.render_services());
        {
            let w = this.me.clone();
            stack.connect_visible_child_name_notify(move |s| {
                let Some(t) = w.upgrade() else { return };
                match s.visible_child_name().as_deref() {
                    Some("sensors") if t.sensors.first_child().is_none() => t.load_sensors(),
                    Some("services") if t.services.borrow().is_empty() => t.load_services(),
                    Some("developer") => t.developer.shown(),
                    _ => {}
                }
            });
        }
        this.load_procs();
        this
    }

    fn load_procs(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let w = self.me.clone();
        self.proc_busy.set(true);
        util::run_bg(
            move || adb.process_overview(&serial),
            move |r| {
                let Some(t) = w.upgrade() else { return };
                t.proc_busy.set(false);
                t.proc_last.set(adbscan_core::models::now_secs());
                match r {
                    Ok(o) => {
                        *t.procs.borrow_mut() = o.processes;
                        *t.user_pkgs.borrow_mut() = o.user_packages.into_iter().collect();
                        t.render_procs();
                    }
                    Err(e) => t.proc_info.set_text(&e.to_string()),
                }
            },
        );
    }

    fn render_procs(&self) {
        use crate::widgets::RowItem;
        let q = self.proc_search.text().to_lowercase();
        let kernel = self.show_kernel.is_active();
        let labels = crate::pages::apps::cached_labels();
        let user = self.user_pkgs.borrow();
        let procs = self.procs.borrow();
        let mut matches: Vec<&ProcessInfo> = procs
            .iter()
            .filter(|p| kernel || !p.is_kernel_thread())
            .filter(|p| {
                q.is_empty()
                    || p.name.to_lowercase().contains(&q)
                    || p.user.to_lowercase().contains(&q)
                    || p.pid.to_string() == q
                    || p.args.to_lowercase().contains(&q)
                    || labels.get(p.package()).is_some_and(|l| l.to_lowercase().contains(&q))
            })
            .collect();
        match self.proc_sort.selected() {
            1 => matches.sort_by(|a, b| b.cpu.unwrap_or(0.0).total_cmp(&a.cpu.unwrap_or(0.0)).then(b.rss_kib.cmp(&a.rss_kib))),
            2 => matches.sort_by_key(|p| labels.get(p.package()).cloned().unwrap_or_else(|| p.name.clone()).to_lowercase()),
            _ => matches.sort_by_key(|p| std::cmp::Reverse(p.rss_kib)),
        }
        let items: Vec<RowItem> = matches
            .iter()
            .map(|p| {
                let pkg = p.package().to_string();
                let label = labels.get(&pkg);
                let cpu = p.cpu.map(|c| format!(" · CPU {c:.1} %")).unwrap_or_default();
                // State according to Android (on screen, service, cached…): apps
                // only.
                let android = p.android_state.as_deref().map(|s| format!(" · {}", adbscan_core::app_inspect::proc_state_label(s))).unwrap_or_default();
                RowItem {
                    title: match label {
                        Some(l) if p.name != pkg => format!("{l} ({})", p.name.trim_start_matches(&pkg).trim_start_matches(':')),
                        Some(l) => l.clone(),
                        None => p.name.clone(),
                    },
                    subtitle: self.ctx.shown(&t!("{} · PID {} · {} · {}{}{} · state {}", p.name, p.pid, p.user, format::human_kib(Some(p.rss_kib)), cpu, android, p.stat)),
                    tooltip: if p.args.is_empty() || p.args == p.name { String::new() } else { self.ctx.shown(&p.args) },
                    // Stop only user apps: system ones restart by themselves or break things.
                    action: if user.contains(&pkg) { pkg } else { String::new() },
                    ..Default::default()
                }
            })
            .collect();
        let shown = matches.len();
        let hidden = if kernel { 0 } else { procs.iter().filter(|p| p.is_kernel_thread()).count() };
        let ram = self.ctx.snapshot().and_then(|s| s.memory.total_kib);
        self.proc_info.set_text(&t!("{} processes{}. CPU is shown as % of one core: a process using several at once can exceed 100 % (the maximum is 100 % per core, {} in total). Memory is shown as RSS: it includes shared libraries counted in every process, which is why the sum ({}) exceeds the real RAM{}.", shown, if hidden > 0 { t!(" ({} kernel threads hidden)", hidden) } else { String::new() }, self.ctx.snapshot().map(|s| s.cpu.cores.len()).filter(|n| *n > 0).map(|n| format!("{} %", n * 100)).unwrap_or_else(|| tr("more").into()), format::human_kib(Some(procs.iter().map(|p| p.rss_kib).sum())), ram.map(|r| format!(" ({})", format::human_kib(Some(r)))).unwrap_or_default()));
        self.proc_info.set_wrap(true);
        self.proc_list.set_items(items);
    }

    fn stop_app(&self, button: &gtk::Button, pkg: &str) {
        let (c, pkg, w) = (self.ctx.clone(), pkg.to_string(), self.me.clone());
        util::confirm(button, &t!("Force stop {}?", pkg), tr("The app will close and may lose unsaved data."), tr("Stop"), true, move || {
            let p3 = pkg.clone();
            let w2 = w.clone();
            c.device_op(
                "force_stop",
                &pkg,
                tr("App stopped"),
                move |adb, s| adb.force_stop(&s, &p3).map(|r| r.summary()),
                move |_| {
                    if let Some(t) = w2.upgrade() {
                        t.load_procs();
                    }
                },
            );
        });
    }

    fn load_sensors(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let w = self.me.clone();
        util::run_bg(
            move || adb.sensors(&serial).unwrap_or_default(),
            move |list: Vec<SensorInfo>| {
                let Some(t) = w.upgrade() else { return };
                while let Some(c) = t.sensors.first_child() {
                    t.sensors.remove(&c);
                }
                if list.is_empty() {
                    t.sensors.append(&adw::ActionRow::builder().title(tr("Could not read the sensors")).build());
                }
                for s in list {
                    let rate = if s.max_rate.is_empty() { s.min_rate.clone() } else { t!("up to {} Hz", s.max_rate) };
                    let row = adw::ActionRow::builder()
                        .title(glib::markup_escape_text(&s.name))
                        .subtitle(glib::markup_escape_text(&format!("{} · {} · {}{}", s.vendor, s.kind.trim_start_matches("android.sensor."), rate, if s.wakeup { " · wake-up" } else { "" })))
                        .build();
                    t.sensors.append(&row);
                }
            },
        );
    }

    fn load_services(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let w = self.me.clone();
        util::run_bg(
            move || adb.services(&serial).unwrap_or_default(),
            move |list| {
                if let Some(t) = w.upgrade() {
                    *t.services.borrow_mut() = list;
                    t.render_services();
                }
            },
        );
    }

    fn render_services(&self) {
        use crate::widgets::RowItem;
        let q = self.service_search.text().to_lowercase();
        let items: Vec<RowItem> = self
            .services
            .borrow()
            .iter()
            .filter(|s| q.is_empty() || s.name.to_lowercase().contains(&q) || s.descriptor.to_lowercase().contains(&q))
            .map(|s| RowItem::new(s.name.clone(), s.descriptor.clone()))
            .collect();
        self.service_list.set_items(items);
    }
}

impl Page for ToolsPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn shown(&self) {
        if self.tabs.visible_child_name().as_deref() == Some("developer") {
            self.developer.shown();
        }
    }
    fn on_event(&self, ev: &Event) {
        self.developer.on_event(ev);
        if matches!(ev, Event::Privacy) {
            self.render_procs();
        }
        if matches!(ev, Event::Devices) && self.ctx.ready_serial().is_some() && self.procs.borrow().is_empty() {
            self.load_procs();
        }
        // Telemetry only reaches the visible page: that way the processes
        // refresh every 3 s only while they are being looked at.
        if matches!(ev, Event::Snapshot)
            && self.proc_live.is_active()
            && !self.proc_busy.get()
            && self.tabs.visible_child_name().as_deref() == Some("procs")
            && adbscan_core::models::now_secs() - self.proc_last.get() >= 3.0
        {
            self.load_procs();
        }
    }
}
