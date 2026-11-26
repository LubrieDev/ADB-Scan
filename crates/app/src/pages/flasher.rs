//! Flasher: fastboot with safeguards, ROM plans, A/B slots, temporary boot and
//! sideload.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adbscan_core::exec::{StreamEvent, StreamHandle};
use adbscan_core::fastboot::{self, Confirm, FastbootInfo, FlashPlan, Slot};
use adbscan_core::format;
use adbscan_core::models::DeviceState;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

struct FlasherPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    state_row: adw::ActionRow,
    info_group: adw::PreferencesGroup,
    info_rows: RefCell<Vec<gtk::Widget>>,
    /// Log lines exactly as they arrived (painted according to private mode).
    log_lines: RefCell<Vec<String>>,
    info: RefCell<Option<FastbootInfo>>,
    fb_only: Vec<gtk::Widget>,
    adb_only: Vec<gtk::Widget>,
    part_entry: adw::EntryRow,
    part_hint: gtk::Label,
    image: RefCell<Option<PathBuf>>,
    image_row: adw::ActionRow,
    slot_combo: adw::ComboRow,
    flash_btn: gtk::Button,
    plan: RefCell<Option<FlashPlan>>,
    plan_list: gtk::ListBox,
    plan_btn: gtk::Button,
    plan_progress: gtk::ProgressBar,
    slot_label: gtk::Label,
    sideload_row: adw::ActionRow,
    log: gtk::TextView,
    busy: RefCell<Option<StreamHandle>>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    FlasherPage::new(ctx)
}

impl FlasherPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);

        let banner = adw::Banner::builder().title(tr("Flashing the wrong partitions can brick the phone. Use only images for your exact model.")).revealed(true).build();
        content.append(&banner);

        // Status
        let status = adw::PreferencesGroup::builder().title(tr("Status")).build();
        let state_row = adw::ActionRow::builder().title(tr("No device in fastboot")).build();
        state_row.add_prefix(&util::icon("adb-system-software-install-symbolic"));
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Read variables"));
        refresh.add_css_class("flat");
        refresh.set_valign(gtk::Align::Center);
        state_row.add_suffix(&refresh);
        status.add(&state_row);
        // Reboots from ADB
        let to_bl = adw::ActionRow::builder().title(tr("Enter the bootloader")).subtitle("adb reboot bootloader").activatable(true).build();
        to_bl.set_action_name(Some("win.reboot"));
        to_bl.set_action_target_value(Some(&"bootloader".to_variant()));
        let to_fbd = adw::ActionRow::builder().title(tr("Enter fastbootd")).subtitle(tr("Required for dynamic partitions (super)")).activatable(true).build();
        to_fbd.set_action_name(Some("win.reboot"));
        to_fbd.set_action_target_value(Some(&"fastbootd".to_variant()));
        let to_sl = adw::ActionRow::builder().title(tr("Enter sideload mode")).subtitle("adb reboot sideload").activatable(true).build();
        to_sl.set_action_name(Some("win.reboot"));
        to_sl.set_action_target_value(Some(&"sideload".to_variant()));
        for r in [&to_bl, &to_fbd, &to_sl] {
            r.add_suffix(&util::icon("adb-system-reboot-symbolic"));
            status.add(r);
        }
        content.append(&status);

        let info_group = adw::PreferencesGroup::builder().title(tr("Bootloader information")).build();
        content.append(&info_group);

        // Flash a partition
        let single = adw::PreferencesGroup::builder().title(tr("Flash a partition")).build();
        let part_entry = adw::EntryRow::builder().title(tr("Partition (e.g. boot, init_boot, vendor_boot, recovery)")).build();
        let part_hint = util::label("", &["caption", "dim"]);
        part_hint.set_margin_start(12);
        part_hint.set_wrap(true);
        let suggestions = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(8).column_spacing(4).row_spacing(4).build();
        for p in ["boot", "init_boot", "vendor_boot", "dtbo", "vbmeta", "recovery"] {
            let b = gtk::Button::with_label(p);
            b.add_css_class("pill");
            let e = part_entry.clone();
            b.connect_clicked(move |_| e.set_text(p));
            suggestions.append(&b);
        }
        let image_row = adw::ActionRow::builder().title(tr("Image")).subtitle(tr("None selected")).activatable(true).build();
        image_row.add_suffix(&util::icon("adb-folder-open-symbolic"));
        let slot_combo = adw::ComboRow::builder().title(tr("Slot")).model(&gtk::StringList::new(&[tr("Active (default)"), "A", "B", tr("Both")])).build();
        let flash_btn = gtk::Button::with_label(tr("Flash"));
        flash_btn.add_css_class("destructive-action");
        flash_btn.add_css_class("pill");
        flash_btn.set_halign(gtk::Align::End);
        flash_btn.set_margin_top(8);
        let boot_btn = gtk::Button::with_label(tr("Boot without flashing"));
        boot_btn.add_css_class("pill");
        boot_btn.set_tooltip_text(Some(tr("fastboot boot: tries a boot image without writing it")));
        boot_btn.set_margin_top(8);
        let single_actions = util::hbox(8);
        single_actions.set_halign(gtk::Align::End);
        single_actions.append(&boot_btn);
        single_actions.append(&flash_btn);
        let part_box = util::vbox(6);
        let part_list = gtk::ListBox::new();
        part_list.add_css_class("boxed-list");
        part_list.set_selection_mode(gtk::SelectionMode::None);
        part_list.append(&part_entry);
        part_box.append(&part_list);
        part_box.append(&suggestions);
        part_box.append(&part_hint);
        let rest = gtk::ListBox::new();
        rest.add_css_class("boxed-list");
        rest.set_selection_mode(gtk::SelectionMode::None);
        rest.set_margin_top(6);
        rest.append(&image_row);
        rest.append(&slot_combo);
        part_box.append(&rest);
        part_box.append(&single_actions);
        single.add(&part_box);
        content.append(&single);

        // Full ROM
        let rom = adw::PreferencesGroup::builder()
            .title(tr("Full ROM (flashing plan)"))
            .description(tr("Choose the ROM folder. flash_all.sh is read if it exists (without running it) and the plan is shown before anything is written."))
            .build();
        let rom_btn = util::pill_button("adb-folder-open-symbolic", tr("Choose folder…"));
        rom.set_header_suffix(Some(&rom_btn));
        let plan_list = gtk::ListBox::new();
        plan_list.add_css_class("boxed-list");
        plan_list.set_selection_mode(gtk::SelectionMode::None);
        plan_list.append(&adw::ActionRow::builder().title(tr("No plan loaded")).build());
        let plan_progress = gtk::ProgressBar::builder().show_text(true).visible(false).build();
        let plan_btn = gtk::Button::with_label(tr("Run plan"));
        plan_btn.add_css_class("destructive-action");
        plan_btn.add_css_class("pill");
        plan_btn.set_halign(gtk::Align::End);
        plan_btn.set_sensitive(false);
        plan_btn.set_margin_top(8);
        rom.add(&plan_list);
        rom.add(&plan_progress);
        rom.add(&plan_btn);
        content.append(&rom);

        // Slots
        let slots = adw::PreferencesGroup::builder().title(tr("A/B slots")).build();
        let slot_row = adw::ActionRow::builder().title(tr("Active slot")).build();
        let slot_label = util::label("—", &["numeric", "heading"]);
        slot_row.add_suffix(&slot_label);
        let set_a = gtk::Button::with_label(tr("Activate A"));
        let set_b = gtk::Button::with_label(tr("Activate B"));
        for b in [&set_a, &set_b] {
            b.set_valign(gtk::Align::Center);
            slot_row.add_suffix(b);
        }
        slots.add(&slot_row);
        content.append(&slots);

        // Sideload
        let side = adw::PreferencesGroup::builder().title(tr("Sideload (recovery)")).description(tr("Sends an OTA update or a recovery ZIP with adb sideload")).build();
        let sideload_row = adw::ActionRow::builder().title(tr("Send ZIP…")).subtitle(tr("The phone must be in “Apply update from ADB”")).activatable(true).build();
        sideload_row.add_suffix(&util::icon("adb-send-to-symbolic"));
        side.add(&sideload_row);
        content.append(&side);

        // Boot
        let boot = adw::PreferencesGroup::builder().title(tr("Reboot from fastboot")).build();
        let flow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(5).column_spacing(6).row_spacing(6).build();
        let mut reboot_buttons = Vec::new();
        for (label, target) in [(tr("Continue booting"), "continue"), (tr("System"), "system"), ("Bootloader", "bootloader"), ("Recovery", "recovery"), ("Fastbootd", "fastboot")] {
            let b = gtk::Button::with_label(label);
            b.add_css_class("pill");
            flow.append(&b);
            reboot_buttons.push((b, target));
        }
        boot.add(&flow);
        content.append(&boot);

        // Dangerous
        let danger = adw::PreferencesGroup::builder().title(tr("Danger zone")).description(tr("They erase all data on the phone. You are asked to type the serial number.")).build();
        let wipe = adw::ActionRow::builder().title(tr("Erase user data")).subtitle(tr("fastboot -w (factory reset)")).activatable(true).build();
        let unlock = adw::ActionRow::builder().title(tr("Unlock the bootloader")).subtitle(tr("fastboot flashing unlock · erases everything and disables verified boot")).activatable(true).build();
        let lock = adw::ActionRow::builder().title(tr("Lock the bootloader")).subtitle(tr("fastboot flashing lock · erases everything; only with factory firmware")).activatable(true).build();
        let devinfo = adw::ActionRow::builder().title(tr("OEM information")).subtitle(tr("fastboot oem device-info (read-only)")).activatable(true).build();
        for r in [&devinfo, &wipe, &unlock, &lock] {
            danger.add(r);
        }
        content.append(&danger);

        // Log
        let log = gtk::TextView::builder().editable(false).monospace(true).wrap_mode(gtk::WrapMode::WordChar).cursor_visible(false).build();
        log.add_css_class("console");
        let log_scroll = gtk::ScrolledWindow::builder().child(&log).min_content_height(180).build();
        log_scroll.add_css_class("card");
        content.append(&util::label(tr("Log"), &["section-title"]));
        content.append(&log_scroll);

        let fb_only: Vec<gtk::Widget> = vec![single.clone().upcast(), rom.clone().upcast(), slots.clone().upcast(), boot.clone().upcast(), danger.clone().upcast(), info_group.clone().upcast()];
        let adb_only: Vec<gtk::Widget> = vec![to_bl.upcast(), to_fbd.upcast(), to_sl.upcast()];

        let root = util::page_scroller(&content, 900);
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            state_row,
            info_group,
            info_rows: RefCell::new(Vec::new()),
            log_lines: RefCell::new(Vec::new()),
            info: RefCell::new(None),
            fb_only,
            adb_only,
            part_entry,
            part_hint,
            image: RefCell::new(None),
            image_row,
            slot_combo,
            flash_btn: flash_btn.clone(),
            plan: RefCell::new(None),
            plan_list,
            plan_btn,
            plan_progress,
            slot_label,
            sideload_row,
            log,
            busy: RefCell::new(None),
        });

        macro_rules! on {
            ($w:expr, $sig:ident, |$t:ident, $a:ident| $body:expr) => {{
                let w = this.me.clone();
                $w.$sig(move |$a| {
                    if let Some($t) = w.upgrade() {
                        $body;
                    }
                });
            }};
        }
        on!(refresh, connect_clicked, |t, _a| t.load_info());
        on!(this.part_entry, connect_changed, |t, _a| t.validate_part());
        on!(this.image_row, connect_activated, |t, a| {
            let w = t.me.clone();
            util::pick_file(a, tr("Image to flash"), &[(tr("Pictures"), &["*.img", "*.bin", "*.mbn", "*.elf"])], move |p| {
                if let Some(t) = w.upgrade() {
                    t.image_row.set_subtitle(&format!("{} · {}", p.display(), format::human_bytes(std::fs::metadata(&p).ok().map(|m| m.len() as f64))));
                    if t.part_entry.text().is_empty() {
                        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                            t.part_entry.set_text(stem);
                        }
                    }
                    *t.image.borrow_mut() = Some(p);
                    t.validate_part();
                }
            })
        });
        on!(flash_btn, connect_clicked, |t, _a| t.flash_single());
        on!(boot_btn, connect_clicked, |t, _a| t.boot_image());
        on!(rom_btn, connect_clicked, |t, a| {
            let w = t.me.clone();
            util::pick_folder(a, tr("ROM folder"), move |dir| {
                if let Some(t) = w.upgrade() {
                    t.load_plan(dir);
                }
            })
        });
        on!(this.plan_btn, connect_clicked, |t, _a| t.confirm_plan());
        on!(set_a, connect_clicked, |t, _a| t.set_slot(Slot::A));
        on!(set_b, connect_clicked, |t, _a| t.set_slot(Slot::B));
        on!(this.sideload_row, connect_activated, |t, a| {
            let w = t.me.clone();
            util::pick_file(a, tr("ZIP for sideload"), &[("ZIP", &["*.zip"])], move |p| {
                if let Some(t) = w.upgrade() {
                    t.sideload(p);
                }
            })
        });
        on!(devinfo, connect_activated, |t, _a| t.simple_fb("oem device-info"));
        on!(wipe, connect_activated, |t, _a| t.guarded("wipe"));
        on!(unlock, connect_activated, |t, _a| t.guarded("unlock"));
        on!(lock, connect_activated, |t, _a| t.guarded("lock"));
        for (b, target) in reboot_buttons {
            let w = this.me.clone();
            b.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    t.reboot_fb(target);
                }
            });
        }
        this.refresh_state();
        this
    }

    /// Dialog text in private mode (or `None` if it is not active).
    fn masked(&self, body: &str) -> Option<String> {
        self.ctx.private_mode().then(|| self.ctx.shown(body))
    }

    fn log(&self, text: &str) {
        // The original is kept: it is repainted when private mode changes.
        self.log_lines.borrow_mut().push(text.to_string());
        let buf = self.log.buffer();
        let mut end = buf.end_iter();
        buf.insert(&mut end, &self.ctx.shown(text));
        if !text.ends_with('\n') {
            let mut end = buf.end_iter();
            buf.insert(&mut end, "\n");
        }
        let mark = buf.create_mark(None, &buf.end_iter(), false);
        self.log.scroll_mark_onscreen(&mark);
        buf.delete_mark(&mark);
    }

    fn fb_device(&self) -> Option<(adbscan_core::fastboot::Fastboot, String)> {
        let dev = self.ctx.selected_device().filter(|d| d.state.is_fastboot())?;
        Some((self.ctx.fastboot()?, dev.serial))
    }

    fn refresh_state(&self) {
        let dev = self.ctx.selected_device();
        let in_fb = dev.as_ref().is_some_and(|d| d.state.is_fastboot());
        let in_adb = dev.as_ref().is_some_and(|d| d.connected());
        for w in &self.fb_only {
            w.set_sensitive(in_fb);
        }
        for w in &self.adb_only {
            w.set_visible(in_adb);
        }
        self.sideload_row.set_sensitive(dev.as_ref().is_some_and(|d| d.state == DeviceState::Sideload));
        match (&dev, self.ctx.fastboot()) {
            (_, None) => {
                self.state_row.set_title(tr("fastboot is not installed"));
                self.state_row.set_subtitle(if cfg!(windows) {
                    tr("Reinstall ADB Scan: fastboot.exe comes in its platform-tools folder. Or set its path in Preferences")
                } else {
                    tr("Install it with “sudo apt install fastboot” or set its path in Preferences")
                });
            }
            (Some(d), _) if in_fb => {
                self.state_row.set_title(&format!("{} · {}", self.ctx.device_label(d), d.state_label()));
                self.state_row.set_subtitle(&self.ctx.shown_value(&d.serial));
                if self.info.borrow().is_none() {
                    self.load_info();
                }
            }
            (Some(d), _) if d.state == DeviceState::Sideload => {
                self.state_row.set_title(tr("Device in sideload mode"));
                self.state_row.set_subtitle(tr("You can now send a ZIP"));
            }
            (Some(d), _) if in_adb => {
                self.state_row.set_title(&t!("{} is running Android", self.ctx.device_label(d)));
                self.state_row.set_subtitle(tr("Reboot into the bootloader or fastbootd to flash"));
            }
            _ => {
                self.state_row.set_title(tr("No device in fastboot"));
                self.state_row.set_subtitle(tr("Hold Volume − and Power with the phone off, or reboot into the bootloader from here"));
            }
        }
        if !in_fb {
            *self.info.borrow_mut() = None;
            self.render_info();
        }
        self.update_flash_btn();
    }

    /// “Flash” is only enabled with a phone in fastboot, a valid partition and
    /// an existing image; the reason is shown as a tooltip.
    fn update_flash_btn(&self) {
        let part = self.part_entry.text().trim().to_string();
        let image_ok = self.image.borrow().as_ref().is_some_and(|p| p.is_file());
        let missing = if self.fb_device().is_none() {
            Some(tr("Connect the phone in fastboot mode"))
        } else if part.is_empty() || fastboot::validate_partition(&part).is_err() {
            Some(tr("Type a valid partition"))
        } else if !image_ok {
            Some(tr("Choose the image file"))
        } else if self.busy.borrow().is_some() {
            Some(tr("An operation is in progress"))
        } else {
            None
        };
        self.flash_btn.set_sensitive(missing.is_none());
        self.flash_btn.set_tooltip_text(Some(missing.unwrap_or(tr("Flash the image to the partition"))));
    }

    /// Compares fastboot's `product` with what ADB saw for this serial number
    /// and with what the ROM requires. `Some(false)` blocks flashing.
    fn identity_check(&self, required: &[String]) -> (Option<bool>, String) {
        let Some((_, serial)) = self.fb_device() else {
            return (None, String::new());
        };
        let product = self.info.borrow().as_ref().map(|i| i.product().to_string()).unwrap_or_default();
        if product.is_empty() {
            return (None, tr("⚠ Could not read the model (“getvar product”): check by hand that the image is for this phone.").into());
        }
        if !required.is_empty() && !fastboot::product_matches(&product, required) {
            return (Some(false), t!("✖ The ROM is for {} and the phone is a {}.", required.join(" / "), product));
        }
        let known = self.ctx.settings.borrow().device_ids.get(&serial).cloned().unwrap_or_default();
        if known.is_empty() {
            let rom = if required.is_empty() { String::new() } else { tr(" It matches what the ROM requires.").into() };
            return (None, t!("Model in fastboot: {}. ADB Scan has not seen this phone in Android yet, so it cannot compare it.{}", product, rom));
        }
        if fastboot::product_matches(&product, &known) {
            (Some(true), t!("✔ Model checked: {} (the same one ADB saw: {}).", product, known.join(", ")))
        } else {
            (Some(false), t!("✖ The phone in fastboot claims to be a {}, but ADB saw {} with this serial number.", product, known.join(", ")))
        }
    }

    /// Common warnings before writing partitions.
    fn flash_warnings(&self, partitions: &[&str], slot: Option<Slot>) -> String {
        let mut out = Vec::new();
        if partitions.iter().any(|p| fastboot::base_partition(p).starts_with("vbmeta")) {
            out.push(tr("⚠ vbmeta controls verified boot. A wrong image, or “--disable-verity/verification” with a locked bootloader, leaves the phone unable to boot.").to_string());
        }
        let active = self.info.borrow().as_ref().and_then(|i| i.current_slot());
        match (slot, active.as_deref()) {
            (Some(Slot::All), _) => out.push(tr("⚠ It will be written to BOTH slots: there will be no fallback slot to go back to.").into()),
            (Some(Slot::A), Some(a)) if a != "a" => out.push(t!("It will be written to slot A, which is NOT the active one ({}): the change will not be used until it is activated.", a.to_uppercase())),
            (Some(Slot::B), Some(a)) if a != "b" => out.push(t!("It will be written to slot B, which is NOT the active one ({}): the change will not be used until it is activated.", a.to_uppercase())),
            (None, Some(a)) => out.push(t!("It will be written to the active slot ({}).", a.to_uppercase())),
            _ => {}
        }
        out.push(tr("Before continuing: back up your data and keep the original image at hand so you can go back (partitions cannot be copied without root).").into());
        out.join("\n\n")
    }

    fn load_info(&self) {
        let Some((fb, serial)) = self.fb_device() else {
            return;
        };
        let w = self.me.clone();
        util::run_bg(
            move || fb.info(&serial),
            move |r| {
                let Some(t) = w.upgrade() else { return };
                match r {
                    Ok(info) => {
                        *t.info.borrow_mut() = Some(info);
                        t.render_info();
                        t.validate_part();
                    }
                    Err(e) => t.log(&t!("Could not read the variables: {}", e)),
                }
            },
        );
    }

    fn render_info(&self) {
        for r in self.info_rows.borrow_mut().drain(..) {
            self.info_group.remove(&r);
        }
        let info = self.info.borrow();
        let Some(info) = info.as_ref() else {
            self.slot_label.set_text("—");
            return;
        };
        let rows = [
            (tr("Product"), info.product().to_string()),
            (
                "Bootloader",
                match info.unlocked() {
                    Some(true) => tr("Unlocked").into(),
                    Some(false) => tr("Locked").into(),
                    None => "—".into(),
                },
            ),
            (tr("Mode"), if info.is_userspace() { tr("fastbootd (userspace)").into() } else { "bootloader".into() }),
            (tr("Active slot"), info.current_slot().unwrap_or("—".into())),
            (tr("Secure boot"), info.get("secure").to_string()),
            (tr("Bootloader version"), info.get("version-bootloader").to_string()),
            (tr("Baseband"), info.get("version-baseband").to_string()),
            (tr("Maximum download"), info.get("max-download-size").to_string()),
        ];
        let mut list = self.info_rows.borrow_mut();
        for (t, v) in rows {
            if v.is_empty() {
                continue;
            }
            let (row, l) = util::info_row(t);
            l.set_text(&self.ctx.shown(&v));
            self.info_group.add(&row);
            list.push(row.upcast());
        }
        self.slot_label.set_text(&info.current_slot().map(|s| s.to_uppercase()).unwrap_or("—".into()));
    }

    fn validate_part(&self) {
        self.update_flash_btn();
        let p = self.part_entry.text().trim().to_string();
        if p.is_empty() {
            self.part_hint.set_text("");
            return;
        }
        match fastboot::validate_partition(&p) {
            Err(e) => self.part_hint.set_text(&e.to_string()),
            Ok(()) => {
                let mut text = match fastboot::required_confirmation(&p) {
                    Confirm::Critical => tr("⚠ Critical boot or modem partition: a mistake can brick the phone.").to_string(),
                    Confirm::Destructive => tr("⚠ Erases user data.").to_string(),
                    Confirm::None => tr("Valid partition.").to_string(),
                };
                if let Some(info) = self.info.borrow().as_ref() {
                    let slot = info.current_slot().map(|s| format!("{p}_{s}")).unwrap_or(p.clone());
                    if let Some(size) = info.partition_size(&slot).or(info.partition_size(&p)) {
                        text.push_str(&t!(" Partition size: {}.", format::human_bytes(Some(size as f64))));
                        if let Some(img) = self.image.borrow().as_ref().and_then(|i| std::fs::metadata(i).ok()) {
                            if img.len() > size {
                                text.push_str(tr(" ⚠ The image is LARGER than the partition."));
                            }
                        }
                    }
                }
                self.part_hint.set_text(&text);
            }
        }
    }

    fn selected_slot(&self) -> Option<Slot> {
        match self.slot_combo.selected() {
            1 => Some(Slot::A),
            2 => Some(Slot::B),
            3 => Some(Slot::All),
            _ => None,
        }
    }

    fn run_stream<F>(&self, label: &str, start: F)
    where
        F: FnOnce(Box<dyn Fn(StreamEvent) + Send + Sync>) -> adbscan_core::Result<StreamHandle>,
    {
        if self.busy.borrow().is_some() {
            self.ctx.toast(tr("An operation is in progress"));
            return;
        }
        self.log(&format!("▶ {label}"));
        let (tx, rx) = async_channel::unbounded::<StreamEvent>();
        match start(Box::new(move |ev| {
            let _ = tx.send_blocking(ev);
        })) {
            Ok(h) => {
                *self.busy.borrow_mut() = Some(h);
                self.update_flash_btn();
                let w = self.me.clone();
                let label = label.to_string();
                glib::spawn_future_local(async move {
                    while let Ok(ev) = rx.recv().await {
                        let Some(t) = w.upgrade() else { break };
                        match ev {
                            StreamEvent::Stdout(l) | StreamEvent::Stderr(l) => t.log(&l),
                            StreamEvent::Exit { code, .. } => {
                                t.busy.borrow_mut().take();
                                t.update_flash_btn();
                                let ok = code == Some(0);
                                t.log(if ok { tr("✔ Done") } else { tr("✖ Failed") });
                                t.ctx.audit("fastboot", Some(&label), ok, None);
                                t.ctx.toast(if ok { tr("Operation complete") } else { tr("The operation failed: check the log") });
                                t.load_info();
                                break;
                            }
                        }
                    }
                });
            }
            Err(e) => self.log(&format!("✖ {e}")),
        }
    }

    fn flash_single(&self) {
        let Some((fb, serial)) = self.fb_device() else {
            return;
        };
        let part = self.part_entry.text().trim().to_string();
        let Some(image) = self.image.borrow().clone() else {
            self.ctx.toast(tr("Choose an image"));
            return;
        };
        if let Err(e) = fastboot::validate_partition(&part) {
            self.ctx.toast(&e.to_string());
            return;
        }
        let slot = self.selected_slot();
        let needed = fastboot::required_confirmation(&part);
        let preview = match fb.flash_args(&part, &image, slot, Confirm::Destructive) {
            Ok(a) => format!("fastboot -s {serial} {}", a.join(" ")),
            Err(e) => {
                self.ctx.toast(&e.to_string());
                return;
            }
        };
        let (matches, identity) = self.identity_check(&[]);
        if matches == Some(false) {
            util::show_error(&self.root, tr("The model does not match"), &t!("{}\n\nNothing will be flashed: an image for another model can brick the phone.", identity));
            self.log(&t!("✖ Flashing cancelled: {}", identity));
            return;
        }
        let image_desc =
            format!("{} ({})", image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), format::human_bytes(std::fs::metadata(&image).ok().map(|m| m.len() as f64)));
        let danger = match needed {
            Confirm::Critical => tr("⚠ CRITICAL boot or modem partition: a mistake can brick the phone.\n\n"),
            Confirm::Destructive => tr("⚠ This partition contains your data: it will be erased.\n\n"),
            Confirm::None => "",
        };
        let body = t!("{}Image: {}\nPartition: {}\n{}\n\n{}\n\n{}\n\nType the serial number ({}) to confirm.", danger, image_desc, part, preview, identity, self.flash_warnings(&[part.as_str()], slot), serial);
        let w = self.me.clone();
        let detail = format!("{part} ← {image_desc}");
        let typed = serial.clone();
        let heading = t!("Flash “{}”?", part);
        let go = move || {
            let Some(t) = w.upgrade() else { return };
            t.ctx.audit("flash_request", Some(&detail), true, None);
            let (fb, serial, part, image) = (fb.clone(), serial.clone(), part.clone(), image.clone());
            t.run_stream(&format!("flash {detail}"), move |cb| fb.flash(&serial, &part, &image, slot, Confirm::Destructive, cb));
        };
        util::confirm_typed(&self.root, &heading, &body, &typed, tr("Flash"), self.masked(&body), go);
    }

    fn boot_image(&self) {
        let Some((fb, serial)) = self.fb_device() else {
            return;
        };
        let Some(image) = self.image.borrow().clone() else {
            self.ctx.toast(tr("Choose a boot image"));
            return;
        };
        self.run_stream(tr("boot (temporary)"), move |cb| fb.boot(&serial, &image, cb));
    }

    fn load_plan(&self, dir: PathBuf) {
        match fastboot::scan_rom_folder(&dir) {
            Ok(mut plan) => {
                for info in ["android-info.txt", "images/android-info.txt"] {
                    if let Ok(text) = std::fs::read_to_string(dir.join(info)) {
                        for p in fastboot::parse_android_info(&text) {
                            if !plan.required_products.contains(&p) {
                                plan.required_products.push(p);
                            }
                        }
                    }
                }
                while let Some(c) = self.plan_list.first_child() {
                    self.plan_list.remove(&c);
                }
                let head = adw::ActionRow::builder()
                    .title(t!("{} steps · {}", plan.steps.len(), format::human_bytes(Some(plan.total_bytes() as f64))))
                    .subtitle(glib::markup_escape_text(&t!("Source: {}{}{}", plan.source, if plan.required_products.is_empty() { String::new() } else { t!(" · for {}", plan.required_products.join(" / ")) }, if plan.wipes_data { tr(" · ERASES THE DATA") } else { "" })))
                    .build();
                self.plan_list.append(&head);
                for w in &plan.warnings {
                    let r = adw::ActionRow::builder().title(glib::markup_escape_text(w)).build();
                    r.add_prefix(&util::icon("adb-dialog-warning-symbolic"));
                    self.plan_list.append(&r);
                }
                for s in &plan.steps {
                    let mut sub = format!("{} · {}", s.image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), format::human_bytes(Some(s.size as f64)));
                    if !s.extra_args.is_empty() {
                        sub.push_str(&format!(" · {}", s.extra_args.join(" ")));
                    }
                    if !s.exists {
                        sub = t!("MISSING · {}", sub);
                    }
                    let r = adw::ActionRow::builder().title(glib::markup_escape_text(&s.partition)).subtitle(glib::markup_escape_text(&sub)).build();
                    let icon = if !s.exists {
                        "adb-circle-x-symbolic"
                    } else if s.critical {
                        "adb-dialog-warning-symbolic"
                    } else {
                        "adb-emblem-ok-symbolic"
                    };
                    r.add_prefix(&util::icon(icon));
                    self.plan_list.append(&r);
                }
                let runnable = plan.steps.iter().any(|s| s.exists);
                self.plan_btn.set_sensitive(runnable);
                self.log(&t!("Plan loaded from {}", dir.display()));
                *self.plan.borrow_mut() = Some(plan);
            }
            Err(e) => self.ctx.toast(&e.to_string()),
        }
    }

    fn confirm_plan(&self) {
        let Some(plan) = self.plan.borrow().clone() else {
            return;
        };
        let Some((_, serial)) = self.fb_device() else {
            return;
        };
        let (matches, identity) = self.identity_check(&plan.required_products);
        if matches == Some(false) {
            util::show_error(&self.root, tr("The model does not match"), &t!("{}\n\nNothing will be flashed.", identity));
            self.log(&t!("✖ Plan cancelled: {}", identity));
            return;
        }
        let critical: Vec<String> = plan.steps.iter().filter(|s| s.critical).map(|s| s.partition.clone()).collect();
        let mut body = t!("{} partitions will be flashed ({}).", plan.steps.iter().filter(|s| s.exists).count(), format::human_bytes(Some(plan.total_bytes() as f64)));
        if !critical.is_empty() {
            body.push_str(&t!("\n\nIncludes critical partitions: {}.", critical.join(", ")));
        }
        if plan.wipes_data {
            body.push_str(tr("\n\nThe original script also ERASES THE DATA (done at the end)."));
        }
        let missing = plan.steps.iter().filter(|s| !s.exists).count();
        if missing > 0 {
            body.push_str(&t!("\n\n{} steps whose images are missing will be skipped.", missing));
        }
        let parts: Vec<&str> = plan.steps.iter().filter(|s| s.exists).map(|s| s.partition.as_str()).collect();
        body.push_str(&format!("\n\n{identity}\n\n{}", self.flash_warnings(&parts, None)));
        body.push_str(&t!("\n\nType the serial number ({}) to confirm.", serial));
        let w = self.me.clone();
        let masked = self.masked(&body);
        util::confirm_typed(&self.root, tr("Run the flashing plan?"), &body, &serial, tr("Flash all"), masked, move || {
            if let Some(t) = w.upgrade() {
                t.run_plan(plan.clone());
            }
        });
    }

    fn run_plan(&self, plan: FlashPlan) {
        let Some((fb, serial)) = self.fb_device() else {
            return;
        };
        let steps: Vec<_> = plan.steps.iter().filter(|s| s.exists).cloned().collect();
        let total = steps.len();
        self.plan_progress.set_visible(true);
        self.plan_btn.set_sensitive(false);
        let (tx, rx) = async_channel::unbounded::<(usize, String, Option<bool>)>();
        let wipes = plan.wipes_data;
        let set_active = plan.set_active.clone();
        std::thread::spawn(move || {
            for (i, s) in steps.iter().enumerate() {
                let _ = tx.send_blocking((i, format!("▶ flash {} ← {}", s.partition, s.image.display()), None));
                let mut args: Vec<String> = s.extra_args.clone();
                match fb.flash_args(&s.partition, &s.image, None, Confirm::Destructive) {
                    Ok(a) => args.extend(a),
                    Err(e) => {
                        let _ = tx.send_blocking((i, format!("✖ {e}"), Some(false)));
                        return;
                    }
                }
                let (dtx, drx) = std::sync::mpsc::channel();
                let tx2 = tx.clone();
                let handle = fb.stream(&serial, &args, move |ev| match ev {
                    StreamEvent::Stdout(l) | StreamEvent::Stderr(l) => {
                        let _ = tx2.send_blocking((i, l, None));
                    }
                    StreamEvent::Exit { code, .. } => {
                        let _ = dtx.send(code == Some(0));
                    }
                });
                let ok = handle.is_ok() && drx.recv().unwrap_or(false);
                if !ok {
                    let _ = tx.send_blocking((i, t!("✖ Failed at {}; the plan stops", s.partition), Some(false)));
                    return;
                }
            }
            if wipes {
                let _ = tx.send_blocking((total, "▶ fastboot -w".into(), None));
                if let Err(e) = fb.wipe_data(&serial, Confirm::Destructive) {
                    let _ = tx.send_blocking((total, format!("✖ {e}"), Some(false)));
                    return;
                }
            }
            if let Some(slot) = set_active {
                let s = if slot == "b" { Slot::B } else { Slot::A };
                let _ = fb.set_active(&serial, s);
            }
            let _ = tx.send_blocking((total, tr("✔ Plan complete. You can now reboot into the system.").into(), Some(true)));
        });
        let w = self.me.clone();
        glib::spawn_future_local(async move {
            while let Ok((i, line, done)) = rx.recv().await {
                let Some(t) = w.upgrade() else { break };
                t.log(&line);
                t.plan_progress.set_fraction(i as f64 / total.max(1) as f64);
                t.plan_progress.set_text(Some(&t!("{} of {}", i.min(total), total)));
                if let Some(ok) = done {
                    t.plan_btn.set_sensitive(true);
                    t.ctx.audit("flash_plan", Some(&t!("{} steps", total)), ok, Some(&line));
                    t.ctx.toast(if ok { tr("Plan complete") } else { tr("The plan stopped because of an error") });
                    if ok {
                        t.plan_progress.set_fraction(1.0);
                    }
                    break;
                }
            }
        });
    }

    fn set_slot(&self, slot: Slot) {
        let Some((fb, serial)) = self.fb_device() else {
            return;
        };
        let w = self.me.clone();
        util::run_bg(
            move || fb.set_active(&serial, slot),
            move |r| {
                let Some(t) = w.upgrade() else { return };
                match r {
                    Ok(_) => {
                        t.log(&t!("✔ Slot {} activated", slot.arg().to_uppercase()));
                        t.ctx.audit("set_active", Some(slot.arg()), true, None);
                        t.load_info();
                    }
                    Err(e) => t.log(&format!("✖ {e}")),
                }
            },
        );
    }

    fn sideload(&self, zip: PathBuf) {
        let Some(adb) = self.ctx.adb() else { return };
        let Some(dev) = self.ctx.selected_device().filter(|d| d.state == DeviceState::Sideload) else {
            self.ctx.toast(tr("The phone is not in sideload mode"));
            return;
        };
        let serial = dev.serial;
        self.run_stream(&format!("sideload {}", zip.display()), move |cb| adb.sideload(&serial, &zip, cb));
    }

    fn reboot_fb(&self, target: &str) {
        let Some((fb, serial)) = self.fb_device() else {
            return;
        };
        let target = target.to_string();
        let w = self.me.clone();
        util::run_bg(
            move || match target.as_str() {
                "continue" => fb.continue_boot(&serial),
                "system" => fb.reboot(&serial, None),
                t => fb.reboot(&serial, Some(t)),
            },
            move |r| {
                if let Some(t) = w.upgrade() {
                    t.log(&match r {
                        Ok(_) => tr("✔ Restarting").to_string(),
                        Err(e) => format!("✖ {e}"),
                    });
                }
            },
        );
    }

    fn simple_fb(&self, what: &str) {
        let Some((fb, serial)) = self.fb_device() else {
            return;
        };
        let cmd = what.trim_start_matches("oem ").to_string();
        let w = self.me.clone();
        util::run_bg(
            move || fb.oem_info(&serial, &cmd),
            move |r| {
                if let Some(t) = w.upgrade() {
                    t.log(&match r {
                        Ok(r) => r.combined(),
                        Err(e) => format!("✖ {e}"),
                    });
                }
            },
        );
    }

    fn guarded(&self, what: &'static str) {
        let Some((fb, serial)) = self.fb_device() else {
            return;
        };
        let (title, body) = match what {
            "wipe" => (tr("Erase all data?"), tr("Apps, photos, accounts and settings will be deleted from the phone. This cannot be undone.")),
            "unlock" => (
                tr("Unlock the bootloader?"),
                tr("ALL DATA WILL BE ERASED. The phone will show a warning on every boot, some apps (banking, payments) may stop working and the warranty may be affected. On many models you must first turn on “OEM unlocking” in Developer options and confirm with the phone's buttons."),
            ),
            _ => (
                tr("Lock the bootloader?"),
                tr("ALL DATA WILL BE ERASED. Lock only with the original factory firmware: locking with a modified system can stop the phone from booting."),
            ),
        };
        let w = self.me.clone();
        let s2 = serial.clone();
        let body = t!("{}\n\nType the serial number ({}) to confirm.", body, serial);
        let masked = self.masked(&body);
        util::confirm_typed(&self.root, title, &body, &serial, tr("Continue"), masked, move || {
            let Some(t) = w.upgrade() else { return };
            let (fb, serial) = (fb.clone(), s2.clone());
            t.log(&format!("▶ {what}"));
            let w2 = t.me.clone();
            util::run_bg(
                move || match what {
                    "wipe" => fb.wipe_data(&serial, Confirm::Destructive),
                    "unlock" => fb.flashing(&serial, true, Confirm::Destructive),
                    _ => fb.flashing(&serial, false, Confirm::Destructive),
                },
                move |r| {
                    let Some(t) = w2.upgrade() else { return };
                    t.ctx.audit(what, None, r.is_ok(), None);
                    t.log(&match r {
                        Ok(r) => t!("{}\n✔ Command sent. Follow the instructions on the phone's screen.", r.combined()),
                        Err(e) => format!("✖ {e}"),
                    });
                },
            );
        });
    }
}

impl Page for FlasherPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn on_event(&self, ev: &Event) {
        if matches!(ev, Event::Devices | Event::Tools | Event::Privacy) {
            self.refresh_state();
        }
        if matches!(ev, Event::Privacy) {
            self.render_info();
            let lines = self.log_lines.borrow().clone();
            let text: String = lines.iter().map(|l| if l.ends_with('\n') { self.ctx.shown(l) } else { format!("{}\n", self.ctx.shown(l)) }).collect();
            self.log.buffer().set_text(&text);
        }
    }
    fn shown(&self) {
        self.refresh_state();
    }
}
