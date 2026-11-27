//! Drawn widgets: animated indicator ring, live chart with smooth scrolling,
//! stat card and usage bar.

use adbscan_core::i18n::tr;
use adw::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::f64::consts::PI;
use std::rc::Rc;
use std::time::Instant;

#[allow(deprecated)]
fn named_color(widget: &impl IsA<gtk::Widget>, name: &str) -> gdk::RGBA {
    widget.as_ref().style_context().lookup_color(name).unwrap_or_else(|| gdk::RGBA::parse(crate::theme::accent_hex("orange")).unwrap())
}

fn fg(widget: &impl IsA<gtk::Widget>) -> gdk::RGBA {
    widget.as_ref().color()
}

fn set_rgba(cr: &gtk::cairo::Context, c: &gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64 * alpha);
}

/// Colour by threshold: accent → warning → error.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Normal,
    High,
    Critical,
}

fn level_color(widget: &impl IsA<gtk::Widget>, level: Level) -> gdk::RGBA {
    match level {
        Level::Normal => named_color(widget, "accent_bg_color"),
        Level::High => named_color(widget, "warning_color"),
        Level::Critical => named_color(widget, "error_color"),
    }
}

// ---------------------------------------------------------------------------
// Ring
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Gauge {
    pub root: gtk::Overlay,
    area: gtk::DrawingArea,
    value_label: gtk::Label,
    caption: gtk::Label,
    shown: Rc<Cell<f64>>,
    target: Rc<Cell<Option<f64>>>,
    level: Rc<Cell<Level>>,
    anim: adw::TimedAnimation,
}

impl Gauge {
    pub fn new(size: i32, thickness: f64) -> Self {
        let area = gtk::DrawingArea::builder().content_width(size).content_height(size).build();
        let value_label = gtk::Label::new(Some("—"));
        value_label.add_css_class("gauge-value");
        let caption = gtk::Label::new(None);
        caption.add_css_class("gauge-caption");
        let center = gtk::Box::new(gtk::Orientation::Vertical, 0);
        center.set_valign(gtk::Align::Center);
        center.set_halign(gtk::Align::Center);
        center.append(&value_label);
        center.append(&caption);
        let root = gtk::Overlay::builder().child(&area).build();
        root.add_overlay(&center);
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Center);

        let shown: Rc<Cell<f64>> = Rc::new(Cell::new(0.0));
        let target: Rc<Cell<Option<f64>>> = Rc::new(Cell::new(None));
        let level = Rc::new(Cell::new(Level::Normal));
        {
            let (shown, target, level) = (shown.clone(), target.clone(), level.clone());
            area.set_draw_func(move |a, cr, w, h| {
                let (w, h) = (w as f64, h as f64);
                let r = w.min(h) / 2.0 - thickness / 2.0 - 1.0;
                let (cx, cy) = (w / 2.0, h / 2.0);
                let start = 0.75 * PI;
                let sweep = 1.5 * PI;
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                cr.set_line_width(thickness);
                set_rgba(cr, &fg(a), 0.10);
                cr.arc(cx, cy, r, start, start + sweep);
                let _ = cr.stroke();
                if target.get().is_some() {
                    let frac = (shown.get() / 100.0).clamp(0.0, 1.0);
                    if frac > 0.002 {
                        let c = level_color(a, level.get());
                        set_rgba(cr, &c, 1.0);
                        cr.arc(cx, cy, r, start, start + sweep * frac);
                        let _ = cr.stroke();
                    }
                }
            });
        }
        let a2 = area.clone();
        let s2 = shown.clone();
        let anim_target = adw::CallbackAnimationTarget::new(move |v| {
            s2.set(v);
            a2.queue_draw();
        });
        let anim = adw::TimedAnimation::builder().widget(&area).value_from(0.0).value_to(0.0).duration(650).easing(adw::Easing::EaseOutCubic).target(&anim_target).build();
        Self { root, area, value_label, caption, shown, target, level, anim }
    }

    /// `percent` in 0–100; `text` is what is shown in the centre.
    pub fn set(&self, percent: Option<f64>, text: &str, caption: &str, level: Level) {
        self.value_label.set_text(text);
        self.caption.set_text(caption);
        self.caption.set_visible(!caption.is_empty());
        let lv_changed = self.level.replace(level) != level;
        match percent {
            Some(p) => {
                let p = p.clamp(0.0, 100.0);
                let first = self.target.get().is_none();
                if self.target.get() != Some(p) {
                    self.target.set(Some(p));
                    self.anim.pause();
                    self.anim.set_value_from(if first { 0.0 } else { self.shown.get() });
                    self.anim.set_value_to(p);
                    self.anim.play();
                } else if lv_changed {
                    self.area.queue_draw();
                }
            }
            None => {
                self.target.set(None);
                self.area.queue_draw();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Live chart
// ---------------------------------------------------------------------------

pub struct Series {
    pub values: VecDeque<f64>,
    pub label: String,
    /// None = accent colour.
    pub color: Option<gdk::RGBA>,
    pub fill: bool,
}

struct ChartState {
    series: Vec<Series>,
    capacity: usize,
    min: f64,
    max: f64,
    auto_max: bool,
    unit: String,
    last_push: Instant,
    interval_ms: f64,
    hover_x: Option<f64>,
}

#[derive(Clone)]
pub struct Chart {
    pub area: gtk::DrawingArea,
    state: Rc<RefCell<ChartState>>,
}

impl Chart {
    pub fn new(height: i32, capacity: usize, min: f64, max: f64, unit: &str) -> Self {
        let area = gtk::DrawingArea::builder().content_height(height).hexpand(true).build();
        let state = Rc::new(RefCell::new(ChartState { series: Vec::new(), capacity, min, max, auto_max: false, unit: unit.to_string(), last_push: Instant::now(), interval_ms: 500.0, hover_x: None }));
        {
            let st = state.clone();
            area.set_draw_func(move |a, cr, w, h| draw_chart(a, cr, w as f64, h as f64, &st.borrow()));
        }
        // Smooth scrolling between samples (~30 fps at most).
        {
            let st = state.clone();
            let last_frame = Cell::new(0i64);
            area.add_tick_callback(move |a, clock| {
                if !crate::theme::animations_enabled() {
                    return glib::ControlFlow::Continue;
                }
                let now = clock.frame_time();
                let s = st.borrow();
                let fresh = (s.last_push.elapsed().as_millis() as f64) < s.interval_ms * 1.2;
                if fresh && now - last_frame.get() >= 33_000 {
                    last_frame.set(now);
                    a.queue_draw();
                }
                glib::ControlFlow::Continue
            });
        }
        {
            let motion = gtk::EventControllerMotion::new();
            let st = state.clone();
            let a = area.clone();
            motion.connect_motion(move |_, x, _| {
                st.borrow_mut().hover_x = Some(x);
                a.queue_draw();
            });
            let st = state.clone();
            let a = area.clone();
            motion.connect_leave(move |_| {
                st.borrow_mut().hover_x = None;
                a.queue_draw();
            });
            area.add_controller(motion);
        }
        Self { area, state }
    }

    pub fn add_series(&self, label: &str, color: Option<&str>, fill: bool) -> usize {
        let mut s = self.state.borrow_mut();
        s.series.push(Series { values: VecDeque::new(), label: label.to_string(), color: color.and_then(|c| gdk::RGBA::parse(c).ok()), fill });
        s.series.len() - 1
    }

    pub fn set_auto_max(&self, auto: bool) {
        self.state.borrow_mut().auto_max = auto;
    }

    pub fn set_interval(&self, ms: u32) {
        self.state.borrow_mut().interval_ms = ms as f64;
    }

    pub fn push(&self, idx: usize, value: Option<f64>) {
        let mut s = self.state.borrow_mut();
        let cap = s.capacity;
        if let Some(series) = s.series.get_mut(idx) {
            let v = value.unwrap_or_else(|| series.values.back().copied().unwrap_or(f64::NAN));
            series.values.push_back(v);
            while series.values.len() > cap {
                series.values.pop_front();
            }
        }
        if idx == 0 {
            s.last_push = Instant::now();
        }
        drop(s);
        self.area.queue_draw();
    }

    /// Replaces all values; the chart adapts to the number of points.
    pub fn replace(&self, idx: usize, values: &[f64]) {
        let mut s = self.state.borrow_mut();
        if let Some(series) = s.series.get_mut(idx) {
            series.values = values.iter().copied().collect();
        }
        s.capacity = values.len().max(2);
        drop(s);
        self.area.queue_draw();
    }

    pub fn clear(&self) {
        for s in self.state.borrow_mut().series.iter_mut() {
            s.values.clear();
        }
        self.area.queue_draw();
    }

    /// Fills a series with values already received without changing the
    /// capacity (when returning to a hidden page). `None` repeats the previous
    /// value, same as [`Chart::push`].
    pub fn refill(&self, idx: usize, values: impl IntoIterator<Item = Option<f64>>) {
        let mut s = self.state.borrow_mut();
        let cap = s.capacity;
        if let Some(series) = s.series.get_mut(idx) {
            series.values.clear();
            for v in values {
                let v = v.unwrap_or_else(|| series.values.back().copied().unwrap_or(f64::NAN));
                series.values.push_back(v);
            }
            while series.values.len() > cap {
                series.values.pop_front();
            }
        }
        drop(s);
        self.area.queue_draw();
    }
}

fn draw_chart(a: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: f64, h: f64, s: &ChartState) {
    let pad_top = 6.0;
    let pad_bottom = 4.0;
    let plot_h = h - pad_top - pad_bottom;
    let fgc = fg(a);
    // Grid
    cr.set_line_width(1.0);
    for i in 0..=3 {
        let y = (pad_top + plot_h * i as f64 / 3.0).round() + 0.5;
        set_rgba(cr, &fgc, if i == 3 { 0.14 } else { 0.06 });
        cr.move_to(0.0, y);
        cr.line_to(w, y);
        let _ = cr.stroke();
    }
    let max = if s.auto_max { s.series.iter().flat_map(|x| x.values.iter()).copied().filter(|v| v.is_finite()).fold(s.max, f64::max) * 1.1 } else { s.max };
    let span = (max - s.min).max(1e-9);
    let step = w / (s.capacity.max(2) - 1) as f64;
    let phase = if crate::theme::animations_enabled() { (s.last_push.elapsed().as_millis() as f64 / s.interval_ms).clamp(0.0, 1.0) } else { 1.0 };
    let accent = named_color(a, "accent_bg_color");
    for series in &s.series {
        let n = series.values.len();
        if n < 2 {
            continue;
        }
        let color = series.color.unwrap_or(accent);
        // The last point ends at the right edge when the transition finishes.
        let offset = (1.0 - phase) * step;
        let x_of = |i: usize| w - (n - 1 - i) as f64 * step + offset;
        let y_of = |v: f64| pad_top + plot_h * (1.0 - ((v - s.min) / span).clamp(0.0, 1.0));
        let mut started = false;
        cr.new_path();
        for (i, v) in series.values.iter().enumerate() {
            if !v.is_finite() {
                started = false;
                continue;
            }
            let (x, y) = (x_of(i), y_of(*v));
            if started {
                cr.line_to(x, y);
            } else {
                cr.move_to(x, y);
                started = true;
            }
        }
        let path = cr.copy_path();
        if series.fill {
            if let Ok(path) = &path {
                cr.new_path();
                cr.append_path(path);
                let last_x = x_of(n - 1);
                let first_x = series.values.iter().position(|v| v.is_finite()).map(x_of).unwrap_or(last_x);
                cr.line_to(last_x, pad_top + plot_h);
                cr.line_to(first_x, pad_top + plot_h);
                cr.close_path();
                let grad = gtk::cairo::LinearGradient::new(0.0, pad_top, 0.0, pad_top + plot_h);
                grad.add_color_stop_rgba(0.0, color.red() as f64, color.green() as f64, color.blue() as f64, 0.32);
                grad.add_color_stop_rgba(1.0, color.red() as f64, color.green() as f64, color.blue() as f64, 0.02);
                let _ = cr.set_source(&grad);
                let _ = cr.fill();
            }
        }
        if let Ok(path) = &path {
            cr.new_path();
            cr.append_path(path);
            set_rgba(cr, &color, 1.0);
            cr.set_line_width(2.0);
            cr.set_line_join(gtk::cairo::LineJoin::Round);
            let _ = cr.stroke();
        }
    }
    // Hover indicator
    if let Some(hx) = s.hover_x {
        if let Some(first) = s.series.first() {
            let n = first.values.len();
            if n >= 2 {
                let offset = (1.0 - phase) * step;
                let idx_f = (n as f64 - 1.0) - ((w + offset - hx) / step);
                let idx = idx_f.round().clamp(0.0, n as f64 - 1.0) as usize;
                let x = w - (n - 1 - idx) as f64 * step + offset;
                set_rgba(cr, &fgc, 0.35);
                cr.set_line_width(1.0);
                cr.move_to(x.round() + 0.5, pad_top);
                cr.line_to(x.round() + 0.5, pad_top + plot_h);
                let _ = cr.stroke();
                let texts: Vec<String> = s
                    .series
                    .iter()
                    .filter_map(|se| se.values.get(idx).filter(|v| v.is_finite()).map(|v| (se, *v)))
                    .map(|(se, v)| if se.label.is_empty() { format!("{v:.1}{}", s.unit) } else { format!("{}: {v:.1}{}", se.label, s.unit) })
                    .collect();
                if !texts.is_empty() {
                    let text = texts.join("  ·  ");
                    let layout = a.create_pango_layout(Some(&text));
                    let (tw, th) = layout.pixel_size();
                    let bx = (x + 8.0).min(w - tw as f64 - 12.0).max(2.0);
                    let by = pad_top + 2.0;
                    set_rgba(cr, &fgc, 0.85);
                    rounded_rect(cr, bx, by, tw as f64 + 10.0, th as f64 + 6.0, 6.0);
                    let _ = cr.fill();
                    let bg = named_color(a, "window_bg_color");
                    set_rgba(cr, &bg, 1.0);
                    cr.move_to(bx + 5.0, by + 3.0);
                    pangocairo::functions::show_layout(cr, &layout);
                }
            }
        }
    }
}

fn rounded_rect(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

// ---------------------------------------------------------------------------
// Cards and bars
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct StatCard {
    pub root: gtk::Box,
    pub value: gtk::Label,
    pub sub: gtk::Label,
    pub extra: gtk::Box,
}

impl StatCard {
    pub fn new(icon_name: &str, title: &str) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.add_css_class("card");
        root.add_css_class("stat-card");
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.add_css_class("stat-icon");
        let t = gtk::Label::new(Some(title));
        t.add_css_class("stat-title");
        t.set_xalign(0.0);
        head.append(&icon);
        head.append(&t);
        let value = gtk::Label::new(Some("—"));
        value.add_css_class("stat-value");
        value.set_xalign(0.0);
        value.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let sub = gtk::Label::new(None);
        sub.add_css_class("stat-sub");
        sub.set_xalign(0.0);
        sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let extra = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.append(&head);
        root.append(&value);
        root.append(&sub);
        root.append(&extra);
        root.set_hexpand(true);
        Self { root, value, sub, extra }
    }

    pub fn value_text(&self) -> String {
        self.value.text().to_string()
    }

    pub fn set(&self, value: &str, sub: &str) {
        self.value.set_text(value);
        self.sub.set_text(sub);
        self.sub.set_visible(!sub.is_empty());
    }
}

pub fn usage_bar() -> gtk::LevelBar {
    let bar = gtk::LevelBar::builder().min_value(0.0).max_value(100.0).build();
    bar.add_css_class("usage");
    // Colours are handled with our own classes.
    bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_LOW));
    bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_HIGH));
    bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_FULL));
    bar
}

pub fn set_usage(bar: &gtk::LevelBar, percent: Option<f64>, high: f64, critical: f64) {
    let p = percent.unwrap_or(0.0).clamp(0.0, 100.0);
    bar.set_value(p);
    bar.remove_css_class("high");
    bar.remove_css_class("critical");
    if p >= critical {
        bar.add_css_class("critical");
    } else if p >= high {
        bar.add_css_class("high");
    }
}

/// Health bar: full is good, so the colour follows the rating and not the
/// percentage (the opposite of [`set_usage`]).
pub fn set_health(bar: &gtk::LevelBar, percent: Option<f64>, level: Option<adbscan_core::models::HealthLevel>) {
    use adbscan_core::models::HealthLevel;
    bar.set_value(percent.unwrap_or(0.0).clamp(0.0, 100.0));
    bar.remove_css_class("high");
    bar.remove_css_class("critical");
    match level {
        Some(HealthLevel::Fair) => bar.add_css_class("high"),
        Some(HealthLevel::Poor) => bar.add_css_class("critical"),
        _ => {}
    }
}

/// Staggered appearance (fade + slide) for lists of cards.
pub fn reveal_staggered(widgets: &[gtk::Widget]) {
    if !crate::theme::animations_enabled() {
        return;
    }
    for (i, w) in widgets.iter().enumerate() {
        w.set_opacity(0.0);
        let w2 = w.clone();
        let target = adw::CallbackAnimationTarget::new(move |v| {
            w2.set_opacity(v);
            w2.set_margin_top(((1.0 - v) * 14.0) as i32);
        });
        let anim = adw::TimedAnimation::builder().widget(w).value_from(0.0).value_to(1.0).duration(380).easing(adw::Easing::EaseOutCubic).target(&target).build();
        let a = anim.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(40 * i as u64), move || a.play());
        // If the widget is hidden before it animates, leave it visible.
        let w3 = w.clone();
        anim.connect_done(move |_| {
            w3.set_opacity(1.0);
            w3.set_margin_top(0);
        });
    }
}

/// Status pill with a coloured dot.
#[derive(Clone)]
pub struct StatusPill {
    pub root: gtk::Box,
    dot: gtk::Box,
    label: gtk::Label,
}

impl StatusPill {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        root.add_css_class("pill");
        root.set_valign(gtk::Align::Center);
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("status-dot");
        dot.set_valign(gtk::Align::Center);
        let label = gtk::Label::new(Some(tr("No device")));
        root.append(&dot);
        root.append(&label);
        Self { root, dot, label }
    }

    pub fn set(&self, text: &str, kind: &str) {
        self.label.set_text(text);
        for c in ["ok", "warn", "bad", "accent"] {
            self.dot.remove_css_class(c);
            self.root.remove_css_class(c);
        }
        if !kind.is_empty() {
            self.dot.add_css_class(kind);
            self.root.add_css_class(kind);
        }
    }
}

/// Level bar whose value changes with a smooth transition.
#[derive(Clone)]
pub struct AnimatedBar {
    pub bar: gtk::LevelBar,
    anim: adw::TimedAnimation,
    current: Rc<Cell<f64>>,
}

impl AnimatedBar {
    pub fn new() -> Self {
        let bar = usage_bar();
        let current: Rc<Cell<f64>> = Rc::new(Cell::new(0.0));
        let b2 = bar.clone();
        let c2 = current.clone();
        let target = adw::CallbackAnimationTarget::new(move |v| {
            c2.set(v);
            b2.set_value(v);
        });
        let anim = adw::TimedAnimation::builder().widget(&bar).value_from(0.0).value_to(0.0).duration(420).easing(adw::Easing::EaseOutCubic).target(&target).build();
        Self { bar, anim, current }
    }

    pub fn set(&self, percent: Option<f64>, high: f64, critical: f64) {
        let p = percent.unwrap_or(0.0).clamp(0.0, 100.0);
        self.bar.remove_css_class("high");
        self.bar.remove_css_class("critical");
        if p >= critical {
            self.bar.add_css_class("critical");
        } else if p >= high {
            self.bar.add_css_class("high");
        }
        if (self.anim.value_to() - p).abs() < 0.05 {
            return;
        }
        self.anim.pause();
        self.anim.set_value_from(self.current.get());
        self.anim.set_value_to(p);
        self.anim.play();
    }
}

/// [`RowList`] row: optional icon (with its colour class), title, subtitle,
/// tooltip and, if `action` is not empty, a button at the end that receives
/// that value.
#[derive(Clone, Default)]
pub struct RowItem {
    pub title: String,
    pub subtitle: String,
    pub tooltip: String,
    pub action: String,
    pub icon: String,
    pub icon_class: String,
    /// Group heading (a day, a category): bold title, no button.
    pub heading: bool,
}

const SEP: char = '\u{1f}';

impl RowItem {
    pub fn new(title: impl Into<String>, subtitle: impl Into<String>) -> Self {
        Self { title: title.into(), subtitle: subtitle.into(), ..Default::default() }
    }
    /// With an icon on the left (`class`: e.g. `severity-ok`).
    pub fn with_icon(mut self, icon: &str, class: &str) -> Self {
        self.icon = icon.into();
        self.icon_class = class.into();
        self
    }
    /// Heading of the rows that follow it.
    pub fn heading(title: impl Into<String>) -> Self {
        Self { title: title.into(), heading: true, ..Default::default() }
    }
    fn encode(&self) -> String {
        let clean = |s: &str| s.replace(SEP, " ");
        format!("{}{SEP}{}{SEP}{}{SEP}{}{SEP}{}{SEP}{}{SEP}{}", clean(&self.title), clean(&self.subtitle), clean(&self.tooltip), clean(&self.action), clean(&self.icon), clean(&self.icon_class), if self.heading { "h" } else { "" })
    }
    fn decode(s: &str) -> Self {
        let mut it = s.split(SEP).map(String::from);
        Self {
            title: it.next().unwrap_or_default(),
            subtitle: it.next().unwrap_or_default(),
            tooltip: it.next().unwrap_or_default(),
            action: it.next().unwrap_or_default(),
            icon: it.next().unwrap_or_default(),
            icon_class: it.next().unwrap_or_default(),
            heading: it.next().is_some_and(|h| h == "h"),
        }
    }
}

/// Action of each [`RowList`] row's button (button, row text).
type RowAction = Rc<RefCell<Option<Box<dyn Fn(&gtk::Button, &str)>>>>;

/// Virtualised list: it only creates the widgets of the visible rows, so
/// thousands of items do not block the interface (unlike a `GtkListBox`, which
/// creates and measures every row).
pub struct RowList {
    pub root: gtk::ScrolledWindow,
    model: gtk::StringList,
    action: RowAction,
    /// Rows received while the list had no size (hidden tab or page not drawn
    /// yet): they are applied when it is shown.
    pending: Rc<RefCell<Option<Vec<String>>>>,
}

impl RowList {
    /// List inside a page, with a maximum height (560 px).
    /// `action_icon`: icon of each row's button (if the row has an action).
    pub fn new(action_icon: Option<(&'static str, &'static str)>) -> Self {
        Self::build(action_icon, false)
    }

    /// List that takes all the available space (a whole tab), with its width
    /// limited like the other pages.
    pub fn filling(action_icon: Option<(&'static str, &'static str)>) -> Self {
        Self::build(action_icon, true)
    }

    fn build(action_icon: Option<(&'static str, &'static str)>, fill: bool) -> Self {
        let model = gtk::StringList::new(&[]);
        let action: RowAction = Rc::new(RefCell::new(None));
        let factory = gtk::SignalListItemFactory::new();
        {
            let action = action.clone();
            factory.connect_setup(move |_, obj| {
                let Some(item) = obj.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                row.add_css_class("rowlist-row");
                let icon = gtk::Image::new();
                icon.set_valign(gtk::Align::Center);
                icon.set_visible(false);
                row.append(&icon);
                let texts = gtk::Box::new(gtk::Orientation::Vertical, 2);
                texts.set_hexpand(true);
                texts.set_valign(gtk::Align::Center);
                let title = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
                let subtitle = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).selectable(!fill).build();
                subtitle.add_css_class("dim");
                subtitle.add_css_class("caption");
                texts.append(&title);
                texts.append(&subtitle);
                row.append(&texts);
                if let Some((icon, tip)) = action_icon {
                    let b = gtk::Button::from_icon_name(icon);
                    b.set_tooltip_text(Some(tip));
                    b.add_css_class("flat");
                    b.set_valign(gtk::Align::Center);
                    let weak_item = item.downgrade();
                    let action = action.clone();
                    b.connect_clicked(move |b| {
                        let Some(item) = weak_item.upgrade() else {
                            return;
                        };
                        let Some(s) = item.item().and_downcast::<gtk::StringObject>() else {
                            return;
                        };
                        let data = RowItem::decode(&s.string());
                        if let Some(f) = action.borrow().as_ref() {
                            f(b, &data.action);
                        }
                    });
                    row.append(&b);
                }
                item.set_activatable(false);
                item.set_child(Some(&row));
            });
        }
        factory.connect_bind(|_, obj| {
            let Some(item) = obj.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let (Some(s), Some(row)) = (item.item().and_downcast::<gtk::StringObject>(), item.child()) else {
                return;
            };
            let data = RowItem::decode(&s.string());
            let Some(icon) = row.first_child().and_downcast::<gtk::Image>() else {
                return;
            };
            icon.set_visible(!data.icon.is_empty());
            if !data.icon.is_empty() {
                icon.set_icon_name(Some(&data.icon));
                // Replaces the classes: the row is reused for other items.
                let classes: Vec<&str> = [data.icon_class.as_str()].into_iter().filter(|c| !c.is_empty()).collect();
                icon.set_css_classes(&classes);
            }
            let Some(texts) = icon.next_sibling() else {
                return;
            };
            if let Some(t) = texts.first_child().and_downcast::<gtk::Label>() {
                t.set_text(&data.title);
                if data.heading {
                    t.add_css_class("heading");
                } else {
                    t.remove_css_class("heading");
                }
            }
            if data.heading {
                row.add_css_class("rowlist-heading");
            } else {
                row.remove_css_class("rowlist-heading");
            }
            if let Some(st) = texts.last_child().and_downcast::<gtk::Label>() {
                st.set_text(&data.subtitle);
                st.set_visible(!data.subtitle.is_empty());
            }
            row.set_tooltip_text((!data.tooltip.is_empty()).then_some(data.tooltip.as_str()));
            if let Some(b) = row.last_child().filter(|w| w.is::<gtk::Button>()) {
                b.set_visible(!data.action.is_empty());
            }
        });
        let selection = gtk::NoSelection::new(Some(model.clone()));
        let list = gtk::ListView::builder().model(&selection).factory(&factory).build();
        list.add_css_class("rowlist");
        let root = if fill {
            // The frame goes on the list (with its limited width), not on the
            // whole scrolled area.
            list.add_css_class("rowlist-frame");
            let clamp = adw::ClampScrollable::builder().maximum_size(1000).tightening_threshold(800).child(&list).build();
            clamp.set_margin_top(14);
            clamp.set_margin_bottom(24);
            clamp.set_margin_start(18);
            clamp.set_margin_end(18);
            gtk::ScrolledWindow::builder().child(&clamp).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build()
        } else {
            let root = gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).min_content_height(160).max_content_height(560).propagate_natural_height(true).build();
            root.add_css_class("rowlist-frame");
            root
        };
        let pending: Rc<RefCell<Option<Vec<String>>>> = Rc::new(RefCell::new(None));
        {
            // When shown: wait for the first frame in which it already has a
            // height (before that, GTK would prepare every row).
            let (pending, model) = (pending.clone(), model.clone());
            root.connect_map(move |root| {
                let (pending, model) = (pending.clone(), model.clone());
                root.add_tick_callback(move |root, _| {
                    if !root.is_mapped() {
                        return glib::ControlFlow::Break;
                    }
                    if root.height() == 0 {
                        return glib::ControlFlow::Continue;
                    }
                    if let Some(items) = pending.borrow_mut().take() {
                        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
                        model.splice(0, model.n_items(), &refs);
                    }
                    glib::ControlFlow::Break
                });
            });
        }
        Self { root, model, action, pending }
    }

    /// Action of the row button; receives the item's `action` field.
    pub fn connect_action(&self, f: impl Fn(&gtk::Button, &str) + 'static) {
        *self.action.borrow_mut() = Some(Box::new(f));
    }

    /// Replaces all the content (a single notification to the model).
    ///
    /// A `GtkListView` without a size does not know which rows are visible and
    /// prepares all of them (~0.7 ms per row). That is why, if it has not been
    /// placed on screen yet, the rows wait until it is shown.
    pub fn set_items(&self, items: impl IntoIterator<Item = RowItem>) {
        let encoded: Vec<String> = items.into_iter().map(|i| i.encode()).collect();
        if !self.root.is_mapped() || self.root.height() == 0 {
            *self.pending.borrow_mut() = Some(encoded);
            return;
        }
        self.pending.borrow_mut().take();
        let refs: Vec<&str> = encoded.iter().map(String::as_str).collect();
        self.model.splice(0, self.model.n_items(), &refs);
    }
}

// ---------------------------------------------------------------------------
// Real icon of an app
// ---------------------------------------------------------------------------

fn pixbuf(bytes: &[u8]) -> Option<gtk::gdk_pixbuf::Pixbuf> {
    let stream = gtk::gio::MemoryInputStream::from_bytes(&glib::Bytes::from(bytes));
    gtk::gdk_pixbuf::Pixbuf::from_stream(&stream, gtk::gio::Cancellable::NONE).ok()
}

/// Real icon of an app (see `adbscan_core::models::AppIcon`), `size` pixels
/// wide. An adaptive one is drawn the way Android does: the foreground layer
/// (which has a margin of 18 out of 108 units) scaled up and clipped to a
/// rounded square over its background. `None` if the image cannot be read.
pub fn app_icon(icon: &adbscan_core::models::AppIcon, size: i32) -> Option<gtk::DrawingArea> {
    let front = pixbuf(&icon.image)?;
    let back = icon.background_image.as_deref().and_then(pixbuf);
    let color = icon.background_color;
    let adaptive = icon.adaptive;
    let area = gtk::DrawingArea::builder().content_width(size).content_height(size).valign(gtk::Align::Center).build();
    area.set_draw_func(move |_, cr, w, h| {
        use gtk::prelude::GdkCairoContextExt;
        let s = f64::from(w.min(h));
        let paint = |pb: &gtk::gdk_pixbuf::Pixbuf, scale_to: f64| {
            let (pw, ph) = (f64::from(pb.width()), f64::from(pb.height()));
            let k = scale_to / pw.max(ph);
            cr.save().ok();
            cr.translate((s - pw * k) / 2.0, (s - ph * k) / 2.0);
            cr.scale(k, k);
            cr.set_source_pixbuf(pb, 0.0, 0.0);
            // Smoothing when scaling down (icons usually come at 192 px or
            // more).
            cr.source().set_filter(gtk::cairo::Filter::Good);
            cr.paint().ok();
            cr.restore().ok();
        };
        if !adaptive {
            paint(&front, s);
            return;
        }
        // Rounded square (radius ~22 %, like Android 12+ icons).
        let r = s * 0.22;
        cr.new_sub_path();
        cr.arc(s - r, r, r, -PI / 2.0, 0.0);
        cr.arc(s - r, s - r, r, 0.0, PI / 2.0);
        cr.arc(r, s - r, r, PI / 2.0, PI);
        cr.arc(r, r, r, PI, 3.0 * PI / 2.0);
        cr.close_path();
        cr.clip();
        let argb = color.unwrap_or(0xffff_ffff);
        let ch = |shift: u32| f64::from((argb >> shift) & 0xff) / 255.0;
        cr.set_source_rgba(ch(16), ch(8), ch(0), if color.is_some() { ch(24) } else { 1.0 });
        cr.paint().ok();
        // Layers measure 108 units and the central 72 are visible: ×1.5.
        if let Some(b) = &back {
            paint(b, s * 1.5);
        }
        paint(&front, s * 1.5);
    });
    area.set_tooltip_text(Some(tr("App icon")));
    Some(area)
}
