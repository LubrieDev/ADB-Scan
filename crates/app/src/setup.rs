//! Tools assistant: tells what there is of adb, fastboot and scrcpy and
//! downloads whatever is missing from their official sources (see
//! `adbscan_core::tools_setup`), or lets you choose another path.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use adbscan_core::adb::Tool;
use adbscan_core::format;
use adbscan_core::tools_setup::{self as ts, Origin, Package, Stage, ToolState};

use crate::ctx::{Ctx, Event};
use crate::util;

const TOOLS: [Tool; 3] = [Tool::Adb, Tool::Fastboot, Tool::Scrcpy];

fn purpose(t: Tool) -> &'static str {
    match t {
        Tool::Adb => tr("Essential: connection with the phone"),
        Tool::Fastboot => tr("For flashing and bootloader mode"),
        Tool::Scrcpy => tr("For mirroring and controlling the screen"),
    }
}

fn configured(ctx: &Ctx, t: Tool) -> String {
    let s = ctx.settings.borrow();
    match t {
        Tool::Adb => s.adb_path.clone(),
        Tool::Fastboot => s.fastboot_path.clone(),
        Tool::Scrcpy => s.scrcpy_path.clone(),
    }
}

fn set_configured(ctx: &Ctx, t: Tool, path: String) {
    let mut s = ctx.settings.borrow_mut();
    match t {
        Tool::Adb => s.adb_path = path,
        Tool::Fastboot => s.fastboot_path = path,
        Tool::Scrcpy => s.scrcpy_path = path,
    }
}

/// Applies a tools change across the whole app (like Preferences).
fn tools_changed(ctx: &Rc<Ctx>) {
    ctx.save_settings();
    ctx.detect_tools();
    ctx.start_monitors();
    ctx.emit(Event::Tools);
}

/// At startup: the assistant only appears if adb is missing or, once, if
/// fastboot or scrcpy is missing.
pub fn maybe_show(ctx: &Rc<Ctx>, parent: &gtk::Widget) {
    let cfg: Vec<(Tool, String)> = TOOLS.iter().map(|t| (*t, configured(ctx, *t))).collect();
    let seen = ctx.settings.borrow().tools_wizard_seen;
    let (ctx, parent) = (ctx.clone(), parent.clone());
    util::run_bg(
        move || cfg.iter().filter(|(t, c)| adbscan_core::adb::find_tool(*t, Some(c)).is_none()).map(|(t, _)| *t).collect::<Vec<_>>(),
        move |missing: Vec<Tool>| {
            if missing.contains(&Tool::Adb) || (!missing.is_empty() && !seen) {
                show(&ctx, &parent);
            }
        },
    );
}

struct ToolRow {
    tool: Tool,
    row: adw::ActionRow,
    icon: gtk::Image,
    reset: gtk::Button,
}

struct PackageRow {
    pkg: Package,
    row: adw::ActionRow,
    button: gtk::Button,
    progress: gtk::ProgressBar,
    cancel: RefCell<Option<Arc<AtomicBool>>>,
}

struct Wizard {
    ctx: Rc<Ctx>,
    tools: Vec<ToolRow>,
    packages: Vec<Rc<PackageRow>>,
    /// State read last time (for the download texts).
    states: RefCell<Vec<ToolState>>,
}

pub fn show(ctx: &Rc<Ctx>, parent: &impl IsA<gtk::Widget>) {
    ctx.settings.borrow_mut().tools_wizard_seen = true;
    ctx.save_settings();

    let dialog = adw::Dialog::builder().title(tr("Tools")).content_width(600).content_height(640).build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());

    let content = util::vbox(18);
    let intro = util::hbox(16);
    let big = gtk::Image::from_icon_name("adb-wrench-symbolic");
    big.set_pixel_size(48);
    big.add_css_class("dim");
    big.set_valign(gtk::Align::Start);
    intro.append(&big);
    let texts = util::vbox(6);
    texts.append(&util::label(tr("Set up the tools"), &["title-2"]));
    let desc = util::label(
        tr("ADB Scan uses adb, fastboot and scrcpy. If they are already on the system, those are used. Missing ones can be downloaded from their official sources: they are checked against their SHA-256 sum and saved in your user folder, without root."),
        &["dim"],
    );
    desc.set_wrap(true);
    texts.append(&desc);
    intro.append(&texts);
    content.append(&intro);

    let tg = adw::PreferencesGroup::builder().title(tr("On this computer")).build();
    let mut tools = Vec::new();
    let mut picks = Vec::new();
    for t in TOOLS {
        let row = adw::ActionRow::builder().title(t.name()).subtitle(tr("Searching…")).build();
        let icon = util::icon("adb-dialog-warning-symbolic");
        row.add_prefix(&icon);
        let reset = util::icon_button("adb-edit-clear-symbolic", tr("Remove the chosen path and look for it automatically again"));
        reset.add_css_class("flat");
        reset.set_valign(gtk::Align::Center);
        reset.set_visible(false);
        let pick = gtk::Button::with_label(tr("Choose…"));
        pick.add_css_class("flat");
        pick.set_valign(gtk::Align::Center);
        pick.set_tooltip_text(Some(&t!("Use another {} executable", t.name())));
        row.add_suffix(&reset);
        row.add_suffix(&pick);
        tg.add(&row);
        tools.push(ToolRow { tool: t, row, icon, reset });
        picks.push(pick);
    }
    content.append(&tg);

    let dg = adw::PreferencesGroup::builder()
        .title(tr("Download"))
        .description(tr("Pinned, verified versions. When finished, ADB Scan uses the downloaded copy instead of the system one; the clear button next to the tool goes back to the system one."))
        .build();
    let mut packages = Vec::new();
    for pkg in ts::PACKAGES {
        let what = pkg.provides.iter().map(|t| t.name()).collect::<Vec<_>>().join(" y ");
        let row = adw::ActionRow::builder().title(format!("{} {} ({what})", tr(pkg.title), pkg.version)).build();
        let progress = gtk::ProgressBar::builder().valign(gtk::Align::Center).width_request(120).visible(false).build();
        let button = gtk::Button::with_label(tr("Download"));
        button.set_valign(gtk::Align::Center);
        row.add_suffix(&progress);
        row.add_suffix(&button);
        dg.add(&row);
        packages.push(Rc::new(PackageRow { pkg, row, button, progress, cancel: RefCell::new(None) }));
    }
    if !ts::supported() {
        dg.set_description(Some(tr("Google and scrcpy only publish these tools for 64-bit Linux (x86-64). On this computer, install them with your package manager and choose their paths above.")));
        for p in &packages {
            p.button.set_sensitive(false);
        }
    }
    content.append(&dg);

    let done = gtk::Button::with_label(tr("Done"));
    done.add_css_class("suggested-action");
    done.add_css_class("pill");
    done.set_halign(gtk::Align::Center);
    content.append(&done);

    let clamp = adw::Clamp::builder().maximum_size(560).child(&content).build();
    clamp.set_margin_top(12);
    clamp.set_margin_bottom(24);
    clamp.set_margin_start(18);
    clamp.set_margin_end(18);
    view.set_content(Some(&gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&clamp).build()));
    dialog.set_child(Some(&view));

    let wiz = Rc::new(Wizard { ctx: ctx.clone(), tools, packages, states: RefCell::new(Vec::new()) });

    for (r, pick) in wiz.tools.iter().zip(picks) {
        let (w, t) = (Rc::downgrade(&wiz), r.tool);
        pick.connect_clicked(move |b| {
            let w = w.clone();
            util::pick_file(b, &t!("Choose {}", t.name()), &[], move |p| {
                let Some(wiz) = w.upgrade() else { return };
                if !adbscan_core::exec::is_executable(&p) {
                    wiz.ctx.toast(&t!("{} is not an executable", p.display()));
                    return;
                }
                set_configured(&wiz.ctx, t, p.display().to_string());
                tools_changed(&wiz.ctx);
                wiz.refresh();
            });
        });
        let (w, t) = (Rc::downgrade(&wiz), r.tool);
        r.reset.connect_clicked(move |_| {
            let Some(wiz) = w.upgrade() else { return };
            set_configured(&wiz.ctx, t, String::new());
            tools_changed(&wiz.ctx);
            wiz.refresh();
        });
    }
    for p in &wiz.packages {
        let (w, p2) = (Rc::downgrade(&wiz), p.clone());
        p.button.connect_clicked(move |_| {
            if let Some(wiz) = w.upgrade() {
                wiz.toggle_download(&p2);
            }
        });
    }
    {
        let d = dialog.clone();
        done.connect_clicked(move |_| {
            d.close();
        });
    }
    {
        // Closing halfway cancels the downloads.
        let w = wiz.clone();
        dialog.connect_closed(move |_| {
            for p in &w.packages {
                if let Some(c) = p.cancel.borrow().as_ref() {
                    c.store(true, Ordering::SeqCst);
                }
            }
        });
    }
    wiz.refresh();
    dialog.present(Some(parent));
}

impl Wizard {
    /// Looks for the tools again and repaints the rows.
    fn refresh(self: &Rc<Self>) {
        let cfg: Vec<(Tool, String)> = self.tools.iter().map(|r| (r.tool, configured(&self.ctx, r.tool))).collect();
        let w = Rc::downgrade(self);
        util::run_bg(
            move || cfg.iter().map(|(t, c)| ts::tool_state(*t, c)).collect::<Vec<_>>(),
            move |states: Vec<ToolState>| {
                let Some(wiz) = w.upgrade() else { return };
                for (r, st) in wiz.tools.iter().zip(&states) {
                    let (icon, class) = match &st.found {
                        Some(_) => ("adb-emblem-ok-symbolic", "severity-ok"),
                        None if r.tool == Tool::Adb => ("adb-triangle-alert-symbolic", "severity-high"),
                        None => ("adb-dialog-warning-symbolic", "severity-medium"),
                    };
                    r.icon.set_icon_name(Some(icon));
                    for c in ["severity-ok", "severity-high", "severity-medium"] {
                        r.icon.remove_css_class(c);
                    }
                    r.icon.add_css_class(class);
                    let text = match &st.found {
                        Some((path, origin)) => {
                            let from = match origin {
                                Origin::Managed => tr("downloaded by ADB Scan"),
                                Origin::Configured => tr("chosen path"),
                                Origin::System => tr("system"),
                            };
                            let version = st.version.as_deref().map(short_version).unwrap_or_default();
                            format!("{} · {from}\n{}", if version.is_empty() { tr("Unknown version").to_string() } else { version }, util::tilde(path))
                        }
                        None => t!("Not found. {}", purpose(r.tool)),
                    };
                    r.row.set_subtitle(&glib::markup_escape_text(&text));
                    r.reset.set_visible(!configured(&wiz.ctx, r.tool).trim().is_empty());
                }
                *wiz.states.borrow_mut() = states;
                for p in &wiz.packages {
                    wiz.paint_package(p);
                }
            },
        );
    }

    /// Text and button of a download according to what is installed.
    fn paint_package(&self, p: &PackageRow) {
        if p.cancel.borrow().is_some() {
            return;
        }
        let installed = ts::installed_version(&p.pkg);
        let missing: Vec<&str> = self.states.borrow().iter().filter(|s| s.found.is_none() && p.pkg.provides.contains(&s.tool)).map(|s| s.tool.name()).collect();
        let size = format::human_bytes(Some(p.pkg.size as f64));
        let host = p.pkg.url.split('/').nth(2).unwrap_or("");
        let (sub, label) = match installed {
            Some(v) if v == p.pkg.version => (t!("Downloaded ({}) · {}", v, host), tr("Reinstall")),
            Some(v) => (t!("Version {} downloaded; a newer one is available · {}", v, size), tr("Refresh")),
            None if !missing.is_empty() => (t!("{} missing · {} from {}", missing.join(" y "), size, host), tr("Download")),
            None => (t!("You already have it on the system · {} from {}", size, host), tr("Download")),
        };
        p.row.set_subtitle(&sub);
        p.button.set_label(label);
        p.button.remove_css_class("destructive-action");
        if missing.is_empty() {
            p.button.remove_css_class("suggested-action");
        } else {
            p.button.add_css_class("suggested-action");
        }
    }

    fn toggle_download(self: &Rc<Self>, p: &Rc<PackageRow>) {
        // During the download, the button cancels it.
        if let Some(c) = p.cancel.borrow().as_ref() {
            c.store(true, Ordering::SeqCst);
            p.button.set_sensitive(false);
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *p.cancel.borrow_mut() = Some(cancel.clone());
        p.button.set_label(tr("Cancel"));
        p.button.remove_css_class("suggested-action");
        p.button.add_css_class("destructive-action");
        p.progress.set_fraction(0.0);
        p.progress.set_visible(true);
        p.row.set_subtitle(tr("Connecting…"));

        let (tx, rx) = async_channel::unbounded::<Result<Stage, String>>();
        let pkg = p.pkg;
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            let res = ts::install(&pkg, &move |s| drop(tx2.send_blocking(Ok(s))), &cancel);
            // The last message is the result (a success arrives as the last phase).
            let _ = tx.send_blocking(res.map(|_| Stage::Extracting).map_err(|e| e.to_string()));
        });
        let (w, p) = (Rc::downgrade(self), p.clone());
        glib::spawn_future_local(async move {
            let mut last: Option<Result<Stage, String>> = None;
            while let Ok(msg) = rx.recv().await {
                if let Ok(stage) = &msg {
                    match *stage {
                        Stage::Downloading(got, total) => {
                            p.progress.set_fraction(got as f64 / total.max(1) as f64);
                            p.row.set_subtitle(&t!("Downloading… {} of {}", format::human_bytes(Some(got as f64)), format::human_bytes(Some(total as f64))));
                        }
                        Stage::Verifying => {
                            p.progress.pulse();
                            p.row.set_subtitle(tr("Checking the SHA-256 sum…"));
                        }
                        Stage::Extracting => {
                            p.progress.pulse();
                            p.row.set_subtitle(tr("Extracting…"));
                        }
                    }
                }
                last = Some(msg);
            }
            let Some(wiz) = w.upgrade() else { return };
            p.cancel.borrow_mut().take();
            p.progress.set_visible(false);
            p.button.set_sensitive(true);
            match last {
                Some(Err(e)) => {
                    p.row.set_subtitle(&glib::markup_escape_text(&t!("Not installed: {}", e)));
                    p.button.set_label(tr("Retry"));
                    p.button.remove_css_class("destructive-action");
                }
                _ => {
                    // Downloaded at the user's request: this copy is used.
                    for t in p.pkg.provides {
                        set_configured(&wiz.ctx, *t, ts::managed_path(*t).display().to_string());
                    }
                    tools_changed(&wiz.ctx);
                    wiz.ctx.toast(&t!("{} {} installed", tr(p.pkg.title), p.pkg.version));
                    wiz.refresh();
                }
            }
        });
    }
}

/// “Android Debug Bridge version 1.0.41 · Version 37.0.1-15733141” → “37.0.1”.
fn short_version(v: &str) -> String {
    let re = regex::Regex::new(r"(\d+\.\d+(?:\.\d+)?)").unwrap();
    // adb's second number (the platform-tools one) is the one that matters.
    let found: Vec<&str> = re.find_iter(v).map(|m| m.as_str()).collect();
    match found.as_slice() {
        [_, second, ..] if v.contains("Version ") => second.to_string(),
        [first, ..] => first.to_string(),
        [] => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::short_version;

    #[test]
    fn versions() {
        assert_eq!(short_version("Android Debug Bridge version 1.0.41 · Version 37.0.1-15733141"), "37.0.1");
        assert_eq!(short_version("Android Debug Bridge version 1.0.41 · Version 34.0.4-debian"), "34.0.4");
        assert_eq!(short_version("fastboot version 34.0.4-debian"), "34.0.4");
        assert_eq!(short_version("scrcpy 5.0.1 <https://github.com/Genymobile/scrcpy>"), "5.0.1");
    }
}
