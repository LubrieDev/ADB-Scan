//! Behaviour of the sidebar list: each section can be folded (click its
//! title), any page can be pinned to the top (right click or long press),
//! and the search box filters the pages. What is folded and pinned is saved.
//!
//! Rows are found by their page id (the row's widget name), never by
//! position: pinned pages move to the top.

use adbscan_core::i18n::tr;
use adw::prelude::*;
use std::rc::{Rc, Weak};

use crate::ctx::Ctx;
use crate::util;
use crate::window::PageDef;

struct Entry {
    id: &'static str,
    title: &'static str,
    section: &'static str,
    /// Saved key of the section: the id of its first page (stable across
    /// languages).
    group: &'static str,
}

pub struct Sidebar {
    me: Weak<Self>,
    ctx: Rc<Ctx>,
    pub list: gtk::ListBox,
    search: gtk::SearchEntry,
    entries: Vec<Entry>,
}

impl Sidebar {
    /// Takes over `list` (with one row per page, named after its id) and the
    /// search box.
    pub fn new(ctx: &Rc<Ctx>, list: &gtk::ListBox, search: &gtk::SearchEntry, defs: &[PageDef]) -> Rc<Self> {
        let entries = defs
            .iter()
            .map(|d| Entry { id: d.id, title: d.title, section: d.section, group: defs.iter().find(|o| o.section == d.section).map(|o| o.id).unwrap_or(d.id) })
            .collect();
        let this = Rc::new_cyclic(|me| Self { me: me.clone(), ctx: ctx.clone(), list: list.clone(), search: search.clone(), entries });
        {
            let w = this.me.clone();
            list.set_filter_func(move |row| w.upgrade().map_or(true, |s| s.matches(row)));
        }
        {
            let w = this.me.clone();
            list.set_sort_func(move |a, b| match w.upgrade() {
                Some(s) => s.order(a).cmp(&s.order(b)).into(),
                None => gtk::Ordering::Equal,
            });
        }
        {
            let w = this.me.clone();
            list.set_header_func(move |row, before| {
                if let Some(s) = w.upgrade() {
                    s.header(row, before);
                }
            });
        }
        {
            let w = this.me.clone();
            search.connect_search_changed(move |_| {
                if let Some(s) = w.upgrade() {
                    s.list.invalidate_filter();
                    s.list.invalidate_headers();
                    s.refresh();
                }
            });
        }
        let mut child = list.first_child();
        while let Some(c) = child {
            if let Some(row) = c.downcast_ref::<gtk::ListBoxRow>() {
                this.add_pin_menu(row);
            }
            child = c.next_sibling();
        }
        this.refresh();
        this
    }

    fn entry(&self, row: &gtk::ListBoxRow) -> Option<&Entry> {
        let name = row.widget_name();
        self.entries.iter().find(|e| e.id == name.as_str())
    }

    /// Page id of a row.
    pub fn id_of(&self, row: &gtk::ListBoxRow) -> Option<&'static str> {
        self.entry(row).map(|e| e.id)
    }

    /// Row of a page.
    pub fn row(&self, id: &str) -> Option<gtk::ListBoxRow> {
        let mut child = self.list.first_child();
        while let Some(c) = child {
            if let Some(row) = c.downcast_ref::<gtk::ListBoxRow>() {
                if row.widget_name() == id {
                    return Some(row.clone());
                }
            }
            child = c.next_sibling();
        }
        None
    }

    /// First page left by the search, in the order shown.
    pub fn first_visible(&self) -> Option<&'static str> {
        (0..).map_while(|i| self.list.row_at_index(i)).find(|r| r.is_child_visible()).and_then(|r| self.id_of(&r))
    }

    fn searching(&self) -> bool {
        !self.search.text().trim().is_empty()
    }

    fn pinned(&self, id: &str) -> Option<usize> {
        self.ctx.settings.borrow().sidebar_pinned.iter().position(|p| p == id)
    }

    fn folded(&self, group: &str) -> bool {
        self.ctx.settings.borrow().sidebar_folded.iter().any(|g| g == group)
    }

    fn matches(&self, row: &gtk::ListBoxRow) -> bool {
        let query = self.search.text();
        self.entry(row).map_or(true, |e| crate::search::page_matches(e.id, e.title, &query))
    }

    /// Pinned pages first (in the order they were pinned), then the rest in
    /// their usual order.
    fn order(&self, row: &gtk::ListBoxRow) -> (u8, usize) {
        let Some(e) = self.entry(row) else { return (2, 0) };
        match self.pinned(e.id) {
            Some(i) => (0, i),
            None => (1, self.entries.iter().position(|o| o.id == e.id).unwrap_or(0)),
        }
    }

    /// Group a row is shown in: “pinned” or its section.
    fn group_of(&self, row: &gtk::ListBoxRow) -> Option<(&'static str, &'static str)> {
        let e = self.entry(row)?;
        Some(if self.pinned(e.id).is_some() { ("pinned", tr("PINNED")) } else { (e.group, e.section) })
    }

    fn header(&self, row: &gtk::ListBoxRow, before: Option<&gtk::ListBoxRow>) {
        let Some((group, label)) = self.group_of(row) else { return };
        if before.and_then(|b| self.group_of(b)).map(|g| g.0) == Some(group) {
            row.set_header(gtk::Widget::NONE);
            return;
        }
        let text = gtk::Label::new(Some(label));
        text.add_css_class("sidebar-section");
        text.set_xalign(0.0);
        if group == "pinned" {
            row.set_header(Some(&text));
            return;
        }
        // A section title folds and unfolds the section.
        let folded = self.folded(group) && !self.searching();
        let chevron = util::icon("adb-chevron-down-symbolic");
        chevron.add_css_class("sidebar-chevron");
        if folded {
            chevron.add_css_class("folded");
        }
        let content = util::hbox(0);
        text.set_hexpand(true);
        content.append(&text);
        content.append(&chevron);
        let button = gtk::Button::builder().child(&content).tooltip_text(if folded { tr("Unfold the section") } else { tr("Fold the section · right-click a page to pin it to the top") }).build();
        button.add_css_class("flat");
        button.add_css_class("sidebar-section-toggle");
        let w = self.me.clone();
        button.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                s.toggle_fold(group);
            }
        });
        row.set_header(Some(&button));
    }

    fn toggle_fold(&self, group: &'static str) {
        {
            let mut s = self.ctx.settings.borrow_mut();
            if let Some(i) = s.sidebar_folded.iter().position(|g| g == group) {
                s.sidebar_folded.remove(i);
            } else {
                s.sidebar_folded.push(group.to_string());
            }
        }
        self.ctx.save_settings();
        self.list.invalidate_headers();
        self.refresh();
    }

    fn toggle_pin(&self, id: &'static str) {
        {
            let mut s = self.ctx.settings.borrow_mut();
            if let Some(i) = s.sidebar_pinned.iter().position(|p| p == id) {
                s.sidebar_pinned.remove(i);
            } else {
                s.sidebar_pinned.push(id.to_string());
            }
        }
        self.ctx.save_settings();
        self.list.invalidate_sort();
        self.list.invalidate_headers();
        self.refresh();
    }

    /// Right click or long press on a page: pin it to the top or unpin it.
    fn add_pin_menu(&self, row: &gtk::ListBoxRow) {
        let open = {
            let w = self.me.clone();
            let row = row.clone();
            move |x: f64, y: f64| {
                let Some(s) = w.upgrade() else { return };
                let Some(id) = s.id_of(&row) else { return };
                let pinned = s.pinned(id).is_some();
                let button = gtk::Button::with_label(if pinned { tr("Unpin") } else { tr("Pin to the top") });
                button.add_css_class("flat");
                let popover = gtk::Popover::builder().child(&button).has_arrow(true).build();
                popover.set_parent(&row);
                popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                popover.connect_closed(|p| p.unparent());
                let (p2, w2) = (popover.clone(), w.clone());
                button.connect_clicked(move |_| {
                    p2.popdown();
                    if let Some(s) = w2.upgrade() {
                        s.toggle_pin(id);
                    }
                });
                popover.popup();
            }
        };
        let click = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_SECONDARY).build();
        {
            let open = open.clone();
            click.connect_pressed(move |_, _, x, y| open(x, y));
        }
        row.add_controller(click);
        let press = gtk::GestureLongPress::new();
        press.connect_pressed(move |_, x, y| open(x, y));
        row.add_controller(press);
    }

    /// Folds the rows of the folded sections. The open page stays in view,
    /// and a search shows every match.
    pub fn refresh(&self) {
        let searching = self.searching();
        let mut child = self.list.first_child();
        while let Some(c) = child {
            if let Some(row) = c.downcast_ref::<gtk::ListBoxRow>() {
                if let Some(e) = self.entry(row) {
                    let fold = !searching && self.pinned(e.id).is_none() && self.folded(e.group) && !row.is_selected();
                    if let Some(content) = row.child() {
                        content.set_visible(!fold);
                    }
                    if fold {
                        row.add_css_class("folded");
                    } else {
                        row.remove_css_class("folded");
                    }
                    row.set_can_focus(!fold);
                }
            }
            child = c.next_sibling();
        }
    }
}
