//! Control: the phone's quick settings, the remote (media, calls, links,
//! finding the phone) and the automations on connect, in one page with three
//! tabs. Each tab is its own page (`controls`, `remote`, `automations`).

use adbscan_core::i18n::tr;
use adw::prelude::*;
use std::rc::Rc;

use crate::ctx::{Ctx, Event};
use crate::window::Page;

struct ControlPage {
    root: gtk::Box,
    stack: adw::ViewStack,
    tabs: Vec<(&'static str, Rc<dyn Page>)>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    let stack = adw::ViewStack::new();
    let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();
    switcher.set_halign(gtk::Align::Center);
    switcher.set_margin_top(10);
    let tabs: Vec<(&'static str, Rc<dyn Page>)> = vec![("controls", super::controls::build(ctx)), ("remote", super::remote::build(ctx)), ("automations", super::automations::build(ctx))];
    stack.add_titled_with_icon(&tabs[0].1.widget(), Some("controls"), tr("Quick settings"), "adb-sliders-horizontal-symbolic");
    stack.add_titled_with_icon(&tabs[1].1.widget(), Some("remote"), tr("Remote"), "adb-monitor-smartphone-symbolic");
    stack.add_titled_with_icon(&tabs[2].1.widget(), Some("automations"), tr("Automations"), "adb-alarm-symbolic");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&switcher);
    root.append(&stack);
    stack.set_vexpand(true);
    let page = Rc::new(ControlPage { root, stack: stack.clone(), tabs });
    // The tab that comes into view catches up, as a page does when shown.
    let weak = Rc::downgrade(&page);
    stack.connect_visible_child_name_notify(move |_| {
        if let Some(p) = weak.upgrade() {
            p.shown();
        }
    });
    page
}

impl ControlPage {
    fn visible(&self) -> Option<&Rc<dyn Page>> {
        let name = self.stack.visible_child_name()?;
        self.tabs.iter().find(|(id, _)| *id == name.as_str()).map(|(_, p)| p)
    }
}

impl Page for ControlPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn on_event(&self, ev: &Event) {
        // Telemetry is their clock: only the tab in view reads the phone.
        if matches!(ev, Event::Snapshot) {
            if let Some(p) = self.visible() {
                p.on_event(ev);
            }
            return;
        }
        for (_, p) in &self.tabs {
            p.on_event(ev);
        }
    }

    fn shown(&self) {
        if let Some(p) = self.visible() {
            p.shown();
        }
    }
}
