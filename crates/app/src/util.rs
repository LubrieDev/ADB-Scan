//! Interface utilities.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::{gio, glib};
use std::path::PathBuf;

/// Runs `work` in a thread and delivers the result on the UI thread. Never
/// blocks the main loop.
pub fn run_bg<T, W, D>(work: W, done: D)
where
    T: Send + 'static,
    W: FnOnce() -> T + Send + 'static,
    D: FnOnce(T) + 'static,
{
    glib::spawn_future_local(async move {
        match gio::spawn_blocking(work).await {
            Ok(v) => done(v),
            Err(_) => log::error!("a background task panicked"),
        }
    });
}

pub fn icon(name: &str) -> gtk::Image {
    gtk::Image::from_icon_name(name)
}

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    for c in classes {
        l.add_css_class(c);
    }
    l
}

pub fn icon_button(icon_name: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon_name);
    b.set_tooltip_text(Some(tooltip));
    b
}

pub fn pill_button(icon_name: &str, text: &str) -> gtk::Button {
    let content = adw::ButtonContent::builder().icon_name(icon_name).label(text).use_underline(true).build();

    gtk::Button::builder().child(&content).build()
}

pub fn copy_to_clipboard(widget: &impl IsA<gtk::Widget>, text: &str) {
    widget.as_ref().clipboard().set_text(text);
}

pub fn parent_window(widget: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    widget.as_ref().root().and_downcast::<gtk::Window>()
}

/// Confirmation dialog. `destructive` paints the button red.
pub fn confirm<F: FnOnce() + 'static>(parent: &impl IsA<gtk::Widget>, heading: &str, body: &str, accept: &str, destructive: bool, on_accept: F) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.set_body_use_markup(false);
    dialog.add_responses(&[("cancel", tr("Cancel")), ("ok", accept)]);
    dialog.set_response_appearance("ok", if destructive { adw::ResponseAppearance::Destructive } else { adw::ResponseAppearance::Suggested });
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let cell = std::cell::Cell::new(Some(on_accept));
    dialog.connect_response(None, move |_, resp| {
        if resp == "ok" {
            if let Some(f) = cell.take() {
                f();
            }
        }
    });
    dialog.present(Some(parent.as_ref()));
}

/// Strong confirmation: `expected` (e.g. the serial number) must be typed to
/// enable the button. For irreversible operations.
/// `masked_body`: in private mode, the text with the data hidden. The dialog
/// then adds “Show serial number”, which reveals it only here.
pub fn confirm_typed<F: FnOnce() + 'static>(parent: &impl IsA<gtk::Widget>, heading: &str, body: &str, expected: &str, accept: &str, masked_body: Option<String>, on_accept: F) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(masked_body.as_deref().unwrap_or(body)));
    // The placeholder text cannot contain the data if it is hidden.
    let placeholder = if masked_body.is_some() { tr("Type the serial number").to_string() } else { t!("Type {}", expected) };
    let entry = gtk::Entry::builder().placeholder_text(placeholder).build();
    entry.add_css_class("mono");
    match masked_body {
        Some(masked) => {
            let extra = vbox(8);
            let reveal =
                gtk::ToggleButton::builder().child(&adw::ButtonContent::builder().icon_name("adb-view-reveal-symbolic").label(tr("Show serial number")).build()).halign(gtk::Align::Center).build();
            reveal.add_css_class("flat");
            let (d, full) = (dialog.clone(), body.to_string());
            reveal.connect_toggled(move |b| d.set_body(if b.is_active() { &full } else { &masked }));
            extra.append(&reveal);
            extra.append(&entry);
            dialog.set_extra_child(Some(&extra));
        }
        None => dialog.set_extra_child(Some(&entry)),
    }
    dialog.add_responses(&[("cancel", tr("Cancel")), ("ok", accept)]);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Destructive);
    dialog.set_response_enabled("ok", false);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let expected = expected.to_string();
    let d2 = dialog.clone();
    entry.connect_changed(move |e| d2.set_response_enabled("ok", e.text().trim() == expected));
    let cell = std::cell::Cell::new(Some(on_accept));
    dialog.connect_response(None, move |_, resp| {
        if resp == "ok" {
            if let Some(f) = cell.take() {
                f();
            }
        }
    });
    dialog.present(Some(parent.as_ref()));
}

pub fn show_error(parent: &impl IsA<gtk::Widget>, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("ok", tr("OK"));
    dialog.present(Some(parent.as_ref()));
}

fn filter(name: &str, patterns: &[&str]) -> gtk::FileFilter {
    let f = gtk::FileFilter::new();
    f.set_name(Some(name));
    for p in patterns {
        f.add_pattern(p);
    }
    f
}

fn filters(list: &[(&str, &[&str])]) -> Option<gio::ListStore> {
    if list.is_empty() {
        return None;
    }
    let store = gio::ListStore::new::<gtk::FileFilter>();
    for (name, pats) in list {
        store.append(&filter(name, pats));
    }
    Some(store)
}

pub fn pick_files<F: Fn(Vec<PathBuf>) + 'static>(parent: &impl IsA<gtk::Widget>, title: &str, file_filters: &[(&str, &[&str])], on_done: F) {
    let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
    if let Some(f) = filters(file_filters) {
        dialog.set_filters(Some(&f));
    }
    let win = parent_window(parent);
    dialog.open_multiple(win.as_ref(), gio::Cancellable::NONE, move |res| {
        if let Ok(model) = res {
            let paths: Vec<PathBuf> = (0..model.n_items()).filter_map(|i| model.item(i).and_downcast::<gio::File>()).filter_map(|f| f.path()).collect();
            if !paths.is_empty() {
                on_done(paths);
            }
        }
    });
}

pub fn pick_file<F: Fn(PathBuf) + 'static>(parent: &impl IsA<gtk::Widget>, title: &str, file_filters: &[(&str, &[&str])], on_done: F) {
    let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
    if let Some(f) = filters(file_filters) {
        dialog.set_filters(Some(&f));
    }
    let win = parent_window(parent);
    dialog.open(win.as_ref(), gio::Cancellable::NONE, move |res| {
        if let Some(p) = res.ok().and_then(|f| f.path()) {
            on_done(p);
        }
    });
}

pub fn pick_folder<F: Fn(PathBuf) + 'static>(parent: &impl IsA<gtk::Widget>, title: &str, on_done: F) {
    let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
    let win = parent_window(parent);
    dialog.select_folder(win.as_ref(), gio::Cancellable::NONE, move |res| {
        if let Some(p) = res.ok().and_then(|f| f.path()) {
            on_done(p);
        }
    });
}

pub fn save_file<F: Fn(PathBuf) + 'static>(parent: &impl IsA<gtk::Widget>, title: &str, suggested: &str, on_done: F) {
    let dialog = gtk::FileDialog::builder().title(title).modal(true).initial_name(suggested).build();
    let win = parent_window(parent);
    dialog.save(win.as_ref(), gio::Cancellable::NONE, move |res| {
        if let Some(p) = res.ok().and_then(|f| f.path()) {
            on_done(p);
        }
    });
}

/// Opens a file or folder with the system's default application.
pub fn open_path(path: &std::path::Path) {
    let uri = gio::File::for_path(path).uri();
    let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
}

/// Shows the file in the file manager (opens the folder).
pub fn reveal(path: &std::path::Path) {
    let dir = if path.is_dir() { path.to_path_buf() } else { path.parent().map(|p| p.to_path_buf()).unwrap_or_default() };
    open_path(&dir);
}

/// Detail row with a selectable value and a copy button.
pub fn info_row(title: &str) -> (adw::ActionRow, gtk::Label) {
    let row = adw::ActionRow::builder().title(title).build();
    let value = gtk::Label::builder().label("—").selectable(true).xalign(1.0).build();
    value.add_css_class("dim");
    value.add_css_class("numeric");
    // A short value (“4.188 V”) must not wrap; only long ones wrap.
    value.set_max_width_chars(48);
    value.connect_label_notify(|l| {
        let long = l.text().chars().count() > 28;
        l.set_wrap(long);
        // With a line cap, GTK measures the label far fewer times (see
        // `cap_wrapping`).
        l.set_lines(if long { WRAP_LINES } else { -1 });
        l.set_ellipsize(if long { gtk::pango::EllipsizeMode::End } else { gtk::pango::EllipsizeMode::None });
    });
    row.add_suffix(&value);
    (row, value)
}

/// Path for display, with the home folder abbreviated to `~`.
pub fn tilde(path: &std::path::Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if path.starts_with(&home) => format!("~/{}", path.strip_prefix(&home).unwrap_or(path).display()),
        _ => path.display().to_string(),
    }
}

/// Line cap for a label that wraps its text: generous, it does not cut anything
/// in practice.
const WRAP_LINES: i32 = 20;

/// Caps the lines of the labels that wrap text under `root` (titles and
/// subtitles of libadwaita rows, group descriptions…). Without a cap, GTK
/// measures each one again dozens of times on every layout: with a hundred
/// rows, an app's details view took up to 1 s per frame and the list stopped
/// responding. With a cap (and ellipsis, which is what changes the way it is
/// measured) it costs about ten times less.
pub fn cap_wrapping(root: &impl IsA<gtk::Widget>) {
    fn walk(w: &gtk::Widget) {
        if let Some(l) = w.downcast_ref::<gtk::Label>() {
            if l.wraps() && l.lines() < 1 && l.ellipsize() == gtk::pango::EllipsizeMode::None {
                l.set_lines(WRAP_LINES);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            walk(&c);
            child = c.next_sibling();
        }
    }
    walk(root.as_ref());
}

/// Scrollable container with a maximum width (AdwClamp) for pages.
pub fn page_scroller(child: &impl IsA<gtk::Widget>, max_width: i32) -> gtk::ScrolledWindow {
    child.as_ref().set_valign(gtk::Align::Start);
    let clamp = adw::Clamp::builder().maximum_size(max_width).tightening_threshold(max_width - 200).child(child).build();
    clamp.set_margin_top(18);
    clamp.set_margin_bottom(24);
    clamp.set_margin_start(18);
    clamp.set_margin_end(18);
    gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&clamp).build()
}

pub fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}

pub fn hbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, spacing)
}

/// Logs UI-thread operations that take longer than a long frame (diagnosis of
/// stalls; visible with ADB_SCAN_LOG=debug).
pub fn log_slow(what: &str, since: std::time::Instant) {
    let ms = since.elapsed().as_secs_f64() * 1000.0;
    if ms >= 50.0 {
        log::warn!("slow: {what} {ms:.0} ms");
    } else if ms >= 16.0 {
        log::debug!("slow: {what} {ms:.0} ms");
    }
}

/// With ADB_SCAN_LOG=debug, warns about slow frames of `window`, separating
/// layout (measure and allocate) from drawing.
pub fn watch_frames(window: &impl IsA<gtk::Widget>) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    window.connect_realize(|w| {
        let Some(clock) = w.frame_clock() else { return };
        let marks: std::rc::Rc<std::cell::Cell<[Option<std::time::Instant>; 3]>> = Default::default();
        let m = marks.clone();
        clock.connect_update(move |_| m.set([Some(std::time::Instant::now()), None, None]));
        let m = marks.clone();
        clock.connect_layout(move |_| {
            let mut v = m.get();
            v[1] = Some(std::time::Instant::now());
            m.set(v);
        });
        let m = marks.clone();
        clock.connect_paint(move |_| {
            let mut v = m.get();
            v[2] = Some(std::time::Instant::now());
            m.set(v);
        });
        clock.connect_after_paint(move |_| {
            let v = marks.get();
            let now = std::time::Instant::now();
            if let [Some(u), Some(l), Some(p)] = v {
                let ms = |a: std::time::Instant, b: std::time::Instant| b.duration_since(a).as_secs_f64() * 1000.0;
                let (layout, paint, total) = (ms(u, l), ms(l, p), ms(u, now));
                if total >= 50.0 {
                    log::warn!("slow frame {total:.0} ms: layout {layout:.0} · paint {paint:.0}");
                }
            }
        });
    });
}

/// Calls [`log_slow`] when leaving the scope.
pub struct SlowGuard(pub &'static str, pub std::time::Instant);

impl Drop for SlowGuard {
    fn drop(&mut self) {
        log_slow(self.0, self.1);
    }
}

/// Warns when the main loop is blocked (only with ADB_SCAN_LOG=debug): a 50 ms
/// timer that measures its own delay.
pub fn install_stall_detector() {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    let last = std::cell::Cell::new(std::time::Instant::now());
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        let late = last.get().elapsed().as_millis();
        if late > 150 {
            log::warn!("main loop blocked ~{} ms", late - 50);
        }
        last.set(std::time::Instant::now());
        glib::ControlFlow::Continue
    });
}
