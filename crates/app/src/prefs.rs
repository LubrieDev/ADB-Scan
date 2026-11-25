//! Preferences (AdwPreferencesDialog).

use adbscan_core::{
    i18n::{tr, Language},
    t,
};
use adw::prelude::*;
use std::rc::Rc;

use adbscan_core::adb::{find_tool, tool_version, Tool};
use adbscan_core::paths;
use adbscan_core::settings::ThemeMode;

use crate::ctx::{Ctx, Event};
use crate::theme;
use crate::util;

pub fn show(ctx: &Rc<Ctx>, parent: &gtk::Widget) {
    let dialog = adw::PreferencesDialog::builder().title(tr("Preferences")).search_enabled(true).build();
    let s = ctx.settings.borrow().clone();

    // -- General ----------------------------------------------------------------
    let general = adw::PreferencesPage::builder().title("General").icon_name("adb-settings-symbolic").build();
    let look = adw::PreferencesGroup::builder().title(tr("Appearance")).build();
    // The texts are built once at startup: a new language applies after restarting.
    let language_row = adw::ComboRow::builder()
        .title(tr("Language"))
        .subtitle(tr("Applied when ADB Scan restarts"))
        .model(&gtk::StringList::new(&Language::ALL.map(|l| l.native_name())))
        .build();
    language_row.set_selected(Language::ALL.iter().position(|l| *l == s.language).unwrap_or(0) as u32);
    look.add(&language_row);
    let theme_row = adw::ComboRow::builder().title(tr("Style")).model(&gtk::StringList::new(&[tr("Follow the system"), tr("Light"), tr("Dark")])).build();
    theme_row.set_selected(match s.theme {
        ThemeMode::System => 0,
        ThemeMode::Light => 1,
        ThemeMode::Dark => 2,
    });
    look.add(&theme_row);
    let accent_row = adw::ActionRow::builder().title(tr("Accent colour")).build();
    let accents = util::hbox(6);
    accents.set_valign(gtk::Align::Center);
    let mut first: Option<gtk::ToggleButton> = None;
    for (key, name, bg, _) in theme::ACCENTS {
        let b = gtk::ToggleButton::new();
        b.set_tooltip_text(Some(tr(name)));
        b.set_size_request(26, 26);
        b.add_css_class("circular");
        let css = gtk::CssProvider::new();
        css.load_from_string(&format!("button.accent-{key} {{ background: {bg}; min-width: 22px; min-height: 22px; }} button.accent-{key}:checked {{ box-shadow: 0 0 0 3px alpha({bg}, 0.45); }}"));
        gtk::style_context_add_provider_for_display(&b.display(), &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        b.add_css_class(&format!("accent-{key}"));
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        b.set_active(s.accent == key);
        let ctx2 = ctx.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                ctx2.settings.borrow_mut().accent = key.to_string();
                theme::apply_accent(key);
                ctx2.save_settings();
            }
        });
        accents.append(&b);
    }
    accent_row.add_suffix(&accents);
    look.add(&accent_row);
    let anim = adw::SwitchRow::builder().title(tr("Animations")).subtitle(tr("Transitions, smooth charts and animated indicators")).active(s.animations).build();
    look.add(&anim);
    general.add(&look);

    let resid = adw::PreferencesGroup::builder().title(tr("Background")).build();
    let close_tray = adw::SwitchRow::builder().title(tr("Close to the tray")).subtitle(tr("When the window is closed, ADB Scan stays in the tray")).active(s.close_to_tray).build();
    let hidden = adw::SwitchRow::builder().title(tr("Start hidden")).subtitle(tr("Only the tray icon when the app opens")).active(s.start_hidden).build();
    let autostart = adw::SwitchRow::builder().title(tr("Start with the session")).subtitle(tr("Opens in the tray when you log in")).active(autostart_enabled()).build();
    for r in [&close_tray, &hidden, &autostart] {
        resid.add(r);
    }
    general.add(&resid);

    let modes = adw::PreferencesGroup::builder()
        .title(tr("Analysis"))
        .description(tr("The basic analysis works without developer options. USB debugging is optional and only adds information."))
        .build();
    let invite = adw::SwitchRow::builder()
        .title(tr("Invite to the advanced analysis"))
        .subtitle(tr("Shows the optional suggestion to turn on USB debugging when the phone does not have it on"))
        .active(s.show_advanced_invite)
        .build();
    modes.add(&invite);
    general.add(&modes);

    dialog.add(&general);

    // -- Monitoring: telemetry and alerts ------------------------------------------
    let monitor = adw::PreferencesPage::builder().title(tr("Monitoring")).icon_name("adb-activity-symbolic").build();
    let live = adw::PreferencesGroup::builder().title(tr("Live telemetry")).description(tr("A single ADB stream shared by all sections")).build();
    let live_ms = adw::SpinRow::builder()
        .title(tr("Interval with the window open"))
        .subtitle(tr("Milliseconds (250–5000)"))
        .adjustment(&gtk::Adjustment::new(s.live_refresh_ms as f64, 250.0, 5000.0, 50.0, 250.0, 0.0))
        .build();
    let bg_ms = adw::SpinRow::builder()
        .title(tr("Background interval"))
        .subtitle(tr("Only the tray icon: saves battery and CPU"))
        .adjustment(&gtk::Adjustment::new(s.background_refresh_ms as f64, 1000.0, 30000.0, 500.0, 1000.0, 0.0))
        .build();
    live.add(&live_ms);
    live.add(&bg_ms);
    monitor.add(&live);

    let ng = adw::PreferencesGroup::builder().title(tr("Notifications")).description(tr("Desktop notifications, also while ADB Scan is in the tray")).build();
    let n_all = adw::SwitchRow::builder().title(tr("Show notifications")).active(s.notifications_enabled).build();
    ng.add(&n_all);
    monitor.add(&ng);
    let ncon = adw::PreferencesGroup::builder().title(tr("Connection")).build();
    let n_usb = adw::SwitchRow::builder().title(tr("Phone detected over USB")).active(s.notify_usb_detected).build();
    let n_auth = adw::SwitchRow::builder().title(tr("Waiting for authorisation")).active(s.notify_authorization).build();
    let n_conn = adw::SwitchRow::builder().title(tr("Connected")).active(s.notify_connected).build();
    let n_disc = adw::SwitchRow::builder().title(tr("Disconnected")).active(s.notify_disconnected).build();
    let n_err = adw::SwitchRow::builder().title(tr("Connection errors")).active(s.notify_errors).build();
    let n_unstable = adw::SwitchRow::builder()
        .title(tr("Unstable USB connection"))
        .subtitle(tr("The phone drops off USB several times in a minute: loose plug, worn cable or dirty port"))
        .active(s.notify_usb_unstable)
        .build();
    let n_slow = adw::SwitchRow::builder()
        .title(tr("The cable limits the speed"))
        .subtitle(tr("The phone connects at USB 2.0 although it reached USB 3 on this PC before"))
        .active(s.notify_usb_slow)
        .build();
    for r in [&n_usb, &n_auth, &n_conn, &n_disc, &n_err, &n_unstable, &n_slow] {
        ncon.add(r);
    }
    monitor.add(&ncon);
    let nbat = adw::PreferencesGroup::builder().title(tr("Battery and temperature")).build();
    let n_full = adw::SwitchRow::builder().title(tr("Battery charged to 100 %")).active(s.notify_battery_full).build();
    let n_low = adw::SwitchRow::builder().title(tr("Low battery")).subtitle(tr("While not charging")).active(s.notify_battery_low).build();
    let low = adw::SpinRow::builder().title(tr("Low battery threshold")).subtitle("%").adjustment(&gtk::Adjustment::new(s.battery_low_percent as f64, 5.0, 50.0, 1.0, 5.0, 0.0)).build();
    let n_temp = adw::SwitchRow::builder().title(tr("High battery temperature")).subtitle(tr("Also any sensor above its limit for 30 s")).active(s.notify_temperature).build();
    let temp = adw::SpinRow::builder().title(tr("Temperature threshold")).subtitle("°C").adjustment(&gtk::Adjustment::new(s.temperature_alert_c, 30.0, 70.0, 1.0, 5.0, 0.0)).build();
    for r in [n_full.upcast_ref::<gtk::Widget>(), n_low.upcast_ref(), low.upcast_ref(), n_temp.upcast_ref(), temp.upcast_ref()] {
        nbat.add(r);
    }
    monitor.add(&nbat);
    let nperf = adw::PreferencesGroup::builder().title(tr("Performance")).build();
    let n_cpu = adw::SwitchRow::builder().title(tr("Phone CPU at full load")).subtitle(tr("More than a minute above 80 %, with the top consumers")).active(s.notify_cpu).build();
    nperf.add(&n_cpu);
    monitor.add(&nperf);
    for r in [&n_usb, &n_auth, &n_conn, &n_disc, &n_err, &n_unstable, &n_slow, &n_full, &n_low, &n_cpu, &n_temp] {
        n_all.bind_property("active", r, "sensitive").sync_create().build();
    }
    // Each threshold only matters with its alert on.
    for (alert, threshold) in [(&n_low, &low), (&n_temp, &temp)] {
        let t = threshold.clone();
        let update = move |a: &adw::SwitchRow| t.set_sensitive(a.is_active() && a.is_sensitive());
        update(alert);
        let u = update.clone();
        alert.connect_active_notify(move |a| u(a));
        alert.connect_sensitive_notify(move |a| update(a));
    }
    dialog.add(&monitor);

    // -- Connection: tools and what is remembered about connections --------------
    let tools = adw::PreferencesPage::builder().title(tr("Connection")).icon_name("adb-link-symbolic").build();
    let tg = adw::PreferencesGroup::builder().title(tr("Routes")).description(tr("Leave them empty to look for them automatically in the PATH and the usual locations")).build();
    let adb_row = adw::EntryRow::builder().title("adb").text(&s.adb_path).show_apply_button(true).build();
    let fb_row = adw::EntryRow::builder().title("fastboot").text(&s.fastboot_path).show_apply_button(true).build();
    let sc_row = adw::EntryRow::builder().title("scrcpy").text(&s.scrcpy_path).show_apply_button(true).build();
    let ts_row = adw::EntryRow::builder().title("tshark").text(&s.tshark_path).show_apply_button(true).build();
    for r in [&adb_row, &fb_row, &sc_row, &ts_row] {
        tg.add(r);
    }
    tools.add(&tg);
    let detected = adw::PreferencesGroup::builder().title(tr("Detected")).build();
    let det_rows: Vec<(Tool, adw::ActionRow)> = [Tool::Adb, Tool::Fastboot, Tool::Scrcpy]
        .into_iter()
        .map(|t| {
            let r = adw::ActionRow::builder().title(t.name()).subtitle(tr("Searching…")).build();
            detected.add(&r);
            (t, r)
        })
        .collect();
    let wizard = adw::ActionRow::builder()
        .title(tr("Tools assistant"))
        .subtitle(tr("Downloads the missing ones from their official (verified) sources or lets you choose another path. Also: sudo apt install adb fastboot scrcpy"))
        .activatable(true)
        .build();
    wizard.add_suffix(&util::icon("adb-wrench-symbolic"));
    {
        let (c, d) = (ctx.clone(), dialog.clone());
        wizard.connect_activated(move |_| crate::setup::show(&c, &d));
    }
    detected.add(&wizard);
    tools.add(&detected);
    let remembered = adw::PreferencesGroup::builder().title(tr("Remembered")).build();
    let wifi_known = adw::ActionRow::builder()
        .title(tr("Wireless addresses"))
        .subtitle(&t!("{} addresses used with wireless ADB, offered when connecting again", s.wireless_history.len()))
        .build();
    let usb_known = adw::ActionRow::builder()
        .title(tr("USB speed of each phone"))
        .subtitle(&t!("{} phones. The fastest link each one reached tells a slow cable from a slow phone", s.usb_best_speed.len()))
        .build();
    for (row, which) in [(&wifi_known, 0), (&usb_known, 1)] {
        let forget = gtk::Button::with_label(tr("Forget"));
        forget.set_valign(gtk::Align::Center);
        let (c, r) = (ctx.clone(), row.clone());
        forget.connect_clicked(move |b| {
            {
                let mut s = c.settings.borrow_mut();
                if which == 0 {
                    s.wireless_history.clear();
                } else {
                    s.usb_best_speed.clear();
                }
            }
            c.save_settings();
            r.set_subtitle(tr("Forgotten"));
            b.set_sensitive(false);
        });
        forget.set_sensitive(if which == 0 { !s.wireless_history.is_empty() } else { !s.usb_best_speed.is_empty() });
        row.add_suffix(&forget);
        remembered.add(row);
    }
    tools.add(&remembered);
    dialog.add(&tools);

    // -- Data and security --------------------------------------------------------------
    let data = adw::PreferencesPage::builder().title(tr("Privacy and data")).icon_name("adb-shield-symbolic").build();
    let privacy_group = adw::PreferencesGroup::builder()
        .title(tr("Privacy"))
        .description(tr("It only changes what is shown: the data, the queries to the phone, the history and what is copied or exported are not modified."))
        .build();
    let privacy =
        adw::SwitchRow::builder().title(tr("Private mode")).subtitle(tr("Hides serial numbers, IP and MAC addresses, Wi-Fi networks and nicknames across the whole app (Ctrl+Shift+P)")).active(s.privacy_mode).build();
    privacy_group.add(&privacy);
    data.add(&privacy_group);
    {
        let c = ctx.clone();
        privacy.connect_active_notify(move |r| c.set_private_mode(r.is_active()));
    }
    let updates_group = adw::PreferencesGroup::builder().title(tr("Updates")).build();
    let updates = adw::SwitchRow::builder()
        .title(tr("Check for new versions"))
        .subtitle(tr("Once at startup, on GitHub. It only shows a notice with a link: nothing is downloaded or installed"))
        .active(s.check_updates)
        .build();
    updates_group.add(&updates);
    data.add(&updates_group);
    let safety = adw::PreferencesGroup::builder().title(tr("Log")).build();
    let audit = adw::SwitchRow::builder().title(tr("Log operations")).subtitle(tr("Installs, deletions, flashing… in the history")).active(s.audit_log).build();
    safety.add(&audit);
    data.add(&safety);
    let lg = adw::PreferencesGroup::builder().title(tr("Logcat and terminal")).build();
    let lc_save = adw::SwitchRow::builder().title(tr("Save logcat sessions to disk")).active(s.logcat_save_sessions).build();
    let lc_lines = adw::SpinRow::builder().title(tr("Logcat lines in memory")).adjustment(&gtk::Adjustment::new(s.logcat_buffer_lines as f64, 1000.0, 200000.0, 1000.0, 10000.0, 0.0)).build();
    let term_lines = adw::SpinRow::builder().title(tr("Terminal lines")).adjustment(&gtk::Adjustment::new(s.terminal_scrollback_lines as f64, 500.0, 100000.0, 500.0, 5000.0, 0.0)).build();
    lg.add(&lc_save);
    lg.add(&lc_lines);
    lg.add(&term_lines);
    data.add(&lg);
    let store = adw::PreferencesGroup::builder()
        .title(tr("Local storage"))
        .description(tr("Nothing leaves this computer and there is no telemetry. It only connects to the Internet when you ask: the public IP (queried by the phone), the tools assistant's downloads and, if it is on, the check for new versions."))
        .build();
    let retention = adw::SpinRow::builder().title(tr("Keep history")).subtitle(tr("Days")).adjustment(&gtk::Adjustment::new(s.retain_session_days as f64, 1.0, 3650.0, 1.0, 30.0, 0.0)).build();
    store.add(&retention);
    for (title, path) in [
        (tr("Configuration"), paths::config_file()),
        (tr("History (SQLite)"), paths::database_file()),
        (tr("Logcat captures"), paths::logcat_dir()),
        (tr("Screenshots"), paths::screenshots_dir()),
        (tr("Recordings"), paths::recordings_dir()),
    ] {
        let row = adw::ActionRow::builder().title(title).subtitle(path.display().to_string()).activatable(true).build();
        row.set_subtitle_selectable(true);
        row.add_suffix(&util::icon("adb-folder-open-symbolic"));
        row.connect_activated(move |_| util::reveal(&path));
        store.add(&row);
    }
    data.add(&store);
    dialog.add(&data);

    // -- Tool detection in the background ---------------------------------------
    let refresh_detect = {
        let ctx = ctx.clone();
        let rows = det_rows.clone();
        Rc::new(move || {
            let s = ctx.settings.borrow().clone();
            let cfg = [s.adb_path.clone(), s.fastboot_path.clone(), s.scrcpy_path.clone()];
            for ((tool, row), c) in rows.iter().zip(cfg) {
                let tool = *tool;
                let row = row.clone();
                util::run_bg(
                    move || find_tool(tool, Some(&c)).map(|p| (tool_version(&p, tool).unwrap_or_default(), p)),
                    move |r| match r {
                        Some((v, p)) => {
                            row.set_subtitle(&format!("{} · {}", p.display(), if v.is_empty() { tr("unknown version").into() } else { v }));
                            row.remove_css_class("error");
                        }
                        None => {
                            row.set_subtitle(tr("Not found"));
                            row.add_css_class("error");
                        }
                    },
                );
            }
        })
    };
    refresh_detect();

    // -- Saving ----------------------------------------------------------------------
    macro_rules! bind_switch {
        ($row:expr, $field:ident) => {{
            let c = ctx.clone();
            $row.connect_active_notify(move |r| {
                c.settings.borrow_mut().$field = r.is_active();
                c.save_settings();
            });
        }};
    }
    bind_switch!(close_tray, close_to_tray);
    {
        let c = ctx.clone();
        invite.connect_active_notify(move |r| {
            c.settings.borrow_mut().show_advanced_invite = r.is_active();
            c.save_settings();
            c.emit(Event::SettingsChanged);
        });
    }
    bind_switch!(hidden, start_hidden);
    bind_switch!(n_all, notifications_enabled);
    bind_switch!(n_usb, notify_usb_detected);
    bind_switch!(n_auth, notify_authorization);
    bind_switch!(n_conn, notify_connected);
    bind_switch!(n_disc, notify_disconnected);
    bind_switch!(n_err, notify_errors);
    bind_switch!(n_full, notify_battery_full);
    bind_switch!(n_temp, notify_temperature);
    bind_switch!(n_cpu, notify_cpu);
    bind_switch!(n_low, notify_battery_low);
    bind_switch!(n_unstable, notify_usb_unstable);
    bind_switch!(n_slow, notify_usb_slow);
    bind_switch!(audit, audit_log);
    bind_switch!(updates, check_updates);
    bind_switch!(lc_save, logcat_save_sessions);
    {
        let c = ctx.clone();
        anim.connect_active_notify(move |r| {
            c.settings.borrow_mut().animations = r.is_active();
            if let Some(gs) = gtk::Settings::default() {
                if r.is_active() {
                    gs.reset_property("gtk-enable-animations");
                } else {
                    gs.set_gtk_enable_animations(false);
                }
            }
            c.save_settings();
        });
    }
    {
        let c = ctx.clone();
        let d = dialog.clone();
        language_row.connect_selected_notify(move |r| {
            let language = Language::ALL.get(r.selected() as usize).copied().unwrap_or_default();
            if c.settings.borrow().language == language {
                return;
            }
            c.settings.borrow_mut().language = language;
            c.save_settings();
            let toast = adw::Toast::builder().title(tr("The new language is applied when ADB Scan restarts")).button_label(tr("Restart now")).timeout(0).build();
            let c2 = c.clone();
            toast.connect_button_clicked(move |_| restart(&c2));
            d.add_toast(toast);
        });
    }
    {
        let c = ctx.clone();
        theme_row.connect_selected_notify(move |r| {
            c.settings.borrow_mut().theme = match r.selected() {
                1 => ThemeMode::Light,
                2 => ThemeMode::Dark,
                _ => ThemeMode::System,
            };
            theme::apply(&c.settings.borrow());
            c.save_settings();
        });
    }
    {
        let c = ctx.clone();
        autostart.connect_active_notify(move |r| {
            match set_autostart(r.is_active()) {
                Ok(()) => c.settings.borrow_mut().autostart = r.is_active(),
                Err(e) => c.toast(&t!("Could not change autostart: {}", e)),
            }
            c.save_settings();
        });
    }
    macro_rules! bind_spin {
        ($row:expr, |$s:ident, $v:ident| $body:expr) => {{
            let c = ctx.clone();
            $row.connect_value_notify(move |r| {
                {
                    let $v = r.value();
                    let mut $s = c.settings.borrow_mut();
                    $body;
                }
                c.save_settings();
            });
        }};
    }
    bind_spin!(live_ms, |s, v| s.live_refresh_ms = v as u32);
    bind_spin!(bg_ms, |s, v| s.background_refresh_ms = v as u32);
    bind_spin!(temp, |s, v| s.temperature_alert_c = v);
    bind_spin!(low, |s, v| s.battery_low_percent = v as u32);
    bind_spin!(lc_lines, |s, v| s.logcat_buffer_lines = v as u32);
    bind_spin!(term_lines, |s, v| s.terminal_scrollback_lines = v as u32);
    bind_spin!(retention, |s, v| s.retain_session_days = v as u32);
    {
        // Rate changes: restarts the stream with the new interval.
        let c = ctx.clone();
        live_ms.connect_value_notify(move |_| c.sync_live());
        let c = ctx.clone();
        bg_ms.connect_value_notify(move |_| c.sync_live());
    }
    for (row, which) in [(&adb_row, 0), (&fb_row, 1), (&sc_row, 2), (&ts_row, 3)] {
        let c = ctx.clone();
        let rd = refresh_detect.clone();
        row.connect_apply(move |e| {
            let v = e.text().trim().to_string();
            {
                let mut s = c.settings.borrow_mut();
                match which {
                    0 => s.adb_path = v,
                    1 => s.fastboot_path = v,
                    2 => s.scrcpy_path = v,
                    _ => s.tshark_path = v,
                }
            }
            c.save_settings();
            c.detect_tools();
            if which == 0 {
                c.start_monitors();
            }
            c.emit(Event::Tools);
            rd();
            c.toast(tr("Path saved"));
        });
    }
    dialog.present(Some(parent));
}

fn autostart_enabled() -> bool {
    paths::autostart_file().exists()
}

/// Creates or removes `~/.config/autostart/adb-scan.desktop`.
pub fn set_autostart(enable: bool) -> std::io::Result<()> {
    let file = paths::autostart_file();
    if !enable {
        if file.exists() {
            std::fs::remove_file(&file)?;
        }
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    // If it is installed, the system launcher is used.
    let exec = if exe.starts_with("/usr") { "adb-scan".to_string() } else { format!("\"{}\"", exe.display()) };
    let content = format!(
        "[Desktop Entry]\nType=Application\nName=ADB Scan\nComment=Android device monitoring\nExec={exec} --hidden\nIcon=adb-scan\nTerminal=false\nX-GNOME-Autostart-enabled=true\nX-GNOME-Autostart-Delay=5\n"
    );
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, content)
}

/// Quits and starts ADB Scan again. The new process waits until this one has
/// exited: ADB Scan is a single instance, and started earlier it would just
/// hand over to the old one.
fn restart(ctx: &Rc<Ctx>) {
    let Ok(exe) = std::env::current_exe() else {
        ctx.toast(tr("Could not restart: reopen ADB Scan by hand"));
        return;
    };
    let spawned = std::process::Command::new("sh")
        .args(["-c", "tail --pid=\"$1\" -f /dev/null; exec \"$2\"", "adb-scan-restart", &std::process::id().to_string()])
        .arg(exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        Ok(_) => ctx.app.quit(),
        Err(e) => ctx.toast(&t!("Could not restart: {}", e)),
    }
}
