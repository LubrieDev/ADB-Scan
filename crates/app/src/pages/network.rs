//! Network: Wi-Fi, interfaces, routes and capture with TShark. (Wireless ADB
//! is in Connection.)

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adbscan_core::exec::{self, StreamEvent, StreamHandle, StreamOptions};
use adbscan_core::format;
use adbscan_core::models::{now_secs, WifiStatus};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

struct NetworkPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    wifi_rows: Vec<(&'static str, gtk::Label)>,
    ifaces: gtk::ListBox,
    routes: gtk::ListBox,
    key: RefCell<String>,
    last_wifi: std::cell::Cell<f64>,
    capture: RefCell<Option<StreamHandle>>,
    capture_btn: gtk::Button,
    capture_status: gtk::Label,
    capture_iface: adw::ComboRow,
    capture_filter: adw::EntryRow,
    /// Replaces the filter (it usually contains an IP) in private mode; the
    /// real filter is still used when capturing.
    filter_hidden: adw::ActionRow,
    capture_file: RefCell<Option<std::path::PathBuf>>,
    capture_group: adw::PreferencesGroup,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    NetworkPage::new(ctx)
}

impl NetworkPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);

        let wifi = adw::PreferencesGroup::builder().title("Wi-Fi").build();
        let mut wifi_rows = Vec::new();
        for (k, t) in [
            ("state", tr("Status")),
            ("ssid", tr("Network (SSID)")),
            ("ip", tr("IP address")),
            ("signal", tr("Signal")),
            ("speed", tr("Link speed")),
            ("band", tr("Frequency")),
            ("standard", tr("Standard")),
            ("security", tr("Security")),
            ("bssid", "BSSID"),
        ] {
            let (row, v) = util::info_row(t);
            wifi.add(&row);
            wifi_rows.push((k, v));
        }
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Refresh"));
        refresh.add_css_class("flat");
        wifi.set_header_suffix(Some(&refresh));
        content.append(&wifi);

        let ig = adw::PreferencesGroup::builder().title("Interfaces").build();
        let ifaces = gtk::ListBox::new();
        ifaces.add_css_class("boxed-list");
        ifaces.set_selection_mode(gtk::SelectionMode::None);
        ig.add(&ifaces);
        content.append(&ig);

        let rg = adw::PreferencesGroup::builder().title(tr("Routes")).build();
        let routes = gtk::ListBox::new();
        routes.add_css_class("boxed-list");
        routes.set_selection_mode(gtk::SelectionMode::None);
        rg.add(&routes);
        content.append(&rg);

        // Capture
        let capture_group = adw::PreferencesGroup::builder()
            .title(tr("Traffic capture (TShark)"))
            .description(tr("Use it only on your own networks and devices, or with their owners' permission: capturing other people's communications is illegal in many countries. The capture happens on the PC; to see an Android app's traffic without root, use an exportable source such as PCAPdroid, or a hotspot on this PC."))
            .build();
        let capture_iface = adw::ComboRow::builder().title(tr("PC interface")).build();
        let capture_filter = adw::EntryRow::builder().title(tr("Capture filter (BPF), e.g. host 192.168.1.20")).build();
        let capture_btn = gtk::Button::with_label(tr("Start capture"));
        capture_btn.add_css_class("suggested-action");
        let capture_status = util::label("", &["dim", "caption"]);
        let cap_row = adw::ActionRow::builder().title(tr(".pcapng file in Downloads")).build();
        cap_row.add_suffix(&capture_btn);
        capture_btn.set_valign(gtk::Align::Center);
        capture_group.add(&capture_iface);
        capture_group.add(&capture_filter);
        let filter_hidden =
            adw::ActionRow::builder().title(tr("Capture filter hidden")).subtitle(tr("It may contain IP addresses. It is still applied; turn off private mode to see or change it")).build();
        filter_hidden.add_prefix(&util::icon("adb-eye-off-symbolic"));
        capture_group.add(&filter_hidden);
        capture_group.add(&cap_row);
        capture_group.add(&capture_status);
        content.append(&capture_group);

        let root = util::page_scroller(&content, 980);
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            wifi_rows,
            ifaces,
            routes,
            key: RefCell::new(String::new()),
            last_wifi: std::cell::Cell::new(0.0),
            capture: RefCell::new(None),
            capture_btn,
            capture_status,
            capture_iface,
            capture_filter,
            filter_hidden,
            capture_file: RefCell::new(None),
            capture_group,
        });
        {
            let w = this.me.clone();
            refresh.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    t.load_wifi();
                }
            });
        }
        {
            let w = this.me.clone();
            this.capture_btn.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    t.toggle_capture();
                }
            });
        }
        this.capture_filter.set_text(&ctx.settings.borrow().capture_filter);
        this.apply_privacy();
        this.load_capture_ifaces();
        this.update();
        this.load_wifi();
        this
    }

    fn set(&self, k: &str, v: String) {
        if let Some((_, l)) = self.wifi_rows.iter().find(|(key, _)| *key == k) {
            // Network and BSSID are entirely sensitive data; the rest is free
            // text.
            let shown = if matches!(k, "ssid" | "bssid") { self.ctx.shown_value(&v) } else { self.ctx.shown(&v) };
            l.set_text(if v.is_empty() { "—" } else { &shown });
        }
    }

    fn apply_privacy(&self) {
        let private = self.ctx.private_mode();
        self.capture_filter.set_visible(!private);
        self.filter_hidden.set_visible(private);
    }

    fn show_wifi(&self, w: &WifiStatus) {
        self.set(
            "state",
            match w.enabled {
                Some(true) if !w.ssid.is_empty() && matches!(w.state.as_str(), "" | "COMPLETED") => tr("Connected").into(),
                Some(true) if !w.ssid.is_empty() => {
                    t!("Connecting ({})", w.state.to_lowercase().replace('_', " "))
                }
                Some(true) => tr("On, not connected").into(),
                Some(false) => tr("Off").into(),
                None => String::new(),
            },
        );
        self.set("ssid", w.ssid.clone());
        self.set("ip", w.ip.clone());
        self.set(
            "signal",
            w.rssi
                .map(|r| {
                    let q = match r {
                        r if r >= -55 => tr("excellent"),
                        r if r >= -67 => tr("good"),
                        r if r >= -75 => "regular",
                        _ => tr("weak"),
                    };
                    format!("{r} dBm ({q})")
                })
                .unwrap_or_default(),
        );
        self.set("speed", w.link_speed.clone());
        self.set(
            "band",
            w.frequency_mhz
                .map(|f| {
                    format!(
                        "{f} MHz ({})",
                        if f >= 5900 {
                            "6 GHz"
                        } else if f >= 4900 {
                            "5 GHz"
                        } else {
                            "2,4 GHz"
                        }
                    )
                })
                .unwrap_or_default(),
        );
        self.set("standard", w.standard.clone());
        self.set("security", w.security.clone());
        self.set("bssid", w.bssid.clone());
    }

    fn load_wifi(&self) {
        self.last_wifi.set(now_secs());
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        let w = self.me.clone();
        util::run_bg(
            move || adb.wifi_status(&serial),
            move |st| {
                if let Some(t) = w.upgrade() {
                    t.show_wifi(&st);
                }
            },
        );
    }

    fn update(&self) {
        let Some(s) = self.ctx.snapshot() else { return };
        match &s.wifi {
            // Live from the telemetry (every second).
            Some(w) => self.show_wifi(w),
            // Android 10 or earlier: separate query every 15 s.
            None if now_secs() - self.last_wifi.get() > 15.0 => self.load_wifi(),
            None => {}
        }
        let key = format!("{:?}{:?}", s.network.interfaces, s.network.routes);
        if *self.key.borrow() == key {
            return;
        }
        *self.key.borrow_mut() = key;
        while let Some(c) = self.ifaces.first_child() {
            self.ifaces.remove(&c);
        }
        let mut list = s.network.interfaces.clone();
        list.sort_by_key(|i| (i.state != "UP", i.addresses.is_empty(), i.name.clone()));
        for i in list.iter().filter(|i| i.name != "lo" && (i.state == "UP" || !i.addresses.is_empty())) {
            let addrs: Vec<String> = i.addresses.iter().map(|a| format!("{}/{}", a.address, a.prefix)).collect();
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&i.name))
                .subtitle(glib::markup_escape_text(&if addrs.is_empty() { tr("No address").to_string() } else { self.ctx.shown(&addrs.join("  ·  ")) }))
                .subtitle_selectable(true)
                .build();
            let icon = if i.name.starts_with("wlan") {
                "adb-wifi-symbolic"
            } else if i.name.starts_with("rmnet") || i.name.starts_with("ccmni") {
                "adb-activity-symbolic"
            } else {
                "adb-network-symbolic"
            };
            row.add_prefix(&util::icon(icon));
            let meta = util::label(&format!("{}{}", i.state, i.mtu.map(|m| format!(" · MTU {m}")).unwrap_or_default()), &["dim", "caption"]);
            row.add_suffix(&meta);
            if !i.mac.is_empty() {
                row.set_tooltip_text(Some(&self.ctx.shown(&format!("MAC {}", i.mac))));
            }
            self.ifaces.append(&row);
        }
        let hidden = list.iter().filter(|i| i.name == "lo" || (i.state != "UP" && i.addresses.is_empty())).count();
        if hidden > 0 {
            self.ifaces.append(&adw::ActionRow::builder().title(t!("{} inactive interfaces hidden", hidden)).build());
        }
        while let Some(c) = self.routes.first_child() {
            self.routes.remove(&c);
        }
        for r in s.network.routes.iter().take(40) {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&self.ctx.shown(&r.destination)))
                .subtitle(glib::markup_escape_text(
                    &self.ctx.shown(
                        &[r.via.as_ref().map(|v| t!("via {}", v)), r.dev.as_ref().map(|d| t!("on {}", d)), r.src.as_ref().map(|s| t!("source {}", s))]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · "),
                    ),
                ))
                .build();
            self.routes.append(&row);
        }
        if s.network.routes.is_empty() {
            self.routes.append(&adw::ActionRow::builder().title(tr("No routes visible to the shell user")).build());
        }
    }

    fn tshark(&self) -> Option<std::path::PathBuf> {
        let configured = self.ctx.settings.borrow().tshark_path.clone();
        adbscan_core::adb::find_tool_named("tshark", &configured)
    }

    fn load_capture_ifaces(&self) {
        let Some(tshark) = self.tshark() else {
            self.capture_status.set_text(tr("TShark is not installed. Install it with “sudo apt install tshark” or set its path in Preferences."));
            self.capture_btn.set_sensitive(false);
            self.capture_iface.set_sensitive(false);
            self.capture_filter.set_sensitive(false);
            return;
        };
        let w = self.me.clone();
        util::run_bg(
            move || exec::run(&tshark, &["-D".into()], Duration::from_secs(10), None).map(|r| r.stdout).unwrap_or_default(),
            move |out| {
                let Some(t) = w.upgrade() else { return };
                let names: Vec<String> = out.lines().filter_map(|l| l.split_once(". ").map(|(_, rest)| rest.split_whitespace().next().unwrap_or("").to_string())).filter(|n| !n.is_empty()).collect();
                if names.is_empty() {
                    t.capture_status.set_text(tr("TShark lists no interfaces. Your user may not be in the wireshark group."));
                }
                let model = gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
                t.capture_iface.set_model(Some(&model));
                let saved = t.ctx.settings.borrow().capture_interface.clone();
                if let Some(pos) = names.iter().position(|n| *n == saved) {
                    t.capture_iface.set_selected(pos as u32);
                }
            },
        );
    }

    fn toggle_capture(&self) {
        if let Some(h) = self.capture.borrow_mut().take() {
            h.cancel();
            return;
        }
        let Some(tshark) = self.tshark() else { return };
        let iface = self.capture_iface.selected_item().and_downcast::<gtk::StringObject>().map(|s| s.string().to_string());
        let Some(iface) = iface else {
            self.ctx.toast(tr("Choose an interface"));
            return;
        };
        let filter = self.capture_filter.text().trim().to_string();
        if filter.chars().any(|c| c == '\n' || c == '\0') {
            return;
        }
        {
            let mut s = self.ctx.settings.borrow_mut();
            s.capture_interface = iface.clone();
            s.capture_filter = filter.clone();
        }
        self.ctx.save_settings();
        let dir = adbscan_core::paths::downloads_dir();
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join(format!("capture-{}.pcapng", format::file_stamp(now_secs())));
        let mut args = vec!["-i".to_string(), iface, "-w".into(), file.display().to_string(), "-q".into()];
        if !filter.is_empty() {
            args.push("-f".into());
            args.push(filter);
        }
        let (tx, rx) = async_channel::unbounded::<StreamEvent>();
        match exec::stream(&tshark, &args, StreamOptions::default(), move |ev| {
            let _ = tx.send_blocking(ev);
        }) {
            Ok(h) => {
                *self.capture.borrow_mut() = Some(h);
                *self.capture_file.borrow_mut() = Some(file.clone());
                self.capture_btn.set_label(tr("Stop capture"));
                self.capture_btn.remove_css_class("suggested-action");
                self.capture_btn.add_css_class("destructive-action");
                self.capture_status.set_text(&t!("Capturing to {}", file.display()));
                let w = self.me.clone();
                glib::spawn_future_local(async move {
                    let mut last_err = String::new();
                    while let Ok(ev) = rx.recv().await {
                        match ev {
                            StreamEvent::Stderr(l) => last_err = l,
                            StreamEvent::Exit { cancelled, .. } => {
                                let Some(t) = w.upgrade() else { break };
                                t.capture.borrow_mut().take();
                                t.capture_btn.set_label(tr("Start capture"));
                                t.capture_btn.remove_css_class("destructive-action");
                                t.capture_btn.add_css_class("suggested-action");
                                let file = t.capture_file.borrow().clone();
                                match file.filter(|f| f.exists()) {
                                    Some(f) => {
                                        t.capture_status.set_text(&t!("Saved to {}", f.display()));
                                        t.ctx.toast_action(tr("Capture saved to Downloads"), tr("Open"), move || util::open_path(&f));
                                    }
                                    None if !cancelled => t.capture_status.set_text(&t!("TShark exited: {}", last_err)),
                                    None => t.capture_status.set_text(tr("Capture stopped")),
                                }
                                break;
                            }
                            _ => {}
                        }
                    }
                });
            }
            Err(e) => self.ctx.toast(&t!("Could not start TShark: {}", e)),
        }
    }
}

impl Page for NetworkPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn on_event(&self, ev: &Event) {
        match ev {
            Event::Snapshot => self.update(),
            Event::Devices => {
                self.key.borrow_mut().clear();
                self.load_wifi();
            }
            Event::Privacy => {
                self.apply_privacy();
                self.key.borrow_mut().clear();
                self.update();
            }
            _ => {}
        }
    }
    fn shown(&self) {
        self.update();
        self.load_wifi();
        let _ = &self.capture_group;
    }
}
