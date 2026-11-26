//! Developer tab (in Tools): switches, animation speed, screen resolution and
//! density, the status bar's demo mode and port forwarding (see
//! `adbscan_core::devtools`).

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adbscan_core::devtools::{DevState, PortRule, Switch, ANIMATION_SCALES};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

/// Seconds before a new resolution or density is undone if not confirmed.
const REVERT_SECS: u32 = 15;

struct DeveloperPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    switches: Vec<(Switch, adw::SwitchRow)>,
    anim: adw::ComboRow,
    screen_info: adw::ActionRow,
    width: adw::SpinRow,
    height: adw::SpinRow,
    density: adw::SpinRow,
    rules: adw::PreferencesGroup,
    rule_rows: RefCell<Vec<adw::ActionRow>>,
    direction: adw::ComboRow,
    port_phone: adw::SpinRow,
    port_pc: adw::SpinRow,
    state: RefCell<Option<DevState>>,
    /// True while the widgets are being filled from the phone (their signals
    /// must not send anything back).
    filling: Cell<bool>,
    read_for: RefCell<Option<String>>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    DeveloperPage::new(ctx)
}

fn spin(title: &str, min: f64, max: f64) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(min, max, 1.0);
    row.set_title(title);
    row.set_numeric(true);
    row
}

fn scale_label(s: &str) -> String {
    match s {
        "0" => tr("Off").to_string(),
        "1" => tr("1× (normal)").to_string(),
        other => format!("{other}×"),
    }
}

impl DeveloperPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);

        let sw_group = adw::PreferencesGroup::builder().title(tr("Developer switches")).description(tr("The same switches as in Developer options, without looking for them on the phone.")).build();
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Read again"));
        refresh.add_css_class("flat");
        refresh.set_valign(gtk::Align::Center);
        sw_group.set_header_suffix(Some(&refresh));
        let switches: Vec<(Switch, adw::SwitchRow)> = Switch::ALL
            .iter()
            .map(|s| {
                let row = adw::SwitchRow::builder().title(s.title()).subtitle(s.description()).build();
                sw_group.add(&row);
                (*s, row)
            })
            .collect();
        let labels: Vec<String> = ANIMATION_SCALES.iter().map(|s| scale_label(s)).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let anim = adw::ComboRow::builder().title(tr("Animation speed")).subtitle(tr("Window, transition and animator scale. Off makes the phone feel faster.")).model(&gtk::StringList::new(&refs)).build();
        sw_group.add(&anim);
        content.append(&sw_group);

        let screen = adw::PreferencesGroup::builder()
            .title(tr("Screen"))
            .description(t!("Resolution and density (what “Display size” changes). After applying, you have {} seconds to keep the change: if you do not answer, it is undone by itself.", REVERT_SECS))
            .build();
        let screen_info = adw::ActionRow::builder().title(tr("Current")).build();
        let width = spin(tr("Width (px)"), 240.0, 8192.0);
        let height = spin(tr("Height (px)"), 240.0, 8192.0);
        let density = spin(tr("Density (dpi)"), 72.0, 1200.0);
        let actions = util::hbox(8);
        actions.set_halign(gtk::Align::End);
        actions.set_margin_top(8);
        let reset = gtk::Button::with_label(tr("Restore the original"));
        reset.add_css_class("pill");
        let apply = gtk::Button::with_label(tr("Apply"));
        apply.add_css_class("pill");
        apply.add_css_class("suggested-action");
        actions.append(&reset);
        actions.append(&apply);
        screen.add(&screen_info);
        screen.add(&width);
        screen.add(&height);
        screen.add(&density);
        content.append(&screen);
        content.append(&actions);

        let demo = adw::PreferencesGroup::builder()
            .title(tr("Status bar demo mode"))
            .description(tr("Full battery, full signal, 12:00 and no notification icons: clean screenshots and recordings. It does not change the real battery or network."))
            .build();
        let demo_row = adw::ActionRow::builder().title(tr("Demo mode")).build();
        let demo_on = gtk::Button::with_label(tr("Turn on"));
        let demo_off = gtk::Button::with_label(tr("Turn off"));
        for b in [&demo_off, &demo_on] {
            b.set_valign(gtk::Align::Center);
            demo_row.add_suffix(b);
        }
        demo.add(&demo_row);
        content.append(&demo);

        let rules = adw::PreferencesGroup::builder()
            .title(tr("Port forwarding"))
            .description(tr("“Phone → PC” makes a server running on the PC reachable on the phone as localhost (web development, Metro, Vite). “PC → phone” does the opposite. Rules last until ADB or the phone restarts."))
            .build();
        let direction = adw::ComboRow::builder().title(tr("Direction")).model(&gtk::StringList::new(&[tr("Phone → PC (adb reverse)"), tr("PC → phone (adb forward)")])).build();
        let port_phone = spin(tr("Port on the phone"), 1.0, 65535.0);
        let port_pc = spin(tr("Port on the PC"), 1.0, 65535.0);
        port_phone.set_value(3000.0);
        port_pc.set_value(3000.0);
        let add = gtk::Button::with_label(tr("Add rule"));
        add.add_css_class("pill");
        add.set_halign(gtk::Align::End);
        add.set_margin_top(8);
        let add_group = adw::PreferencesGroup::new();
        add_group.add(&direction);
        add_group.add(&port_phone);
        add_group.add(&port_pc);
        content.append(&rules);
        content.append(&add_group);
        content.append(&add);

        let root = util::page_scroller(&content, 900);
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            switches,
            anim: anim.clone(),
            screen_info,
            width,
            height,
            density,
            rules,
            rule_rows: RefCell::new(Vec::new()),
            direction,
            port_phone,
            port_pc,
            state: RefCell::new(None),
            filling: Cell::new(false),
            read_for: RefCell::new(None),
        });
        macro_rules! on {
            ($w:expr, $sig:ident, |$t:ident, $a:pat_param| $body:expr) => {{
                let w = this.me.clone();
                $w.$sig(move |$a| {
                    if let Some($t) = w.upgrade() {
                        $body;
                    }
                });
            }};
        }
        on!(refresh, connect_clicked, |t, _| t.load());
        for (s, row) in &this.switches {
            let s = *s;
            on!(row, connect_active_notify, |t, r| t.set_switch(s, r.is_active()));
        }
        on!(anim, connect_selected_notify, |t, c| t.set_anim(c.selected() as usize));
        on!(apply, connect_clicked, |t, b| t.apply_screen(b));
        on!(reset, connect_clicked, |t, _| t.reset_screen());
        on!(demo_on, connect_clicked, |t, _| t.demo(true));
        on!(demo_off, connect_clicked, |t, _| t.demo(false));
        on!(add, connect_clicked, |t, _| t.add_rule());
        this
    }

    fn load(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        *self.read_for.borrow_mut() = Some(serial.clone());
        let w = self.me.clone();
        util::run_bg(
            move || (adb.dev_state(&serial), adb.port_rules(&serial)),
            move |(state, rules)| {
                let Some(t) = w.upgrade() else { return };
                match state {
                    Ok(s) => t.fill(s),
                    Err(e) => t.ctx.toast(&t!("Could not read the developer settings: {}", e)),
                }
                t.fill_rules(rules);
            },
        );
    }

    fn fill(&self, s: DevState) {
        self.filling.set(true);
        for (sw, row) in &self.switches {
            row.set_active(s.switch(*sw));
        }
        if let Some(i) = s.animation_scale.as_deref().and_then(|a| ANIMATION_SCALES.iter().position(|x| *x == a)) {
            self.anim.set_selected(i as u32);
        }
        let size = s.override_size.or(s.physical_size);
        let dpi = s.override_density.or(s.physical_density);
        if let Some((w, h)) = size {
            self.width.set_value(w as f64);
            self.height.set_value(h as f64);
        }
        if let Some(d) = dpi {
            self.density.set_value(d as f64);
        }
        let fmt = |v: Option<(u32, u32)>| v.map(|(w, h)| format!("{w} × {h}")).unwrap_or_else(|| "—".into());
        let changed = s.override_size.is_some() || s.override_density.is_some();
        self.screen_info.set_subtitle(&t!(
            "{} at {} dpi{}",
            fmt(size),
            dpi.map(|d| d.to_string()).unwrap_or_else(|| "—".into()),
            if changed { t!(" (original: {} at {} dpi)", fmt(s.physical_size), s.physical_density.map(|d| d.to_string()).unwrap_or_else(|| "—".into())) } else { String::new() }
        ));
        *self.state.borrow_mut() = Some(s);
        self.filling.set(false);
    }

    fn fill_rules(&self, rules: Vec<PortRule>) {
        for r in self.rule_rows.borrow_mut().drain(..) {
            self.rules.remove(&r);
        }
        let mut rows = Vec::new();
        if rules.is_empty() {
            let r = adw::ActionRow::builder().title(tr("No rules")).build();
            self.rules.add(&r);
            rows.push(r);
        }
        for rule in rules {
            let (title, dir) = if rule.reverse {
                (t!("Phone {} → PC {}", rule.local, rule.remote), "adb reverse")
            } else {
                (t!("PC {} → phone {}", rule.local, rule.remote), "adb forward")
            };
            let r = adw::ActionRow::builder().title(title).subtitle(dir).build();
            let del = util::icon_button("adb-edit-delete-symbolic", tr("Remove the rule"));
            del.add_css_class("flat");
            del.set_valign(gtk::Align::Center);
            let w = self.me.clone();
            del.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    let (rule, w2) = (rule.clone(), t.me.clone());
                    t.ctx.device_op("port_rule_remove", &rule.local.clone(), tr("Rule removed"), move |adb, s| adb.remove_port_rule(&s, &rule).map(|r| r.summary()), move |_| {
                        if let Some(t) = w2.upgrade() {
                            t.load();
                        }
                    });
                }
            });
            r.add_suffix(&del);
            self.rules.add(&r);
            rows.push(r);
        }
        *self.rule_rows.borrow_mut() = rows;
    }

    fn set_switch(&self, s: Switch, on: bool) {
        if self.filling.get() {
            return;
        }
        let w = self.me.clone();
        self.ctx.device_op("dev_switch", s.title(), if on { tr("Turned on") } else { tr("Turned off") }, move |adb, serial| adb.dev_switch(&serial, s, on).map(|r| r.summary()), move |ok| {
            if !ok {
                if let Some(t) = w.upgrade() {
                    t.load();
                }
            }
        });
    }

    fn set_anim(&self, i: usize) {
        if self.filling.get() {
            return;
        }
        let Some(scale) = ANIMATION_SCALES.get(i).copied() else { return };
        self.ctx.device_op("animation_scale", scale, &t!("Animation speed: {}", scale_label(scale)), move |adb, s| adb.set_animation_scale(&s, scale).map(|r| r.summary()), |_| {});
    }

    fn apply_screen(&self, button: &gtk::Button) {
        let Some(state) = self.state.borrow().clone() else { return };
        let size = (self.width.value() as u32, self.height.value() as u32);
        let dpi = self.density.value() as u32;
        let current_size = state.override_size.or(state.physical_size);
        let current_dpi = state.override_density.or(state.physical_density);
        let new_size = (Some(size) != current_size).then_some(size);
        let new_dpi = (Some(dpi) != current_dpi).then_some(dpi);
        if new_size.is_none() && new_dpi.is_none() {
            self.ctx.toast(tr("Nothing changed"));
            return;
        }
        // What to go back to: the previous override, or the original.
        let (old_size, old_dpi) = (state.override_size, state.override_density);
        let (w, parent) = (self.me.clone(), button.clone());
        self.ctx.device_op(
            "display_set",
            &format!("{}x{} {}dpi", size.0, size.1, dpi),
            tr("Screen changed"),
            move |adb, s| {
                if let Some(sz) = new_size {
                    adb.set_display_size(&s, Some(sz))?;
                }
                if let Some(d) = new_dpi {
                    adb.set_density(&s, Some(d))?;
                }
                Ok(String::new())
            },
            move |ok| {
                let Some(t) = w.upgrade() else { return };
                t.load();
                if ok {
                    t.confirm_screen(&parent, new_size.map(|_| old_size), new_dpi.map(|_| old_dpi));
                }
            },
        );
    }

    /// Asks to keep the new screen; without an answer in [`REVERT_SECS`],
    /// it goes back. `None` = that value was not changed.
    fn confirm_screen(&self, parent: &gtk::Button, old_size: Option<Option<(u32, u32)>>, old_dpi: Option<Option<u32>>) {
        let dialog = adw::AlertDialog::new(Some(tr("Keep this screen?")), Some(&t!("It will be undone in {} seconds.", REVERT_SECS)));
        dialog.add_responses(&[("revert", tr("Undo")), ("keep", tr("Keep"))]);
        dialog.set_response_appearance("keep", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("keep"));
        dialog.set_close_response("revert");
        let answered = Rc::new(Cell::new(false));
        let revert = {
            let w = self.me.clone();
            move || {
                let Some(t) = w.upgrade() else { return };
                let w2 = t.me.clone();
                t.ctx.device_op(
                    "display_revert",
                    "",
                    tr("Screen restored"),
                    move |adb, s| {
                        if let Some(sz) = old_size {
                            adb.set_display_size(&s, sz)?;
                        }
                        if let Some(d) = old_dpi {
                            adb.set_density(&s, d)?;
                        }
                        Ok(String::new())
                    },
                    move |_| {
                        if let Some(t) = w2.upgrade() {
                            t.load();
                        }
                    },
                );
            }
        };
        let revert = Rc::new(revert);
        {
            let (answered, revert) = (answered.clone(), revert.clone());
            dialog.connect_response(None, move |_, resp| {
                if answered.replace(true) {
                    return;
                }
                if resp != "keep" {
                    revert();
                }
            });
        }
        let left = Rc::new(Cell::new(REVERT_SECS));
        let d2 = dialog.clone();
        glib::timeout_add_seconds_local(1, move || {
            if answered.get() {
                return glib::ControlFlow::Break;
            }
            let n = left.get().saturating_sub(1);
            left.set(n);
            if n == 0 {
                answered.set(true);
                d2.close();
                revert();
                return glib::ControlFlow::Break;
            }
            d2.set_body(&t!("It will be undone in {} seconds.", n));
            glib::ControlFlow::Continue
        });
        dialog.present(Some(parent));
    }

    fn reset_screen(&self) {
        let w = self.me.clone();
        self.ctx.device_op(
            "display_reset",
            "",
            tr("Screen restored"),
            |adb, s| {
                adb.set_display_size(&s, None)?;
                adb.set_density(&s, None)?;
                Ok(String::new())
            },
            move |_| {
                if let Some(t) = w.upgrade() {
                    t.load();
                }
            },
        );
    }

    fn demo(&self, on: bool) {
        self.ctx.device_op("demo_mode", if on { "on" } else { "off" }, if on { tr("Demo mode on") } else { tr("Demo mode off") }, move |adb, s| adb.demo_mode(&s, on).map(|r| r.summary()), |_| {});
    }

    fn add_rule(&self) {
        let reverse = self.direction.selected() == 0;
        let (phone, pc) = (self.port_phone.value() as u32, self.port_pc.value() as u32);
        // `reverse`: local = the phone's port, remote = the PC's. `forward`:
        // local = the PC's, remote = the phone's.
        let (local, remote) = if reverse { (phone, pc) } else { (pc, phone) };
        let w = self.me.clone();
        self.ctx.device_op("port_rule_add", &format!("{local}:{remote}"), tr("Rule added"), move |adb, s| adb.add_port_rule(&s, reverse, local, remote).map(|r| r.summary()), move |_| {
            if let Some(t) = w.upgrade() {
                t.load();
            }
        });
    }
}

impl Page for DeveloperPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn shown(&self) {
        if self.state.borrow().is_none() || *self.read_for.borrow() != self.ctx.ready_serial() {
            self.load();
        }
    }
    fn on_event(&self, ev: &Event) {
        if matches!(ev, Event::Devices) && self.root.is_mapped() && *self.read_for.borrow() != self.ctx.ready_serial() {
            self.load();
        }
    }
}
