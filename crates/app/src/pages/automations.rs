//! Automations tab (in Control): actions that run by themselves when a phone
//! connects (see `adbscan_core::automation`; `Ctx::run_automations` runs
//! them).

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

use adbscan_core::automation::{AutoAction, Automation};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

struct AutomationsPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    list: adw::PreferencesGroup,
    rows: RefCell<Vec<adw::ActionRow>>,
    action: adw::ComboRow,
    phone: adw::ComboRow,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    AutomationsPage::new(ctx)
}

/// Hardware serial and model of the selected phone.
fn this_phone(ctx: &Ctx) -> Option<(String, String)> {
    let s = ctx.snapshot()?;
    let hw = s.prop("ro.serialno").to_string();
    Some((if hw.is_empty() { s.serial.clone() } else { hw }, s.model()))
}

impl AutomationsPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);
        let list = adw::PreferencesGroup::builder()
            .title(tr("When a phone connects"))
            .description(t!(
                "ADB Scan runs these actions by itself when the phone connects (by cable or Wi-Fi) or when it starts with the phone connected, while it is open or in the tray. Each one runs at most once every {} minutes per phone, and every run is recorded in History.",
                adbscan_core::automation::COOLDOWN_SECS as u32 / 60
            ))
            .build();
        content.append(&list);

        let add_group = adw::PreferencesGroup::builder().title(tr("New automation")).build();
        let labels: Vec<&str> = AutoAction::ALL.iter().map(|a| a.label()).collect();
        let action = adw::ComboRow::builder().title(tr("Action")).model(&gtk::StringList::new(&labels)).build();
        let phone = adw::ComboRow::builder().title(tr("Phone")).build();
        add_group.add(&action);
        add_group.add(&phone);
        let add = gtk::Button::with_label(tr("Add"));
        add.add_css_class("pill");
        add.add_css_class("suggested-action");
        add.set_halign(gtk::Align::End);
        let note = util::label(tr("Copying photos uses the folder chosen in Files → Copy photos; choose it there once."), &["dim", "caption"]);
        note.set_wrap(true);
        content.append(&add_group);
        content.append(&note);
        content.append(&add);

        let root = util::page_scroller(&content, 900);
        let this = Rc::new_cyclic(|me| Self { me: me.clone(), ctx: ctx.clone(), root, list, rows: RefCell::new(Vec::new()), action, phone });
        let w = this.me.clone();
        add.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.add();
            }
        });
        this.fill_phone_choice();
        this.render();
        this
    }

    fn fill_phone_choice(&self) {
        let mut items = vec![tr("Any phone").to_string()];
        if let Some((_, model)) = this_phone(&self.ctx) {
            items.push(t!("Only this phone ({})", model));
        }
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let selected = self.phone.selected().min(refs.len() as u32 - 1);
        self.phone.set_model(Some(&gtk::StringList::new(&refs)));
        self.phone.set_selected(selected);
    }

    fn render(&self) {
        for r in self.rows.borrow_mut().drain(..) {
            self.list.remove(&r);
        }
        let list = self.ctx.settings.borrow().automations.clone();
        let mut rows = Vec::new();
        if list.is_empty() {
            let r = adw::ActionRow::builder().title(tr("No automations yet")).subtitle(tr("Add one below.")).build();
            self.list.add(&r);
            rows.push(r);
        }
        for a in list {
            let who = if a.device.is_empty() { tr("Any phone").to_string() } else { t!("Only {}", self.ctx.shown_device_name(&a.device_name)) };
            let row = adw::ActionRow::builder().title(a.action.label()).subtitle(glib::markup_escape_text(&who)).build();
            let sw = gtk::Switch::builder().active(a.enabled).valign(gtk::Align::Center).tooltip_text(tr("Enabled")).build();
            row.add_prefix(&sw);
            let id = a.id;
            let w = self.me.clone();
            sw.connect_active_notify(move |s| {
                if let Some(t) = w.upgrade() {
                    if let Some(x) = t.ctx.settings.borrow_mut().automations.iter_mut().find(|x| x.id == id) {
                        x.enabled = s.is_active();
                    }
                    t.ctx.save_settings();
                }
            });
            let del = util::icon_button("adb-edit-delete-symbolic", tr("Delete"));
            del.add_css_class("flat");
            del.set_valign(gtk::Align::Center);
            let w = self.me.clone();
            del.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    t.ctx.settings.borrow_mut().automations.retain(|x| x.id != id);
                    t.ctx.save_settings();
                    t.render();
                }
            });
            row.add_suffix(&del);
            self.list.add(&row);
            rows.push(row);
        }
        *self.rows.borrow_mut() = rows;
    }

    fn add(&self) {
        let Some(action) = AutoAction::ALL.get(self.action.selected() as usize).copied() else { return };
        let (device, device_name) = match (self.phone.selected(), this_phone(&self.ctx)) {
            (1, Some((hw, model))) => (hw, model),
            _ => (String::new(), String::new()),
        };
        {
            let mut s = self.ctx.settings.borrow_mut();
            if s.automations.iter().any(|a| a.action == action && a.device == device) {
                drop(s);
                self.ctx.toast(tr("That automation already exists"));
                return;
            }
            let id = s.automations.iter().map(|a| a.id).max().unwrap_or(0) + 1;
            s.automations.push(Automation { id, enabled: true, action, device, device_name });
        }
        self.ctx.save_settings();
        self.ctx.toast(tr("Automation added: it runs the next time the phone connects"));
        self.render();
    }
}

impl Page for AutomationsPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn shown(&self) {
        self.fill_phone_choice();
    }
    fn on_event(&self, ev: &Event) {
        match ev {
            Event::Devices => self.fill_phone_choice(),
            Event::Privacy => self.render(),
            _ => {}
        }
    }
}
