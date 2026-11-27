//! Apps → details → Tuning: how smoothly the app draws (read-only), its own
//! language and, for games, the game mode (see `adbscan_core::app_tuning`).
//! Every change is checked by reading it back, recorded in History and undone
//! by choosing the previous value (“System language”, “Standard”).

use adbscan_core::{format, i18n::tr, t};
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use adbscan_core::app_tuning::{self as tuning, AppTuning, FPS_CAPS, LANGUAGES, SCALES};

use crate::ctx::Ctx;
use crate::util;

struct State {
    ctx: Rc<Ctx>,
    package: String,
    name: String,
    gen: u64,
    live: Arc<AtomicU64>,
    smooth: adw::ActionRow,
    language: adw::ComboRow,
    /// Language tags in the list's order (`None`: the system's).
    language_tags: RefCell<Vec<Option<String>>>,
    mode: adw::ComboRow,
    modes: RefCell<Vec<String>>,
    scale: adw::ComboRow,
    fps: adw::ComboRow,
    /// Painting what was read must not apply anything.
    painting: Cell<bool>,
    last: RefCell<Option<AppTuning>>,
}

pub fn group(ctx: &Rc<Ctx>, package: &str, name: &str, gen: u64, live: Arc<AtomicU64>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(tr("Tuning")).build();
    let smooth = adw::ActionRow::builder().title(tr("Smoothness")).subtitle(tr("reading…")).build();
    smooth.add_prefix(&util::icon("adb-activity-symbolic"));
    let again = gtk::Button::from_icon_name("adb-refresh-cw-symbolic");
    again.add_css_class("flat");
    again.set_valign(gtk::Align::Center);
    again.set_tooltip_text(Some(tr("Measure again")));
    smooth.add_suffix(&again);
    group.add(&smooth);

    let language = adw::ComboRow::builder().title(tr("Language")).subtitle(tr("Only this app; the rest of the phone keeps its language")).visible(false).build();
    group.add(&language);

    let mode = adw::ComboRow::builder().title(tr("Game mode")).visible(false).build();
    let scale_names: Vec<String> = SCALES.iter().map(|s| if *s >= 0.99 { tr("Full").to_string() } else { format!("{} %", (s * 100.0).round()) }).collect();
    let scale = adw::ComboRow::builder()
        .title(tr("Resolution"))
        .subtitle(tr("Lower: less heat and battery, a less sharp image"))
        .model(&gtk::StringList::new(&scale_names.iter().map(String::as_str).collect::<Vec<_>>()))
        .visible(false)
        .build();
    let fps_names: Vec<String> = FPS_CAPS.iter().map(|f| if *f == 0 { tr("No cap").to_string() } else { format!("{f} FPS") }).collect();
    let fps = adw::ComboRow::builder()
        .title(tr("Frame rate cap"))
        .model(&gtk::StringList::new(&fps_names.iter().map(String::as_str).collect::<Vec<_>>()))
        .visible(false)
        .build();
    group.add(&mode);
    group.add(&scale);
    group.add(&fps);

    let st = Rc::new(State {
        ctx: ctx.clone(),
        package: package.to_string(),
        name: name.to_string(),
        gen,
        live,
        smooth,
        language: language.clone(),
        language_tags: RefCell::new(Vec::new()),
        mode: mode.clone(),
        modes: RefCell::new(Vec::new()),
        scale: scale.clone(),
        fps: fps.clone(),
        painting: Cell::new(false),
        last: RefCell::new(None),
    });
    st.read();
    let w = Rc::downgrade(&st);
    again.connect_clicked(move |_| {
        if let Some(st) = w.upgrade() {
            st.smooth.set_subtitle(tr("reading…"));
            st.read();
        }
    });
    let w = Rc::downgrade(&st);
    language.connect_selected_notify(move |row| {
        let Some(st) = w.upgrade() else { return };
        if st.painting.get() {
            return;
        }
        let Some(tag) = st.language_tags.borrow().get(row.selected() as usize).cloned() else { return };
        let label = row.selected_item().and_downcast::<gtk::StringObject>().map(|s| s.string().to_string()).unwrap_or_default();
        let pkg = st.package.clone();
        st.apply("app_language", t!("{} · language: {}", st.name, label), move |adb, serial| adb.set_app_locale(&serial, &pkg, tag.as_deref()));
    });
    let w = Rc::downgrade(&st);
    mode.connect_selected_notify(move |_| {
        if let Some(st) = w.upgrade() {
            st.apply_game();
        }
    });
    for row in [&scale, &fps] {
        let w = Rc::downgrade(&st);
        row.connect_selected_notify(move |_| {
            if let Some(st) = w.upgrade() {
                st.apply_game();
            }
        });
    }
    // The group owns the state (the rows only keep weak references): it lives
    // as long as the details are shown. No cycle: the state does not hold the
    // group.
    group.connect_unmap(move |_| {
        let _ = &st;
    });
    group
}

impl State {
    fn current(&self) -> bool {
        self.live.load(Ordering::Relaxed) == self.gen
    }

    fn read(self: &Rc<Self>) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else { return };
        let pkg = self.package.clone();
        let w = Rc::downgrade(self);
        util::run_bg(
            move || adb.app_tuning(&serial, &pkg),
            move |r| {
                let Some(st) = w.upgrade() else { return };
                if !st.current() {
                    return;
                }
                match r {
                    Ok(t) => st.paint(&t),
                    Err(e) => st.smooth.set_subtitle(&t!("Could not read it: {}", e)),
                }
            },
        );
    }

    fn paint(&self, t: &AppTuning) {
        self.painting.set(true);
        match t.smoothness.ok() {
            Some(s) => {
                let pct = format::number(Some(s.janky_pct), if s.janky_pct < 10.0 { 1 } else { 0 });
                self.smooth.set_subtitle(&t!(
                    "{} · {} % slow frames of {} · 9 of 10 frames in {} ms or less",
                    s.verdict(),
                    pct,
                    format::number(Some(s.frames as f64), 0),
                    s.p90
                ));
                self.smooth.set_tooltip_text(Some(&t!("Frame times: half under {} ms, 95 % under {} ms, 99 % under {} ms. Since the app was opened.", s.p50, s.p95, s.p99)));
            }
            None => {
                self.smooth.set_subtitle(tr("Open the app on the phone and use it for a while, then measure again"));
                self.smooth.set_tooltip_text(Some(tr("Games that draw with their own engine are not measured here: see Performance → FPS.")));
            }
        }

        // Language (Android 13+).
        match t.locales.ok() {
            Some(current) if t.sdk.unwrap_or(0) >= 33 => {
                let mut tags: Vec<Option<String>> = vec![None];
                let mut names: Vec<String> = vec![tr("System language").into()];
                for (tag, name) in LANGUAGES {
                    tags.push(Some(tag.to_string()));
                    names.push(name.to_string());
                }
                // A language set elsewhere that is not in the list.
                let now = current.first().cloned();
                if let Some(n) = &now {
                    if !tags.iter().flatten().any(|t| t == n) {
                        tags.push(Some(n.clone()));
                        names.push(n.clone());
                    }
                }
                self.language.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
                let idx = tags.iter().position(|t| *t == now).unwrap_or(0);
                self.language.set_selected(idx as u32);
                *self.language_tags.borrow_mut() = tags;
                self.language.set_visible(true);
                self.language.set_sensitive(true);
            }
            _ => self.language.set_visible(false),
        }

        // Game mode (only games).
        match t.game.ok() {
            Some(g) if !g.available.is_empty() => {
                let names: Vec<String> = g.available.iter().map(|m| tuning::mode_label(m)).collect();
                self.mode.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
                self.mode.set_selected(g.available.iter().position(|m| *m == g.current).unwrap_or(0) as u32);
                *self.modes.borrow_mut() = g.available.clone();
                self.mode.set_subtitle(if g.available.len() <= 2 && g.available.iter().any(|m| m == "custom") {
                    tr("This game offers no modes of its own: “Custom” lowers its resolution or caps its frame rate")
                } else {
                    tr("The modes this game and the phone offer")
                });
                self.mode.set_visible(true);
                self.mode.set_sensitive(true);
                let custom = g.current == "custom";
                let s = g.scale.unwrap_or(1.0);
                let si = SCALES.iter().position(|v| (v - s).abs() < 0.01).unwrap_or(0);
                self.scale.set_selected(si as u32);
                let fi = FPS_CAPS.iter().position(|v| Some(*v) == g.fps).unwrap_or(0);
                self.fps.set_selected(fi as u32);
                self.scale.set_visible(custom);
                self.fps.set_visible(custom);
                self.scale.set_sensitive(true);
                self.fps.set_sensitive(true);
            }
            _ => {
                self.mode.set_visible(false);
                self.scale.set_visible(false);
                self.fps.set_visible(false);
            }
        }
        *self.last.borrow_mut() = Some(t.clone());
        self.painting.set(false);
    }

    fn apply_game(self: &Rc<Self>) {
        if self.painting.get() {
            return;
        }
        let Some(mode) = self.modes.borrow().get(self.mode.selected() as usize).cloned() else { return };
        let scale = SCALES.get(self.scale.selected() as usize).copied().unwrap_or(1.0);
        let mut fps = FPS_CAPS.get(self.fps.selected() as usize).copied().unwrap_or(0);
        let msg = if mode == "custom" {
            // Choosing “Custom” first: a gentle start (60 FPS cap).
            if scale >= 0.99 && fps == 0 {
                fps = 60;
            }
            let res = if scale >= 0.99 { tr("full resolution").to_string() } else { t!("{} % resolution", (scale * 100.0).round()) };
            let cap = if fps == 0 { tr("no frame rate cap").to_string() } else { t!("up to {} FPS", fps) };
            t!("{} · game mode: {}, {}", self.name, res, cap)
        } else {
            t!("{} · game mode: {}", self.name, tuning::mode_label(&mode))
        };
        let pkg = self.package.clone();
        self.apply("game_mode", msg, move |adb, serial| adb.set_game_mode(&serial, &pkg, &mode, scale, fps));
    }

    /// Applies a change in the background, then paints what the phone has.
    fn apply<F>(self: &Rc<Self>, action: &'static str, msg: String, f: F)
    where
        F: FnOnce(&adbscan_core::adb::Adb, String) -> adbscan_core::Result<AppTuning> + Send + 'static,
    {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else { return };
        for row in [&self.language, &self.mode, &self.scale, &self.fps] {
            row.set_sensitive(false);
        }
        let pkg = self.package.clone();
        let w = Rc::downgrade(self);
        util::run_bg(
            move || {
                let r = f(&adb, serial.clone());
                let now = if r.is_err() { adb.app_tuning(&serial, &pkg).ok() } else { None };
                (r, now)
            },
            move |(r, now)| {
                let Some(st) = w.upgrade() else { return };
                let target = format!("{} · {}", st.package, msg);
                match &r {
                    Ok(_) => {
                        st.ctx.audit(action, Some(&target), true, None);
                        st.ctx.toast(&msg);
                    }
                    Err(e) => {
                        st.ctx.audit(action, Some(&target), false, Some(&e.to_string()));
                        st.ctx.toast(&t!("Could not change it: {}", e));
                    }
                }
                if !st.current() {
                    return;
                }
                match r.ok().or(now).or_else(|| st.last.borrow().clone()) {
                    Some(t) => st.paint(&t),
                    None => {
                        for row in [&st.language, &st.mode, &st.scale, &st.fps] {
                            row.set_sensitive(true);
                        }
                    }
                }
            },
        );
    }
}
