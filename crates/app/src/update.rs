//! “Update now”: downloads the new version with its progress, checks it and
//! installs it (see `adbscan_core::updater`). On Linux apt installs the
//! `.deb` after `pkexec` asks for the password, and ADB Scan offers to
//! restart; on Windows the installer runs silently, after ADB Scan closes,
//! and opens it again.

use adbscan_core::format::human_bytes;
use adbscan_core::updater::{self, Update};
use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::ctx::Ctx;

/// Whether the update notice can offer “Update now” for `update`.
pub fn available(update: &Update) -> bool {
    update.package.is_some() && updater::can_install()
}

/// Download progress shared with the worker thread.
#[derive(Default)]
struct Shared {
    got: u64,
    total: u64,
}

/// Opens the update dialog and starts the download.
pub fn start(ctx: &Rc<Ctx>, parent: &gtk::Widget, update: &Update) {
    let Some(pkg) = update.package.clone() else { return };
    let version = update.version.to_string();
    let page = update.url.clone();
    let dialog = adw::AlertDialog::new(Some(&t!("Update to ADB Scan {}", version)), Some(&t!("Downloading {}…", pkg.name)));
    let bar = gtk::ProgressBar::builder().show_text(true).text("0 %").build();
    dialog.set_extra_child(Some(&bar));
    dialog.add_response("cancel", tr("Cancel"));
    dialog.set_close_response("cancel");

    let cancel = Arc::new(AtomicBool::new(false));
    // True once the download is over: “Cancel” no longer stops anything.
    let downloaded = Rc::new(Cell::new(false));
    {
        let (cancel, downloaded, ctx, page) = (cancel.clone(), downloaded.clone(), ctx.clone(), page.clone());
        dialog.connect_response(None, move |_, resp| match resp {
            "restart" => crate::prefs::restart(&ctx),
            "page" => open_page(&page),
            _ if !downloaded.get() => cancel.store(true, Ordering::Relaxed),
            _ => {}
        });
    }

    let shared = Arc::new(Mutex::new(Shared { total: pkg.size, ..Default::default() }));
    {
        let (shared, bar) = (shared.clone(), bar.clone());
        let done = downloaded.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            if done.get() {
                return glib::ControlFlow::Break;
            }
            let s = shared.lock().unwrap();
            if s.total > 0 {
                let f = s.got as f64 / s.total as f64;
                bar.set_fraction(f);
                bar.set_text(Some(&t!("{} % · {} of {}", (f * 100.0).floor() as u32, human_bytes(Some(s.got as f64)), human_bytes(Some(s.total as f64)))));
            }
            glib::ControlFlow::Continue
        });
    }

    let (ctx2, d, bar2) = (ctx.clone(), dialog.clone(), bar.clone());
    let (cn, sh) = (cancel.clone(), shared.clone());
    crate::util::run_bg(
        move || {
            let progress = |got: u64, total: u64| {
                let mut s = sh.lock().unwrap();
                s.got = got;
                s.total = total;
            };
            updater::download(&pkg, &progress, &cn).map_err(|e| e.to_string())
        },
        move |r: Result<PathBuf, String>| {
            downloaded.set(true);
            match r {
                Ok(file) => {
                    bar2.set_fraction(1.0);
                    bar2.set_text(Some("100 %"));
                    install(&ctx2, &d, &bar2, file, version);
                }
                // Cancelled: the dialog is already closed.
                Err(_) if cancel.load(Ordering::Relaxed) => {}
                Err(e) => {
                    ctx2.audit("app_update", Some(&version), false, Some(&e));
                    failed(&d, &bar2, &t!("Could not download the update: {}", e));
                }
            }
        },
    );
    dialog.present(Some(parent));
}

/// Shows an error, with Close and a button to the release page.
fn failed(dialog: &adw::AlertDialog, bar: &gtk::ProgressBar, text: &str) {
    bar.set_visible(false);
    dialog.set_body(text);
    dialog.set_response_label("cancel", tr("Close"));
    dialog.set_response_enabled("cancel", true);
    if !dialog.has_response("page") {
        dialog.add_response("page", tr("View release"));
    }
    dialog.set_can_close(true);
}

fn open_page(url: &str) {
    if let Err(e) = gtk::gio::AppInfo::launch_default_for_uri(url, gtk::gio::AppLaunchContext::NONE) {
        log::warn!("could not open {url}: {e}");
    }
}

#[cfg(unix)]
fn install(ctx: &Rc<Ctx>, dialog: &adw::AlertDialog, bar: &gtk::ProgressBar, deb: PathBuf, version: String) {
    dialog.set_body(tr("Installing… Type your password in the window that opens."));
    bar.pulse();
    dialog.set_response_enabled("cancel", false);
    dialog.set_can_close(false);
    let (ctx, d, bar) = (ctx.clone(), dialog.clone(), bar.clone());
    let pulse = {
        let bar = bar.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
            bar.pulse();
            glib::ControlFlow::Continue
        })
    };
    crate::util::run_bg(
        move || updater::install_deb(&deb).map_err(|e| e.to_string()),
        move |r| {
            pulse.remove();
            ctx.audit("app_update", Some(&version), r.is_ok(), r.as_ref().err().map(String::as_str));
            match r {
                Ok(()) => {
                    updater::clean_downloads();
                    bar.set_visible(false);
                    d.set_heading(Some(&t!("ADB Scan {} is installed", version)));
                    d.set_body(tr("Restart ADB Scan to use the new version."));
                    d.set_response_label("cancel", tr("Later"));
                    d.set_response_enabled("cancel", true);
                    d.add_response("restart", tr("Restart now"));
                    d.set_response_appearance("restart", adw::ResponseAppearance::Suggested);
                    d.set_default_response(Some("restart"));
                    d.set_can_close(true);
                }
                Err(e) => failed(&d, &bar, &t!("Could not install the update: {}", e)),
            }
        },
    );
}

#[cfg(windows)]
fn install(ctx: &Rc<Ctx>, dialog: &adw::AlertDialog, bar: &gtk::ProgressBar, setup: PathBuf, version: String) {
    match updater::start_installer(&setup) {
        Ok(()) => {
            ctx.audit("app_update", Some(&version), true, None);
            dialog.set_body(tr("Installing: ADB Scan closes now and opens again by itself in a few seconds."));
            dialog.set_response_enabled("cancel", false);
            dialog.set_can_close(false);
            bar.pulse();
            // The installer waits for ADB Scan to close.
            let app = ctx.app.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || app.quit());
        }
        Err(e) => {
            ctx.audit("app_update", Some(&version), false, Some(&e.to_string()));
            failed(dialog, bar, &t!("Could not install the update: {}", e));
        }
    }
}
