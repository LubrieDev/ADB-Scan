//! Performance: CPU (total and per core), memory and thermal sensors.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use adbscan_core::format;
use adbscan_core::models::Snapshot;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::widgets::{AnimatedBar, Chart, StatCard};
use crate::window::Page;

struct CoreRow {
    root: gtk::Box,
    bar: AnimatedBar,
    freq: gtk::Label,
    gov: gtk::Label,
}

struct Performance {
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    load: StatCard,
    freq: StatCard,
    temp: StatCard,
    loadavg: StatCard,
    cpu_chart: Chart,
    mem_chart: Chart,
    temp_chart: Chart,
    mem_label: gtk::Label,
    swap_label: gtk::Label,
    cores_box: gtk::FlowBox,
    cores: RefCell<Vec<CoreRow>>,
    me: RefCell<std::rc::Weak<Self>>,
    gpu: StatCard,
    fps: StatCard,
    fps_chart: Chart,
    session_row: adw::ActionRow,
    /// GPU/FPS reading in progress and time of the last one.
    gpu_busy: std::cell::Cell<bool>,
    gpu_last: std::cell::Cell<f64>,
    prev_frame: std::cell::Cell<Option<i64>>,
    thermal_list: gtk::ListBox,
    thermal_key: RefCell<String>,
    /// (sensor name, value label) and, per group, (type, maximum label, row).
    thermal_values: RefCell<Vec<(String, gtk::Label)>>,
    thermal_groups: RefCell<Vec<(String, gtk::Label, adw::ExpanderRow)>>,
    thermal_top: adw::ActionRow,
    thermal_top_value: gtk::Label,
    memory: Rc<crate::pages::memory_view::MemoryView>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    Performance::new(ctx)
}

fn chart_card(title: &str, chart: &Chart, extra: Option<&gtk::Widget>) -> gtk::Box {
    let b = util::vbox(10);
    b.add_css_class("card");
    b.add_css_class("stat-card");
    let head = util::hbox(8);
    head.append(&util::label(title, &["section-title"]));
    if let Some(e) = extra {
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        head.append(&spacer);
        head.append(e);
    }
    b.append(&head);
    b.append(&chart.area);
    b
}

impl Performance {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(16);
        let top = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).homogeneous(true).max_children_per_line(3).min_children_per_line(2).column_spacing(12).row_spacing(12).build();
        let load = StatCard::new("adb-cpu-symbolic", tr("CPU usage"));
        let freq = StatCard::new("adb-activity-symbolic", tr("Frequency"));
        let temp = StatCard::new("adb-thermometer-symbolic", tr("CPU temperature"));
        let loadavg = StatCard::new("adb-chart-line-symbolic", tr("Load average"));
        let gpu = StatCard::new("adb-video-display-symbolic", "GPU");
        let fps = StatCard::new("adb-activity-symbolic", tr("Smoothness"));
        for c in [&load, &freq, &temp, &loadavg, &gpu, &fps] {
            top.append(&c.root);
        }
        content.append(&top);

        let cpu_chart = Chart::new(150, 120, 0.0, 100.0, " %");
        cpu_chart.add_series("CPU", None, true);
        content.append(&chart_card(tr("CPU usage (last 60 s)"), &cpu_chart, None));

        content.append(&util::label(tr("Cores"), &["section-title"]));
        let cores_box = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).homogeneous(true).max_children_per_line(4).min_children_per_line(1).column_spacing(10).row_spacing(10).build();
        content.append(&cores_box);

        let mem_chart = Chart::new(110, 120, 0.0, 100.0, " %");
        mem_chart.add_series("RAM", None, true);
        let mem_label = util::label("—", &["dim", "numeric"]);
        let mem_card = chart_card(tr("Memory"), &mem_chart, Some(mem_label.upcast_ref()));
        let swap_label = util::label("—", &["dim", "numeric"]);
        mem_card.append(&swap_label);
        content.append(&mem_card);
        let memory = crate::pages::memory_view::MemoryView::new(ctx);
        content.append(&memory.root);

        let temp_chart = Chart::new(110, 120, 20.0, 60.0, " °C");
        temp_chart.set_auto_max(true);
        temp_chart.add_series("CPU", None, false);
        temp_chart.add_series(tr("Battery"), Some("#2ec27e"), false);
        content.append(&chart_card(tr("Temperatures"), &temp_chart, None));

        let fps_chart = Chart::new(110, 120, 0.0, 60.0, " FPS");
        fps_chart.set_auto_max(true);
        fps_chart.add_series("FPS", Some("#3584e4"), true);
        content.append(&chart_card(tr("Smoothness of the foreground app (FPS)"), &fps_chart, None));

        // Session log: everything measured while the phone is connected.
        let session_row = adw::ActionRow::builder().title(tr("Session log")).subtitle("—").build();
        session_row.add_prefix(&util::icon("adb-document-save-symbolic"));
        let export = util::pill_button("adb-save-symbolic", tr("Export CSV"));
        export.set_valign(gtk::Align::Center);
        session_row.add_suffix(&export);
        let session_list = gtk::ListBox::new();
        session_list.add_css_class("boxed-list");
        session_list.set_selection_mode(gtk::SelectionMode::None);
        session_list.append(&session_row);
        content.append(&session_list);

        content.append(&util::label(tr("Thermal sensors"), &["section-title"]));
        let thermal_top = adw::ActionRow::builder().title(tr("Hottest sensor")).subtitle("—").build();
        thermal_top.add_prefix(&util::icon("adb-thermometer-symbolic"));
        let thermal_top_value = util::label("—", &["numeric", "heading"]);
        thermal_top.add_suffix(&thermal_top_value);
        let top_list = gtk::ListBox::new();
        top_list.add_css_class("boxed-list");
        top_list.set_selection_mode(gtk::SelectionMode::None);
        top_list.append(&thermal_top);
        top_list.set_margin_bottom(8);
        content.append(&top_list);
        let thermal_list = gtk::ListBox::new();
        thermal_list.add_css_class("boxed-list");
        thermal_list.set_selection_mode(gtk::SelectionMode::None);
        content.append(&thermal_list);

        let root = util::page_scroller(&content, 1200);
        let this = Rc::new(Self {
            me: RefCell::new(std::rc::Weak::new()),
            gpu,
            fps,
            fps_chart,
            session_row,
            gpu_busy: std::cell::Cell::new(false),
            gpu_last: std::cell::Cell::new(0.0),
            prev_frame: std::cell::Cell::new(None),
            ctx: ctx.clone(),
            root,
            load,
            freq,
            temp,
            loadavg,
            cpu_chart,
            mem_chart,
            temp_chart,
            mem_label,
            swap_label,
            cores_box,
            cores: RefCell::new(Vec::new()),
            thermal_list,
            thermal_key: RefCell::new(String::new()),
            thermal_values: RefCell::new(Vec::new()),
            thermal_groups: RefCell::new(Vec::new()),
            thermal_top,
            thermal_top_value,
            memory,
        });
        for m in ctx.metrics() {
            this.cpu_chart.push(0, m.cpu_load);
            this.mem_chart.push(0, m.ram_pct);
            this.temp_chart.push(0, m.cpu_temp);
            this.temp_chart.push(1, m.batt_temp);
        }
        if let Some(s) = ctx.snapshot() {
            this.update(&s, false);
        }
        *this.me.borrow_mut() = Rc::downgrade(&this);
        {
            let w = Rc::downgrade(&this);
            export.connect_clicked(move |b| {
                if let Some(t) = w.upgrade() {
                    t.export_session(b);
                }
            });
        }
        this
    }

    /// GPU and FPS every ~1.5 s, only while the page is visible (only then does
    /// it receive the telemetry that calls this).
    fn poll_gpu(&self) {
        let now = adbscan_core::models::now_secs();
        if self.gpu_busy.get() || now - self.gpu_last.get() < 1.5 {
            return;
        }
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        self.gpu_busy.set(true);
        self.gpu_last.set(now);
        let prev = self.prev_frame.get();
        let w = self.me.borrow().clone();
        crate::util::run_bg(
            move || adb.gpu_fps(&serial, prev),
            move |m: adbscan_core::models::GpuFps| {
                let Some(t) = w.upgrade() else { return };
                t.gpu_busy.set(false);
                t.prev_frame.set(m.newest_frame.or(t.prev_frame.get()));
                if let Some(g) = m.gpu_busy {
                    t.gpu.set(&format!("{g:.0} %"), tr("GPU usage (Adreno)"));
                } else if t.gpu_last.get() > 0.0 && t.gpu.value_text() == "—" {
                    t.gpu.set("—", tr("This phone does not expose GPU usage"));
                }
                let app = crate::pages::apps::cached_labels().get(&m.app).cloned().unwrap_or(m.app.clone());
                match m.fps {
                    Some(f) => {
                        t.fps.set(&format!("{f:.0} FPS"), if f == 0.0 { tr("Screen idle") } else { &app });
                        t.fps_chart.push(0, Some(f));
                    }
                    None => t.fps.set("—", tr("No frame data")),
                }
                t.ctx.note_gpu_fps(m.gpu_busy, m.fps);
            },
        );
    }

    fn refresh_session(&self) {
        let (n, since) = self.ctx.session_info();
        let text = match since {
            Some(t) if n > 0 => t!("{} samples since {}", n, format::clock(t)),
            _ => tr("No data yet").to_string(),
        };
        self.session_row.set_subtitle(&text);
    }

    fn export_session(&self, button: &gtk::Button) {
        let csv = self.ctx.session_csv();
        if csv.lines().count() <= 1 {
            self.ctx.toast(tr("There is no session data yet"));
            return;
        }
        let ctx = self.ctx.clone();
        let name = format!("sesion-{}.csv", format::file_stamp(adbscan_core::models::now_secs()));
        crate::util::save_file(button, tr("Export the session log"), &name, move |path| match std::fs::write(&path, &csv) {
            Ok(()) => ctx.toast(&t!("Session saved to {}", path.display())),
            Err(e) => ctx.toast(&t!("Could not save: {}", e)),
        });
    }

    fn ensure_cores(&self, n: usize) {
        let mut cores = self.cores.borrow_mut();
        if cores.len() == n {
            return;
        }
        while let Some(c) = self.cores_box.first_child() {
            self.cores_box.remove(&c);
        }
        cores.clear();
        for i in 0..n {
            let root = util::vbox(4);
            root.add_css_class("core-bar");
            let head = util::hbox(6);
            head.append(&util::label(&t!("Core {}", i), &["caption-heading"]));
            let freq = util::label("—", &["numeric", "caption"]);
            freq.set_hexpand(true);
            freq.set_xalign(1.0);
            head.append(&freq);
            let bar = AnimatedBar::new();
            let gov = util::label("", &["dim", "caption"]);
            root.append(&head);
            root.append(&bar.bar);
            root.append(&gov);
            self.cores_box.append(&root);
            cores.push(CoreRow { root, bar, freq, gov });
        }
    }

    fn update(&self, s: &Snapshot, push: bool) {
        let interval = self.ctx.live_interval();
        for c in [&self.cpu_chart, &self.mem_chart, &self.temp_chart] {
            c.set_interval(interval);
        }
        let cpu = &s.cpu;
        self.load.set(&format::percent(cpu.load_percent), &t!("{} cores", cpu.core_count.unwrap_or(0)));
        self.freq.set(&format::frequency_khz(cpu.cur_freq_khz()), &t!("max. {}", format::frequency_khz(cpu.max_freq_khz())));
        self.temp.set(&format::temperature(s.cpu_temperature()), &t!("Thermal state: {}", s.thermal_status_label()));
        self.loadavg.set(&cpu.loadavg.map(|l| format!("{:.2}", l.0)).unwrap_or("—".into()), &cpu.loadavg.map(|l| format!("5 min {:.2} · 15 min {:.2}", l.1, l.2)).unwrap_or_default());
        if push {
            self.cpu_chart.push(0, cpu.load_percent);
            self.mem_chart.push(0, s.memory.percent());
            self.temp_chart.push(0, s.cpu_temperature());
            self.temp_chart.push(1, s.battery_temperature());
        }
        self.ensure_cores(cpu.cores.len());
        for (row, core) in self.cores.borrow().iter().zip(cpu.cores.iter()) {
            row.bar.set(core.load_hint(), 85.0, 97.0);
            row.freq.set_text(&format!("{} / {}", format::frequency_khz(core.cur_khz), format::frequency_khz(core.max_khz)));
            row.gov.set_text(&core.governor);
            row.root.set_tooltip_text(Some(&t!("Min. {}", format::frequency_khz(core.min_khz))));
        }
        let m = &s.memory;
        self.mem_label.set_text(&t!("{} of {} · {} available", format::human_kib(m.used_kib()), format::human_kib(m.total_kib), format::human_kib(m.available_kib)));
        self.swap_label.set_text(&match (m.swap_total_kib, m.swap_used_kib()) {
            (Some(t), Some(u)) if t > 0 => {
                t!("Swap/ZRAM: {} of {} · Cache {} · Buffers {}", format::human_kib(Some(u)), format::human_kib(Some(t)), format::human_kib(m.cached_kib), format::human_kib(m.buffers_kib))
            }
            _ => t!("No swap · Cache {}", format::human_kib(m.cached_kib)),
        });
        self.update_thermal(s);
    }
}

fn heat_class(label: &gtk::Label, t: &adbscan_core::models::Temperature) {
    let (warn, hot) = t.thresholds();
    label.remove_css_class("severity-high");
    label.remove_css_class("severity-medium");
    label.remove_css_class("severity-ok");
    label.add_css_class(if t.value_c >= hot {
        "severity-high"
    } else if t.value_c >= warn {
        "severity-medium"
    } else {
        "severity-ok"
    });
}

impl Performance {
    /// Sensors grouped by type (CPU, GPU, NPU, battery…), with each group's
    /// maximum and colours according to that type's thresholds.
    fn update_thermal(&self, s: &Snapshot) {
        let temps: Vec<_> = s.temperatures.iter().filter(|t| t.is_temperature()).cloned().collect();
        // Groups sorted by their maximum; within each, from hottest to coolest.
        let mut kinds: Vec<&'static str> = Vec::new();
        for t in &temps {
            if !kinds.contains(&t.kind()) {
                kinds.push(t.kind());
            }
        }
        let max_of = |k: &str| temps.iter().filter(|t| t.kind() == k).map(|t| t.value_c).fold(f64::MIN, f64::max);
        kinds.sort_by(|a, b| max_of(b).total_cmp(&max_of(a)));
        let mut names: Vec<String> = temps.iter().map(|t| format!("{}:{}", t.kind(), t.name)).collect();
        names.sort();
        let key = names.join("|");
        if *self.thermal_key.borrow() != key {
            *self.thermal_key.borrow_mut() = key;
            while let Some(c) = self.thermal_list.first_child() {
                self.thermal_list.remove(&c);
            }
            let mut values = self.thermal_values.borrow_mut();
            let mut groups = self.thermal_groups.borrow_mut();
            values.clear();
            groups.clear();
            if temps.is_empty() {
                self.thermal_list.append(&adw::ActionRow::builder().title(tr("The device does not expose thermal sensors")).build());
            }
            for k in &kinds {
                let mut members: Vec<_> = temps.iter().filter(|t| t.kind() == *k).collect();
                members.sort_by(|a, b| a.name.cmp(&b.name));
                let exp = adw::ExpanderRow::builder().title(*k).subtitle(t!("{} sensors", members.len())).build();
                let max_label = util::label("", &["numeric", "heading"]);
                exp.add_suffix(&max_label);
                for t in members {
                    let row = adw::ActionRow::builder().title(gtk::glib::markup_escape_text(&t.name)).build();
                    let v = util::label("", &["numeric"]);
                    row.add_suffix(&v);
                    exp.add_row(&row);
                    values.push((t.name.clone(), v));
                }
                self.thermal_list.append(&exp);
                groups.push((k.to_string(), max_label, exp));
            }
        }
        for (name, label) in self.thermal_values.borrow().iter() {
            if let Some(t) = temps.iter().find(|t| &t.name == name) {
                label.set_text(&format::temperature(Some(t.value_c)));
                heat_class(label, t);
            }
        }
        for (kind, label, exp) in self.thermal_groups.borrow().iter() {
            let members: Vec<_> = temps.iter().filter(|t| t.kind() == kind).collect();
            if let Some(hot) = members.iter().max_by(|a, b| a.value_c.total_cmp(&b.value_c)) {
                label.set_text(&t!("max. {}", format::temperature(Some(hot.value_c))));
                heat_class(label, hot);
                let avg = members.iter().map(|t| t.value_c).sum::<f64>() / members.len() as f64;
                exp.set_subtitle(&t!("{} sensors · average {}", members.len(), format::temperature(Some(avg))));
            }
        }
        match temps.iter().max_by(|a, b| a.value_c.total_cmp(&b.value_c)) {
            Some(hot) => {
                let (warn, high) = hot.thresholds();
                self.thermal_top.set_subtitle(&t!("{} · {} (watch from {} °C, high from {} °C)", hot.name, hot.kind(), format!("{:.0}", warn), format!("{:.0}", high)));
                self.thermal_top_value.set_text(&format::temperature(Some(hot.value_c)));
                heat_class(&self.thermal_top_value, hot);
            }
            None => {
                self.thermal_top.set_subtitle("—");
                self.thermal_top_value.set_text("—");
            }
        }
    }
}

impl Page for Performance {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn on_event(&self, ev: &Event) {
        self.memory.on_event(ev);
        match ev {
            Event::Snapshot => {
                if let Some(s) = self.ctx.snapshot() {
                    self.update(&s, true);
                }
                self.poll_gpu();
                self.refresh_session();
            }
            Event::Devices if self.ctx.snapshot().is_none() => {
                for c in [&self.cpu_chart, &self.mem_chart, &self.temp_chart] {
                    c.clear();
                }
            }
            _ => {}
        }
    }
    fn shown(&self) {
        let m = self.ctx.metrics();
        self.cpu_chart.refill(0, m.iter().map(|m| m.cpu_load));
        self.mem_chart.refill(0, m.iter().map(|m| m.ram_pct));
        self.temp_chart.refill(0, m.iter().map(|m| m.cpu_temp));
        self.temp_chart.refill(1, m.iter().map(|m| m.batt_temp));
        if let Some(s) = self.ctx.snapshot() {
            self.update(&s, false);
        }
    }
}
