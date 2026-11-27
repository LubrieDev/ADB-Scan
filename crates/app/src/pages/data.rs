//! Phone data: public sheet (to share) and private identifiers (IMEI, SIM,
//! accounts, MAC, public IP).
//!
//! Privacy of the private view:
//! - It is read when opened, never in the background or in the telemetry.
//! - Values start hidden (“••••”) and are revealed one by one or all at once.
//! - Nothing is stored in the history, the settings or the logs.
//! - When leaving the page (or hiding the window) it returns to the public view
//!   and the data is wiped from memory.
//! - When a value is copied, it is cleared from the clipboard after 30 s if it
//!   is still there.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adbscan_core::identity::{self, PrivateInfo, SheetGroup};
use adbscan_core::models::Snapshot;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

const MASK: &str = "••••••••••••";
const CLIPBOARD_SECONDS: u32 = 30;

// ---------------------------------------------------------------------------
// Private value row
// ---------------------------------------------------------------------------

/// Row with a hidden value, an eye button and a copy button.
struct SecretRow {
    row: adw::ActionRow,
    label: gtk::Label,
    eye: gtk::ToggleButton,
    copy: gtk::Button,
    value: RefCell<Option<String>>,
}

impl SecretRow {
    fn new(ctx: &Rc<Ctx>, title: &str, subtitle: &str) -> Rc<Self> {
        let row = adw::ActionRow::builder().title(title).build();
        if !subtitle.is_empty() {
            row.set_subtitle(subtitle);
        }
        let label = gtk::Label::builder().label("—").xalign(1.0).selectable(false).build();
        label.add_css_class("secret-value");
        label.add_css_class("numeric");
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        // An IMEI, an ICCID or a MAC fit whole; only long email addresses are
        // shortened.
        label.set_width_chars(20);
        label.set_max_width_chars(34);
        let eye = gtk::ToggleButton::builder().icon_name("adb-view-reveal-symbolic").tooltip_text(tr("Show")).valign(gtk::Align::Center).build();
        eye.add_css_class("flat");
        let copy = util::icon_button("adb-copy-symbolic", tr("Copy (cleared from the clipboard after 30 s)"));
        copy.add_css_class("flat");
        copy.set_valign(gtk::Align::Center);
        row.add_suffix(&label);
        row.add_suffix(&eye);
        row.add_suffix(&copy);
        let this = Rc::new(Self { row, label, eye, copy, value: RefCell::new(None) });
        let weak = Rc::downgrade(&this);
        this.eye.connect_toggled(move |_| {
            if let Some(t) = weak.upgrade() {
                t.render();
            }
        });
        let weak = Rc::downgrade(&this);
        let ctx = ctx.clone();
        this.copy.connect_clicked(move |b| {
            let Some(t) = weak.upgrade() else { return };
            let value = t.value.borrow().clone();
            if let Some(v) = value {
                copy_secret(&ctx, b, v);
            }
        });
        this.set(None);
        this
    }

    fn set(&self, value: Option<String>) {
        let has = value.is_some();
        *self.value.borrow_mut() = value;
        self.eye.set_sensitive(has);
        self.copy.set_sensitive(has);
        self.render();
    }

    fn set_revealed(&self, revealed: bool) {
        self.eye.set_active(revealed);
    }

    fn render(&self) {
        let revealed = self.eye.is_active();
        self.eye.set_icon_name(if revealed { "adb-eye-off-symbolic" } else { "adb-view-reveal-symbolic" });
        self.eye.set_tooltip_text(Some(if revealed { tr("Hide") } else { tr("Show") }));
        match self.value.borrow().as_deref() {
            None => {
                self.label.set_text("—");
                self.label.remove_css_class("masked");
            }
            Some(v) if revealed => {
                self.label.set_text(v);
                self.label.remove_css_class("masked");
            }
            Some(_) => {
                // Fixed length: not even the number of characters can be guessed.
                self.label.set_text(MASK);
                self.label.add_css_class("masked");
            }
        }
        self.label.set_selectable(revealed);
    }
}

/// Copies and, if it is still on the clipboard after 30 s, clears it.
fn copy_secret(ctx: &Rc<Ctx>, widget: &impl IsA<gtk::Widget>, value: String) {
    let clipboard = widget.as_ref().clipboard();
    clipboard.set_text(&value);
    ctx.toast(&t!("Copied. It will be cleared from the clipboard in {} s", CLIPBOARD_SECONDS));
    glib::timeout_add_seconds_local_once(CLIPBOARD_SECONDS, move || {
        glib::spawn_future_local(async move {
            if let Ok(Some(current)) = clipboard.read_text_future().await {
                if current.as_str() == value {
                    clipboard.set_text("");
                }
            }
        });
    });
}

// ---------------------------------------------------------------------------
// Choice cards
// ---------------------------------------------------------------------------

/// Large card: phone with a badge, title, description and tag.
fn choice(badge_icon: &str, badge_class: &str, title: &str, text: &str, tag: &str, tag_class: &str) -> gtk::ToggleButton {
    let content = util::hbox(18);
    let overlay = gtk::Overlay::new();
    let phone = util::icon("adb-smartphone-symbolic");
    phone.set_pixel_size(64);
    phone.add_css_class("choice-icon");
    overlay.set_child(Some(&phone));
    let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    badge.add_css_class("choice-badge");
    badge.add_css_class(badge_class);
    let badge_img = util::icon(badge_icon);
    badge_img.set_pixel_size(18);
    badge.append(&badge_img);
    badge.set_halign(gtk::Align::End);
    badge.set_valign(gtk::Align::End);
    overlay.add_overlay(&badge);
    overlay.set_valign(gtk::Align::Center);
    content.append(&overlay);

    let text_box = util::vbox(4);
    text_box.set_valign(gtk::Align::Center);
    text_box.set_hexpand(true);
    text_box.append(&util::label(title, &["choice-title"]));
    let desc = util::label(text, &["dim"]);
    desc.set_wrap(true);
    text_box.append(&desc);
    let pill = util::label(tag, &["pill", tag_class]);
    pill.set_halign(gtk::Align::Start);
    pill.set_margin_top(6);
    text_box.append(&pill);
    content.append(&text_box);

    let b = gtk::ToggleButton::builder().child(&content).hexpand(true).build();
    b.add_css_class("data-choice");
    b
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

struct DataPage {
    me: Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    public_btn: gtk::ToggleButton,
    private_btn: gtk::ToggleButton,
    views: gtk::Stack,
    /// Public sheet: rows per group, in the order of `public_sheet`.
    sheet_rows: Vec<Vec<gtk::Label>>,
    // -- private view --
    status: gtk::Label,
    reveal_all: gtk::ToggleButton,
    refresh: gtk::Button,
    secrets: Vec<Rc<SecretRow>>,
    imei1: Rc<SecretRow>,
    imei2: Rc<SecretRow>,
    serial: Rc<SecretRow>,
    android_id: Rc<SecretRow>,
    device_name: Rc<SecretRow>,
    imsi: Rc<SecretRow>,
    iccid: Rc<SecretRow>,
    number: Rc<SecretRow>,
    public_ip: Rc<SecretRow>,
    public_ip_btn: gtk::Button,
    local_ip: Rc<SecretRow>,
    ssid: Rc<SecretRow>,
    wifi_mac: Rc<SecretRow>,
    bt_name: Rc<SecretRow>,
    bt_mac: Rc<SecretRow>,
    sim_group: adw::PreferencesGroup,
    sim_rows: RefCell<Vec<adw::ActionRow>>,
    accounts_group: adw::PreferencesGroup,
    before_reset: crate::pages::personal_copy::BeforeReset,
    account_rows: RefCell<Vec<Rc<SecretRow>>>,
    /// Increases when wiping: discards reads that finish afterwards.
    generation: Cell<u64>,
    loaded_for: RefCell<Option<String>>,
    loading: Cell<bool>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    DataPage::new(ctx)
}

impl DataPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);

        // -- choice -------------------------------------------------------
        let choices = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(14).homogeneous(true).build();
        let public_btn =
            choice("adb-globe-symbolic", "public", tr("Public information"), tr("Model, system and hardware. Useful to ask for help on a forum or from technical support."), tr("Can be shared"), "ok");
        let private_btn =
            choice("adb-system-lock-screen-symbolic", "private", tr("Private information"), tr("IMEI, SIM, accounts, MAC addresses and public IP. They identify the phone and you."), tr("Only for you"), "warn");
        private_btn.set_group(Some(&public_btn));
        public_btn.set_active(true);
        choices.append(&public_btn);
        choices.append(&private_btn);
        content.append(&choices);

        let views = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).transition_duration(160).vhomogeneous(false).build();

        // -- public view ----------------------------------------------------
        let public_view = util::vbox(18);
        let head = util::hbox(12);
        let note = util::label(tr("What identifies the model, not the person: no IMEI, serial number or accounts."), &["dim"]);
        note.set_wrap(true);
        note.set_hexpand(true);
        head.append(&note);
        let copy_sheet = util::pill_button("adb-copy-symbolic", tr("Copy sheet"));
        copy_sheet.add_css_class("suggested-action");
        copy_sheet.add_css_class("pill");
        copy_sheet.set_valign(gtk::Align::Center);
        head.append(&copy_sheet);
        public_view.append(&head);
        let mut sheet_rows = Vec::new();
        for group in identity::public_sheet(&Snapshot::default(), "") {
            let g = adw::PreferencesGroup::builder().title(group.title).build();
            let mut labels = Vec::new();
            for (title, _) in &group.rows {
                let (row, value) = util::info_row(title);
                g.add(&row);
                labels.push(value);
            }
            public_view.append(&g);
            sheet_rows.push(labels);
        }
        views.add_named(&public_view, Some("public"));

        // -- private view ----------------------------------------------------
        let private_view = util::vbox(18);
        let banner = util::hbox(12);
        banner.add_css_class("card");
        banner.add_css_class("privacy-note");
        let shield = util::icon("adb-smartphone-lock-symbolic");
        shield.set_pixel_size(28);
        shield.set_valign(gtk::Align::Center);
        banner.append(&shield);
        let banner_text = util::vbox(2);
        banner_text.set_hexpand(true);
        banner_text.append(&util::label(tr("This data does not leave your computer"), &["heading"]));
        let banner_sub = util::label(tr("It is not stored in the history, the settings or the logs. It is wiped from memory when you leave this page."), &["dim"]);
        banner_sub.set_wrap(true);
        banner_text.append(&banner_sub);
        banner.append(&banner_text);
        private_view.append(&banner);

        let bar = util::hbox(8);
        let status = util::label("", &["dim", "caption"]);
        status.set_hexpand(true);
        bar.append(&status);
        let reveal_all = gtk::ToggleButton::builder().child(&adw::ButtonContent::builder().icon_name("adb-view-reveal-symbolic").label(tr("Show all")).build()).build();
        reveal_all.add_css_class("pill");
        let refresh = util::pill_button("adb-refresh-cw-symbolic", tr("Read again"));
        refresh.add_css_class("pill");
        bar.append(&reveal_all);
        bar.append(&refresh);
        private_view.append(&bar);

        let mut secrets = Vec::new();
        let mut secret = |title: &str, subtitle: &str| {
            let r = SecretRow::new(ctx, title, subtitle);
            secrets.push(r.clone());
            r
        };
        let phone_group = adw::PreferencesGroup::builder().title(tr("Phone")).build();
        let imei1 = secret("IMEI 1", tr("Identifies the phone on the mobile network; it can be used to block it if stolen"));
        let imei2 = secret("IMEI 2", "");
        let serial = secret(tr("Serial number"), "");
        let android_id = secret("Android ID", tr("Different for each signing app and user since Android 8"));
        let device_name = secret(tr("Device name"), tr("The one other devices see (Bluetooth, hotspot, Quick Share)"));
        for r in [&imei1, &imei2, &serial, &android_id, &device_name] {
            phone_group.add(&r.row);
        }
        private_view.append(&phone_group);

        let sim_group = adw::PreferencesGroup::builder().title("SIM").build();
        let imsi = secret("IMSI", tr("Subscriber identity with the carrier"));
        let iccid = secret("ICCID", tr("Number printed on the SIM card"));
        let number = secret(tr("Phone number"), tr("Only if the SIM stores it (many do not)"));
        for r in [&imsi, &iccid, &number] {
            sim_group.add(&r.row);
        }
        private_view.append(&sim_group);

        let net_group = adw::PreferencesGroup::builder().title(tr("Network")).build();
        let public_ip = secret(tr("Public IP"), &t!("Queried from {} by the phone, only when you press", identity::PUBLIC_IP_SERVICE.trim_start_matches("https://")));
        let public_ip_btn = gtk::Button::with_label(tr("Query"));
        public_ip_btn.add_css_class("pill");
        public_ip_btn.set_valign(gtk::Align::Center);
        public_ip.row.add_suffix(&public_ip_btn);
        let local_ip = secret(tr("Local IP (Wi-Fi)"), "");
        let ssid = secret(tr("Wi-Fi network"), "");
        let wifi_mac = secret("MAC Wi-Fi", "");
        let bt_name = secret(tr("Bluetooth name"), "");
        let bt_mac = secret(tr("Bluetooth MAC"), "");
        for r in [&public_ip, &local_ip, &ssid, &wifi_mac, &bt_name, &bt_mac] {
            net_group.add(&r.row);
        }
        private_view.append(&net_group);

        let accounts_group = adw::PreferencesGroup::builder().title(tr("Accounts on the phone")).build();
        private_view.append(&accounts_group);
        private_view.append(&crate::pages::personal_copy::group(ctx));
        let before_reset = crate::pages::personal_copy::BeforeReset::new(ctx);
        private_view.append(&before_reset.group);
        views.add_named(&private_view, Some("private"));
        content.append(&views);

        let root = util::page_scroller(&content, 1000);
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            public_btn,
            private_btn,
            views,
            sheet_rows,
            status,
            reveal_all,
            refresh,
            secrets,
            imei1,
            imei2,
            serial,
            android_id,
            device_name,
            imsi,
            iccid,
            number,
            public_ip,
            public_ip_btn,
            local_ip,
            ssid,
            wifi_mac,
            bt_name,
            bt_mac,
            sim_group,
            sim_rows: RefCell::new(Vec::new()),
            accounts_group,
            before_reset,
            account_rows: RefCell::new(Vec::new()),
            generation: Cell::new(0),
            loaded_for: RefCell::new(None),
            loading: Cell::new(false),
        });
        this.connect(&copy_sheet);
        this.update_public();
        this
    }

    fn connect(self: &Rc<Self>, copy_sheet: &gtk::Button) {
        let weak = self.me.clone();
        self.private_btn.connect_toggled(move |b| {
            let Some(t) = weak.upgrade() else { return };
            if b.is_active() {
                t.views.set_visible_child_name("private");
                t.load_private(false);
            } else {
                t.views.set_visible_child_name("public");
                t.forget();
            }
        });
        let weak = self.me.clone();
        copy_sheet.connect_clicked(move |b| {
            let Some(t) = weak.upgrade() else { return };
            let Some(s) = t.ctx.snapshot() else { return };
            // Without the user's nickname: the sheet is for sharing.
            let hint = t.commercial_hint();
            let title = if hint.is_empty() { s.model() } else { hint.clone() };
            util::copy_to_clipboard(b, &identity::sheet_text(&title, &identity::public_sheet(&s, &hint)));
            t.ctx.toast(tr("Sheet copied to the clipboard"));
        });
        let weak = self.me.clone();
        self.reveal_all.connect_toggled(move |b| {
            let Some(t) = weak.upgrade() else { return };
            let on = b.is_active();
            if let Some(c) = b.child().and_downcast::<adw::ButtonContent>() {
                c.set_label(if on { tr("Hide all") } else { tr("Show all") });
                c.set_icon_name(if on { "adb-eye-off-symbolic" } else { "adb-view-reveal-symbolic" });
            }
            for r in t.all_secrets() {
                r.set_revealed(on);
            }
        });
        let weak = self.me.clone();
        self.refresh.connect_clicked(move |_| {
            if let Some(t) = weak.upgrade() {
                t.load_private(true);
            }
        });
        let weak = self.me.clone();
        self.public_ip_btn.connect_clicked(move |_| {
            if let Some(t) = weak.upgrade() {
                t.query_public_ip();
            }
        });
        // When leaving the page or hiding the window: public view and nothing in
        // memory.
        let weak = self.me.clone();
        self.root.connect_unmap(move |_| {
            if let Some(t) = weak.upgrade() {
                t.public_btn.set_active(true);
                t.forget();
            }
        });
    }

    fn all_secrets(&self) -> Vec<Rc<SecretRow>> {
        let mut v = self.secrets.clone();
        v.extend(self.account_rows.borrow().iter().cloned());
        v
    }

    // -- public sheet ----------------------------------------------------------

    /// Marketing name seen over USB (without the nickname the user gave it).
    fn commercial_hint(&self) -> String {
        self.ctx.selected_device().map(|d| d.display_name()).unwrap_or_default()
    }

    fn update_public(&self) {
        let Some(s) = self.ctx.snapshot() else { return };
        let groups: Vec<SheetGroup> = identity::public_sheet(&s, &self.commercial_hint());
        for (labels, group) in self.sheet_rows.iter().zip(groups.iter()) {
            for (label, (_, value)) in labels.iter().zip(group.rows.iter()) {
                if label.text() != value.as_str() {
                    label.set_text(value);
                }
            }
        }
    }

    // -- private view ------------------------------------------------------------

    /// Wipes everything read and discards reads in progress.
    fn forget(&self) {
        self.generation.set(self.generation.get() + 1);
        self.loading.set(false);
        *self.loaded_for.borrow_mut() = None;
        self.reveal_all.set_active(false);
        for r in &self.secrets {
            r.set_revealed(false);
            r.set(None);
        }
        for row in self.sim_rows.borrow_mut().drain(..) {
            self.sim_group.remove(&row);
        }
        for r in self.account_rows.borrow_mut().drain(..) {
            self.accounts_group.remove(&r.row);
        }
        self.accounts_group.set_description(None);
        self.status.set_text("");
        self.public_ip_btn.set_sensitive(true);
        self.public_ip_btn.set_label(tr("Query"));
    }

    fn load_private(&self, force: bool) {
        let Some(serial) = self.ctx.ready_serial() else {
            self.status.set_text(tr("No device is ready"));
            return;
        };
        if self.loading.get() || (!force && self.loaded_for.borrow().as_deref() == Some(serial.as_str())) {
            return;
        }
        let Some(adb) = self.ctx.adb() else { return };
        self.loading.set(true);
        self.refresh.set_sensitive(false);
        self.status.set_text(tr("Reading from the phone…"));
        let generation = self.generation.get();
        let weak = self.me.clone();
        let serial2 = serial.clone();
        util::run_bg(
            move || adb.private_info(&serial2),
            move |result| {
                let Some(t) = weak.upgrade() else { return };
                t.refresh.set_sensitive(true);
                if t.generation.get() != generation {
                    return;
                }
                t.loading.set(false);
                match result {
                    Ok(info) => {
                        *t.loaded_for.borrow_mut() = Some(serial);
                        t.show_private(&info);
                        t.status.set_text(tr("Read just now · only in memory while this view is open"));
                    }
                    Err(e) => t.status.set_text(&t!("Could not read: {}", e)),
                }
            },
        );
    }

    fn show_private(&self, info: &PrivateInfo) {
        let revealed = self.reveal_all.is_active();
        self.imei1.set(info.imei[0].clone());
        self.imei2.set(info.imei[1].clone());
        self.imei2.row.set_visible(info.imei[1].is_some());
        self.serial.set(info.serial.clone());
        self.android_id.set(info.android_id.clone());
        self.device_name.set(info.device_name.clone());
        self.imsi.set(info.imsi.clone());
        self.iccid.set(info.iccid.clone());
        self.number.set(info.phone_number.clone());
        self.bt_name.set(info.bluetooth_name.clone());
        self.bt_mac.set(info.bluetooth_mac.clone());
        self.update_network();

        for row in self.sim_rows.borrow_mut().drain(..) {
            self.sim_group.remove(&row);
        }
        let mut sim_rows = Vec::new();
        for (i, sim) in info.sims.iter().enumerate() {
            let mut detail = sim.state_label().to_string();
            if sim.present() && !sim.operator.is_empty() {
                detail = format!("{} · {detail}", sim.operator);
                if !sim.country.is_empty() {
                    detail.push_str(&format!(" · {}", sim.country));
                }
            }
            let (row, value) = util::info_row(&t!("Slot {}", i + 1));
            value.set_text(&detail);
            self.sim_group.add(&row);
            sim_rows.push(row);
        }
        *self.sim_rows.borrow_mut() = sim_rows;

        for r in self.account_rows.borrow_mut().drain(..) {
            self.accounts_group.remove(&r.row);
        }
        let mut rows = Vec::new();
        for a in &info.accounts {
            let r = SecretRow::new(&self.ctx, &a.kind_label(), "");
            r.set(Some(a.name.clone()));
            r.set_revealed(revealed);
            self.accounts_group.add(&r.row);
            rows.push(r);
        }
        self.accounts_group.set_description(Some(&match rows.len() {
            0 => tr("No accounts, or Android does not let them be seen").to_string(),
            1 => tr("1 account").to_string(),
            n => t!("{} accounts", n),
        }));
        *self.account_rows.borrow_mut() = rows;
        self.before_reset.set_accounts(&info.accounts);
        for r in &self.secrets {
            r.set_revealed(revealed);
        }
    }

    /// Local IP, network and Wi-Fi MAC: they already arrive in the telemetry.
    fn update_network(&self) {
        if self.loaded_for.borrow().is_none() && !self.loading.get() {
            return;
        }
        let Some(s) = self.ctx.snapshot() else { return };
        self.local_ip.set(s.network.wifi_ipv4());
        self.ssid.set(s.wifi.as_ref().map(|w| w.ssid.clone()).filter(|v| !v.is_empty()));
        let mac = s.network.interfaces.iter().find(|i| i.name == "wlan0").and_then(|i| identity::valid_mac(&i.mac));
        self.wifi_mac.row.set_subtitle(match &mac {
            Some(m) if identity::is_randomized_mac(m) => tr("Randomised for this network: Android does not show the factory one"),
            Some(_) => tr("Factory"),
            None => "",
        });
        if self.wifi_mac.value.borrow().as_ref() != mac.as_ref() {
            self.wifi_mac.set(mac);
        }
    }

    fn query_public_ip(&self) {
        let (Some(serial), Some(adb)) = (self.ctx.ready_serial(), self.ctx.adb()) else {
            return;
        };
        self.public_ip_btn.set_sensitive(false);
        self.public_ip_btn.set_label(tr("Querying…"));
        let generation = self.generation.get();
        let weak = self.me.clone();
        util::run_bg(
            move || adb.public_ip(&serial),
            move |result| {
                let Some(t) = weak.upgrade() else { return };
                if t.generation.get() != generation {
                    return;
                }
                t.public_ip_btn.set_sensitive(true);
                t.public_ip_btn.set_label(tr("Query again"));
                match result {
                    Ok(ip) => {
                        // Hidden like the rest: asking for it is not asking to
                        // see it (someone may be looking at the screen). It is
                        // only shown with its eye or with “Show all”.
                        t.public_ip.set_revealed(t.reveal_all.is_active());
                        t.public_ip.set(Some(ip.to_string()));
                        t.ctx.toast(tr("Public IP obtained: press the eye to see it"));
                    }
                    Err(e) => t.ctx.toast(&e),
                }
            },
        );
    }
}

impl Page for DataPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn on_event(&self, ev: &Event) {
        match ev {
            Event::Snapshot => {
                self.update_public();
                self.update_network();
            }
            Event::Devices => {
                // Another phone (or none): what was read is no longer valid.
                let current = self.ctx.ready_serial();
                let loaded = self.loaded_for.borrow().clone();
                if loaded.is_some() && loaded != current {
                    self.forget();
                    if self.private_btn.is_active() {
                        self.load_private(false);
                    }
                }
                self.update_public();
            }
            _ => {}
        }
    }

    fn shown(&self) {
        self.update_public();
    }
}
