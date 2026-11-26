//! App migration, from the Apps ⋮ menu: export the user's apps to a folder
//! and install them on another phone (see `adbscan_core::migration`).

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use adbscan_core::migration::{self, ExportedApp, Manifest};

use crate::ctx::Ctx;
use crate::util;

/// Progress shared with the worker thread.
#[derive(Default)]
struct Status {
    done: usize,
    total: usize,
    current: String,
    finished: bool,
    failed: Vec<(String, String)>,
    ok: usize,
}

/// Dialog with a progress bar, a Cancel button that becomes Close, and an
/// optional folder to open at the end.
fn progress_dialog(parent: &gtk::Widget, heading: &str, status: Arc<Mutex<Status>>, cancel: Arc<AtomicBool>, folder: Option<PathBuf>, on_end: impl Fn(&Status) -> String + 'static) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(tr("Starting…")));
    let bar = gtk::ProgressBar::builder().show_text(true).build();
    dialog.set_extra_child(Some(&bar));
    dialog.add_response("cancel", tr("Cancel"));
    if folder.is_some() {
        dialog.add_response("open", tr("Open folder"));
        dialog.set_response_enabled("open", false);
    }
    dialog.set_close_response("cancel");
    let finished = Rc::new(Cell::new(false));
    {
        let (cancel, finished) = (cancel.clone(), finished.clone());
        dialog.connect_response(None, move |_, resp| {
            if resp == "open" {
                if let Some(f) = &folder {
                    util::open_path(f);
                }
            } else if !finished.get() {
                cancel.store(true, Ordering::Relaxed);
            }
        });
    }
    let d = dialog.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
        let s = status.lock().unwrap();
        if s.total > 0 {
            bar.set_fraction(s.done as f64 / s.total as f64);
            bar.set_text(Some(&format!("{} / {}", s.done, s.total)));
        }
        if !s.finished {
            if !s.current.is_empty() {
                d.set_body(&s.current);
            }
            return glib::ControlFlow::Continue;
        }
        finished.set(true);
        d.set_body(&on_end(&s));
        d.set_response_label("cancel", tr("Close"));
        if d.has_response("open") {
            d.set_response_enabled("open", true);
        }
        glib::ControlFlow::Break
    });
    dialog.present(Some(parent));
}

fn failures(s: &Status) -> String {
    if s.failed.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = s.failed.iter().take(10).map(|(p, e)| format!("• {p}: {e}")).collect();
    if s.failed.len() > 10 {
        lines.push(t!("… and {} more", s.failed.len() - 10));
    }
    format!("\n\n{}\n{}", tr("Not done:"), lines.join("\n"))
}

// -- export ----------------------------------------------------------------------

pub fn export(ctx: &Rc<Ctx>, parent: &gtk::Widget, busy: &Rc<Cell<bool>>) {
    let (Some(adb), Some(serial)) = (ctx.adb(), ctx.ready_serial()) else {
        ctx.toast(tr("No device is ready"));
        return;
    };
    let (ctx, parent2, busy) = (ctx.clone(), parent.clone(), busy.clone());
    util::pick_folder(parent, tr("Folder to export the apps to"), move |base| {
        let snap = ctx.snapshot();
        let model = snap.as_ref().map(|s| s.model()).unwrap_or_default();
        let android = snap.as_ref().map(|s| s.android_version()).unwrap_or_default();
        let date = glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d %H.%M").ok()).map(|d| d.to_string()).unwrap_or_default();
        let dir = base.join(util::file_safe(&t!("Apps of {} {}", model, date)));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            ctx.toast(&t!("Could not create the folder: {}", e));
            return;
        }
        let labels = crate::pages::apps::cached_labels();
        let status = Arc::new(Mutex::new(Status::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (adb, serial, st, cn, d2) = (adb.clone(), serial.clone(), status.clone(), cancel.clone(), dir.clone());
        busy.set(true);
        let (busy2, ctx2, dir_txt) = (busy.clone(), ctx.clone(), dir.display().to_string());
        util::run_bg(
            move || {
                let set = |f: &dyn Fn(&mut Status)| f(&mut st.lock().unwrap());
                let packages = match adb.user_packages(&serial) {
                    Ok(p) => p,
                    Err(e) => {
                        set(&|s| {
                            s.failed.push((tr("App list").into(), e.to_string()));
                            s.finished = true;
                        });
                        return 0;
                    }
                };
                set(&|s| s.total = packages.len());
                let mut apps = Vec::new();
                for (pkg, code) in packages {
                    if cn.load(Ordering::Relaxed) {
                        break;
                    }
                    let label = labels.get(&pkg).cloned().unwrap_or_else(|| pkg.clone());
                    let msg = t!("Copying {}…", label);
                    set(&|s| s.current = msg.clone());
                    match adb.export_app(&serial, &pkg, &d2) {
                        Ok(files) => {
                            apps.push(ExportedApp { package: pkg.clone(), label, version_code: code, files });
                            set(&|s| s.ok += 1);
                        }
                        Err(e) => set(&|s| s.failed.push((pkg.clone(), e.to_string()))),
                    }
                    set(&|s| s.done += 1);
                }
                let n = apps.len();
                let m = Manifest { format: migration::FORMAT, created: adbscan_core::models::now_secs() as i64, source_model: model, source_android: android, apps };
                if let Err(e) = migration::save(&d2, &m) {
                    set(&|s| s.failed.push((migration::MANIFEST.into(), e.to_string())));
                }
                set(&|s| s.finished = true);
                n
            },
            move |n| {
                busy2.set(false);
                ctx2.audit("apps_export", Some(&dir_txt), true, Some(&n.to_string()));
            },
        );
        let cancel_flag = cancel.clone();
        progress_dialog(&parent2, tr("Exporting apps"), status, cancel, Some(dir.clone()), move |s| {
            let head = if cancel_flag.load(Ordering::Relaxed) { t!("Cancelled: {} apps exported.", s.ok) } else { t!("{} apps exported.", s.ok) };
            format!("{head} {}{}", tr("Only the apps are copied, not their data: restore each app's data from its own backup."), failures(s))
        });
    });
}

// -- import ----------------------------------------------------------------------

pub fn import(ctx: &Rc<Ctx>, parent: &gtk::Widget, busy: &Rc<Cell<bool>>) {
    let (ctx, parent2, busy) = (ctx.clone(), parent.clone(), busy.clone());
    util::pick_folder(parent, tr("Folder of an export"), move |dir| {
        let manifest = match migration::load(&dir) {
            Ok(m) => m,
            Err(e) => {
                util::show_error(&parent2, tr("This folder cannot be installed"), &e.to_string());
                return;
            }
        };
        let (Some(adb), Some(serial)) = (ctx.adb(), ctx.ready_serial()) else {
            ctx.toast(tr("No device is ready"));
            return;
        };
        // What the phone already has, to leave it unchecked.
        let (ctx2, parent3, busy2) = (ctx.clone(), parent2.clone(), busy.clone());
        util::run_bg(
            move || adb.user_packages(&serial).unwrap_or_default(),
            move |installed| {
                let installed: std::collections::HashMap<String, Option<i64>> = installed.into_iter().collect();
                choose(&ctx2, &parent3, &busy2, dir.clone(), manifest.clone(), installed);
            },
        );
    });
}

fn choose(ctx: &Rc<Ctx>, parent: &gtk::Widget, busy: &Rc<Cell<bool>>, dir: PathBuf, m: Manifest, installed: std::collections::HashMap<String, Option<i64>>) {
    let date = adbscan_core::format::timestamp(Some(m.created as f64));
    let dialog = adw::AlertDialog::new(
        Some(&t!("Install apps from {}", if m.source_model.is_empty() { "?" } else { &m.source_model })),
        Some(&t!("{} apps exported on {}. Apps the phone already has are unchecked. Only the apps are installed: their data comes from each app's own backup.", m.apps.len(), date)),
    );
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let mut checks = Vec::new();
    for app in &m.apps {
        let have = installed.get(&app.package);
        let sub = match have {
            Some(Some(v)) if Some(*v) >= app.version_code => tr("Already installed"),
            Some(_) => tr("Installed, older version"),
            None => "",
        };
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(&app.label)).subtitle(glib::markup_escape_text(&if sub.is_empty() { app.package.clone() } else { format!("{} · {}", app.package, sub) })).build();
        let check = gtk::CheckButton::builder().active(have.is_none()).valign(gtk::Align::Center).build();
        row.add_prefix(&check);
        row.set_activatable_widget(Some(&check));
        list.append(&row);
        checks.push((check, app.clone()));
    }
    let scroll = gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).min_content_height(220).max_content_height(380).propagate_natural_height(true).build();
    dialog.set_extra_child(Some(&scroll));
    dialog.add_responses(&[("cancel", tr("Cancel")), ("install", tr("Install"))]);
    dialog.set_response_appearance("install", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("cancel");
    let (ctx, parent2, busy) = (ctx.clone(), parent.clone(), busy.clone());
    dialog.connect_response(None, move |_, resp| {
        if resp != "install" {
            return;
        }
        let chosen: Vec<ExportedApp> = checks.iter().filter(|(c, _)| c.is_active()).map(|(_, a)| a.clone()).collect();
        if chosen.is_empty() {
            return;
        }
        install(&ctx, &parent2, &busy, dir.clone(), chosen);
    });
    dialog.present(Some(parent));
}

fn install(ctx: &Rc<Ctx>, parent: &gtk::Widget, busy: &Rc<Cell<bool>>, dir: PathBuf, apps: Vec<ExportedApp>) {
    let (Some(adb), Some(serial)) = (ctx.adb(), ctx.ready_serial()) else {
        ctx.toast(tr("No device is ready"));
        return;
    };
    let status = Arc::new(Mutex::new(Status { total: apps.len(), ..Default::default() }));
    let cancel = Arc::new(AtomicBool::new(false));
    let (st, cn) = (status.clone(), cancel.clone());
    busy.set(true);
    let (busy2, ctx2, dir_txt) = (busy.clone(), ctx.clone(), dir.display().to_string());
    util::run_bg(
        move || {
            let set = |f: &dyn Fn(&mut Status)| f(&mut st.lock().unwrap());
            let opts = adbscan_core::adb::InstallOptions { replace: true, ..Default::default() };
            for app in apps {
                if cn.load(Ordering::Relaxed) {
                    break;
                }
                let msg = t!("Installing {}…", app.label);
                set(&|s| s.current = msg.clone());
                let r = app.paths(&dir).and_then(|paths| adb.install(&serial, &paths, &opts));
                match r {
                    Ok(_) => set(&|s| s.ok += 1),
                    Err(e) => set(&|s| s.failed.push((app.label.clone(), e.to_string()))),
                }
                set(&|s| s.done += 1);
            }
            set(&|s| s.finished = true);
        },
        move |_| {
            busy2.set(false);
            ctx2.audit("apps_import", Some(&dir_txt), true, None);
        },
    );
    let cancel_flag = cancel.clone();
    progress_dialog(parent, tr("Installing apps"), status, cancel, None, move |s| {
        let head = if cancel_flag.load(Ordering::Relaxed) { t!("Cancelled: {} apps installed.", s.ok) } else { t!("{} apps installed.", s.ok) };
        format!("{head}{}", failures(s))
    });
}
