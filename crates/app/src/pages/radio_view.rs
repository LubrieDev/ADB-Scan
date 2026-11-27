//! Network → mobile signal and nearby Wi-Fi networks (see
//! `adbscan_core::radio`). Read-only. The signal is read when the page is
//! shown; nearby networks only when “Scan” is pressed. Network names follow
//! private mode; access point addresses are never shown.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adbscan_core::radio::{self, Band, MobileSignal, Network};

use crate::ctx::Ctx;
use crate::util;

/// Networks listed (the loudest).
const MAX_NETWORKS: usize = 40;

pub struct RadioView {
    me: Weak<Self>,
    ctx: Rc<Ctx>,
    pub root: gtk::Box,
    tech: gtk::Label,
    strength: gtk::Label,
    quality: gtk::Label,
    scan_btn: gtk::Button,
    scan_status: gtk::Label,
    advice: adw::ActionRow,
    advice_box: gtk::ListBox,
    list: gtk::ListBox,
    mobile: RefCell<Option<Option<MobileSignal>>>,
    networks: RefCell<Option<Vec<Network>>>,
    /// Frequency of the network the phone is on (to say which channel it
    /// uses).
    pub connected_freq: Cell<Option<u32>>,
    busy: Cell<bool>,
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

impl RadioView {
    pub fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let root = util::vbox(24);

        let mobile = adw::PreferencesGroup::builder().title(tr("Mobile signal")).description(tr("Of the SIM in use, as the phone receives it now")).build();
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Read again"));
        refresh.add_css_class("flat");
        mobile.set_header_suffix(Some(&refresh));
        let (row_tech, tech) = util::info_row(tr("Network"));
        let (row_strength, strength) = util::info_row(tr("Strength"));
        let (row_quality, quality) = util::info_row(tr("Quality"));
        row_strength.set_subtitle(tr("RSRP: above −90 dBm is good; below −110 dBm calls and data suffer"));
        row_quality.set_subtitle(tr("RSRQ and SNR: interference and noise; higher is better"));
        for r in [&row_tech, &row_strength, &row_quality] {
            mobile.add(r);
        }
        root.append(&mobile);

        let wifi = adw::PreferencesGroup::builder()
            .title(tr("Nearby Wi-Fi networks"))
            .description(tr("What the phone hears around it: to choose your router's least crowded channel or find where the signal is weak. Only names and channels; nothing is joined."))
            .build();
        let scan_btn = util::pill_button("adb-wifi-symbolic", tr("Scan"));
        scan_btn.add_css_class("pill");
        scan_btn.set_valign(gtk::Align::Center);
        wifi.set_header_suffix(Some(&scan_btn));
        let scan_status = util::label(tr("Not scanned yet."), &["dim", "caption"]);
        scan_status.set_xalign(0.0);
        scan_status.set_wrap(true);
        wifi.add(&scan_status);
        let advice = adw::ActionRow::builder().title(tr("Least crowded 2.4 GHz channel")).build();
        advice.add_prefix(&util::icon("adb-wifi-symbolic"));
        let advice_box = gtk::ListBox::new();
        advice_box.add_css_class("boxed-list");
        advice_box.set_selection_mode(gtk::SelectionMode::None);
        advice_box.set_margin_top(6);
        advice_box.append(&advice);
        advice_box.set_visible(false);
        wifi.add(&advice_box);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        list.set_margin_top(12);
        list.set_visible(false);
        wifi.add(&list);
        root.append(&wifi);

        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            tech,
            strength,
            quality,
            scan_btn: scan_btn.clone(),
            scan_status,
            advice,
            advice_box,
            list,
            mobile: RefCell::new(None),
            networks: RefCell::new(None),
            connected_freq: Cell::new(None),
            busy: Cell::new(false),
        });
        let w = this.me.clone();
        refresh.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.load_mobile();
            }
        });
        let w = this.me.clone();
        scan_btn.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.scan();
            }
        });
        this
    }

    pub fn load_mobile(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else { return };
        let w = self.me.clone();
        util::run_bg(
            move || adb.mobile_signal(&serial),
            move |r| {
                let Some(t) = w.upgrade() else { return };
                match r {
                    Ok(m) => {
                        *t.mobile.borrow_mut() = Some(m);
                        t.render_mobile();
                    }
                    Err(e) => t.tech.set_text(&t!("Could not read it: {}", e)),
                }
            },
        );
    }

    fn render_mobile(&self) {
        let m = self.mobile.borrow();
        let Some(m) = m.as_ref() else { return };
        let Some(m) = m else {
            self.tech.set_text(tr("No SIM in service"));
            self.strength.set_text("—");
            self.quality.set_text("—");
            return;
        };
        self.tech.set_text(&m.summary());
        self.strength.set_text(&match m.rsrp {
            Some(r) => format!("{r} dBm · {}", m.quality()),
            None => "—".into(),
        });
        let mut q = Vec::new();
        if let Some(v) = m.rsrq {
            q.push(format!("RSRQ {v} dB"));
        }
        if let Some(v) = m.snr {
            q.push(format!("SNR {v} dB"));
        }
        self.quality.set_text(&if q.is_empty() { "—".to_string() } else { q.join(" · ") });
    }

    fn scan(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else { return };
        if self.busy.replace(true) {
            return;
        }
        self.scan_btn.set_sensitive(false);
        crate::anim::set_busy(&self.scan_btn, true);
        self.scan_status.set_text(tr("Scanning (about 5 seconds)…"));
        let w = self.me.clone();
        util::run_bg(
            move || adb.wifi_scan(&serial),
            move |r| {
                let Some(t) = w.upgrade() else { return };
                t.busy.set(false);
                t.scan_btn.set_sensitive(true);
                crate::anim::set_busy(&t.scan_btn, false);
                match r {
                    Ok(n) => {
                        *t.networks.borrow_mut() = Some(n);
                        t.render_networks();
                    }
                    Err(e) => t.scan_status.set_text(&t!("Could not scan: {}", e)),
                }
            },
        );
    }

    fn render_networks(&self) {
        let nets = self.networks.borrow();
        let Some(nets) = nets.as_ref() else { return };
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let count = |b: Band| nets.iter().filter(|n| n.band() == Some(b)).count();
        let old = nets.iter().filter(|n| n.age_s.is_none()).count();
        let mut status = t!("{} networks · 2.4 GHz: {} · 5 GHz: {} · 6 GHz: {}", nets.len(), count(Band::G2), count(Band::G5), count(Band::G6));
        if old > 0 {
            status.push_str(" · ");
            status.push_str(&t!("{} heard more than 15 minutes ago (Android limits how often it scans)", old));
        }
        self.scan_status.set_text(&status);

        // Advice for 2.4 GHz, where channels overlap.
        match radio::best_24(nets).filter(|_| count(Band::G2) > 0) {
            Some(best) => {
                let loads = radio::channel_loads(nets);
                let detail: Vec<String> = loads.iter().filter(|l| l.band == Band::G2).map(|l| format!("{} → {}", l.channel, l.networks)).collect();
                self.advice.set_title(&t!("Least crowded 2.4 GHz channel: {}", best));
                let mine = self
                    .connected_freq
                    .get()
                    .and_then(|f| Network { freq_mhz: f, ..Default::default() }.channel().filter(|_| (2400..=2500).contains(&f)))
                    .map(|c| t!("your network is on channel {}", c));
                let mut sub = t!("networks per channel: {}", detail.join(" · "));
                if let Some(m) = mine {
                    sub = format!("{m} · {sub}");
                }
                self.advice.set_subtitle(&sub);
                self.advice_box.set_visible(true);
            }
            None => self.advice_box.set_visible(false),
        }

        for n in nets.iter().take(MAX_NETWORKS) {
            let name = if n.ssid.is_empty() { tr("Hidden network").to_string() } else { self.ctx.shown_value(&n.ssid) };
            let mut sub = vec![];
            if let (Some(b), Some(c)) = (n.band(), n.channel()) {
                sub.push(t!("channel {} · {}", c, b.label()));
            }
            sub.push(n.security.clone());
            let row = adw::ActionRow::builder().title(esc(&name)).subtitle(esc(&sub.join(" · "))).build();
            let quality = match n.bars() {
                4 => tr("excellent"),
                3 => tr("good"),
                2 => tr("fair"),
                _ => tr("weak"),
            };
            row.add_suffix(&util::label(&format!("{} dBm · {quality}", n.rssi), &["dim", "numeric"]));
            self.list.append(&row);
        }
        self.list.set_visible(!nets.is_empty());
    }

    /// Private mode changed: names are shown again (masked or not).
    pub fn on_privacy(&self) {
        if self.networks.borrow().is_some() {
            self.render_networks();
        }
    }
}
