//! Phone data → private view → “Copy to this computer”: contacts (vCard),
//! call log and SMS (CSV), read from the phone and written only to the file
//! the user chooses (see `adbscan_core::personal_export`). Nothing is kept in
//! memory, the history or the logs beyond the count.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use std::rc::Rc;

use adbscan_core::personal_export::Kind;

use crate::ctx::Ctx;
use crate::util;

pub fn group(ctx: &Rc<Ctx>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Copy to this computer"))
        .description(tr("Read from the phone and saved only in the file you choose: nothing is uploaded and nothing changes on the phone. Contact photos and MMS are not included."))
        .build();
    for kind in Kind::ALL {
        let subtitle = match kind {
            Kind::Contacts => tr("vCard file: opens in any address book and imports on any phone"),
            Kind::Calls => tr("CSV file for a spreadsheet: date, type, duration and number"),
            Kind::Messages => tr("CSV file for a spreadsheet: date, number and text of each message"),
        };
        let row = adw::ActionRow::builder().title(kind.label()).subtitle(subtitle).build();
        let icon = match kind {
            Kind::Contacts => "adb-file-text-symbolic",
            Kind::Calls => "adb-call-start-symbolic",
            Kind::Messages => "adb-send-to-symbolic",
        };
        row.add_prefix(&util::icon(icon));
        let button = gtk::Button::with_label(tr("Save…"));
        button.add_css_class("pill");
        button.set_valign(gtk::Align::Center);
        row.add_suffix(&button);
        let ctx = ctx.clone();
        button.connect_clicked(move |b| save(&ctx, b, kind));
        group.add(&row);
    }
    group
}

fn save(ctx: &Rc<Ctx>, button: &gtk::Button, kind: Kind) {
    if ctx.ready_serial().is_none() {
        ctx.toast(tr("No device is ready"));
        return;
    }
    let stamp = adbscan_core::format::file_stamp(adbscan_core::models::now_secs());
    let name = format!("{}-{}.{}", kind.key(), stamp.split('_').next().unwrap_or(&stamp), kind.extension());
    let (ctx, button) = (ctx.clone(), button.clone());
    util::save_file(&button.clone(), kind.label(), &name, move |path| {
        let (Some(adb), Some(serial)) = (ctx.adb(), ctx.ready_serial()) else { return };
        button.set_sensitive(false);
        let (ctx, button, target) = (ctx.clone(), button.clone(), path.clone());
        util::run_bg(
            move || {
                let (text, n) = adb.personal_export(&serial, kind)?;
                write_private(&target, text.as_bytes()).map_err(|e| adbscan_core::Error::Invalid(e.to_string()))?;
                Ok::<_, adbscan_core::Error>(n)
            },
            move |r| {
                button.set_sensitive(true);
                match r {
                    Ok(n) => {
                        ctx.audit("personal_export", Some(&format!("{} · {n}", kind.key())), true, None);
                        let msg = match kind {
                            Kind::Contacts => t!("{} contacts saved", n),
                            Kind::Calls => t!("{} calls saved", n),
                            Kind::Messages => t!("{} messages saved", n),
                        };
                        let p = path.clone();
                        ctx.toast_action(&msg, tr("Open folder"), move || util::reveal(&p));
                    }
                    Err(e) => {
                        ctx.audit("personal_export", Some(kind.key()), false, Some(&e.to_string()));
                        ctx.toast(&t!("Could not copy them: {}", e));
                    }
                }
            },
        );
    });
}

/// Writes the file readable only by the user (it holds personal data).
fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(path)?;
    f.write_all(data)?;
    f.sync_all()
}

/// Phone data → private view → “Before selling or resetting the phone”: a
/// checklist that leads to what ADB Scan already does. It changes nothing.
pub struct BeforeReset {
    pub group: adw::PreferencesGroup,
    accounts: adw::ActionRow,
}

impl BeforeReset {
    pub fn new(ctx: &Rc<Ctx>) -> Self {
        let group = adw::PreferencesGroup::builder()
            .title(tr("Before selling or resetting the phone"))
            .description(tr("What to keep and what to remove first. ADB Scan never erases the phone: the reset is done on the phone."))
            .build();
        let step = |icon: &str, title: &str, subtitle: &str| {
            let row = adw::ActionRow::builder().title(title).subtitle(subtitle).build();
            row.add_prefix(&util::icon(icon));
            group.add(&row);
            row
        };
        let link = |row: &adw::ActionRow, label: &str, page: &'static str| {
            let b = gtk::Button::with_label(label);
            b.add_css_class("flat");
            b.set_valign(gtk::Align::Center);
            let ctx = ctx.clone();
            b.connect_clicked(move |_| ctx.emit(crate::ctx::Event::ShowPage(page)));
            row.add_suffix(&b);
        };
        let photos = step("adb-images-symbolic", tr("Photos and videos"), tr("Copy them to this computer: Files → Copy new photos and videos to the laptop"));
        link(&photos, tr("Open Files"), "files");
        let apps = step("adb-package-symbolic", tr("Apps"), tr("Save them to install them on the new phone: Apps → ⋮ → Export apps"));
        link(&apps, tr("Open Apps"), "apps");
        step("adb-file-text-symbolic", tr("Contacts, calls and messages"), tr("Copy them with the buttons above. WhatsApp and other chats keep their own backup inside the app."));
        let accounts = step("adb-key-round-symbolic", tr("Accounts"), tr("reading…"));
        let open = gtk::Button::with_label(tr("Open on the phone"));
        open.add_css_class("flat");
        open.set_valign(gtk::Align::Center);
        let c = ctx.clone();
        open.connect_clicked(move |_| {
            let (Some(adb), Some(serial)) = (c.adb(), c.ready_serial()) else { return };
            let action = adbscan_core::controls::Action::OpenSettings("android.settings.SYNC_SETTINGS");
            let c = c.clone();
            util::run_bg(move || adb.control_action(&serial, action), move |r| {
                if let Err(e) = r {
                    c.toast(&t!("Could not open it: {}", e));
                }
            });
        });
        accounts.add_suffix(&open);
        step("adb-system-reboot-symbolic", tr("Reset"), tr("On the phone: Settings → System → Reset options → Erase all data."));
        Self { group, accounts }
    }

    pub fn set_accounts(&self, accounts: &[adbscan_core::identity::Account]) {
        let google = accounts.iter().filter(|a| a.kind == "com.google").count();
        self.accounts.set_subtitle(&match google {
            0 if accounts.is_empty() => tr("No accounts on the phone: nothing to remove.").to_string(),
            0 => t!("{} accounts on the phone, none from Google: remove them in Settings → Accounts if you want.", accounts.len()),
            _ => t!(
                "{} Google accounts on the phone: remove them in Settings → Accounts before resetting, or after the reset the phone will ask for them again (Factory Reset Protection).",
                google
            ),
        });
    }
}
