//! Device: identity, software, hardware, display and properties.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use adbscan_core::adb::RebootMode;
use adbscan_core::format;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

struct DevicePage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    rows: Vec<(&'static str, gtk::Label)>,
    nickname: adw::EntryRow,
    nick_hidden: adw::ActionRow,
    /// Real values of each row (what gets copied), unmasked.
    raw: Rc<RefCell<std::collections::HashMap<&'static str, String>>>,
    props_list: crate::widgets::RowList,
    props_search: gtk::SearchEntry,
    props_cache: RefCell<Vec<(String, String)>>,
    kernel: RefCell<Option<String>>,
    last_serial: RefCell<Option<String>>,
    /// Platform sheet (SELinux, GPU, clusters, limits…).
    platform_box: gtk::Box,
    platform: RefCell<Option<adbscan_core::platform::PlatformReport>>,
    platform_refresh: gtk::Button,
    platform_head: RefCell<Option<adw::PreferencesGroup>>,
    /// The sheet must be repainted (the read arrived or the properties changed).
    platform_dirty: std::cell::Cell<bool>,
    /// What was shown last time: it is only repainted if it changes (repainting
    /// would collapse the open expanders).
    platform_sig: RefCell<String>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    DevicePage::new(ctx)
}

const GROUPS: [(&str, &[(&str, &str)]); 4] = [
    ("Identity", &[("name", "Marketing name"), ("manufacturer", "Manufacturer"), ("brand", "Brand"), ("model", "Model"), ("device", "Codename"), ("serial", "Serial number")]),
    (
        "Software",
        &[
            ("android", "Android"),
            ("patch", "Security patch"),
            ("build", "Build"),
            ("fingerprint", "Build fingerprint"),
            ("buildtype", "Build type"),
            ("baseband", "Baseband"),
            ("kernel", "Kernel"),
            ("bootloader", "Bootloader"),
        ],
    ),
    ("Hardware", &[("soc", "Platform"), ("abi", "Architecture"), ("cores", "Cores"), ("maxfreq", "Maximum frequency"), ("ram", "RAM"), ("storage", "Storage")]),
    ("Screen", &[("resolution", "Resolution"), ("density", "Density"), ("refresh", "Refresh rate"), ("refresh_all", "Supported refresh rates")]),
];

impl DevicePage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);
        let mut rows = Vec::new();

        let nick_group = adw::PreferencesGroup::builder().title(tr("Nickname")).description(tr("Used in the tray, notifications and file names")).build();
        let nickname = adw::EntryRow::builder().title(tr("Name for this device")).show_apply_button(true).build();
        nick_group.add(&nickname);
        // In private mode the editable field is replaced by this row: that way
        // the nickname is not visible and there is no risk of saving the mask
        // as the nickname.
        let nick_hidden = adw::ActionRow::builder().title(tr("Nickname hidden")).subtitle(tr("Turn off private mode (Ctrl+Shift+P) to see or change it")).build();
        nick_hidden.add_prefix(&util::icon("adb-eye-off-symbolic"));
        nick_group.add(&nick_hidden);
        let raw: Rc<RefCell<std::collections::HashMap<&'static str, String>>> = Rc::default();
        content.append(&nick_group);

        for (title, fields) in GROUPS {
            let group = adw::PreferencesGroup::builder().title(tr(title)).build();
            for (key, label) in fields.iter() {
                let (row, value) = util::info_row(tr(label));
                let copy = util::icon_button("adb-copy-symbolic", tr("Copy"));
                copy.add_css_class("flat");
                copy.set_valign(gtk::Align::Center);
                // Copies the real value, not what is shown (in private mode,
                // “••••”).
                let (raw2, k) = (raw.clone(), *key);
                copy.connect_clicked(move |b| {
                    let v = raw2.borrow().get(k).cloned().unwrap_or_default();
                    util::copy_to_clipboard(b, &v);
                });
                row.add_suffix(&copy);
                group.add(&row);
                rows.push((*key, value));
            }
            content.append(&group);
        }

        // Platform sheet (read separately, ~0.4 s)
        let platform_box = util::vbox(18);
        content.append(&platform_box);
        let platform_refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Read the platform sheet again"));
        platform_refresh.add_css_class("flat");
        platform_refresh.set_valign(gtk::Align::Center);

        // Reboots
        let power = adw::PreferencesGroup::builder().title(tr("Reboot")).description(tr("Asks for confirmation before rebooting")).build();
        let flow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(6).column_spacing(8).row_spacing(8).build();
        for mode in RebootMode::ALL {
            let b = util::pill_button("adb-system-reboot-symbolic", mode.label());
            b.add_css_class("pill");
            let ctx2 = ctx.clone();
            let key = format!("{mode:?}").to_lowercase();
            b.connect_clicked(move |b| confirm_reboot(&ctx2, b.upcast_ref(), &key));
            flow.append(&b);
        }
        power.add(&flow);
        content.append(&power);

        // Properties
        let props_group = adw::PreferencesGroup::builder().title(tr("All properties")).description(tr("getprop: filter by name or value")).build();
        let props_search = gtk::SearchEntry::builder().placeholder_text(tr("Search properties…")).build();
        props_search.set_margin_bottom(8);
        let props_list = crate::widgets::RowList::new(None);
        let pbox = util::vbox(0);
        pbox.append(&props_search);
        pbox.append(&props_list.root);
        props_group.add(&pbox);
        content.append(&props_group);

        let root = util::page_scroller(&content, 980);
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            rows,
            nickname,
            nick_hidden,
            raw,
            props_list,
            props_search,
            props_cache: RefCell::new(Vec::new()),
            kernel: RefCell::new(None),
            last_serial: RefCell::new(None),
            platform_box,
            platform: RefCell::new(None),
            platform_refresh,
            platform_head: RefCell::new(None),
            platform_dirty: std::cell::Cell::new(true),
            platform_sig: RefCell::new(String::new()),
        });
        {
            let weak = Rc::downgrade(&this);
            this.platform_refresh.connect_clicked(move |_| {
                if let Some(t) = weak.upgrade() {
                    t.load_platform();
                }
            });
        }
        {
            let ctx = ctx.clone();
            this.nickname.connect_apply(move |e| {
                let Some(serial) = ctx.ready_serial() else {
                    return;
                };
                let text = e.text().trim().to_string();
                {
                    let mut s = ctx.settings.borrow_mut();
                    if text.is_empty() {
                        s.nicknames.remove(&serial);
                    } else {
                        s.nicknames.insert(serial, text);
                    }
                }
                ctx.save_settings();
                ctx.emit(Event::Devices);
                ctx.toast(tr("Nickname saved"));
            });
        }
        {
            let weak = Rc::downgrade(&this);
            this.props_search.connect_search_changed(move |_| {
                if let Some(t) = weak.upgrade() {
                    t.render_props();
                }
            });
        }
        this.refresh();
        this.apply_privacy();
        this
    }

    fn set(&self, key: &'static str, value: &str) {
        self.raw.borrow_mut().insert(key, value.to_string());
        if let Some((_, l)) = self.rows.iter().find(|(k, _)| *k == key) {
            let shown = if key == "serial" { self.ctx.shown_value(value) } else { self.ctx.shown(value) };
            l.set_text(if value.is_empty() { "—" } else { &shown });
        }
    }

    fn apply_privacy(&self) {
        let private = self.ctx.private_mode();
        self.nickname.set_visible(!private);
        self.nick_hidden.set_visible(private);
    }

    fn refresh(self: &Rc<Self>) {
        let serial = self.ctx.ready_serial();
        if *self.last_serial.borrow() != serial {
            *self.last_serial.borrow_mut() = serial.clone();
            *self.kernel.borrow_mut() = None;
            self.props_cache.borrow_mut().clear();
            self.nickname.set_text(serial.as_deref().and_then(|s| self.ctx.settings.borrow().nickname(s).map(String::from)).as_deref().unwrap_or(""));
            if let (Some(serial), Some(adb)) = (serial.clone(), self.ctx.adb()) {
                let weak = Rc::downgrade(self);
                util::run_bg(
                    move || adb.kernel_version(&serial),
                    move |k| {
                        if let Some(t) = weak.upgrade() {
                            *t.kernel.borrow_mut() = Some(k);
                            t.refresh();
                        }
                    },
                );
            }
            *self.platform.borrow_mut() = None;
            self.platform_dirty.set(true);
            self.load_platform();
        }
        let Some(s) = self.ctx.snapshot() else { return };
        let p = |k: &str| s.prop(k).to_string();
        // Many manufacturers do not publish `ro.product.marketname`; the
        // phone's own USB product name is usually the marketing name.
        let mut name = s.commercial_name();
        if name.is_empty() {
            name = self.ctx.usb_devices().into_iter().find(|u| u.serial == s.serial).map(|u| u.product).unwrap_or_default();
        }
        self.set("name", &name);
        self.set("manufacturer", &s.manufacturer());
        self.set("brand", &s.brand());
        self.set("model", &p("ro.product.model"));
        self.set("device", &p("ro.product.device"));
        self.set("serial", &s.serial);
        self.set("android", &s.android_version());
        self.set("patch", &s.security_patch());
        self.set("build", &p("ro.build.display.id"));
        self.set("fingerprint", &p("ro.build.fingerprint"));
        self.set("buildtype", &format!("{} · {}", p("ro.build.type"), p("ro.build.tags")));
        self.set("baseband", &p("gsm.version.baseband"));
        self.set("kernel", self.kernel.borrow().as_deref().unwrap_or("…"));
        self.set("bootloader", &p("ro.bootloader"));
        self.set("soc", &[p("ro.soc.manufacturer"), p("ro.soc.model"), p("ro.board.platform")].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" · "));
        self.set("abi", &p("ro.product.cpu.abilist"));
        self.set("cores", &s.cpu.core_count.map(|c| c.to_string()).unwrap_or_default());
        self.set("maxfreq", &format::frequency_khz(s.cpu.max_freq_khz()));
        self.set("ram", &format::human_kib(s.memory.total_kib));
        self.set("storage", &format::human_kib(s.data_storage.total_kib));
        self.set("resolution", &s.display.resolution());
        self.set("density", &format!("{} dpi · {}", s.display.density_dpi.unwrap_or(0), s.display.density_bucket));
        self.set("refresh", &s.display.refresh_text());
        self.set("refresh_all", &s.display.supported_refresh.iter().map(|r| format!("{r:.0}")).collect::<Vec<_>>().join(", "));
        // Compare without copying: properties almost never change.
        let same = {
            let cache = self.props_cache.borrow();
            cache.len() == s.props.len() && cache.iter().zip(s.props.iter()).all(|((a, b), (c, d))| a == c && b == d)
        };
        if !same {
            *self.props_cache.borrow_mut() = s.props.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            self.render_props();
            self.platform_dirty.set(true);
        }
        if self.platform_dirty.replace(false) {
            self.render_platform(&s.props);
        }
    }

    /// Reads the platform sheet in the background (read-only).
    fn load_platform(self: &Rc<Self>) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        crate::anim::set_busy(&self.platform_refresh, true);
        let weak = Rc::downgrade(self);
        util::run_bg(
            move || (serial.clone(), adb.platform_report(&serial)),
            move |(serial, r)| {
                let Some(t) = weak.upgrade() else { return };
                crate::anim::set_busy(&t.platform_refresh, false);
                // A report from another phone (changed in the meantime) is not
                // valid.
                if t.ctx.ready_serial().as_deref() != Some(serial.as_str()) {
                    return;
                }
                match r {
                    Ok(report) => *t.platform.borrow_mut() = Some(report),
                    Err(e) => {
                        // Without a response, every item stays as “Could not be
                        // obtained”.
                        *t.platform.borrow_mut() = Some(adbscan_core::platform::parse_platform(""));
                        t.ctx.toast(&t!("Platform sheet: {}", e));
                    }
                }
                t.platform_dirty.set(true);
                t.refresh();
            },
        );
    }

    fn render_platform(&self, props: &std::collections::BTreeMap<String, String>) {
        use adbscan_core::platform;
        let sig = format!(
            "{:?}|{:?}|{:?}|{:?}|{}",
            platform::boot_facts(props),
            platform::platform_facts(props),
            platform::opengles_from_prop(props),
            self.platform.borrow().as_ref().map(|r| r.read_at),
            self.ctx.private_mode()
        );
        if *self.platform_sig.borrow() == sig {
            return;
        }
        *self.platform_sig.borrow_mut() = sig;
        if let Some(old) = self.platform_head.take() {
            old.set_header_suffix(gtk::Widget::NONE);
        }
        while let Some(c) = self.platform_box.first_child() {
            self.platform_box.remove(&c);
        }
        let groups = super::facts::platform_groups(&self.ctx, props, self.platform.borrow().as_ref());
        if let Some(first) = groups.first() {
            first.set_header_suffix(Some(&self.platform_refresh));
            *self.platform_head.borrow_mut() = Some(first.clone());
        }
        for g in groups {
            self.platform_box.append(&g);
        }
    }

    fn render_props(&self) {
        use crate::widgets::RowItem;
        let q = self.props_search.text().to_lowercase();
        let cache = self.props_cache.borrow();
        let mut items: Vec<RowItem> = cache
            .iter()
            .filter(|(k, v)| q.is_empty() || k.to_lowercase().contains(&q) || v.to_lowercase().contains(&q))
            .map(|(k, v)| {
                let shown = self.ctx.shown(v);
                RowItem { tooltip: shown.clone(), ..RowItem::new(k.clone(), if v.is_empty() { tr("(empty)").to_string() } else { shown }) }
            })
            .collect();
        if items.is_empty() {
            items.push(RowItem::new(if cache.is_empty() { tr("Loading properties…") } else { tr("No results") }, ""));
        }
        self.props_list.set_items(items);
    }
}

impl Page for DevicePage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn on_event(&self, ev: &Event) {
        if matches!(ev, Event::Privacy) {
            self.apply_privacy();
            self.render_props();
            self.platform_dirty.set(true);
        }
        if matches!(ev, Event::Snapshot | Event::Devices | Event::Privacy) {
            if let Some(this) = self.me.upgrade() {
                this.refresh();
            }
        }
    }
    fn shown(&self) {
        if let Some(this) = self.me.upgrade() {
            this.refresh();
        }
    }
}

pub fn confirm_reboot(ctx: &Rc<Ctx>, parent: &gtk::Widget, mode: &str) {
    let mode = match mode {
        "recovery" => RebootMode::Recovery,
        "bootloader" => RebootMode::Bootloader,
        "fastbootd" => RebootMode::Fastbootd,
        "sideload" => RebootMode::Sideload,
        "edl" => RebootMode::Edl,
        _ => RebootMode::System,
    };
    let Some(device) = ctx.selected_device() else {
        ctx.toast(tr("No device is selected"));
        return;
    };
    let name = ctx.device_label(&device);
    let body = match mode {
        RebootMode::System => t!("{} will restart. All apps will be closed.", name),
        RebootMode::Edl => t!("{} will enter EDL mode (Qualcomm emergency download). It only works on some devices and you will need specific tools to get out of it.", name),
        _ => t!("{} will restart in {} mode.", name, mode.label()),
    };
    let ctx2 = ctx.clone();
    util::confirm(parent, &t!("Reboot into {}?", mode.label()), &body, tr("Restart"), mode == RebootMode::Edl, move || {
        // In fastboot, the reboot is done with fastboot.
        if device.state.is_fastboot() {
            if let Some(fb) = ctx2.fastboot() {
                let serial = device.serial.clone();
                let target = match mode {
                    RebootMode::Recovery => Some("recovery"),
                    RebootMode::Bootloader => Some("bootloader"),
                    RebootMode::Fastbootd => Some("fastboot"),
                    _ => None,
                };
                let c = ctx2.clone();
                util::run_bg(
                    move || fb.reboot(&serial, target),
                    move |r| match r {
                        Ok(_) => c.toast(tr("Restarting…")),
                        Err(e) => c.toast(&format!("Error: {e}")),
                    },
                );
            }
            return;
        }
        let serial = device.serial.clone();
        let Some(adb) = ctx2.adb() else { return };
        let c = ctx2.clone();
        util::run_bg(
            move || adb.reboot(&serial, mode),
            move |r| {
                c.audit("reboot", Some(mode.label()), r.is_ok(), None);
                match r {
                    Ok(_) => c.toast(&t!("Rebooting into {}…", mode.label())),
                    Err(e) => c.toast(&format!("Error: {e}")),
                }
            },
        );
    });
}
