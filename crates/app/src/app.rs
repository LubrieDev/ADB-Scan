//! Application: single instance, command-line options and actions.

use adbscan_core::i18n::tr;
use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adbscan_core::paths;

use crate::ctx::Ctx;
use crate::panel::QuickPanel;
use crate::tray::{TrayCmd, TrayHandle};
use crate::window::MainWindow;

struct Ui {
    ctx: Rc<Ctx>,
    window: Rc<MainWindow>,
    panel: RefCell<Option<Rc<QuickPanel>>>,
}

thread_local! {
    static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
    static TRAY_OK: Cell<bool> = const { Cell::new(false) };
}

pub fn tray_available() -> bool {
    TRAY_OK.with(|t| t.get())
}

fn ui() -> Option<Rc<Ui>> {
    UI.with(|u| u.borrow().clone())
}

/// Capture mode (development): independent instance without a tray icon, so it
/// does not interfere with the ADB Scan the user has open.
pub fn capture_mode() -> bool {
    std::env::var_os("ADB_SCAN_SCREENSHOT").is_some()
}

pub fn run() -> glib::ExitCode {
    let mut flags = gio::ApplicationFlags::HANDLES_COMMAND_LINE;
    if capture_mode() {
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    let app = adw::Application::builder().application_id(paths::APP_ID).flags(flags).build();
    app.add_main_option("hidden", glib::Char::from(0), glib::OptionFlags::NONE, glib::OptionArg::None, tr("Start hidden in the tray"), None);
    app.add_main_option("panel", glib::Char::from(0), glib::OptionFlags::NONE, glib::OptionArg::None, tr("Open the quick panel"), None);
    app.add_main_option("settings", glib::Char::from(0), glib::OptionFlags::NONE, glib::OptionArg::None, tr("Open preferences"), None);
    app.add_main_option("page", glib::Char::from(0), glib::OptionFlags::NONE, glib::OptionArg::String, tr("Open a section (dashboard, apps, files…)"), Some("ID"));
    app.add_main_option("version", glib::Char::from(b'v'), glib::OptionFlags::NONE, glib::OptionArg::None, tr("Show the version"), None);

    app.connect_handle_local_options(|_, opts| {
        if opts.contains("version") {
            println!("ADB Scan {}", paths::VERSION);
            return std::ops::ControlFlow::Break(glib::ExitCode::SUCCESS);
        }
        std::ops::ControlFlow::Continue(())
    });
    app.connect_startup(|app| {
        let t0 = std::time::Instant::now();
        crate::theme::install_icons();
        crate::util::log_slow("install icons", t0);
        let t0 = std::time::Instant::now();
        crate::theme::load_css();
        crate::util::log_slow("load CSS", t0);
        install_app_actions(app);
    });
    app.connect_command_line(|app, cmd| {
        let opts = cmd.options_dict();
        let first = ui().is_none();
        if first {
            build_ui(app);
        }
        let Some(ui) = ui() else {
            return glib::ExitCode::FAILURE;
        };
        // Without a tray icon (Windows) a hidden start would leave nothing on
        // screen: `--hidden` is ignored there.
        let hidden_asked = opts.contains("hidden") && (cfg!(unix) || tray_available());
        let hidden = hidden_asked || (first && ui.ctx.settings.borrow().start_hidden && tray_available());
        if let Some(page) = opts.lookup::<String>("page").ok().flatten() {
            ui.window.show_page(&page);
        }
        if opts.contains("panel") {
            toggle_panel(&ui, true);
        } else if opts.contains("settings") {
            ui.window.present();
            crate::prefs::show(&ui.ctx, ui.window.window.upcast_ref());
        } else if !hidden || !first {
            ui.window.present();
        }
        // First launch or no adb: the tools assistant.
        if first && !hidden && !capture_mode() {
            crate::setup::maybe_show(&ui.ctx, ui.window.window.upcast_ref());
        }
        // Automatic screenshots to verify the interface (development).
        if let (true, Ok(dir)) = (first, std::env::var("ADB_SCAN_SCREENSHOT")) {
            let pages: Vec<String> = std::env::var("ADB_SCAN_PAGES").unwrap_or_else(|_| "dashboard".into()).split(',').map(String::from).collect();
            let delay = std::env::var("ADB_SCAN_SHOT_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(3500);
            ui.window.present();
            block_real_input(ui.window.window.upcast_ref());
            shoot_pages(ui.clone(), dir, pages, 0, delay);
        }
        glib::ExitCode::SUCCESS
    });
    app.connect_shutdown(|_| {
        if let Some(ui) = ui() {
            ui.ctx.shutdown();
        }
    });
    app.run()
}

/// Capture mode: the window ignores the real keyboard and mouse. It takes the
/// focus when it opens, and a key typed for another window (Enter, Space)
/// landed on the focused button of the page, the dashboard's Screenshot
/// included, and acted on the connected phone. The steps (`click:`, `row:`,
/// `action:`…) do not go through input events, so they keep working.
fn block_real_input(window: &gtk::Window) {
    use gtk::gdk::EventType as E;
    use gtk::prelude::*;
    let guard = gtk::EventControllerLegacy::new();
    guard.set_propagation_phase(gtk::PropagationPhase::Capture);
    guard.connect_event(|_, event| match event.event_type() {
        E::KeyPress | E::KeyRelease | E::ButtonPress | E::ButtonRelease | E::TouchBegin | E::TouchUpdate | E::TouchEnd | E::Scroll => glib::Propagation::Stop,
        _ => glib::Propagation::Proceed,
    });
    window.add_controller(guard);
}

/// Capture mode: moves the largest scrollable area of the visible page to
/// `fraction` of its range.
fn scroll_longest(root: &gtk::Widget, fraction: f64) {
    use gtk::prelude::*;
    fn walk(w: &gtk::Widget, best: &mut Option<(f64, gtk::Adjustment)>) {
        if !w.is_mapped() {
            return;
        }
        if let Some(sw) = w.downcast_ref::<gtk::ScrolledWindow>() {
            // The largest on screen (not the one with the longest range: an
            // inner list of 300 rows has more than the page that contains it).
            let adj = sw.vadjustment();
            let visible = adj.page_size();
            if adj.upper() > visible + 1.0 && visible > best.as_ref().map(|b| b.0).unwrap_or(0.0) {
                *best = Some((visible, adj));
            }
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            walk(&c, best);
            child = c.next_sibling();
        }
    }
    let mut best = None;
    walk(root, &mut best);
    if let Some((_, adj)) = best {
        adj.set_value((adj.upper() - adj.page_size()) * fraction.clamp(0.0, 1.0));
    }
}

/// Capture mode: clicks the first visible button whose text is `label`, also
/// a button with an icon and a text (`util::pill_button`).
fn click_button(root: &gtk::Widget, label: &str) -> bool {
    use gtk::prelude::*;
    fn has_label(w: &gtk::Widget, label: &str) -> bool {
        if w.downcast_ref::<gtk::Label>().is_some_and(|l| l.text() == label) {
            return true;
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            if has_label(&c, label) {
                return true;
            }
            child = c.next_sibling();
        }
        false
    }
    if !root.is_mapped() {
        return false;
    }
    if let Some(b) = root.downcast_ref::<gtk::Button>() {
        if b.label().is_some_and(|l| l == label) || (b.label().is_none() && has_label(b.upcast_ref(), label)) {
            b.emit_clicked();
            return true;
        }
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        if click_button(&c, label) {
            return true;
        }
        child = c.next_sibling();
    }
    false
}

/// Capture mode: selects row `n` of the first visible list.
fn select_row(root: &gtk::Widget, n: u32) {
    use gtk::prelude::*;
    fn find(w: &gtk::Widget) -> Option<gtk::ListView> {
        if !w.is_mapped() {
            return None;
        }
        if let Some(l) = w.downcast_ref::<gtk::ListView>() {
            return Some(l.clone());
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            if let Some(l) = find(&c) {
                return Some(l);
            }
            child = c.next_sibling();
        }
        None
    }
    if let Some(model) = find(root).and_then(|l| l.model()) {
        model.select_item(n, true);
    }
}

fn shoot_pages(ui: Rc<Ui>, dir: String, pages: Vec<String>, i: usize, delay: u64) {
    if i >= pages.len() {
        quit(&ui);
        return;
    }
    let page = pages[i].clone();
    if let Some(rest) = page.strip_prefix("action:") {
        // Accepts detailed names with a parameter: `action:win.connect::qr`.
        match gtk::gio::Action::parse_detailed_name(rest) {
            Ok((name, target)) => {
                let _ = gtk::prelude::WidgetExt::activate_action(&ui.window.window, &name, target.as_ref());
            }
            Err(_) => {
                let _ = gtk::prelude::WidgetExt::activate_action(&ui.window.window, rest, None);
            }
        }
    } else if let Some(pkg) = page.strip_prefix("app:") {
        // Details of a specific app (the Apps page must be open first).
        ui.ctx.emit(crate::ctx::Event::ShowApp(pkg.to_string()));
    } else if let Some(label) = page.strip_prefix("click:") {
        // Clicks the first visible button with that text (in dialogs too).
        click_button(ui.window.window.upcast_ref(), label);
    } else if let Some(n) = page.strip_prefix("row:").and_then(|n| n.parse::<u32>().ok()) {
        // Selects row `n` of the visible multi-selection list (e.g. Apps), like
        // a click.
        select_row(ui.window.window.upcast_ref(), n);
    } else if let Some(pct) = page.strip_prefix("scroll:").and_then(|p| p.parse::<f64>().ok()) {
        // Scrolls the longest visible scrollable area (e.g. an app's details).
        scroll_longest(ui.window.window.upcast_ref(), pct / 100.0);
    } else {
        ui.window.show_page(&page);
    }
    glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
        // Forces a redraw and waits for two real frames: WidgetPaintable
        // reflects the last painted frame, not the current state.
        // With the quick panel open, the panel is captured.
        let panel = ui.panel.borrow().as_ref().filter(|p| p.window.is_visible()).map(|p| p.window.clone().upcast::<gtk::Window>());
        let win = panel.unwrap_or_else(|| ui.window.window.clone().upcast());
        log::debug!("capture: waiting for frames for {page}");
        win.queue_draw();
        let frames = std::cell::Cell::new(0u8);
        win.add_tick_callback(move |w, _| {
            frames.set(frames.get() + 1);
            if frames.get() < 2 {
                w.queue_draw();
                return glib::ControlFlow::Continue;
            }
            let name = page.replace([':', '.'], "_");
            log::debug!("capture: saving {name}");
            // Sometimes the last frame is not available yet: retry.
            let t0 = std::time::Instant::now();
            let saved = std::env::var_os("ADB_SCAN_NO_PNG").is_some() || crate::help::capture(w.upcast_ref(), &format!("{dir}/{name}.png"));
            crate::util::log_slow("capture: save PNG", t0);
            if !saved && frames.get() < 30 {
                w.queue_draw();
                return glib::ControlFlow::Continue;
            }
            let (ui, dir, pages) = (ui.clone(), dir.clone(), pages.clone());
            glib::idle_add_local_once(move || shoot_pages(ui, dir, pages, i + 1, delay));
            glib::ControlFlow::Break
        });
    });
}

fn build_ui(app: &adw::Application) {
    crate::util::install_stall_detector();
    crate::layout_fix::install();
    let t0 = std::time::Instant::now();
    let ctx = Ctx::new(app);
    crate::util::log_slow("create context", t0);
    let t0 = std::time::Instant::now();
    crate::theme::apply(&ctx.settings.borrow());
    crate::util::log_slow("apply theme", t0);
    let t0 = std::time::Instant::now();
    let window = MainWindow::new(&ctx);
    crate::util::watch_frames(&window.window);
    crate::util::log_slow("create window", t0);
    if let Some(tray) = (!capture_mode()).then(|| TrayHandle::spawn(ctx.sender())).flatten() {
        ctx.set_tray(tray);
        TRAY_OK.with(|t| t.set(true));
        // Keeps the app alive even when no windows are visible.
        std::mem::forget(app.hold());
    }
    ctx.start_monitors();
    // One check for a newer release per start, in the background; it fails
    // silently (see `adbscan_core::updater`).
    if !capture_mode() && ctx.settings.borrow().check_updates {
        let weak = Rc::downgrade(&window);
        crate::util::run_bg(
            || {
                // The installer of an earlier update is no longer needed.
                adbscan_core::updater::clean_downloads();
                adbscan_core::updater::check(env!("CARGO_PKG_VERSION"))
            },
            move |update| {
                if let (Some(update), Some(window)) = (update, weak.upgrade()) {
                    window.show_update(&update);
                }
            },
        );
    }
    let ui = Rc::new(Ui { ctx, window, panel: RefCell::new(None) });
    UI.with(|u| *u.borrow_mut() = Some(ui));
}

fn install_app_actions(app: &adw::Application) {
    let add = |name: &str, f: fn(&Rc<Ui>)| {
        let a = gio::SimpleAction::new(name, None);
        a.connect_activate(move |_, _| {
            if let Some(ui) = ui() {
                f(&ui);
            }
        });
        app.add_action(&a);
    };
    add("show", |ui| ui.window.present());
    add("panel", |ui| toggle_panel(ui, false));
    add("settings", |ui| {
        ui.window.present();
        crate::prefs::show(&ui.ctx, ui.window.window.upcast_ref());
    });
    add("about", |ui| crate::help::about(ui.window.window.upcast_ref()));
    add("tools", |ui| {
        ui.window.present();
        crate::setup::show(&ui.ctx, &ui.window.window);
    });
    add("quit", quit);
}

fn quit(ui: &Rc<Ui>) {
    if let Some(p) = ui.panel.borrow().as_ref() {
        p.window.destroy();
    }
    ui.window.window.destroy();
    ui.ctx.shutdown();
    ui.ctx.app.quit();
}

fn toggle_panel(ui: &Rc<Ui>, force_show: bool) {
    let existing = ui.panel.borrow().clone();
    let panel = match existing {
        Some(p) => p,
        None => {
            let p = QuickPanel::new(&ui.ctx);
            *ui.panel.borrow_mut() = Some(p.clone());
            p
        }
    };
    if panel.window.is_visible() && !force_show {
        panel.window.set_visible(false);
    } else {
        panel.window.present();
    }
}

pub fn handle_tray(_ctx: &Rc<Ctx>, cmd: TrayCmd) {
    let Some(ui) = ui() else { return };
    match cmd {
        TrayCmd::ShowWindow => ui.window.present(),
        TrayCmd::TogglePanel => toggle_panel(&ui, false),
        TrayCmd::Screenshot => crate::pages::mirror::quick_screenshot(&ui.ctx, ui.window.window.upcast_ref()),
        TrayCmd::Mirror => crate::pages::mirror::quick_mirror(&ui.ctx, ui.window.window.upcast_ref()),
        TrayCmd::Settings => {
            ui.window.present();
            crate::prefs::show(&ui.ctx, ui.window.window.upcast_ref());
        }
        TrayCmd::Quit => quit(&ui),
    }
}

/// Opens the main window on a section (from the quick panel).
pub fn open_page(id: &str) {
    if let Some(ui) = ui() {
        ui.window.present();
        ui.window.show_page(id);
    }
}
