//! Ways to connect: the three modes, independent of each other.
//!
//! - **Basic analysis**: without developer options or debugging (“Basic
//!   analysis” page; here it is only explained).
//! - **USB debugging**: with a cable; live status of detection and
//!   authorisation.
//! - **Wireless debugging** (Android 11+): pair with a **QR code** or
//!   **manually** (IP address, pairing port and code), and connect to an
//!   already paired phone (IP:port, local network and recent ones).
//!
//! Pairing and connecting use `Adb::pair` and `Adb::connect`; discovery uses
//! `adbscan_core::wireless::discover` (it only listens to the phone's mDNS
//! announcements). The waits for the QR code and for the connection-port search
//! stop when leaving their page or closing the dialog.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adbscan_core::models::DeviceState;
use adbscan_core::usb::UsbMode;
use adbscan_core::wireless::{self as wl, MdnsService, QrPairing, ServiceKind};

use crate::ctx::Ctx;
use crate::util;

/// Time to scan a QR code before it is considered expired.
const QR_TTL: Duration = Duration::from_secs(180);
/// Search for the connection port after pairing.
const CONNECT_SEARCH_TRIES: u32 = 8;
const POLL: Duration = Duration::from_secs(2);

/// Width and modules of the QR code (`true` = black).
type QrMatrix = (usize, Vec<bool>);

/// Opens the dialog on the wireless connection (`win.wireless` action).
pub fn show(ctx: &Rc<Ctx>, parent: &gtk::Widget) {
    show_at(ctx, parent, "wireless");
}

/// Opens the dialog on a page: `root`, `basic`, `usb`, `wireless`, `qr` or
/// `manual` (`win.connect` action).
pub fn show_at(ctx: &Rc<Ctx>, parent: &gtk::Widget, start: &str) {
    let dialog = adw::Dialog::builder().title(tr("Connect")).content_width(560).content_height(680).build();
    let nav = adw::NavigationView::new();
    let ui = Rc::new(Ui { ctx: ctx.clone(), dialog: dialog.clone(), nav: nav.clone(), generation: Rc::new(Cell::new(0)), manual_fields: RefCell::new(None) });

    nav.add(&ui.root_page());
    nav.add(&ui.basic_page());
    nav.add(&ui.usb_page());
    nav.add(&ui.wireless_page());
    nav.add(&ui.qr_page());
    nav.add(&ui.manual_page());

    let stack: &[&str] = match start {
        "basic" | "usb" | "wireless" => &["root", start],
        "qr" | "manual" => &["root", "wireless", start],
        _ => &["root"],
    };
    nav.replace_with_tags(stack);
    dialog.set_child(Some(&nav));
    {
        // Closing stops any wait in progress (QR, port search).
        let g = ui.generation.clone();
        dialog.connect_closed(move |_| g.set(g.get() + 1));
    }
    dialog.present(Some(parent));
}

struct Ui {
    ctx: Rc<Ctx>,
    dialog: adw::Dialog,
    nav: adw::NavigationView,
    /// Changes when the dialog closes or when leaving a page with waits: those
    /// with another number stop.
    generation: Rc<Cell<u64>>,
    /// IP address, port and code of the manual pairing (to fill them in from a
    /// phone discovered on the network).
    manual_fields: RefCell<Option<(adw::EntryRow, adw::EntryRow, adw::EntryRow)>>,
}

fn page(title: &str, tag: &str, content: &gtk::Box) -> adw::NavigationPage {
    content.set_margin_top(12);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    tv.set_content(Some(&gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(content).propagate_natural_height(true).build()));
    adw::NavigationPage::builder().title(title).tag(tag).child(&tv).build()
}

fn text(t: &str, classes: &[&str]) -> gtk::Label {
    let l = util::label(t, classes);
    l.set_wrap(true);
    l.set_xalign(0.0);
    l
}

fn nav_row(title: &str, subtitle: &str, icon: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).subtitle(subtitle).activatable(true).build();
    row.add_prefix(&util::icon(icon));
    row.add_suffix(&util::icon("go-next-symbolic"));
    row
}

fn boxed() -> gtk::ListBox {
    let l = gtk::ListBox::new();
    l.add_css_class("boxed-list");
    l.set_selection_mode(gtk::SelectionMode::None);
    l
}

impl Ui {
    fn push(&self, tag: &'static str) -> impl Fn(&adw::ActionRow) + 'static {
        let nav = self.nav.clone();
        move |_| nav.push_by_tag(tag)
    }

    /// IP field: in private mode it is typed the same, but shown as “•••”.
    fn ip_row(&self, title: &str) -> adw::EntryRow {
        if self.ctx.private_mode() {
            adw::PasswordEntryRow::builder().title(title).build().upcast()
        } else {
            adw::EntryRow::builder().title(title).build()
        }
    }

    fn go_page(&self, id: &str) {
        self.dialog.activate_action("win.go", Some(&id.to_variant())).ok();
        self.dialog.close();
    }

    // -- start -------------------------------------------------------------

    fn root_page(self: &Rc<Self>) -> adw::NavigationPage {
        let c = util::vbox(18);
        c.append(&text(
            tr("Choose how you want ADB Scan to recognise your phone. The modes are independent: the basic analysis works without turning anything on, and debugging (USB or wireless) is a voluntary upgrade."),
            &["dim"],
        ));
        let g = adw::PreferencesGroup::builder().title(tr("Without debugging")).build();
        let basic = nav_row(tr("Basic analysis"), tr("No developer options. Over USB: identification and mode; with “File transfer”, battery and storage."), "adb-search-symbolic");
        basic.connect_activated(self.push("basic"));
        g.add(&basic);
        c.append(&g);
        let g = adw::PreferencesGroup::builder().title(tr("Advanced analysis (voluntary)")).description(tr("Requires turning on developer options and authorising this computer on the phone.")).build();
        let usb = nav_row(tr("USB debugging"), tr("With a cable. Performance, detailed battery, apps, files, screen, security…"), "adb-usb-symbolic");
        usb.connect_activated(self.push("usb"));
        g.add(&usb);
        let wifi = nav_row(tr("Wireless debugging"), tr("Without a cable, on the same Wi-Fi network (Android 11+). Pair with a QR code or manually."), "adb-wifi-symbolic");
        wifi.connect_activated(self.push("wireless"));
        g.add(&wifi);
        c.append(&g);
        page(tr("Connect"), "root", &c)
    }

    // -- basic analysis --------------------------------------------------------

    fn basic_page(self: &Rc<Self>) -> adw::NavigationPage {
        let c = util::vbox(14);
        c.append(&text(tr("Nothing needs to be turned on on the phone. ADB Scan does not use ADB in this mode."), &[]));
        c.append(&text(
            tr("• With the cable, in any mode with data: manufacturer, model, serial number, speed, USB mode and whether it was connected before.\n• With “File transfer” and the screen unlocked: confirmation that it is Android, battery and free space (descriptive data only, no files).\n• In “Charging only” mode many phones share no data at all.\n• There is no basic analysis over Wi-Fi: without wireless debugging the phone does not announce itself on the network."),
            &["dim"],
        ));
        c.append(&text(tr("Performance, apps, files, screen or security need debugging (advanced analysis)."), &["dim"]));
        let open = gtk::Button::with_label(tr("Open Basic analysis"));
        open.add_css_class("pill");
        open.add_css_class("suggested-action");
        open.set_halign(gtk::Align::Center);
        let me = self.clone();
        open.connect_clicked(move |_| me.go_page("basic"));
        c.append(&open);
        page(tr("Basic analysis"), "basic", &c)
    }

    // -- USB ----------------------------------------------------------------------

    fn usb_state(&self) -> (&'static str, String, String) {
        let ctx = &self.ctx;
        if ctx.adb().is_none() {
            return ("adb-circle-x-symbolic", tr("ADB not found").into(), tr("Install it (apt install adb) or download it from Preferences → Tools").into());
        }
        let devices = ctx.devices();
        let usb_dev = devices.iter().find(|d| d.is_usb() && !d.is_wireless());
        match usb_dev.map(|d| (d, d.state)) {
            Some((d, DeviceState::Device)) => ("adb-emblem-ok-symbolic", tr("Connected and authorised").into(), ctx.device_label(d)),
            Some((_, DeviceState::Unauthorized | DeviceState::Authorizing)) => ("adb-key-round-symbolic", tr("Authorisation needed").into(), tr("Unlock the phone and accept “Allow USB debugging?”").into()),
            Some((_, DeviceState::NoPermissions)) => ("adb-triangle-alert-symbolic", tr("No USB permissions").into(), tr("Install ADB Scan's udev rules").into()),
            Some((d, _)) => ("adb-info-symbolic", d.state_label(), String::new()),
            None => {
                let usb = ctx.usb_devices();
                if usb.iter().any(|u| u.has_adb_interface()) {
                    ("adb-key-round-symbolic", tr("Debugging on").into(), tr("Accept the authorisation prompt on the phone").into())
                } else if let Some(u) = usb.iter().find(|u| u.mode == UsbMode::Other) {
                    ("adb-info-symbolic", tr("Phone without debugging").into(), t!("{} · the basic analysis already works", ctx.shown(&u.display_name())))
                } else {
                    ("adb-smartphone-symbolic", tr("No phone over USB").into(), tr("Plug in the cable").into())
                }
            }
        }
    }

    fn usb_page(self: &Rc<Self>) -> adw::NavigationPage {
        let c = util::vbox(14);
        let list = boxed();
        let row = adw::ActionRow::builder().title(tr("Status")).build();
        let icon = util::icon("adb-smartphone-symbolic");
        row.add_prefix(&icon);
        list.append(&row);
        c.append(&list);
        c.append(&text(
            tr("1. On the phone, open Settings → About phone and tap “Build number” 7 times.\n2. In Settings → System → Developer options, turn on “USB debugging”.\n3. Plug in the cable and accept “Allow USB debugging?”. Tick “Always allow from this computer” if you do not want to be asked again."),
            &[],
        ));
        c.append(&text(
            tr("Menu names vary by manufacturer. ADB Scan does not turn anything on for you. You can withdraw the permission in Developer options → “Revoke USB debugging authorisations”."),
            &["dim"],
        ));
        let open = gtk::Button::with_label(tr("Open the Overview"));
        open.add_css_class("pill");
        open.set_halign(gtk::Align::Center);
        let me = self.clone();
        open.connect_clicked(move |_| me.go_page("dashboard"));
        c.append(&open);
        let p = page(tr("USB debugging"), "usb", &c);
        // Live status only while the page is in view.
        let me = Rc::downgrade(self);
        let refresh = move || {
            if let Some(me) = me.upgrade() {
                let (i, t, s) = me.usb_state();
                icon.set_icon_name(Some(i));
                row.set_title(&glib::markup_escape_text(&t));
                row.set_subtitle(&glib::markup_escape_text(&s));
            }
        };
        refresh();
        let alive = Rc::new(Cell::new(false));
        {
            let alive = alive.clone();
            p.connect_shown(move |_| {
                alive.set(true);
                let alive = alive.clone();
                let refresh = refresh.clone();
                refresh();
                glib::timeout_add_local(Duration::from_secs(1), move || {
                    if !alive.get() {
                        return glib::ControlFlow::Break;
                    }
                    refresh();
                    glib::ControlFlow::Continue
                });
            });
        }
        p.connect_hidden(move |_| alive.set(false));
        p
    }

    // -- wireless -------------------------------------------------------------

    fn wireless_page(self: &Rc<Self>) -> adw::NavigationPage {
        let c = util::vbox(18);
        c.append(&text(
            tr("Phone and computer on the same Wi-Fi network, Android 11 or later. On the phone: Developer options → “Wireless debugging”. Pairing is done only once; after that, connecting is enough."),
            &["dim"],
        ));
        let g = adw::PreferencesGroup::builder().title(tr("Pair")).build();
        let qr = nav_row(tr("With a QR code"), tr("Scan a code from “Pair device with QR code”"), "adb-camera-photo-symbolic");
        qr.connect_activated(self.push("qr"));
        g.add(&qr);
        let manual = nav_row(tr("Manually"), tr("IP address, pairing port and 6-digit code"), "adb-input-keyboard-symbolic");
        manual.connect_activated(self.push("manual"));
        g.add(&manual);
        c.append(&g);

        // Connect (already paired or with “adb tcpip”).
        let connect = adw::PreferencesGroup::builder().title(tr("Connect to an already paired phone")).description(tr("IP address and port shown by “Wireless debugging” (or those of “adb tcpip”).")).build();
        let addr = self.ip_row(tr("Address (IP:port)"));
        addr.set_show_apply_button(true);
        if let Some(last) = self.ctx.settings.borrow().wireless_history.first() {
            addr.set_text(last);
        }
        connect.add(&addr);
        c.append(&connect);

        let found = adw::PreferencesGroup::builder().title(tr("On the network")).description(tr("Wireless debugging services announced over mDNS")).build();
        let scan = util::icon_button("adb-refresh-cw-symbolic", tr("Search"));
        scan.add_css_class("flat");
        found.set_header_suffix(Some(&scan));
        let found_list = boxed();
        found.add(&found_list);
        c.append(&found);

        let recent: Vec<String> = self.ctx.settings.borrow().wireless_history.clone();
        if !recent.is_empty() {
            let g = adw::PreferencesGroup::builder().title(tr("Recent")).build();
            for a in recent {
                let row = adw::ActionRow::builder().title(glib::markup_escape_text(&self.ctx.shown(&a))).activatable(true).build();
                row.add_suffix(&util::icon("adb-link-symbolic"));
                let e = addr.clone();
                row.connect_activated(move |_| {
                    e.set_text(&a);
                    e.emit_by_name::<()>("apply", &[]);
                });
                g.add(&row);
            }
            c.append(&g);
        }
        let status = text("", &["dim", "caption"]);
        c.append(&status);

        {
            let me = self.clone();
            let status = status.clone();
            addr.connect_apply(move |e| me.connect(e.text().trim().to_string(), &status));
        }
        let do_scan = {
            let me = Rc::downgrade(self);
            let list = found_list.clone();
            let status = status.clone();
            let addr = addr.clone();
            Rc::new(move || {
                let Some(me) = me.upgrade() else { return };
                while let Some(c) = list.first_child() {
                    list.remove(&c);
                }
                list.append(&adw::ActionRow::builder().title(tr("Searching…")).build());
                let (list, status, addr) = (list.clone(), status.clone(), addr.clone());
                let adb = me.ctx.adb();
                let weak = Rc::downgrade(&me);
                util::run_bg(
                    move || wl::discover(adb.as_ref()),
                    move |services: Vec<MdnsService>| {
                        let Some(me) = weak.upgrade() else { return };
                        while let Some(c) = list.first_child() {
                            list.remove(&c);
                        }
                        if services.is_empty() {
                            let hint = if adbscan_core::exec::which("avahi-browse").is_none() { tr("Turn on “Wireless debugging”. Without avahi-utils, it is only visible if your adb has mDNS") } else { tr("Turn on “Wireless debugging” on the phone") };
                            list.append(&adw::ActionRow::builder().title(tr("No phone was found")).subtitle(hint).build());
                        }
                        for s in services {
                            let pairing = s.kind == ServiceKind::Pairing;
                            let row = adw::ActionRow::builder()
                                .title(glib::markup_escape_text(&me.ctx.shown(&s.name)))
                                .subtitle(format!("{} · {}", me.ctx.shown(&s.address), if pairing { tr("waiting for pairing: pair with its code") } else { tr("ready to connect") }))
                                .activatable(true)
                                .build();
                            row.add_suffix(&util::icon(if pairing { "go-next-symbolic" } else { "adb-link-symbolic" }));
                            let me2 = me.clone();
                            let (status, addr) = (status.clone(), addr.clone());
                            row.connect_activated(move |_| {
                                if pairing {
                                    me2.prefill_manual(&s);
                                } else {
                                    addr.set_text(&s.address);
                                    me2.connect(s.address.clone(), &status);
                                }
                            });
                            list.append(&row);
                        }
                    },
                );
            })
        };
        {
            let s = do_scan.clone();
            scan.connect_clicked(move |_| s());
        }
        let p = page(tr("Wireless debugging"), "wireless", &c);
        // It searches when entering, not when the dialog opens on another page.
        p.connect_shown(move |_| do_scan());
        p
    }

    /// Connects and, if it works, remembers it and closes the dialog.
    fn connect(self: &Rc<Self>, address: String, status: &gtk::Label) {
        let Some(adb) = self.ctx.adb() else {
            status.set_text(tr("ADB is not available"));
            return;
        };
        status.set_text(&self.ctx.shown(&t!("Connecting to {}…", address)));
        let me = self.clone();
        let status = status.clone();
        let a2 = address.clone();
        util::run_bg(
            move || adb.connect(&a2),
            move |r| match r {
                Ok(_) => {
                    {
                        let mut s = me.ctx.settings.borrow_mut();
                        s.wireless_history.retain(|x| *x != address);
                        s.wireless_history.insert(0, address.clone());
                        s.wireless_history.truncate(8);
                    }
                    me.ctx.save_settings();
                    me.ctx.refresh_devices();
                    me.ctx.toast(&me.ctx.shown(&t!("Connected to {}", address)));
                    me.dialog.close();
                }
                Err(e) => status.set_text(&me.ctx.shown(&t!("Could not connect to {}: {}", address, e))),
            },
        );
    }

    /// After pairing: looks for that IP's connection port and connects. If it
    /// does not appear, asks for it (`ask` gets the focus).
    fn connect_after_pair(self: &Rc<Self>, ip: String, status: gtk::Label, my: u64, tries: u32, ask: Option<adw::EntryRow>) {
        if self.generation.get() != my {
            return;
        }
        if tries == 0 {
            status.set_text(tr("Paired. The connection port was not found on the network: type the IP address and port shown by “Wireless debugging” (not the pairing one)."));
            if let Some(a) = ask {
                a.grab_focus();
            }
            return;
        }
        let adb = self.ctx.adb();
        let me = self.clone();
        util::run_bg(
            move || wl::discover(adb.as_ref()),
            move |services: Vec<MdnsService>| {
                if me.generation.get() != my {
                    return;
                }
                match wl::connect_address_for(&services, &ip) {
                    Some(a) => me.connect(a, &status),
                    None => {
                        let me2 = me.clone();
                        glib::timeout_add_local_once(POLL, move || me2.connect_after_pair(ip, status, my, tries - 1, ask));
                    }
                }
            },
        );
    }

    // -- QR -------------------------------------------------------------------------

    fn qr_page(self: &Rc<Self>) -> adw::NavigationPage {
        let c = util::vbox(14);
        c.append(&text(
            tr("1. On the phone: Developer options → “Wireless debugging” → “Pair device with QR code”.\n2. Scan this code. ADB Scan pairs and connects by itself."),
            &[],
        ));
        let matrix: Rc<RefCell<Option<QrMatrix>>> = Rc::new(RefCell::new(None));
        let area = gtk::DrawingArea::builder().content_width(260).content_height(260).halign(gtk::Align::Center).build();
        {
            let matrix = matrix.clone();
            area.set_draw_func(move |_, cr, w, h| {
                // Always black on white, with a margin: that way cameras read it
                // with the dark theme too.
                cr.set_source_rgb(1.0, 1.0, 1.0);
                cr.paint().ok();
                let Some((n, cells)) = matrix.borrow().clone() else { return };
                let total = n as f64 + 8.0;
                let scale = (w.min(h) as f64 / total).floor().max(1.0);
                let off_x = (w as f64 - n as f64 * scale) / 2.0;
                let off_y = (h as f64 - n as f64 * scale) / 2.0;
                cr.set_source_rgb(0.0, 0.0, 0.0);
                for y in 0..n {
                    for x in 0..n {
                        if cells[y * n + x] {
                            cr.rectangle(off_x + x as f64 * scale, off_y + y as f64 * scale, scale, scale);
                        }
                    }
                }
                cr.fill().ok();
            });
        }
        let frame = gtk::Frame::builder().child(&area).halign(gtk::Align::Center).build();
        c.append(&frame);
        let status_row = util::hbox(8);
        status_row.set_halign(gtk::Align::Center);
        let spinner = gtk::Spinner::new();
        let status = text("", &[]);
        status.set_justify(gtk::Justification::Center);
        status.set_xalign(0.5);
        status_row.append(&spinner);
        status_row.append(&status);
        c.append(&status_row);
        let again = gtk::Button::with_label(tr("New code"));
        again.add_css_class("pill");
        again.set_halign(gtk::Align::Center);
        c.append(&again);
        c.append(&text(
            tr("The code only works for this pairing and expires after 3 minutes or when leaving this screen. ADB Scan listens for the phone's mDNS announcement on your network; it does not scan the network. If it does not work (some networks block mDNS), use manual pairing."),
            &["dim", "caption"],
        ));
        if adbscan_core::exec::which("avahi-browse").is_none() {
            c.append(&text(tr("Warning: avahi-browse is missing (avahi-utils package). Without it, the QR code only works if your adb includes mDNS."), &["caption", "warning"]));
        }

        let start = {
            let me = Rc::downgrade(self);
            let (matrix, area, status, spinner) = (matrix.clone(), area.clone(), status.clone(), spinner.clone());
            Rc::new(move || {
                let Some(me) = me.upgrade() else { return };
                me.generation.set(me.generation.get() + 1);
                let my = me.generation.get();
                let creds = QrPairing::new();
                *matrix.borrow_mut() = wl::qr_matrix(&creds.payload());
                area.queue_draw();
                if me.ctx.adb().is_none() {
                    status.set_text(tr("ADB is not available: install it to pair."));
                    return;
                }
                status.set_text(tr("Waiting for the phone to scan the code…"));
                spinner.start();
                let deadline = std::time::Instant::now() + QR_TTL;
                me.qr_poll(creds, my, deadline, status.clone(), spinner.clone());
            })
        };
        {
            let s = start.clone();
            again.connect_clicked(move |_| s());
        }
        let p = page(tr("Pair with a QR code"), "qr", &c);
        p.connect_shown(move |_| start());
        {
            let g = self.generation.clone();
            let spinner = spinner.clone();
            p.connect_hidden(move |_| {
                g.set(g.get() + 1);
                spinner.stop();
            });
        }
        p
    }

    fn qr_poll(self: &Rc<Self>, creds: QrPairing, my: u64, deadline: std::time::Instant, status: gtk::Label, spinner: gtk::Spinner) {
        if self.generation.get() != my {
            return;
        }
        if std::time::Instant::now() >= deadline {
            spinner.stop();
            status.set_text(tr("The code expired. Press “New code”."));
            return;
        }
        let adb = self.ctx.adb();
        let me = self.clone();
        util::run_bg(
            move || wl::discover(adb.as_ref()),
            move |services: Vec<MdnsService>| {
                if me.generation.get() != my {
                    return;
                }
                let Some(found) = creds.find(&services).cloned() else {
                    let me2 = me.clone();
                    glib::timeout_add_local_once(POLL, move || me2.qr_poll(creds, my, deadline, status, spinner));
                    return;
                };
                status.set_text(&me.ctx.shown(&t!("Pairing with {}…", found.address)));
                let Some(adb) = me.ctx.adb() else { return };
                let (addr, pass) = (found.address.clone(), creds.password.clone());
                let me2 = me.clone();
                util::run_bg(
                    move || adb.pair_with_password(&addr, &pass),
                    move |r| {
                        if me2.generation.get() != my {
                            return;
                        }
                        match r {
                            Ok(_) => {
                                me2.ctx.audit("adb_pair", None, true, Some("QR"));
                                status.set_text(tr("Paired. Looking for the connection port…"));
                                me2.connect_after_pair(found.ip().to_string(), status, my, CONNECT_SEARCH_TRIES, None);
                            }
                            Err(e) => {
                                spinner.stop();
                                me2.ctx.audit("adb_pair", None, false, Some("QR"));
                                status.set_text(&me2.ctx.shown(&t!("Could not pair: {}. Press “New code” to try again.", e)));
                            }
                        }
                    },
                );
            },
        );
    }

    // -- manual ---------------------------------------------------------------------

    fn manual_page(self: &Rc<Self>) -> adw::NavigationPage {
        let c = util::vbox(14);
        c.append(&text(
            tr("On the phone: Developer options → “Wireless debugging” → “Pair device with pairing code”. The IP address with the pairing port and a 6-digit code appear."),
            &[],
        ));
        let g = adw::PreferencesGroup::builder().title(tr("Pairing")).build();
        let ip = self.ip_row(tr("IP address"));
        let port = adw::EntryRow::builder().title(tr("Pairing port")).input_purpose(gtk::InputPurpose::Digits).build();
        let code = adw::EntryRow::builder().title(tr("Pairing code (6 digits)")).input_purpose(gtk::InputPurpose::Digits).build();
        g.add(&ip);
        g.add(&port);
        g.add(&code);
        c.append(&g);
        let g2 = adw::PreferencesGroup::builder()
            .title(tr("Connection"))
            .description(tr("It is a different port: the one under “IP address & Port” on the main Wireless debugging screen. If you leave it empty, ADB Scan looks for it on the network after pairing."))
            .build();
        let cport = adw::EntryRow::builder().title(tr("Connection port (optional)")).input_purpose(gtk::InputPurpose::Digits).build();
        g2.add(&cport);
        c.append(&g2);
        let btn = gtk::Button::with_label(tr("Pair and connect"));
        btn.add_css_class("pill");
        btn.add_css_class("suggested-action");
        btn.set_halign(gtk::Align::Center);
        c.append(&btn);
        let status = text("", &["dim", "caption"]);
        c.append(&status);
        *self.manual_fields.borrow_mut() = Some((ip.clone(), port.clone(), code.clone()));
        {
            let me = self.clone();
            let (ip, port, code, cport, status) = (ip.clone(), port.clone(), code.clone(), cport.clone(), status.clone());
            btn.connect_clicked(move |b| {
                let Some(adb) = me.ctx.adb() else {
                    status.set_text(tr("ADB is not available"));
                    return;
                };
                let host = ip.text().trim().to_string();
                let pair_addr = wl::join_host_port(&host, &port.text());
                let host_only = pair_addr.rsplit_once(':').map(|(h, _)| h.to_string()).unwrap_or(host);
                let c = code.text().trim().to_string();
                let conn_port = cport.text().trim().to_string();
                me.generation.set(me.generation.get() + 1);
                let my = me.generation.get();
                b.set_sensitive(false);
                status.set_text(tr("Pairing…"));
                let (me2, b, status, cport) = (me.clone(), b.clone(), status.clone(), cport.clone());
                util::run_bg(
                    move || adb.pair(&pair_addr, &c),
                    move |r| {
                        b.set_sensitive(true);
                        match r {
                            Ok(_) => {
                                me2.ctx.audit("adb_pair", None, true, Some("manual"));
                                if conn_port.is_empty() {
                                    status.set_text(tr("Paired. Looking for the connection port…"));
                                    me2.connect_after_pair(host_only, status, my, CONNECT_SEARCH_TRIES / 2, Some(cport));
                                } else {
                                    me2.connect(format!("{host_only}:{conn_port}"), &status);
                                }
                            }
                            Err(e) => {
                                me2.ctx.audit("adb_pair", None, false, Some("manual"));
                                status.set_text(&me2.ctx.shown(&t!("Could not pair: {}", e)));
                            }
                        }
                    },
                );
            });
        }
        let p = page(tr("Pair manually"), "manual", &c);
        let g = self.generation.clone();
        p.connect_hidden(move |_| g.set(g.get() + 1));
        p
    }

    /// Fills in the manual pairing with a discovered service.
    fn prefill_manual(&self, s: &MdnsService) {
        if let Some((ip, port, code)) = self.manual_fields.borrow().as_ref() {
            ip.set_text(s.ip());
            port.set_text(s.address.rsplit_once(':').map(|(_, p)| p).unwrap_or(""));
            code.set_text("");
            code.grab_focus();
        }
        self.nav.push_by_tag("manual");
    }
}
