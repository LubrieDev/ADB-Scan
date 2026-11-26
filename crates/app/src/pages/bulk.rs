//! Actions on all apps at once, from the menu next to “Install APK” in Apps:
//! close the open apps (as “Clear all” in Recents), stop every app except the
//! ones the phone needs, clear the cache of every app, and export or install
//! the user's apps (see `migration`). Each one reads
//! first, shows what it will do and runs only after it is confirmed (see
//! `adbscan_core::cleanup`).

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

use adbscan_core::cleanup::{OpenApps, StopPlan};
use adbscan_core::format;

use crate::ctx::Ctx;
use crate::util;

thread_local! {
    /// One action on all apps at a time, from the menu or from the search.
    static BUSY: Rc<Cell<bool>> = Rc::new(Cell::new(false));
}

fn busy_flag() -> Rc<Cell<bool>> {
    BUSY.with(|b| b.clone())
}

/// Runs one of the actions by name (`close`, `stop`, `caches`): the search
/// and the `win.bulk` action use it.
pub fn run(ctx: &Rc<Ctx>, parent: &gtk::Widget, which: &str) {
    let busy = busy_flag();
    if busy.get() {
        ctx.toast(tr("Another action on all apps is still running"));
        return;
    }
    match which {
        "close" => close_open_apps(ctx, parent, &busy),
        "stop" => stop_all(ctx, parent, &busy),
        "caches" => clear_all_caches(ctx, parent, &busy),
        "export" => super::migration::export(ctx, parent, &busy),
        "import" => super::migration::import(ctx, parent, &busy),
        _ => {}
    }
}

/// Menu button with the actions on all apps.
pub fn menu_button(ctx: &Rc<Ctx>) -> gtk::MenuButton {
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let actions: [(&str, &str, &str, fn(&Rc<Ctx>, &gtk::Widget, &Rc<Cell<bool>>)); 5] = [
        ("adb-circle-x-symbolic", tr("Close open apps"), tr("Like “Clear all” in Recents"), close_open_apps),
        ("adb-media-playback-stop-symbolic", tr("Stop all apps"), tr("Also those running in the background, except the ones the phone needs"), stop_all),
        ("adb-edit-clear-symbolic", tr("Clear all caches"), tr("Frees space. No data, accounts or settings are deleted"), clear_all_caches),
        ("adb-save-symbolic", tr("Export apps…"), tr("Copies your apps (APKs) to a folder, to install them on another phone"), super::migration::export),
        ("adb-system-software-install-symbolic", tr("Install from an export…"), tr("Installs the apps of a folder exported by ADB Scan"), super::migration::import),
    ];
    let busy = busy_flag();
    let popover = gtk::Popover::builder().child(&list).build();
    let button = gtk::MenuButton::builder().icon_name("adb-ellipsis-vertical-symbolic").tooltip_text(tr("Actions on all apps")).popover(&popover).build();
    for (icon, title, subtitle, run) in actions {
        let row = adw::ActionRow::builder().title(title).subtitle(subtitle).activatable(true).build();
        row.add_prefix(&util::icon(icon));
        let (ctx, busy, popover, button) = (ctx.clone(), busy.clone(), popover.clone(), button.clone());
        row.connect_activated(move |_| {
            popover.popdown();
            if busy.get() {
                ctx.toast(tr("Another action on all apps is still running"));
                return;
            }
            run(&ctx, button.upcast_ref(), &busy);
        });
        list.append(&row);
    }
    button
}

/// Device and serial, or a toast saying there is none.
fn device(ctx: &Rc<Ctx>) -> Option<(adbscan_core::adb::Adb, String)> {
    match (ctx.adb(), ctx.ready_serial()) {
        (Some(adb), Some(serial)) => Some((adb, serial)),
        _ => {
            ctx.toast(tr("No device is ready"));
            None
        }
    }
}

/// “• Name” of each package, sorted, up to 12.
fn bullet_names(packages: &[String]) -> String {
    let labels = super::apps::cached_labels();
    let mut names: Vec<String> = packages.iter().map(|p| format!("• {}", labels.get(p).cloned().unwrap_or_else(|| p.clone()))).collect();
    names.sort_by_key(|s| s.to_lowercase());
    if names.len() > 12 {
        let more = names.len() - 12;
        names.truncate(12);
        names.push(t!("… and {} more", more));
    }
    names.join("\n")
}

fn memory_freed(kib: Option<i64>) -> Option<String> {
    kib.filter(|k| *k > 0).map(|k| t!("{} of memory freed", format::human_bytes(Some(k as f64 * 1024.0))))
}

/// Runs `work` in the background with `busy` set, then `done` on the window.
fn run_busy<T: Send + 'static>(busy: &Rc<Cell<bool>>, work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    busy.set(true);
    let busy = busy.clone();
    util::run_bg(work, move |r| {
        busy.set(false);
        done(r);
    });
}

// -- close open apps ----------------------------------------------------------

fn close_open_apps(ctx: &Rc<Ctx>, parent: &gtk::Widget, busy: &Rc<Cell<bool>>) {
    let Some((adb, serial)) = device(ctx) else { return };
    let (c, p, b) = (ctx.clone(), parent.clone(), busy.clone());
    run_busy(busy, move || adb.open_apps(&serial), move |res| match res {
        Ok(apps) if apps.tasks.is_empty() => c.toast(tr("There are no open apps")),
        Ok(apps) => confirm_close(&c, &p, &b, apps),
        Err(e) => c.toast(&t!("Could not read the open apps: {}", e)),
    });
}

fn confirm_close(ctx: &Rc<Ctx>, parent: &gtk::Widget, busy: &Rc<Cell<bool>>, apps: OpenApps) {
    let packages = apps.packages();
    let heading = if packages.len() == 1 { tr("Close 1 open app?").to_string() } else { t!("Close {} open apps?", packages.len()) };
    let mut body = bullet_names(&packages);
    body.push_str(tr("\n\nThe same as “Clear all” in Recents: their windows are closed and Android ends them if nothing else is using them. Apps that were already closed may keep their card in Recents."));
    let (c, b) = (ctx.clone(), busy.clone());
    util::confirm(parent, &heading, &body, tr("Close all"), false, move || {
        let Some((adb, serial)) = device(&c) else { return };
        c.toast(tr("Closing apps…"));
        let target = format!("{} apps", apps.tasks.len());
        let c2 = c.clone();
        run_busy(&b, move || adb.close_open_apps(&serial, &apps), move |res| match res {
            Ok(r) => {
                let mut parts = vec![if r.closed == 1 { tr("1 app closed").to_string() } else { t!("{} apps closed", r.closed) }];
                parts.extend(memory_freed(r.freed_kib()));
                let msg = parts.join(" · ");
                c2.audit("close_open_apps", Some(&target), true, Some(&msg));
                c2.toast(&msg);
            }
            Err(e) => {
                let msg = e.to_string();
                c2.audit("close_open_apps", Some(&target), false, Some(&msg));
                c2.toast(&format!("Error: {msg}"));
            }
        });
    });
}

// -- stop all apps --------------------------------------------------------------

fn stop_all(ctx: &Rc<Ctx>, parent: &gtk::Widget, busy: &Rc<Cell<bool>>) {
    let Some((adb, serial)) = device(ctx) else { return };
    let (c, p, b) = (ctx.clone(), parent.clone(), busy.clone());
    run_busy(busy, move || adb.stop_plan(&serial), move |res| match res {
        Ok(plan) => confirm_stop(&c, &p, &b, plan),
        Err(e) => c.toast(&t!("Could not read what is running: {}", e)),
    });
}

fn confirm_stop(ctx: &Rc<Ctx>, parent: &gtk::Widget, busy: &Rc<Cell<bool>>, plan: StopPlan) {
    let labels = super::apps::cached_labels();
    let name = |p: &str| labels.get(p).cloned().unwrap_or_else(|| p.to_string());
    let n = plan.stop.len();
    let (heading, mut body) = if n == 0 {
        (tr("None of your apps are running").to_string(), tr("Android may still free background processes that are not needed.").to_string())
    } else {
        (if n == 1 { tr("Stop 1 app?").to_string() } else { t!("Stop {} apps?", n) }, bullet_names(&plan.stop))
    };
    if !plan.keep.is_empty() {
        let kept: Vec<String> = plan.keep.iter().map(|(p, why)| format!("{} ({})", why.label(), name(p))).collect();
        body.push_str(&t!("\n\nKept running: {}.", kept.join(", ")));
    }
    body.push_str(tr("\nSystem apps (network, calls, interface) are not closed; Android only frees the ones that are not needed."));
    if n > 0 {
        body.push_str(tr("\n\nStopped apps will not receive notifications or alarms until you open them again."));
    }
    let accept = if n == 0 { tr("Free memory") } else { tr("Stop all") };
    let (c, b) = (ctx.clone(), busy.clone());
    util::confirm(parent, &heading, &body, accept, true, move || {
        let Some((adb, serial)) = device(&c) else { return };
        c.toast(tr("Stopping processes…"));
        let target = format!("{} apps", plan.stop.len());
        let c2 = c.clone();
        run_busy(&b, move || adb.stop_all(&serial, &plan), move |res| match res {
            Ok(r) => {
                let mut parts = vec![format!("{} {}", r.stopped, if r.stopped == 1 { tr("app stopped") } else { tr("apps stopped") })];
                parts.extend(memory_freed(r.freed_kib()));
                if !r.restarted.is_empty() {
                    parts.push(t!("{} restarted by themselves", r.restarted.len()));
                }
                let msg = parts.join(" · ");
                let mut detail = msg.clone();
                if !r.restarted.is_empty() {
                    detail.push_str(&format!(" ({})", r.restarted.join(", ")));
                }
                c2.audit("stop_all", Some(&target), true, Some(&detail));
                c2.toast(&msg);
            }
            Err(e) => {
                let msg = e.to_string();
                c2.audit("stop_all", Some(&target), false, Some(&msg));
                c2.toast(&format!("Error: {msg}"));
            }
        });
    });
}

// -- clear all caches -------------------------------------------------------------

fn clear_all_caches(ctx: &Rc<Ctx>, parent: &gtk::Widget, busy: &Rc<Cell<bool>>) {
    let (c, b) = (ctx.clone(), busy.clone());
    util::confirm(
        parent,
        tr("Clear the cache of every app?"),
        tr("Android deletes the temporary files apps keep to load faster: thumbnails, downloaded images, web pages… No data, accounts or settings are deleted.\n\nApps may be a little slower the first time they are opened again, while they rebuild it."),
        tr("Clear caches"),
        false,
        move || {
            let Some((adb, serial)) = device(&c) else { return };
            c.toast(tr("Clearing caches…"));
            let c2 = c.clone();
            run_busy(&b, move || adb.trim_caches(&serial), move |res| match res {
                Ok(kib) => {
                    let msg = match kib.filter(|k| *k > 0) {
                        Some(k) => t!("Caches cleared · {} freed", format::human_bytes(Some(k as f64 * 1024.0))),
                        None => tr("Caches cleared").to_string(),
                    };
                    c2.audit("trim_caches", None, true, Some(&msg));
                    c2.toast(&msg);
                }
                Err(e) => {
                    let msg = e.to_string();
                    c2.audit("trim_caches", None, false, Some(&msg));
                    c2.toast(&format!("Error: {msg}"));
                }
            });
        },
    );
}
