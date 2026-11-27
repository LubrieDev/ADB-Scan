//! Root: what this phone needs to be rooted, its state and the steps with
//! Magisk (see `adbscan_core::root`). Reading changes nothing. The steps that
//! touch the phone (installing Magisk, copying images, restarting to the
//! bootloader) each ask first; unlocking and flashing happen in the Flasher,
//! with its own safeguards.

use adbscan_core::{format, i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use adbscan_core::adb::{CancelToken, RebootMode};
use adbscan_core::root::{self, Bootloader, Difficulty, MagiskRelease, RootReport};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

struct RootPage {
    me: Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    status: adw::ActionRow,
    facts: adw::PreferencesGroup,
    fact_rows: RefCell<Vec<gtk::Widget>>,
    guide: adw::ActionRow,
    guide_group: adw::PreferencesGroup,
    /// “Steps from here” and its fastboot buttons (hidden where there is no
    /// fastboot or no unlock).
    unlock: adw::ActionRow,
    fastboot_buttons: [gtk::Button; 2],
    /// The Magisk steps: hidden on a locked phone that cannot be unlocked.
    steps: adw::PreferencesGroup,
    image_step: adw::ActionRow,
    magisk_step: adw::ActionRow,
    flash_step: adw::ActionRow,
    report: RefCell<Option<RootReport>>,
    read_for: RefCell<Option<String>>,
    magisk: RefCell<Option<MagiskRelease>>,
    patched: RefCell<Option<std::path::PathBuf>>,
    busy: Cell<bool>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    RootPage::new(ctx)
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn button(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("pill");
    b.set_valign(gtk::Align::Center);
    b
}

fn step(group: &adw::PreferencesGroup, n: u32, title: &str, subtitle: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(format!("{n}. {}", esc(title))).subtitle(esc(subtitle)).build();
    row.set_subtitle_lines(0);
    group.add(&row);
    row
}

impl RootPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(24);

        let intro = adw::PreferencesGroup::builder()
            .title(tr("Root"))
            .description(tr("Root gives apps full control of the phone. Do it only on phones that are yours or whose owner agrees: unlocking the bootloader ERASES everything on the phone, the warranty may be lost, and banking, payment and some game apps may stop working."))
            .build();
        let refresh = util::icon_button("adb-refresh-cw-symbolic", tr("Read again"));
        refresh.add_css_class("flat");
        intro.set_header_suffix(Some(&refresh));
        let status = adw::ActionRow::builder().title(tr("Root on this phone")).subtitle(tr("reading…")).build();
        status.add_prefix(&util::icon("adb-shield-symbolic"));
        intro.add(&status);
        content.append(&intro);

        let facts = adw::PreferencesGroup::builder()
            .title(tr("What rooting needs to know"))
            .description(tr("The exact build matters: the image to patch must come from the same firmware the phone runs now."))
            .build();
        content.append(&facts);

        let guide_group = adw::PreferencesGroup::builder().title(tr("Unlocking the bootloader")).build();
        let guide = adw::ActionRow::builder().title("—").build();
        guide.set_subtitle_lines(0);
        guide.add_prefix(&util::icon("adb-key-round-symbolic"));
        guide_group.add(&guide);
        let unlock = adw::ActionRow::builder()
            .title(tr("Steps from here"))
            .subtitle(tr("Turn on “OEM unlocking” on the phone, restart it to the bootloader and unlock it in the Flasher (it asks you to type the serial number)."))
            .build();
        unlock.set_subtitle_lines(0);
        let dev = button(tr("Developer options"));
        dev.set_tooltip_text(Some(tr("Opens Developer options on the phone")));
        let to_bl = button(tr("Restart to bootloader"));
        let flasher = button(tr("Open Flasher"));
        let unlock_box = util::hbox(6);
        unlock_box.set_valign(gtk::Align::Center);
        unlock_box.append(&dev);
        unlock_box.append(&to_bl);
        unlock_box.append(&flasher);
        unlock.add_suffix(&unlock_box);
        guide_group.add(&unlock);
        content.append(&guide_group);

        let steps = adw::PreferencesGroup::builder()
            .title(tr("Root with Magisk"))
            .description(tr("Magisk patches the boot image on the phone itself; the patched image is then flashed from the bootloader. The phone's data is not touched by these steps (only unlocking erases it)."))
            .build();
        let magisk_page = gtk::LinkButton::with_label(root::MAGISK_PAGE, tr("About Magisk"));
        steps.set_header_suffix(Some(&magisk_page));
        let backup = step(&steps, 1, tr("Back up first"), tr("Unlocking erases the phone: copy photos, contacts and apps (Phone data → private view)."));
        let to_data = button(tr("Open Phone data"));
        backup.add_suffix(&to_data);
        let image_step = step(&steps, 2, tr("The stock boot image"), "");
        let send = button(tr("Send to the phone…"));
        image_step.add_suffix(&send);
        let magisk_step = step(&steps, 3, tr("Install Magisk"), tr("Downloads the latest Magisk from its official repository (github.com/topjohnwu/Magisk), checks its SHA-256 sum and installs it on the phone."));
        let install = button(tr("Download and install"));
        magisk_step.add_suffix(&install);
        let patch = step(&steps, 4, tr("Patch it on the phone"), tr("In Magisk: Install → Select and patch a file → choose the image in Download. Magisk saves magisk_patched-….img next to it."));
        let open_magisk = button(tr("Open Magisk"));
        patch.add_suffix(&open_magisk);
        let get = step(&steps, 5, tr("Bring the patched image here"), tr("Copies the newest magisk_patched-….img from the phone's Download folder to this computer."));
        let fetch = button(tr("Get it…"));
        get.add_suffix(&fetch);
        let flash_step = step(&steps, 6, tr("Flash it"), "");
        let to_flash = button(tr("Prepare in Flasher"));
        flash_step.add_suffix(&to_flash);
        let verify = step(&steps, 7, tr("Check"), tr("After restarting, open Magisk: it should show its version under “Magisk”. Read this page again to see the root."));
        let _ = verify;
        content.append(&steps);

        let root = util::page_scroller(&content, 1000);
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            status,
            facts,
            fact_rows: RefCell::new(Vec::new()),
            guide,
            guide_group,
            unlock: unlock.clone(),
            fastboot_buttons: [to_bl.clone(), flasher.clone()],
            steps: steps.clone(),
            image_step,
            magisk_step,
            flash_step,
            report: RefCell::new(None),
            read_for: RefCell::new(None),
            magisk: RefCell::new(None),
            patched: RefCell::new(None),
            busy: Cell::new(false),
        });

        macro_rules! on {
            ($w:expr, |$t:ident, $b:ident| $body:expr) => {{
                let w = this.me.clone();
                $w.connect_clicked(move |$b| {
                    if let Some($t) = w.upgrade() {
                        $body;
                    }
                });
            }};
        }
        on!(refresh, |t, _b| t.load());
        on!(dev, |t, _b| t.open_developer());
        on!(to_bl, |t, b| t.reboot_bootloader(b));
        on!(flasher, |t, _b| t.ctx.emit(Event::ShowPage("flasher")));
        on!(to_data, |t, _b| t.ctx.emit(Event::ShowPage("data")));
        on!(send, |t, b| t.send_image(b));
        on!(install, |t, b| t.install_magisk(b));
        on!(open_magisk, |t, _b| t.open_magisk());
        on!(fetch, |t, b| t.fetch_patched(b));
        on!(to_flash, |t, b| t.prepare_flash(b));
        this
    }

    fn device(&self) -> Option<(adbscan_core::adb::Adb, String)> {
        let r = self.ctx.adb().zip(self.ctx.ready_serial());
        if r.is_none() {
            self.ctx.toast(tr("No device is ready"));
        }
        r
    }

    fn load(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            self.status.set_subtitle(tr("Connect a phone with USB debugging to read it."));
            return;
        };
        *self.read_for.borrow_mut() = Some(serial.clone());
        self.status.set_subtitle(tr("reading…"));
        let w = self.me.clone();
        util::run_bg(
            move || adb.root_report(&serial),
            move |r| {
                let Some(t) = w.upgrade() else { return };
                match r {
                    Ok(rep) => {
                        *t.report.borrow_mut() = Some(rep);
                        t.render();
                    }
                    Err(e) => t.status.set_subtitle(&t!("Could not read it: {}", e)),
                }
            },
        );
    }

    fn render(&self) {
        let rep = self.report.borrow();
        let Some(r) = rep.as_ref() else { return };
        let ctx = &self.ctx;

        // Status.
        let managers: Vec<String> = r.managers.iter().map(|(m, v)| match v {
            Some(v) => format!("{} {v}", m.name()),
            None => m.name().to_string(),
        }).collect();
        self.status.set_subtitle(&match (r.rooted(), managers.is_empty()) {
            (true, false) => t!("Rooted · {}", managers.join(", ")),
            (true, true) => tr("Rooted (a “su” command is present)").to_string(),
            (false, false) => t!("Not rooted yet · installed: {}", managers.join(", ")),
            (false, true) => tr("Not rooted").to_string(),
        });

        // Facts.
        for w in self.fact_rows.borrow_mut().drain(..) {
            self.facts.remove(&w);
        }
        let mut rows = self.fact_rows.borrow_mut();
        let mut add = |title: &str, value: String, note: &str| {
            let (row, label) = util::info_row(title);
            label.set_text(&ctx.shown(if value.is_empty() { "—" } else { &value }));
            if !note.is_empty() {
                row.set_subtitle(note);
            }
            self.facts.add(&row);
            rows.push(row.upcast());
        };
        let p = |k: &str| r.prop(k).to_string();
        add(tr("Phone"), format!("{} {} ({})", p("ro.product.manufacturer"), p("ro.product.model"), p("ro.product.device")), tr("The codename (in brackets) is what firmware and guides use"));
        let android = format!("Android {} (API {})", p("ro.build.version.release"), p("ro.build.version.sdk"));
        add("Android", android, "");
        add(tr("Security patch"), p("ro.build.version.security_patch"), "");
        add(tr("Build"), p("ro.build.display.id"), tr("Look for the firmware of exactly this build"));
        add(tr("Build fingerprint"), p("ro.build.fingerprint"), "");
        let bl = match r.bootloader() {
            Bootloader::Locked => tr("Locked").to_string(),
            Bootloader::Unlocked => tr("Unlocked").to_string(),
            Bootloader::Unknown => tr("Unknown").to_string(),
        };
        let vb = p("ro.boot.verifiedbootstate");
        add(tr("Bootloader"), if vb.is_empty() { bl } else { format!("{bl} · {}", t!("verified boot {}", vb)) }, "");
        add(
            tr("OEM unlocking"),
            match r.oem_unlock_allowed() {
                Some(true) => tr("Allowed (the switch in Developer options is on)").to_string(),
                Some(false) => tr("Not allowed: turn it on in Developer options").to_string(),
                None => tr("This phone does not say: look in Developer options").to_string(),
            },
            "",
        );
        add(
            tr("Partition to patch"),
            format!("{}.img", r.target_partition()),
            if r.target_partition() == "init_boot" { tr("This phone has init_boot (it launched with Android 13 or later): Magisk patches init_boot, not boot") } else { tr("Magisk patches boot") },
        );
        add(
            tr("A/B slots"),
            match (r.ab(), r.slot()) {
                (true, Some(s)) => t!("Yes · active slot {}", s),
                (true, None) => tr("Yes").to_string(),
                _ => tr("No").to_string(),
            },
            if r.ab() { tr("The image is flashed to the active slot") } else { "" },
        );
        let gki = r.gki().map(|g| format!(" · GKI {g}")).unwrap_or_default();
        add(
            tr("Kernel"),
            format!("{}{gki}", r.kernel),
            if r.kernelsu_gki_ok() { tr("GKI 5.10 or newer: KernelSU and APatch are an option besides Magisk") } else { tr("Not a GKI 5.10+ kernel: Magisk is the usual choice") },
        );
        add(tr("Architecture"), p("ro.product.cpu.abi"), if r.is_32bit() { tr("32-bit system (Android Go): Magisk supports it; KernelSU and APatch do not") } else { "" });
        add(tr("Chip"), r.chip(), if r.unisoc() { tr("Unisoc: manufacturers rarely give unlock codes for these chips") } else { "" });
        add(tr("Build type"), format!("{} · {}", p("ro.build.type"), p("ro.build.tags")), "");
        drop(rows);

        // Unlock guide.
        let g = r.guide();
        let brand = if g.brand.is_empty() { p("ro.product.manufacturer") } else { g.brand.to_string() };
        self.guide.set_title(&esc(&format!("{brand} · {}", g.difficulty.label())));
        let mut how = g.how.to_string();
        if r.unisoc() {
            how.push_str("\n\n");
            how.push_str(tr("This phone has a Unisoc chip: the manufacturer's unlock programme usually leaves them out. Asking for the code (Flasher → Unlock data) is the safe way to know. Other methods exploit flaws of the chip and can leave the phone unusable."));
        }
        self.guide.set_subtitle(&esc(&how));
        let mut desc = match r.bootloader() {
            Bootloader::Unlocked => tr("This phone's bootloader is already unlocked: skip to the Magisk steps.").to_string(),
            _ if g.difficulty == Difficulty::NotPossible => tr("This brand does not allow it: root is not possible without an exploit.").to_string(),
            _ => tr("Required before any root. It erases the phone.").to_string(),
        };
        // Connected over Wi-Fi: fastboot needs the cable.
        if self.ctx.ready_serial().is_some_and(|s| s.contains(':')) {
            desc.push(' ');
            desc.push_str(tr("The phone is connected over Wi-Fi: the bootloader (fastboot) only works over a USB cable, so connect it before restarting."));
        }
        self.guide_group.set_description(Some(&desc));
        // Without fastboot (Samsung) the unlocking is not done from here, and
        // where it is not possible there are no steps at all.
        self.unlock.set_visible(g.difficulty != Difficulty::NotPossible && r.bootloader() != Bootloader::Unlocked);
        self.steps.set_visible(g.difficulty != Difficulty::NotPossible || r.bootloader() == Bootloader::Unlocked);
        for b in &self.fastboot_buttons {
            b.set_visible(g.fastboot);
        }
        self.unlock.set_subtitle(&esc(if g.fastboot {
            tr("Turn on “OEM unlocking” on the phone, restart it to the bootloader and unlock it in the Flasher (it asks you to type the serial number).")
        } else {
            tr("Turn on “OEM unlocking” on the phone. The unlocking itself is done on the phone in download mode, not from here.")
        }));

        // Steps that depend on the phone.
        let target = r.target_partition();
        self.image_step.set_subtitle(&esc(&t!(
            "{}.img from the official firmware of build {}: in Pixel factory images it is inside the zip; in most others, inside payload.bin of the full OTA. Send it to the phone's Download folder.",
            target,
            p("ro.build.display.id")
        )));
        self.flash_step.set_subtitle(&esc(&if g.fastboot {
            t!("Opens the Flasher with partition “{}” and the patched image chosen; there you restart to the bootloader and confirm. Keep the stock image: flashing it back undoes the root.", target)
        } else {
            tr("This brand has no fastboot: the patched image is flashed with the manufacturer's tool (Odin for Samsung), not from here.").to_string()
        }));
    }

    fn open_developer(&self) {
        let Some((adb, serial)) = self.device() else { return };
        let ctx = self.ctx.clone();
        util::run_bg(
            move || adb.control_action(&serial, adbscan_core::controls::Action::OpenSettings("android.settings.APPLICATION_DEVELOPMENT_SETTINGS")),
            move |r| {
                if let Err(e) = r {
                    ctx.toast(&t!("Could not open it: {}", e));
                }
            },
        );
    }

    fn reboot_bootloader(&self, b: &gtk::Button) {
        let w = self.me.clone();
        util::confirm(b, tr("Restart to the bootloader?"), tr("The phone restarts into the bootloader (fastboot) and the connection over ADB ends. To leave it, choose Start on the phone or use the Flasher."), tr("Restart"), false, move || {
            let Some(t) = w.upgrade() else { return };
            let Some((adb, serial)) = t.device() else { return };
            let ctx = t.ctx.clone();
            util::run_bg(
                move || adb.reboot(&serial, RebootMode::Bootloader),
                move |r| {
                    ctx.audit("reboot_bootloader", None, r.is_ok(), r.as_ref().err().map(|e| e.to_string()).as_deref());
                    match r {
                        Ok(_) => ctx.emit(Event::ShowPage("flasher")),
                        Err(e) => ctx.toast(&t!("Could not restart it: {}", e)),
                    }
                },
            );
        });
    }

    fn send_image(&self, b: &gtk::Button) {
        let w = self.me.clone();
        util::pick_file(b, tr("Stock boot image"), &[(tr("Images"), &["*.img"])], move |path| {
            let Some(t) = w.upgrade() else { return };
            let Some((adb, serial)) = t.device() else { return };
            let Some(name) = path.file_name().and_then(|n| n.to_str()).filter(|n| n.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))).map(str::to_string) else {
                t.ctx.toast(tr("Rename the file with only letters, digits, “.”, “_” or “-”"));
                return;
            };
            let remote = format!("{}/{name}", root::PHONE_DOWNLOADS);
            t.ctx.toast(&t!("Sending {}…", name));
            let ctx = t.ctx.clone();
            util::run_bg(
                move || adb.push(&serial, &path, &remote, &CancelToken::new(), |_| {}),
                move |r| match r {
                    Ok(_) => ctx.toast(&t!("{} is in the phone's Download folder", name)),
                    Err(e) => ctx.toast(&t!("Could not send it: {}", e)),
                },
            );
        });
    }

    fn install_magisk(&self, b: &gtk::Button) {
        if self.busy.replace(true) {
            return;
        }
        b.set_sensitive(false);
        self.magisk_step.set_subtitle(tr("Asking GitHub for the latest Magisk…"));
        let w = self.me.clone();
        let b = b.clone();
        util::run_bg(root::latest_magisk, move |r| {
            let Some(t) = w.upgrade() else { return };
            t.busy.set(false);
            b.set_sensitive(true);
            match r {
                Ok(rel) => {
                    let size = format::human_bytes(Some(rel.apk.size as f64));
                    t.magisk_step.set_subtitle(&esc(&t!("Latest: Magisk {} ({}), from github.com/topjohnwu/Magisk", rel.tag, size)));
                    *t.magisk.borrow_mut() = Some(rel.clone());
                    let w = t.me.clone();
                    util::confirm(
                        &b,
                        &t!("Install Magisk {}?", rel.tag),
                        &t!("{} ({}) is downloaded from Magisk's official releases, checked against the SHA-256 sum GitHub publishes and installed on the phone. Installing the app does not root the phone by itself.", rel.apk.name, size),
                        tr("Install"),
                        false,
                        move || {
                            if let Some(t) = w.upgrade() {
                                t.download_and_install(rel.clone());
                            }
                        },
                    );
                }
                Err(e) => t.magisk_step.set_subtitle(&esc(&e.to_string())),
            }
        });
    }

    fn download_and_install(&self, rel: MagiskRelease) {
        let Some((adb, serial)) = self.device() else { return };
        self.magisk_step.set_subtitle(&esc(&t!("Downloading {}…", rel.apk.name)));
        let w = self.me.clone();
        let ctx = self.ctx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        util::run_bg(
            move || {
                let apk = root::download_magisk(&rel, &|_, _| {}, &cancel)?;
                let opts = adbscan_core::adb::InstallOptions { replace: true, ..Default::default() };
                adb.install(&serial, &[apk], &opts)?;
                Ok::<_, adbscan_core::Error>(rel.tag)
            },
            move |r| {
                let target = "com.topjohnwu.magisk";
                match &r {
                    Ok(tag) => {
                        ctx.audit("install_magisk", Some(&format!("{target} {tag}")), true, None);
                        ctx.toast(&t!("Magisk {} installed", tag));
                    }
                    Err(e) => {
                        ctx.audit("install_magisk", Some(target), false, Some(&e.to_string()));
                        ctx.toast(&t!("Could not install it: {}", e));
                    }
                }
                if let Some(t) = w.upgrade() {
                    t.load();
                }
            },
        );
    }

    fn open_magisk(&self) {
        let Some((adb, serial)) = self.device() else { return };
        let ctx = self.ctx.clone();
        util::run_bg(
            move || adb.launch(&serial, "com.topjohnwu.magisk"),
            move |r| {
                if r.is_err() {
                    ctx.toast(tr("Magisk is not installed on the phone (step 3)"));
                }
            },
        );
    }

    fn fetch_patched(&self, b: &gtk::Button) {
        let Some((adb, serial)) = self.device() else { return };
        let w = self.me.clone();
        let b = b.clone();
        util::run_bg(
            move || adb.find_patched_image(&serial),
            move |r| {
                let Some(t) = w.upgrade() else { return };
                let (remote, size) = match r {
                    Ok(Some(found)) => found,
                    Ok(None) => {
                        t.ctx.toast(tr("There is no magisk_patched-….img in the phone's Download folder yet (step 4)"));
                        return;
                    }
                    Err(e) => {
                        t.ctx.toast(&t!("Could not look for it: {}", e));
                        return;
                    }
                };
                let name = remote.rsplit('/').next().unwrap_or("magisk_patched.img").to_string();
                let w = t.me.clone();
                util::save_file(&b, tr("Patched image"), &name, move |local| {
                    let Some(t) = w.upgrade() else { return };
                    let Some((adb, serial)) = t.device() else { return };
                    let w = t.me.clone();
                    let (remote, target) = (remote.clone(), local.clone());
                    util::run_bg(
                        move || adb.pull(&serial, &remote, &target, size, &CancelToken::new(), |_| {}),
                        move |r| {
                            let Some(t) = w.upgrade() else { return };
                            match r {
                                Ok(_) => {
                                    *t.patched.borrow_mut() = Some(local.clone());
                                    let p = local.clone();
                                    t.ctx.toast_action(&t!("Saved {}", local.display()), tr("Open folder"), move || util::reveal(&p));
                                }
                                Err(e) => t.ctx.toast(&t!("Could not copy it: {}", e)),
                            }
                        },
                    );
                });
            },
        );
    }

    fn prepare_flash(&self, b: &gtk::Button) {
        let target = self.report.borrow().as_ref().map(|r| r.target_partition()).unwrap_or("boot");
        let fastboot = self.report.borrow().as_ref().map(|r| r.guide().fastboot).unwrap_or(true);
        if !fastboot {
            self.ctx.toast(tr("This brand has no fastboot: use the manufacturer's tool"));
            return;
        }
        let ctx = self.ctx.clone();
        let go = move |p: std::path::PathBuf| ctx.emit(Event::PrepareFlash(target.to_string(), p));
        match self.patched.borrow().clone().filter(|p| p.is_file()) {
            Some(p) => go(p),
            None => util::pick_file(b, tr("Patched image"), &[(tr("Images"), &["magisk_patched*.img", "*.img"])], go),
        }
    }
}

impl Page for RootPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn on_event(&self, ev: &Event) {
        match ev {
            Event::Devices if self.root.is_mapped() && *self.read_for.borrow() != self.ctx.ready_serial() => self.load(),
            Event::Privacy => self.render(),
            _ => {}
        }
    }
    fn shown(&self) {
        if self.report.borrow().is_none() || *self.read_for.borrow() != self.ctx.ready_serial() {
            self.load();
        }
    }
}
