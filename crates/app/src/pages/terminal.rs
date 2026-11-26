//! Real embedded terminal (VTE): an `adb shell` session on the phone and
//! another with the PC's shell. Both stay alive when switching pages.

use adbscan_core::{i18n::{tr, trc}, t};
use adw::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::os::unix::process::CommandExt;
use std::rc::Rc;
use vte::prelude::*;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Phone,
    Pc,
}

/// Quick commands: typed into the corresponding session and run.
const QUICK: [(&str, Kind, &str); 12] = [
    ("Model", Kind::Phone, "getprop ro.product.model"),
    ("Battery", Kind::Phone, "dumpsys battery"),
    ("Storage", Kind::Phone, "df -h"),
    ("Processes", Kind::Phone, "top -m 15"),
    ("User apps", Kind::Phone, "pm list packages -3"),
    ("Activity", Kind::Phone, "dumpsys activity activities | grep -E 'mResumedActivity|topResumedActivity'"),
    ("IP", Kind::Phone, "ip -o -4 addr"),
    ("User", Kind::Phone, "id"),
    ("Devices", Kind::Pc, "adb devices -l"),
    ("ADB version", Kind::Pc, "adb version"),
    ("Fastboot: variables", Kind::Pc, "fastboot getvar all"),
    ("Fastboot: devices", Kind::Pc, "fastboot devices -l"),
];

/// Startup of `adb shell` with the history preloaded (a folder writable without
/// root and accessible only to the shell user).
const PHONE_RC: &str = "/data/local/tmp/.adb_scan_mkshrc";

/// A terminal with its process.
struct Session {
    kind: Kind,
    term: vte::Terminal,
    banner: adw::Banner,
    title: gtk::Label,
    /// Content: “term” or “empty” (no device).
    stack: gtk::Stack,
    pid: Cell<Option<i32>>,
    /// Serial of the phone the `adb shell` session is connected to.
    serial: RefCell<Option<String>>,
}

struct TerminalPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::Box,
    sessions: gtk::Stack,
    phone: Rc<Session>,
    pc: Rc<Session>,
    current: Cell<Kind>,
    /// An `adb shell` is starting (uploading the history).
    phone_starting: Cell<bool>,
    switch_phone: gtk::ToggleButton,
    switch_pc: gtk::ToggleButton,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    TerminalPage::new(ctx)
}

fn rgba(hex: &str) -> gdk::RGBA {
    gdk::RGBA::parse(hex).unwrap_or(gdk::RGBA::BLACK)
}

/// Own palette (16 ANSI colours) for dark and light.
fn palette(dark: bool) -> (gdk::RGBA, gdk::RGBA, Vec<gdk::RGBA>) {
    let (fg, bg, colors): (&str, &str, [&str; 16]) = if dark {
        (
            "#e8e6e3",
            "#1b1b1f",
            ["#2e2e33", "#f26d6d", "#7bd88f", "#f5c86b", "#6fa8ff", "#c792ea", "#5fd7d7", "#d0d0d0", "#5c5c66", "#ff8c8c", "#a2f0b1", "#ffe08f", "#9cc4ff", "#ddb3ff", "#8ef0f0", "#ffffff"],
        )
    } else {
        (
            "#2b2b30",
            "#fbfaf8",
            ["#2b2b30", "#c4262e", "#2e8b46", "#a86b00", "#1f5fbf", "#8a3fb0", "#0f7f86", "#6e6e78", "#5c5c66", "#e0393f", "#3aa65a", "#c98800", "#2f74e0", "#a650d0", "#1499a1", "#9a9aa5"],
        )
    };
    (rgba(fg), rgba(bg), colors.iter().map(|c| rgba(c)).collect())
}

impl Session {
    fn new(kind: Kind) -> Rc<Self> {
        let term = vte::Terminal::new();
        term.set_hexpand(true);
        term.set_vexpand(true);
        term.set_cursor_blink_mode(vte::CursorBlinkMode::On);
        term.set_scroll_on_output(false);
        term.set_scroll_on_keystroke(true);
        term.set_bold_is_bright(true);
        term.set_allow_hyperlink(true);
        term.add_css_class("embedded-terminal");

        let scroll = gtk::ScrolledWindow::builder().child(&term).hscrollbar_policy(gtk::PolicyType::Never).build();
        let empty = adw::StatusPage::builder()
            .icon_name("adb-smartphone-symbolic")
            .title(tr("Connect a device"))
            .description(tr("The `adb shell` session will open by itself as soon as the phone is authorised."))
            .build();
        empty.add_css_class("compact");
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).transition_duration(160).build();
        stack.add_named(&scroll, Some("term"));
        stack.add_named(&empty, Some("empty"));

        let banner = adw::Banner::builder().button_label(tr("Restart")).build();
        let title = util::label("", &["term-title"]);
        Rc::new(Self { kind, term, banner, title, stack, pid: Cell::new(None), serial: RefCell::new(None) })
    }

    fn running(&self) -> bool {
        self.pid.get().is_some()
    }

    /// Launches `argv` in a new VTE PTY. The process is created with
    /// `std::process::Command`: the child only calls `vte_pty_child_setup` (new
    /// session, controlling terminal and standard I/O on the PTY).
    fn spawn(self: &Rc<Self>, argv: &[String], cwd: Option<&std::path::Path>, extra_env: &[(&str, String)]) -> Result<(), String> {
        let pty = vte::Pty::new_sync(vte::PtyFlags::DEFAULT, None::<&gtk::gio::Cancellable>).map_err(|e| e.to_string())?;
        let (program, args) = argv.split_first().ok_or("no program")?;
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        cmd.env("TERM", "xterm-256color").env("COLORTERM", "truecolor").env_remove("VTE_VERSION");
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let raw = pty.as_ptr() as usize;
        // SAFETY: between fork and exec only one VTE function is called,
        // designed exactly for this (it does not allocate memory in Rust).
        unsafe {
            cmd.pre_exec(move || {
                vte::ffi::vte_pty_child_setup(raw as *mut vte::ffi::VtePty);
                Ok(())
            });
        }
        let child = cmd.spawn().map_err(|e| format!("{program}: {e}"))?;
        let pid = child.id() as i32;
        // VTE watches and reaps the process (child-exited); do not wait here.
        drop(child);
        self.term.set_pty(Some(&pty));
        self.term.watch_child(glib::Pid(pid));
        self.pid.set(Some(pid));
        self.banner.set_revealed(false);
        self.stack.set_visible_child_name("term");
        Ok(())
    }

    fn kill(&self) {
        if let Some(pid) = self.pid.take() {
            // SIGHUP, as when closing a terminal window.
            unsafe {
                libc::kill(pid, libc::SIGHUP);
            }
        }
    }

    fn apply_style(&self, font: &str, scrollback: u32, accent: &str) {
        let dark = adw::StyleManager::default().is_dark();
        let (fg, bg, pal) = palette(dark);
        let refs: Vec<&gdk::RGBA> = pal.iter().collect();
        self.term.set_colors(Some(&fg), Some(&bg), &refs);
        let accent = rgba(accent);
        self.term.set_color_cursor(Some(&accent));
        self.term.set_color_cursor_foreground(Some(&bg));
        let mut sel = accent;
        sel.set_alpha(0.35);
        self.term.set_color_highlight(Some(&sel));
        self.term.set_color_highlight_foreground(None);
        self.term.set_font(Some(&gtk::pango::FontDescription::from_string(font)));
        self.term.set_scrollback_lines(scrollback as _);
    }
}

impl TerminalPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let root = util::vbox(0);
        root.add_css_class("terminal-page");

        // Top bar: session + actions.
        let top = util::hbox(8);
        top.set_margin_top(10);
        top.set_margin_start(14);
        top.set_margin_end(14);
        top.set_margin_bottom(8);
        let modes = util::hbox(0);
        modes.add_css_class("linked");
        let switch_phone = gtk::ToggleButton::with_label(tr("Phone · adb shell"));
        let switch_pc = gtk::ToggleButton::with_label(tr("PC · adb and fastboot"));
        switch_pc.set_group(Some(&switch_phone));
        switch_phone.set_active(true);
        modes.append(&switch_phone);
        modes.append(&switch_pc);
        top.append(&modes);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        top.append(&spacer);
        let copy = util::icon_button("adb-copy-symbolic", tr("Copy (Ctrl+Shift+C)"));
        let paste = util::icon_button("adb-edit-paste-symbolic", tr("Paste (Ctrl+Shift+V)"));
        let save = util::icon_button("adb-save-symbolic", tr("Save the content"));
        let clear = util::icon_button("adb-edit-clear-symbolic", tr("Clear (Ctrl+Shift+K)"));
        let restart = util::icon_button("adb-refresh-cw-symbolic", tr("Restart the session"));
        for b in [&copy, &paste, &save, &clear, &restart] {
            b.add_css_class("flat");
            top.append(b);
        }
        root.append(&top);

        let quick = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(16).min_children_per_line(3).column_spacing(6).row_spacing(6).homogeneous(false).build();
        quick.set_margin_start(14);
        quick.set_margin_end(14);
        quick.set_margin_bottom(10);
        root.append(&quick);

        // Rounded frame with a header and the two sessions.
        let phone = Session::new(Kind::Phone);
        let pc = Session::new(Kind::Pc);
        let sessions = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).transition_duration(160).vexpand(true).build();
        for s in [&phone, &pc] {
            let frame = util::vbox(0);
            frame.add_css_class("term-frame");
            let header = util::hbox(8);
            header.add_css_class("term-header");
            s.title.set_hexpand(true);
            s.title.set_xalign(0.5);
            s.title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            header.append(&s.title);
            frame.append(&header);
            frame.append(&s.banner);
            frame.append(&s.stack);
            sessions.add_named(&frame, Some(if s.kind == Kind::Phone { "phone" } else { "pc" }));
        }
        sessions.set_margin_start(14);
        sessions.set_margin_end(14);
        sessions.set_margin_bottom(14);
        root.append(&sessions);

        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            sessions,
            phone,
            pc,
            current: Cell::new(Kind::Phone),
            phone_starting: Cell::new(false),
            switch_phone: switch_phone.clone(),
            switch_pc: switch_pc.clone(),
        });

        macro_rules! on {
            ($w:expr, $sig:ident, |$t:ident| $body:expr) => {{
                let w = this.me.clone();
                $w.$sig(move |_| {
                    if let Some($t) = w.upgrade() {
                        $body;
                    }
                });
            }};
        }
        on!(switch_phone, connect_toggled, |t| if t.switch_phone.is_active() {
            t.select(Kind::Phone)
        });
        on!(switch_pc, connect_toggled, |t| if t.switch_pc.is_active() {
            t.select(Kind::Pc)
        });
        on!(copy, connect_clicked, |t| t.copy());
        on!(paste, connect_clicked, |t| t.session().term.paste_clipboard());
        on!(save, connect_clicked, |t| t.save());
        on!(clear, connect_clicked, |t| t.clear());
        on!(restart, connect_clicked, |t| t.restart(t.current.get()));

        for (label, kind, cmd) in QUICK {
            let b = gtk::Button::with_label(tr(label));
            b.add_css_class("pill");
            b.add_css_class("chip");
            b.set_tooltip_text(Some(cmd));
            let w = this.me.clone();
            b.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    t.run_quick(kind, cmd);
                }
            });
            quick.insert(&b, -1);
        }

        for s in [this.phone.clone(), this.pc.clone()] {
            let w = this.me.clone();
            let kind = s.kind;
            s.term.connect_child_exited(move |_, status| {
                if let Some(t) = w.upgrade() {
                    t.on_exit(kind, status);
                }
            });
            let w = this.me.clone();
            s.banner.connect_button_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    t.restart(kind);
                }
            });
            this.install_shortcuts(&s);
            this.install_context_menu(&s);
        }

        // Style according to theme and accent; reapplied when switching
        // light/dark.
        this.restyle();
        {
            let w = this.me.clone();
            adw::StyleManager::default().connect_dark_notify(move |_| {
                if let Some(t) = w.upgrade() {
                    t.restyle();
                }
            });
        }
        this.ensure_phone();
        if std::env::var_os("ADB_SCAN_TERM_DEMO").is_some() {
            this.demo();
        }
        this
    }

    /// Development only: types commands into both sessions and dumps what the
    /// terminals show to the log (verification without a keyboard).
    fn demo(&self) {
        self.ensure_pc();
        let w = self.me.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(2500), move || {
            let Some(t) = w.upgrade() else { return };
            t.run_quick(Kind::Pc, "adb devices -l | head -3; tput cols");
            t.run_quick(Kind::Phone, "echo history-$((20+22))");
        });
        if std::env::var("ADB_SCAN_TERM_DEMO").as_deref() == Ok("history") {
            let w = self.me.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(3500), move || {
                let Some(t) = w.upgrade() else { return };
                t.phone.term.feed_child(b"\x1b[A\r");
            });
            let w = self.me.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(4200), move || {
                if let Some(t) = w.upgrade() {
                    log::info!("before restarting:\n{}", t.phone.term.text_format(vte::Format::Text).unwrap_or_default().trim_end());
                    t.phone.term.feed_child(b"exit\r");
                }
            });
            let w = self.me.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(5200), move || {
                if let Some(t) = w.upgrade() {
                    t.restart(Kind::Phone);
                }
            });
            let w = self.me.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(7500), move || {
                if let Some(t) = w.upgrade() {
                    t.phone.term.feed_child(b"\x1b[A\r");
                }
            });
        }
        let w = self.me.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(9000), move || {
            let Some(t) = w.upgrade() else { return };
            for s in [&t.phone, &t.pc] {
                let text = s.term.text_format(vte::Format::Text).unwrap_or_default();
                log::info!("terminal {:?} ({}):\n{}", s.kind, s.title.text(), text.trim_end());
            }
        });
    }

    fn session(&self) -> &Rc<Session> {
        match self.current.get() {
            Kind::Phone => &self.phone,
            Kind::Pc => &self.pc,
        }
    }

    fn select(&self, kind: Kind) {
        self.current.set(kind);
        self.sessions.set_visible_child_name(if kind == Kind::Phone { "phone" } else { "pc" });
        match kind {
            Kind::Phone => self.ensure_phone(),
            Kind::Pc => self.ensure_pc(),
        }
        self.session().term.grab_focus();
    }

    fn restyle(&self) {
        let s = self.ctx.settings.borrow();
        let accent = crate::theme::accent_color(&s);
        for sess in [&self.phone, &self.pc] {
            sess.apply_style(&s.terminal_font, s.terminal_scrollback_lines, &accent);
        }
    }

    /// Opens (or reopens) `adb shell` if a phone is ready and the session is
    /// not already connected to it.
    fn ensure_phone(&self) {
        let s = &self.phone;
        let ready = self.ctx.ready_serial();
        let Some(serial) = ready else {
            if !s.running() {
                s.stack.set_visible_child_name("empty");
                s.banner.set_revealed(false);
                s.title.set_text(tr("adb shell · no device"));
            }
            return;
        };
        let name = self.ctx.selected_device().map(|d| self.ctx.device_label(&d)).unwrap_or_else(|| self.ctx.shown_value(&serial));
        if s.running() && s.serial.borrow().as_deref() == Some(serial.as_str()) {
            // The marketing name may arrive after the session opens.
            s.title.set_text(&format!("adb shell · {name}"));
            return;
        }
        if s.running() {
            // Another phone selected: close the previous session.
            s.kill();
        }
        let Some(adb) = self.ctx.adb() else {
            s.banner.set_title(tr("adb was not found"));
            s.banner.set_revealed(true);
            return;
        };
        if self.phone_starting.get() {
            return;
        }
        self.phone_starting.set(true);
        // History across sessions: Android's mksh does not save HISTFILE, so it
        // is preloaded with a startup file (`ENV`).
        let history = self.ctx.settings.borrow().terminal_history.clone();
        let rc_local = adbscan_core::paths::cache_dir().join("adb_scan_mkshrc");
        let rc_text = adbscan_core::settings::mksh_history_rc(&history);
        let w = self.me.clone();
        let adb2 = adb.clone();
        let serial2 = serial.clone();
        util::run_bg(
            move || {
                let _ = std::fs::create_dir_all(adbscan_core::paths::cache_dir());
                std::fs::write(&rc_local, rc_text).ok()?;
                adb2.push(&serial2, &rc_local, PHONE_RC, &adbscan_core::adb::CancelToken::new(), |_| {}).ok().map(|_| ())
            },
            move |pushed: Option<()>| {
                let Some(t) = w.upgrade() else { return };
                t.phone_starting.set(false);
                let s = &t.phone;
                let script = if pushed.is_some() { format!("export ENV={PHONE_RC} HISTSIZE=2000; exec sh -l") } else { "export HISTSIZE=2000; exec sh -l".to_string() };
                let argv = vec![adb.path().display().to_string(), "-s".into(), serial.clone(), "shell".into(), "-t".into(), script];
                if s.serial.borrow().is_some() {
                    s.term.feed(t!("\r\n\x1b[2m── new session with {} ──\x1b[0m\r\n", name).as_bytes());
                }
                match s.spawn(&argv, None, &[]) {
                    Ok(()) => {
                        *s.serial.borrow_mut() = Some(serial.clone());
                        s.title.set_text(&format!("adb shell · {name}"));
                        t.ctx.audit("terminal_shell", Some(&serial), true, None);
                    }
                    Err(e) => {
                        s.banner.set_title(&t!("Could not open adb shell: {}", e));
                        s.banner.set_revealed(true);
                    }
                }
            },
        );
    }

    /// PC shell with `adb` and `fastboot` in the PATH.
    fn ensure_pc(&self) {
        let s = &self.pc;
        if s.running() {
            return;
        }
        let shell = std::env::var("SHELL").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/bin/bash".into());
        let home = glib::home_dir();
        let mut path = std::env::var("PATH").unwrap_or_default();
        if let Some(dir) = self.ctx.adb().and_then(|a| a.path().parent().map(|p| p.to_path_buf())) {
            let dir = dir.display().to_string();
            if !path.split(':').any(|p| p == dir) {
                path = format!("{dir}:{path}");
            }
        }
        let argv = vec![shell.clone(), "-l".into()];
        match s.spawn(&argv, Some(&home), &[("PATH", path)]) {
            Ok(()) => {
                let name = std::path::Path::new(&shell).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(shell);
                s.title.set_text(&format!("{name} · {}", glib::user_name().to_string_lossy()));
            }
            Err(e) => {
                s.banner.set_title(&t!("Could not open the shell: {}", e));
                s.banner.set_revealed(true);
            }
        }
    }

    fn on_exit(&self, kind: Kind, status: i32) {
        let s = if kind == Kind::Phone { &self.phone } else { &self.pc };
        s.pid.set(None);
        let disconnected = kind == Kind::Phone && self.ctx.ready_serial().as_deref() != s.serial.borrow().as_deref();
        let msg = if disconnected {
            tr("Session closed: the phone disconnected. It will reopen when it comes back.").to_string()
        } else if status == 0 {
            tr("The session ended").to_string()
        } else {
            t!("The session ended (code {})", status >> 8)
        };
        s.banner.set_title(&msg);
        s.banner.set_revealed(true);
        s.term.feed(format!("\r\n\x1b[2m── {} ──\x1b[0m\r\n", tr("session ended")).as_bytes());
    }

    fn restart(&self, kind: Kind) {
        let s = if kind == Kind::Phone { &self.phone } else { &self.pc };
        s.kill();
        s.pid.set(None);
        if kind == Kind::Phone {
            *s.serial.borrow_mut() = None;
            s.term.reset(true, true);
            self.ensure_phone();
        } else {
            s.term.reset(true, true);
            self.ensure_pc();
        }
        s.term.grab_focus();
    }

    fn run_quick(&self, kind: Kind, cmd: &str) {
        if kind == Kind::Phone {
            self.switch_phone.set_active(true);
        } else {
            self.switch_pc.set_active(true);
        }
        self.select(kind);
        let s = self.session();
        if !s.running() {
            self.ctx.toast(if kind == Kind::Phone { tr("Connect and authorise the phone to use adb shell") } else { tr("The session is not active") });
            return;
        }
        // Ctrl+U clears whatever was half-typed before the command.
        s.term.feed_child(b"\x15");
        s.term.feed_child(format!("{cmd}\r").as_bytes());
        if kind == Kind::Phone {
            self.ctx.settings.borrow_mut().push_history(cmd);
            let _ = self.ctx.settings.borrow().save();
        }
    }

    fn copy(&self) {
        let s = self.session();
        if s.term.has_selection() {
            s.term.copy_clipboard_format(vte::Format::Text);
        } else if let Some(text) = s.term.text_format(vte::Format::Text) {
            util::copy_to_clipboard(&s.term, text.trim_end());
            self.ctx.toast(tr("Terminal content copied"));
        }
    }

    fn clear(&self) {
        let s = self.session();
        s.term.reset(true, true);
        if s.running() {
            // Asks the shell to redraw the prompt.
            s.term.feed_child(b"\x0c");
        }
    }

    fn save(&self) {
        let s = self.session();
        let Some(text) = s.term.text_format(vte::Format::Text) else {
            return;
        };
        let text = text.trim_end().to_string() + "\n";
        let ctx = self.ctx.clone();
        let stamp = glib::DateTime::now_local().ok().and_then(|d| d.format("%Y%m%d-%H%M%S").ok()).map(|g| g.to_string()).unwrap_or_default();
        let name = format!("terminal-{}-{stamp}.txt", if s.kind == Kind::Phone { "telefono" } else { "pc" });
        util::save_file(&self.root, tr("Save the terminal content"), &name, move |path| match std::fs::write(&path, &text) {
            Ok(()) => ctx.toast(&t!("Saved to {}", path.display())),
            Err(e) => ctx.toast(&t!("Could not save: {}", e)),
        });
    }

    /// Ctrl+Shift+C/V/K and Ctrl+plus/minus/0 for the font size.
    fn install_shortcuts(&self, s: &Rc<Session>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = self.me.clone();
        let term = s.term.clone();
        let kind = s.kind;
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(t) = w.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            if kind == Kind::Phone && matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) && !ctrl {
                t.record_command(&term);
            }
            if ctrl && shift {
                match key.to_lower() {
                    gdk::Key::c => {
                        t.copy();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::v => {
                        term.paste_clipboard();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::k => {
                        t.clear();
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }
            if ctrl && !shift || ctrl && matches!(key, gdk::Key::plus) {
                let scale = term.font_scale();
                let next = match key {
                    gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => Some((scale * 1.1).min(3.0)),
                    gdk::Key::minus | gdk::Key::KP_Subtract => Some((scale / 1.1).max(0.5)),
                    gdk::Key::_0 | gdk::Key::KP_0 => Some(1.0),
                    _ => None,
                };
                if let Some(v) = next {
                    term.set_font_scale(v);
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });
        s.term.add_controller(keys);
    }

    /// Saves the command on the cursor's line to the persistent history (only
    /// if it is an mksh prompt: inside `top` or `vi` it does nothing).
    fn record_command(&self, term: &vte::Terminal) {
        let (_, row) = term.cursor_position();
        let cols = term.column_count();
        let (text, _) = term.text_range_format(vte::Format::Text, (row - 3).max(0), 0, row, cols);
        let Some(text) = text else { return };
        // Long commands span several rows: the block from the prompt is joined.
        let lines: Vec<&str> = text.lines().collect();
        for start in (0..lines.len()).rev() {
            let joined: String = lines[start..].concat();
            if let Some(cmd) = adbscan_core::settings::command_after_prompt(&joined) {
                self.ctx.settings.borrow_mut().push_history(&cmd);
                if let Err(e) = self.ctx.settings.borrow().save() {
                    log::warn!("Could not save the history: {e}");
                }
                return;
            }
        }
    }

    /// Context menu: copy, paste, select all, clear.
    fn install_context_menu(&self, s: &Rc<Session>) {
        let menu = gtk::gio::Menu::new();
        menu.append(Some(tr("Copy")), Some("term.copy"));
        menu.append(Some(tr("Paste")), Some("term.paste"));
        menu.append(Some(tr("Select all")), Some("term.select-all"));
        menu.append(Some(trc("terminal", "Clear")), Some("term.clear"));
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&s.term);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);

        let group = gtk::gio::SimpleActionGroup::new();
        let add = |name: &str, f: Box<dyn Fn(&TerminalPage)>| {
            let a = gtk::gio::SimpleAction::new(name, None);
            let w = self.me.clone();
            a.connect_activate(move |_, _| {
                if let Some(t) = w.upgrade() {
                    f(&t);
                }
            });
            group.add_action(&a);
        };
        add("copy", Box::new(|t| t.copy()));
        add("paste", Box::new(|t| t.session().term.paste_clipboard()));
        add("select-all", Box::new(|t| t.session().term.select_all()));
        add("clear", Box::new(|t| t.clear()));
        s.term.insert_action_group("term", Some(&group));

        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let pop = popover.clone();
        click.connect_pressed(move |g, _, x, y| {
            pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.popup();
            g.set_state(gtk::EventSequenceState::Claimed);
        });
        s.term.add_controller(click);
    }
}

impl Page for TerminalPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn shown(&self) {
        match self.current.get() {
            Kind::Phone => self.ensure_phone(),
            Kind::Pc => self.ensure_pc(),
        }
        let term = self.session().term.clone();
        glib::idle_add_local_once(move || {
            term.grab_focus();
        });
    }

    fn on_event(&self, ev: &Event) {
        match ev {
            Event::Devices => self.ensure_phone(),
            Event::SettingsChanged => self.restyle(),
            _ => {}
        }
    }
}

impl Drop for TerminalPage {
    fn drop(&mut self) {
        self.phone.kill();
        self.pc.kill();
    }
}
