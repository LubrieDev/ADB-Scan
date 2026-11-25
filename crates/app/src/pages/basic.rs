//! Basic analysis: what is known about the phone **without** developer options
//! or USB or wireless debugging (see [`adbscan_core::basic`]). It does not use
//! ADB: USB descriptors, MTP/PTP if the phone is in “File transfer” mode, and
//! the local history.
//!
//! The MTP read does not compete with the file manager: it only happens if the
//! interface is free, and the automatic one waits a few seconds after the phone
//! appears so that Linux Mint can mount it first. If it is mounted, the storage
//! is read through GVfs.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use adbscan_core::basic::{self, Fact, Group, Inputs, MtpState, Status};
use adbscan_core::db::KnownDevice;
use adbscan_core::models::now_secs;
use adbscan_core::mtp::{self, GvfsStorage, MtpError};
use adbscan_core::usb::{self, UsbDevice, UsbMode};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

/// Wait from when the phone appears until the automatic read: the file manager
/// mounts it in ~1 s and the interface must not be fought over.
const AUTO_DELAY: f64 = 4.0;

/// What was read from a phone (keyed by [`probe_key`]).
struct Probed {
    mtp: MtpState,
    gvfs: Option<Vec<GvfsStorage>>,
    known: Option<KnownDevice>,
    at: f64,
}

pub struct Basic {
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    invite: gtk::Widget,
    advanced_note: adw::ActionRow,
    body: gtk::Box,
    refresh: gtk::Button,
    probed: RefCell<HashMap<String, Probed>>,
    /// When each phone was first seen (for the wait).
    first_seen: RefCell<HashMap<String, f64>>,
    busy: Cell<u32>,
    last_render: RefCell<String>,
    /// For the `Page` methods, which take `&self`.
    me: RefCell<std::rc::Weak<Basic>>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    Basic::new(ctx)
}

/// Changes when replugging or switching USB mode (another product, another
/// device number): then it has to be read again.
fn probe_key(u: &UsbDevice) -> String {
    format!("{}#{}#{}", u.key(), u.product_id, u.devnum)
}

/// “Want a more complete analysis?” card. Optional: “Continue with the basic
/// analysis” hides it without blocking anything.
pub fn invite_card(ctx: &Rc<Ctx>) -> gtk::Widget {
    let card = util::vbox(8);
    card.add_css_class("card");
    card.add_css_class("advanced-invite");
    let head = util::hbox(10);
    let icon = util::icon("adb-key-round-symbolic");
    icon.add_css_class("accent");
    head.append(&icon);
    head.append(&util::label(tr("Want a more complete analysis?"), &["heading"]));
    card.append(&head);
    let body = util::label(
        tr("Turn on USB debugging to let ADB Scan access additional information about your device. This option is completely voluntary."),
        &[],
    );
    body.set_wrap(true);
    body.set_xalign(0.0);
    card.append(&body);
    let cont = util::label(tr("You can continue with the basic analysis without turning it on."), &["dim"]);
    cont.set_wrap(true);
    cont.set_xalign(0.0);
    card.append(&cont);
    // They wrap onto several lines if the window is narrow.
    let actions = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).column_spacing(8).row_spacing(8).max_children_per_line(3).homogeneous(false).build();
    actions.set_halign(gtk::Align::Start);
    actions.set_margin_top(4);
    let keep = gtk::Button::with_label(tr("Continue with the basic analysis"));
    keep.add_css_class("pill");
    keep.add_css_class("suggested-action");
    let how = gtk::Button::with_label(tr("How to turn it on"));
    how.add_css_class("pill");
    // Cable-free alternative: wireless debugging with a QR code or a pairing
    // code.
    let wifi = gtk::Button::with_label(tr("Without a cable (QR or code)"));
    wifi.add_css_class("pill");
    wifi.set_action_name(Some("win.connect"));
    wifi.set_action_target_value(Some(&"wireless".to_variant()));
    for b in [&keep, &how, &wifi] {
        actions.append(b);
    }
    card.append(&actions);
    {
        let ctx = ctx.clone();
        keep.connect_clicked(move |_| ctx.dismiss_advanced_invite());
    }
    how.connect_clicked(show_how_to);
    card.upcast()
}

/// Steps to turn on debugging. Instructions only: ADB Scan never turns it on or
/// touches the developer options.
pub fn show_how_to(parent: &impl IsA<gtk::Widget>) {
    let d = adw::AlertDialog::builder()
        .heading(tr("Turn on USB debugging (optional)"))
        .body(
            tr("1. On the phone, open Settings → About phone and tap “Build number” 7 times. This reveals the Developer options.\n2. In Settings → System → Developer options, turn on “USB debugging”.\n3. With the cable connected, accept “Allow USB debugging?” on the phone.\n\nMenu names vary by manufacturer. ADB Scan does not turn anything on for you, and you can turn it off whenever you want from the same place.\n\nWithout a cable (Android 11+): turn on “Wireless debugging” in the same options and, in ADB Scan, open “Ways to connect” (device selector) → Wireless debugging. You can pair by scanning a QR code or by typing the IP address, the port and the pairing code."),
        )
        .build();
    d.add_response("ok", tr("Got it"));
    d.set_default_response(Some("ok"));
    d.present(Some(parent));
}

fn status_icon(s: Status) -> (&'static str, &'static str, &'static str) {
    match s {
        Status::Available => ("adb-emblem-ok-symbolic", "severity-ok", tr("Available")),
        Status::Unavailable => ("adb-circle-x-symbolic", "dim", tr("Not available")),
        Status::NeedsAdb => ("adb-key-round-symbolic", "severity-low", tr("Requires USB debugging")),
        Status::Pending => ("adb-refresh-cw-symbolic", "dim", tr("Reading")),
    }
}

fn esc(t: &str) -> String {
    glib::markup_escape_text(t).to_string()
}

fn fact_row(ctx: &Ctx, f: &Fact) -> adw::ActionRow {
    let (row, label) = util::info_row(&esc(&f.label));
    let text = if f.private && f.status == Status::Available { ctx.shown_value(&f.value) } else { ctx.shown(&f.value) };
    label.set_text(&text);
    if f.status != Status::Available {
        label.add_css_class("probe-missing");
    }
    let (icon, class, what) = status_icon(f.status);
    let img = util::icon(icon);
    img.add_css_class(class);
    img.set_tooltip_text(Some(what));
    row.add_prefix(&img);
    row.set_tooltip_text(Some(&t!("{} · Source: {}", what, tr(f.source))));
    row
}

fn group_widget(ctx: &Ctx, g: &Group) -> adw::PreferencesGroup {
    let w = adw::PreferencesGroup::builder().title(esc(tr(g.title))).description(esc(tr(g.description))).build();
    for f in &g.facts {
        w.add(&fact_row(ctx, f));
    }
    w
}

fn legend() -> gtk::Box {
    let b = util::hbox(14);
    b.set_margin_bottom(2);
    for s in [Status::Available, Status::Unavailable, Status::NeedsAdb] {
        let (icon, class, what) = status_icon(s);
        let item = util::hbox(6);
        let img = util::icon(icon);
        img.add_css_class(class);
        item.append(&img);
        item.append(&util::label(what, &["dim", "caption"]));
        b.append(&item);
    }
    b
}

impl Basic {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);

        let head = util::hbox(12);
        let titles = util::vbox(4);
        titles.set_hexpand(true);
        titles.append(&util::label(tr("Basic analysis"), &["title-2"]));
        let sub = util::label(
            tr("What ADB Scan can learn about your phone without developer options or USB or wireless debugging. Read-only: it changes nothing on the phone and does not read your files."),
            &["dim"],
        );
        sub.set_wrap(true);
        sub.set_xalign(0.0);
        titles.append(&sub);
        head.append(&titles);
        let ways = util::icon_button("adb-link-symbolic", tr("Ways to connect: basic, USB or wireless"));
        ways.set_valign(gtk::Align::Start);
        ways.set_action_name(Some("win.connect"));
        ways.set_action_target_value(Some(&"root".to_variant()));
        head.append(&ways);
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Read again"));
        refresh.set_valign(gtk::Align::Start);
        head.append(&refresh);
        content.append(&head);
        content.append(&legend());

        let invite = invite_card(ctx);
        content.append(&invite);

        // With ADB authorised: the advanced analysis is already active.
        let advanced_note = adw::ActionRow::builder().title(tr("Advanced analysis active")).subtitle(tr("USB debugging is authorised: the other sections show the full analysis.")).build();
        let ok = util::icon("adb-emblem-ok-symbolic");
        ok.add_css_class("severity-ok");
        advanced_note.add_prefix(&ok);
        let go = gtk::Button::with_label(tr("Go to Overview"));
        go.add_css_class("flat");
        go.set_valign(gtk::Align::Center);
        go.set_action_name(Some("win.go"));
        go.set_action_target_value(Some(&"dashboard".to_variant()));
        advanced_note.add_suffix(&go);
        let note_list = gtk::ListBox::new();
        note_list.add_css_class("boxed-list");
        note_list.set_selection_mode(gtk::SelectionMode::None);
        note_list.append(&advanced_note);
        content.append(&note_list);

        let body = util::vbox(18);
        content.append(&body);

        let root = util::page_scroller(&content, 1000);
        let this = Rc::new(Self {
            ctx: ctx.clone(),
            root,
            invite,
            advanced_note,
            body,
            refresh: refresh.clone(),
            probed: Default::default(),
            first_seen: Default::default(),
            busy: Cell::new(0),
            last_render: Default::default(),
            me: Default::default(),
        });
        *this.me.borrow_mut() = Rc::downgrade(&this);
        {
            // When it comes into view (also after being built), read whatever is
            // missing.
            let weak = Rc::downgrade(&this);
            this.root.connect_map(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.sync();
                }
            });
        }
        {
            let weak = Rc::downgrade(&this);
            refresh.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.probe_all(true);
                }
            });
        }
        this.sync();
        this
    }

    fn phones(&self) -> Vec<UsbDevice> {
        // Fastboot and EDL are not a running Android: there is nothing to read.
        self.ctx.usb_devices().into_iter().filter(|u| matches!(u.mode, UsbMode::Adb | UsbMode::Other)).collect()
    }

    /// Updates the list of phones, starts the missing reads and repaints if
    /// something changed.
    fn sync(self: &Rc<Self>) {
        let phones = self.phones();
        let keys: Vec<String> = phones.iter().map(probe_key).collect();
        self.probed.borrow_mut().retain(|k, _| keys.contains(k));
        {
            let now = now_secs();
            let mut seen = self.first_seen.borrow_mut();
            seen.retain(|k, _| keys.contains(k));
            for k in &keys {
                seen.entry(k.clone()).or_insert(now);
            }
        }
        // The phone is only read while the page is in view.
        if self.root.is_mapped() {
            self.probe_all(false);
        }
        self.render();
    }

    /// Reads the phones that have not been read (or all of them with `force`).
    /// The automatic read waits [`AUTO_DELAY`] after each phone appeared.
    fn probe_all(self: &Rc<Self>, force: bool) {
        for u in self.phones() {
            let key = probe_key(&u);
            if !force && self.probed.borrow().contains_key(&key) {
                continue;
            }
            let wait = if force { 0.0 } else { (AUTO_DELAY - (now_secs() - self.first_seen.borrow().get(&key).copied().unwrap_or(0.0))).max(0.0) };
            self.probed.borrow_mut().insert(key.clone(), Probed { mtp: MtpState::Pending, gvfs: None, known: None, at: now_secs() });
            let weak = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_secs_f64(wait), move || {
                if let Some(this) = weak.upgrade() {
                    this.probe(key, u);
                }
            });
        }
        self.render();
    }

    fn probe(self: &Rc<Self>, key: String, dev: UsbDevice) {
        // It is gone (unplugged or changed mode during the wait).
        if !self.phones().iter().any(|u| probe_key(u) == key) {
            self.probed.borrow_mut().remove(&key);
            return;
        }
        self.busy.set(self.busy.get() + 1);
        crate::anim::set_busy(&self.refresh, true);
        let history = self.ctx.history.clone();
        let weak = Rc::downgrade(self);
        util::run_bg(
            move || {
                let r = mtp::probe(&dev);
                let gvfs = if matches!(r, Err(MtpError::Busy)) { mtp::gvfs_storages(&dev) } else { None };
                let known = history.and_then(|h| h.known_device(&dev.serial).ok().flatten());
                (r, gvfs, known)
            },
            move |(r, gvfs, known)| {
                let Some(this) = weak.upgrade() else { return };
                this.busy.set(this.busy.get().saturating_sub(1));
                if this.busy.get() == 0 {
                    crate::anim::set_busy(&this.refresh, false);
                }
                if let Err(e) = &r {
                    log::info!("basic analysis: MTP not available ({e:?})");
                }
                if let Some(p) = this.probed.borrow_mut().get_mut(&key) {
                    *p = Probed { mtp: MtpState::Read(r), gvfs, known, at: now_secs() };
                }
                this.render();
            },
        );
    }

    fn render(&self) {
        let ctx = &self.ctx;
        let ready = ctx.ready_serial().is_some();
        self.invite.set_visible(ctx.wants_advanced_invite());
        self.advanced_note.set_visible(ready);
        if let Some(list) = self.advanced_note.parent() {
            list.set_visible(ready);
        }
        let phones = self.phones();
        let adb = ctx.devices();
        let probed = self.probed.borrow();
        let pending = MtpState::Pending;
        let mut sections: Vec<(String, Vec<Group>)> = Vec::new();
        for u in &phones {
            let p = probed.get(&probe_key(u));
            let nickname = ctx.settings.borrow().nickname(&u.serial).map(String::from);
            let inputs = Inputs {
                usb: u,
                node_access: usb::node_accessible(u),
                mtp: p.map(|p| &p.mtp).unwrap_or(&pending),
                gvfs: p.and_then(|p| p.gvfs.as_deref()),
                known: p.and_then(|p| p.known.as_ref()),
                nickname: nickname.as_deref(),
                adb_state: adb.iter().find(|d| !u.serial.is_empty() && d.serial == u.serial).map(|d| d.state),
            };
            sections.push((u.display_name(), basic::report(&inputs)));
        }
        drop(probed);
        // Without changes, it is not rebuilt (device events arrive often).
        let sig = format!("{sections:?}{ready}{}", ctx.private_mode());
        if *self.last_render.borrow() == sig {
            return;
        }
        *self.last_render.borrow_mut() = sig;
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
        if phones.is_empty() {
            let page = adw::StatusPage::builder()
                .icon_name("adb-smartphone-symbolic")
                .title(tr("No phone connected over USB"))
                .description(
                    tr("Plug in the cable: the phone is identified without debugging. To also see the battery and storage, choose “File transfer” in its USB notification with the screen unlocked. In “Charging only” mode many phones share no data at all."),
                )
                .build();
            page.add_css_class("compact");
            self.body.append(&page);
            self.body.append(&group_widget(ctx, &basic::without_usb()));
        }
        for (name, groups) in &sections {
            if phones.len() > 1 {
                self.body.append(&util::label(&ctx.shown(name), &["section-title"]));
            }
            for g in groups {
                self.body.append(&group_widget(ctx, g));
            }
        }
        if !phones.is_empty() && !ready {
            self.body.append(&group_widget(ctx, &basic::advanced_group()));
        }
        util::cap_wrapping(&self.body);
    }
}

impl Basic {
    fn self_rc(&self) -> Option<Rc<Self>> {
        self.me.borrow().upgrade()
    }
}

impl Page for Basic {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn on_event(&self, ev: &Event) {
        if matches!(ev, Event::Privacy) {
            self.last_render.borrow_mut().clear();
        }
        if matches!(ev, Event::Devices | Event::SettingsChanged | Event::Privacy) {
            // `sync` needs the `Rc`: it is rebuilt from the context.
            if let Some(this) = self.self_rc() {
                this.sync();
            } else {
                self.render();
            }
        }
    }

    fn shown(&self) {
        if let Some(this) = self.self_rc() {
            // Reads older than 30 s are repeated when coming back: the battery
            // changes.
            let stale: Vec<String> = this.probed.borrow().iter().filter(|(_, p)| !matches!(p.mtp, MtpState::Pending) && now_secs() - p.at > 30.0).map(|(k, _)| k.clone()).collect();
            for k in stale {
                this.probed.borrow_mut().remove(&k);
            }
            this.sync();
        }
    }
}
