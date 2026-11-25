//! Screen: mirroring and recording with scrcpy, screenshots and remote control.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adbscan_core::exec::{StreamEvent, StreamHandle};
use adbscan_core::format;
use adbscan_core::models::now_secs;
use adbscan_core::paths;
use adbscan_core::scrcpy::{estimate_recording_mb, SessionOptions};
use adbscan_core::settings::ScrcpyPreset;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

thread_local! {
    /// Active scrcpy session (shared with the Ctrl+M quick action).
    static SESSION: RefCell<Option<StreamHandle>> = const { RefCell::new(None) };
    static SESSION_START: Cell<Option<Instant>> = const { Cell::new(None) };
    static LAST_SHOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

fn session_running() -> bool {
    SESSION.with(|s| s.borrow().as_ref().is_some_and(|h| h.running()))
}

struct MirrorPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    status: adw::ActionRow,
    start_btn: gtk::Button,
    record: adw::SwitchRow,
    record_only: adw::SwitchRow,
    camera: adw::SwitchRow,
    webcam: adw::SwitchRow,
    limit: adw::SpinRow,
    timer: gtk::Label,
    preset_combo: adw::ComboRow,
    fields: PresetFields,
    thumb: gtk::Picture,
    thumb_box: gtk::Box,
    log: gtk::Label,
}

struct PresetFields {
    max_size: adw::SpinRow,
    fps: adw::SpinRow,
    bitrate: adw::SpinRow,
    codec: adw::ComboRow,
    rotation: adw::ComboRow,
    audio: adw::SwitchRow,
    stay_awake: adw::SwitchRow,
    screen_off: adw::SwitchRow,
    touches: adw::SwitchRow,
    on_top: adw::SwitchRow,
    fullscreen: adw::SwitchRow,
    borderless: adw::SwitchRow,
    uhid: adw::SwitchRow,
    no_control: adw::SwitchRow,
}

const CODECS: [&str; 3] = ["h264", "h265", "av1"];
const ROTATIONS: [u32; 4] = [0, 90, 180, 270];

impl PresetFields {
    fn new() -> Self {
        let spin =
            |title: &str, sub: &str, min: f64, max: f64, step: f64| adw::SpinRow::builder().title(title).subtitle(sub).adjustment(&gtk::Adjustment::new(min, min, max, step, step * 10.0, 0.0)).build();
        let switch = |title: &str, sub: &str| adw::SwitchRow::builder().title(title).subtitle(sub).build();
        Self {
            max_size: spin(tr("Maximum resolution"), tr("Longest side in pixels (0 = original)"), 0.0, 4096.0, 16.0),
            fps: spin(tr("Maximum FPS"), tr("0 = no limit"), 0.0, 240.0, 1.0),
            bitrate: spin("Bitrate", "Mbit/s", 1.0, 100.0, 1.0),
            codec: adw::ComboRow::builder().title(tr("Video codec")).model(&gtk::StringList::new(&["H.264 (compatible)", tr("H.265 (higher quality)"), "AV1 (Android 14+)"])).build(),
            rotation: adw::ComboRow::builder().title(tr("Orientation")).model(&gtk::StringList::new(&[tr("Automatic"), "90°", "180°", "270°"])).build(),
            audio: switch(tr("Forward audio"), tr("Android 11 or later")),
            stay_awake: switch(tr("Stay awake"), tr("While connected")),
            screen_off: switch(tr("Turn off the phone's screen"), tr("The image stays on the PC")),
            touches: switch(tr("Show touches"), ""),
            on_top: switch(tr("Always on top"), ""),
            fullscreen: switch(tr("Fullscreen"), ""),
            borderless: switch(tr("Borderless"), ""),
            uhid: switch(tr("Physical keyboard (UHID)"), tr("Better with non-English keyboard layouts")),
            no_control: switch(tr("View only (no control)"), ""),
        }
    }

    fn load(&self, p: &ScrcpyPreset) {
        self.max_size.set_value(p.max_size as f64);
        self.fps.set_value(p.max_fps as f64);
        self.bitrate.set_value(p.bitrate_mbps.max(1) as f64);
        self.codec.set_selected(CODECS.iter().position(|c| *c == p.codec).unwrap_or(0) as u32);
        self.rotation.set_selected(ROTATIONS.iter().position(|r| *r == p.rotation).unwrap_or(0) as u32);
        self.audio.set_active(p.audio);
        self.stay_awake.set_active(p.stay_awake);
        self.screen_off.set_active(p.turn_screen_off);
        self.touches.set_active(p.show_touches);
        self.on_top.set_active(p.always_on_top);
        self.fullscreen.set_active(p.fullscreen);
        self.borderless.set_active(p.borderless);
        self.uhid.set_active(p.keyboard_uhid);
        self.no_control.set_active(p.no_control);
    }

    fn read(&self, name: &str) -> ScrcpyPreset {
        ScrcpyPreset {
            name: name.to_string(),
            max_size: self.max_size.value() as u32,
            max_fps: self.fps.value() as u32,
            bitrate_mbps: self.bitrate.value() as u32,
            codec: CODECS[self.codec.selected().min(2) as usize].into(),
            rotation: ROTATIONS[self.rotation.selected().min(3) as usize],
            audio: self.audio.is_active(),
            stay_awake: self.stay_awake.is_active(),
            turn_screen_off: self.screen_off.is_active(),
            show_touches: self.touches.is_active(),
            always_on_top: self.on_top.is_active(),
            fullscreen: self.fullscreen.is_active(),
            borderless: self.borderless.is_active(),
            keyboard_uhid: self.uhid.is_active(),
            no_control: self.no_control.is_active(),
        }
    }

    fn rows(&self) -> Vec<gtk::Widget> {
        vec![
            self.max_size.clone().upcast(),
            self.fps.clone().upcast(),
            self.bitrate.clone().upcast(),
            self.codec.clone().upcast(),
            self.rotation.clone().upcast(),
            self.audio.clone().upcast(),
            self.stay_awake.clone().upcast(),
            self.screen_off.clone().upcast(),
            self.touches.clone().upcast(),
            self.on_top.clone().upcast(),
            self.fullscreen.clone().upcast(),
            self.borderless.clone().upcast(),
            self.uhid.clone().upcast(),
            self.no_control.clone().upcast(),
        ]
    }
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    MirrorPage::new(ctx)
}

impl MirrorPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);

        let session = adw::PreferencesGroup::builder().title(tr("Mirror screen")).build();
        let status = adw::ActionRow::builder().title("scrcpy").build();
        let start_btn = gtk::Button::builder().label(tr("Start")).valign(gtk::Align::Center).build();
        start_btn.add_css_class("suggested-action");
        start_btn.add_css_class("pill");
        status.add_suffix(&start_btn);
        session.add(&status);
        let record = adw::SwitchRow::builder().title(tr("Record to file")).subtitle(t!("MP4 in {}", paths::recordings_dir().display())).build();
        let record_only = adw::SwitchRow::builder().title(tr("Record only (no window)")).build();
        let camera = adw::SwitchRow::builder().title(tr("Use the phone's camera")).subtitle(tr("Instead of the screen (Android 12+, scrcpy 2.2+)")).build();
        let webcam = adw::SwitchRow::builder().title(tr("Send to a virtual webcam on the PC")).build();
        let limit = adw::SpinRow::builder().title(tr("Time limit")).subtitle(tr("Seconds (0 = no limit)")).adjustment(&gtk::Adjustment::new(0.0, 0.0, 7200.0, 10.0, 60.0, 0.0)).build();
        let timer = util::label("", &["numeric", "dim"]);
        let timer_row = adw::ActionRow::builder().title(tr("Session")).build();
        timer_row.add_suffix(&timer);
        for r in [record.upcast_ref::<gtk::Widget>(), record_only.upcast_ref(), camera.upcast_ref(), webcam.upcast_ref(), limit.upcast_ref(), timer_row.upcast_ref()] {
            session.add(r);
        }
        content.append(&session);
        let log = util::label("", &["dim", "caption", "mono"]);
        log.set_wrap(true);
        log.set_selectable(true);
        content.append(&log);

        // Screenshots
        let shots = adw::PreferencesGroup::builder().title(tr("Screenshot")).description(t!("PNG in {}", paths::screenshots_dir().display())).build();
        let shot_btn = util::pill_button("adb-camera-photo-symbolic", tr("Capture"));
        shot_btn.add_css_class("suggested-action");
        shots.set_header_suffix(Some(&shot_btn));
        let thumb = gtk::Picture::builder().can_shrink(true).content_fit(gtk::ContentFit::Contain).height_request(260).build();
        thumb.add_css_class("screenshot-thumb");
        let thumb_box = util::vbox(8);
        let thumb_actions = util::hbox(6);
        thumb_actions.set_halign(gtk::Align::Center);
        let open_btn = util::pill_button("adb-folder-open-symbolic", tr("Open"));
        let folder_btn = util::pill_button("adb-folder-symbolic", tr("Folder"));
        let copy_btn = util::pill_button("adb-copy-symbolic", tr("Copy image"));
        for b in [&open_btn, &folder_btn, &copy_btn] {
            thumb_actions.append(b);
        }
        thumb_box.append(&thumb);
        thumb_box.append(&thumb_actions);
        thumb_box.set_visible(false);
        shots.add(&thumb_box);
        content.append(&shots);

        // Remote control
        let remote = adw::PreferencesGroup::builder().title(tr("Remote control")).description(tr("Sends keys without opening scrcpy")).build();
        let keys = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(8).column_spacing(6).row_spacing(6).build();
        for (icon, text, code) in [
            ("adb-system-shutdown-symbolic", tr("Uptime"), "KEYCODE_POWER"),
            ("adb-audio-volume-high-symbolic", "Vol +", "KEYCODE_VOLUME_UP"),
            ("adb-audio-volume-low-symbolic", "Vol −", "KEYCODE_VOLUME_DOWN"),
            ("adb-go-home-symbolic", tr("Home"), "KEYCODE_HOME"),
            ("adb-go-previous-symbolic", tr("Back"), "KEYCODE_BACK"),
            ("adb-view-app-grid-symbolic", tr("Recent"), "KEYCODE_APP_SWITCH"),
            ("adb-weather-clear-symbolic", tr("Wake up"), "KEYCODE_WAKEUP"),
            ("adb-night-light-symbolic", tr("Sleep"), "KEYCODE_SLEEP"),
        ] {
            let b = util::pill_button(icon, text);
            let c = ctx.clone();
            b.connect_clicked(move |_| {
                let code = code.to_string();
                c.device_op("keyevent", &code.clone(), tr("Key sent"), move |adb, s| adb.keyevent(&s, &code).map(|r| r.summary()), |_| {});
            });
            keys.append(&b);
        }
        remote.add(&keys);
        let text_row = adw::EntryRow::builder().title(tr("Type text into the phone's active field")).show_apply_button(true).build();
        {
            let c = ctx.clone();
            text_row.connect_apply(move |e| {
                let text = e.text().to_string();
                if text.is_empty() {
                    return;
                }
                e.set_text("");
                c.device_op("input_text", "", tr("Text sent"), move |adb, s| adb.input_text(&s, &text).map(|r| r.summary()), |_| {});
            });
        }
        remote.add(&text_row);
        content.append(&remote);

        // Presets
        let presets = adw::PreferencesGroup::builder().title(tr("scrcpy settings")).build();
        let preset_combo = adw::ComboRow::builder().title(tr("Profile")).build();
        let save_btn = util::icon_button("adb-save-symbolic", tr("Save to this profile"));
        save_btn.add_css_class("flat");
        let new_btn = util::icon_button("adb-list-add-symbolic", tr("Save as a new profile"));
        new_btn.add_css_class("flat");
        let hb = util::hbox(0);
        hb.append(&save_btn);
        hb.append(&new_btn);
        presets.set_header_suffix(Some(&hb));
        presets.add(&preset_combo);
        let fields = PresetFields::new();
        for r in fields.rows() {
            presets.add(&r);
        }
        content.append(&presets);

        let root = util::page_scroller(&content, 900);
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            status,
            start_btn,
            record,
            record_only,
            camera,
            webcam: webcam.clone(),
            limit,
            timer,
            preset_combo,
            fields,
            thumb,
            thumb_box,
            log,
        });
        this.reload_presets();
        {
            let w = this.me.clone();
            this.preset_combo.connect_selected_notify(move |c| {
                let Some(t) = w.upgrade() else { return };
                let preset = t.ctx.settings.borrow().scrcpy_presets.get(c.selected() as usize).cloned();
                if let Some(p) = preset {
                    t.ctx.settings.borrow_mut().scrcpy_active_preset = p.name.clone();
                    t.fields.load(&p);
                }
            });
        }
        {
            let w = this.me.clone();
            save_btn.connect_clicked(move |_| {
                let Some(t) = w.upgrade() else { return };
                let idx = t.preset_combo.selected() as usize;
                let name = t.ctx.settings.borrow().scrcpy_presets.get(idx).map(|p| p.name.clone());
                if let Some(name) = name {
                    let p = t.fields.read(&name);
                    t.ctx.settings.borrow_mut().scrcpy_presets[idx] = p;
                    t.ctx.save_settings();
                    t.ctx.toast(&t!("Profile “{}” saved", name));
                }
            });
        }
        {
            let w = this.me.clone();
            new_btn.connect_clicked(move |b| {
                let entry = gtk::Entry::builder().placeholder_text(tr("Profile name")).activates_default(true).build();
                let d = adw::AlertDialog::new(Some(tr("New profile")), None);
                d.set_extra_child(Some(&entry));
                d.add_responses(&[("cancel", tr("Cancel")), ("ok", tr("Save"))]);
                d.set_default_response(Some("ok"));
                d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
                let w = w.clone();
                d.connect_response(None, move |_, r| {
                    let name = entry.text().trim().to_string();
                    let Some(t) = w.upgrade() else { return };
                    if r != "ok" || name.is_empty() {
                        return;
                    }
                    let p = t.fields.read(&name);
                    {
                        let mut s = t.ctx.settings.borrow_mut();
                        s.scrcpy_presets.retain(|x| x.name != name);
                        s.scrcpy_presets.push(p);
                        s.scrcpy_active_preset = name.clone();
                    }
                    t.ctx.save_settings();
                    t.reload_presets();
                });
                d.present(Some(b));
            });
        }
        {
            let w = this.me.clone();
            this.start_btn.connect_clicked(move |b| {
                if let Some(t) = w.upgrade() {
                    if session_running() {
                        stop_session();
                    } else {
                        t.start(b.upcast_ref());
                    }
                }
            });
        }
        {
            let ctx = ctx.clone();
            shot_btn.connect_clicked(move |b| quick_screenshot(&ctx, b.upcast_ref()));
        }
        open_btn.connect_clicked(|_| {
            if let Some(p) = LAST_SHOT.with(|l| l.borrow().clone()) {
                util::open_path(&p);
            }
        });
        folder_btn.connect_clicked(|_| util::open_path(&paths::screenshots_dir()));
        {
            let ctx = ctx.clone();
            copy_btn.connect_clicked(move |b| {
                if let Some(p) = LAST_SHOT.with(|l| l.borrow().clone()) {
                    if let Ok(tex) = gtk::gdk::Texture::from_filename(&p) {
                        b.clipboard().set_texture(&tex);
                        ctx.toast(tr("Image copied to the clipboard"));
                    }
                }
            });
        }
        // Session timer and size estimate
        {
            let w = this.me.clone();
            glib::timeout_add_local(Duration::from_millis(500), move || {
                let Some(t) = w.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                t.tick();
                glib::ControlFlow::Continue
            });
        }
        this.refresh_tool();
        this.refresh_webcam();
        this
    }

    fn reload_presets(&self) {
        let s = self.ctx.settings.borrow().clone();
        let names: Vec<&str> = s.scrcpy_presets.iter().map(|p| p.name.as_str()).collect();
        self.preset_combo.set_model(Some(&gtk::StringList::new(&names)));
        let idx = s.scrcpy_presets.iter().position(|p| p.name == s.scrcpy_active_preset).unwrap_or(0);
        self.preset_combo.set_selected(idx as u32);
        self.fields.load(&s.active_preset());
    }

    fn refresh_tool(&self) {
        match self.ctx.scrcpy() {
            Some(sc) => {
                let v = sc.version.map(|(a, b)| format!("{a}.{b}")).unwrap_or_else(|| "?".into());
                self.status.set_title(&format!("scrcpy {v}"));
                self.status.set_subtitle(&sc.path().display().to_string());
                self.start_btn.set_sensitive(true);
                if sc.version.is_some_and(|v| v < (2, 0)) {
                    self.status.set_subtitle(tr("Old version: no audio or camera. scrcpy 2 or later is recommended."));
                }
            }
            None => {
                self.status.set_title(tr("scrcpy is not installed"));
                self.status.set_subtitle(tr("Install it with “sudo apt install scrcpy” or “sudo snap install scrcpy” (newer version), or set its path in Preferences."));
                self.start_btn.set_sensitive(false);
            }
        }
    }

    fn tick(&self) {
        let running = session_running();
        self.start_btn.set_label(if running { tr("Stop") } else { tr("Start") });
        if running {
            self.start_btn.remove_css_class("suggested-action");
            self.start_btn.add_css_class("destructive-action");
        } else {
            self.start_btn.remove_css_class("destructive-action");
            if self.ctx.scrcpy().is_some() {
                self.start_btn.add_css_class("suggested-action");
            }
        }
        match SESSION_START.with(|s| s.get()).filter(|_| running) {
            Some(start) => {
                let secs = start.elapsed().as_secs();
                let mut text = format::human_duration(Some(secs as f64));
                if self.record.is_active() {
                    let p = self.fields.read("");
                    text.push_str(&t!(" · ~{} MB recorded", format!("{:.0}", estimate_recording_mb(p.bitrate_mbps, secs as u32, p.audio))));
                }
                self.timer.set_text(&text);
            }
            None => {
                let p = self.fields.read("");
                let per_min = estimate_recording_mb(p.bitrate_mbps, 60, p.audio);
                self.timer.set_text(&t!("Estimated recording: {} MB/min", format!("{:.0}", per_min)));
            }
        }
    }

    fn start(&self, parent: &gtk::Widget) {
        let Some(sc) = self.ctx.scrcpy() else { return };
        let Some(dev) = self.ctx.selected_device().filter(|d| d.connected()) else {
            self.ctx.toast(tr("No device is ready"));
            return;
        };
        let preset = self.fields.read("");
        let record = if self.record.is_active() {
            let dir = paths::recordings_dir();
            let _ = std::fs::create_dir_all(&dir);
            let name = sanitize(&self.ctx.device_title(&dev));
            Some(dir.join(format!("{name}-{}.mp4", format::file_stamp(now_secs()))))
        } else {
            None
        };
        let opts = SessionOptions {
            serial: dev.serial.clone(),
            // Title of the scrcpy window (visible on screen): display name.
            title: format!("{} - ADB Scan", self.ctx.device_label(&dev)),
            record: record.clone(),
            record_only: self.record_only.is_active() && record.is_some(),
            time_limit: self.limit.value() as u32,
            camera: self.camera.is_active(),
            v4l2_sink: if self.webcam.is_active() { self.loopback_device() } else { None },
        };
        let log = self.log.clone();
        launch(&self.ctx, parent, &sc, &preset, &opts, Some(log), record);
    }
}

impl MirrorPage {
    fn loopback_device(&self) -> Option<String> {
        adbscan_core::scrcpy::v4l2_loopback_devices(std::path::Path::new("/sys/class/video4linux")).into_iter().next().map(|(d, _)| d)
    }

    /// Virtual webcam: needs the v4l2loopback module loaded (scrcpy sends the
    /// video to /dev/videoN and any video-call app sees it).
    fn refresh_webcam(&self) {
        match adbscan_core::scrcpy::v4l2_loopback_devices(std::path::Path::new("/sys/class/video4linux")).into_iter().next() {
            Some((dev, name)) => {
                self.webcam.set_sensitive(true);
                self.webcam.set_subtitle(&t!("{} ({}): it will appear as a camera in Meet, OBS or Zoom", dev, name));
            }
            None => {
                self.webcam.set_active(false);
                self.webcam.set_sensitive(false);
                self.webcam.set_subtitle(tr("Needs v4l2loopback: sudo apt install v4l2loopback-dkms and sudo modprobe v4l2loopback card_label=\"ADB Scan\" exclusive_caps=1"));
            }
        }
    }
}

/// If the phone disappears from ADB with scrcpy open, the window freezes on the
/// last frame: notify and close it.
fn watch_connection(ctx: &Rc<Ctx>, serial: String) {
    let ctx = ctx.clone();
    let missing = Cell::new(0u8);
    glib::timeout_add_local(std::time::Duration::from_millis(1000), move || {
        if !session_running() {
            return glib::ControlFlow::Break;
        }
        let online = ctx.devices().iter().any(|d| d.serial == serial && d.connected());
        missing.set(if online { 0 } else { missing.get() + 1 });
        // Two seconds in a row without the phone: it is not a USB blip.
        if missing.get() >= 2 {
            stop_session();
            ctx.send_notification("scrcpy-cut", tr("The video was cut off"), tr("The phone disconnected and the scrcpy window was closed."), "adb-video-display-symbolic", false);
            ctx.toast(tr("The phone disconnected: scrcpy was closed"));
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn stop_session() {
    SESSION.with(|s| {
        if let Some(h) = s.borrow_mut().take() {
            h.cancel();
        }
    });
}

fn launch(ctx: &Rc<Ctx>, _parent: &gtk::Widget, sc: &adbscan_core::scrcpy::Scrcpy, preset: &ScrcpyPreset, opts: &SessionOptions, log: Option<gtk::Label>, record: Option<PathBuf>) {
    stop_session();
    let (tx, rx) = async_channel::unbounded::<StreamEvent>();
    let adb_path = ctx.adb().map(|a| a.path().to_path_buf());
    match sc.launch(preset, opts, adb_path.as_deref(), move |ev| {
        let _ = tx.send_blocking(ev);
    }) {
        Ok(h) => {
            SESSION.with(|s| *s.borrow_mut() = Some(h));
            SESSION_START.with(|s| s.set(Some(Instant::now())));
            ctx.audit("scrcpy", Some(&opts.serial), true, record.as_ref().map(|r| r.display().to_string()).as_deref());
            watch_connection(ctx, opts.serial.clone());
            let ctx = ctx.clone();
            let mut warned = false;
            glib::spawn_future_local(async move {
                let mut tail: Vec<String> = Vec::new();
                while let Ok(ev) = rx.recv().await {
                    match ev {
                        StreamEvent::Stdout(l) | StreamEvent::Stderr(l) => {
                            // Video cut: notify instead of leaving the window frozen.
                            let lower = l.to_lowercase();
                            if !warned && (lower.contains("device disconnected") || lower.contains("demuxer") || lower.contains("decoder error") || lower.contains("connection reset")) {
                                warned = true;
                                ctx.send_notification(
                                    "scrcpy-cut",
                                    tr("The video was cut off"),
                                    tr("scrcpy lost the connection to the phone. Mirror the screen again once it is connected."),
                                    "adb-video-display-symbolic",
                                    false,
                                );
                                ctx.toast(tr("The scrcpy video was cut off"));
                            }
                            tail.push(l);
                            if tail.len() > 6 {
                                tail.remove(0);
                            }
                            if let Some(log) = &log {
                                log.set_text(&ctx.shown(&tail.join("\n")));
                            }
                        }
                        StreamEvent::Exit { code, cancelled } => {
                            SESSION.with(|s| s.borrow_mut().take());
                            SESSION_START.with(|s| s.set(None));
                            if let Some(r) = record.clone().filter(|r| r.exists()) {
                                ctx.toast_action(tr("Recording saved"), tr("Open"), move || util::open_path(&r));
                            } else if !cancelled && code != Some(0) {
                                let last = tail.iter().rev().find(|l| l.contains("ERROR")).or(tail.last()).cloned().unwrap_or_default();
                                ctx.toast(&t!("scrcpy exited with an error: {}", last));
                            }
                            break;
                        }
                    }
                }
            });
        }
        Err(e) => ctx.toast(&t!("Could not start scrcpy: {}", e)),
    }
}

/// Quick action (Ctrl+M and the tray menu): mirror with the active profile.
pub fn quick_mirror(ctx: &Rc<Ctx>, parent: &gtk::Widget) {
    if session_running() {
        stop_session();
        return;
    }
    let Some(sc) = ctx.scrcpy() else {
        util::show_error(parent, tr("scrcpy is not installed"), tr("Install it with “sudo apt install scrcpy” or set its path in Preferences → Tools."));
        return;
    };
    let Some(dev) = ctx.selected_device().filter(|d| d.connected()) else {
        ctx.toast(tr("No device is ready"));
        return;
    };
    let preset = ctx.settings.borrow().active_preset();
    let opts = SessionOptions { serial: dev.serial.clone(), title: format!("{} - ADB Scan", ctx.device_label(&dev)), ..Default::default() };
    launch(ctx, parent, &sc, &preset, &opts, None, None);
}

/// Quick action (Ctrl+Shift+S): captures and saves to the “ADB Scan” folder
/// inside the Pictures folder.
pub fn quick_screenshot(ctx: &Rc<Ctx>, _parent: &gtk::Widget) {
    let (Some(adb), Some(dev)) = (ctx.adb(), ctx.selected_device().filter(|d| d.connected())) else {
        ctx.toast(tr("No device is ready"));
        return;
    };
    let dir = paths::screenshots_dir();
    let name = sanitize(&ctx.device_title(&dev));
    let file = dir.join(format!("{name}-{}.png", format::file_stamp(now_secs())));
    let serial = dev.serial.clone();
    let ctx2 = ctx.clone();
    let f2 = file.clone();
    util::run_bg(
        move || -> adbscan_core::Result<()> {
            let png = adb.screenshot_png(&serial)?;
            std::fs::create_dir_all(&dir)?;
            std::fs::write(&f2, png)?;
            Ok(())
        },
        move |r| match r {
            Ok(()) => {
                LAST_SHOT.with(|l| *l.borrow_mut() = Some(file.clone()));
                ctx2.audit("screenshot", Some(&file.display().to_string()), true, None);
                ctx2.emit(Event::Screenshot);
                let f = file.clone();
                ctx2.toast_action(tr("Screenshot saved"), tr("Open"), move || util::open_path(&f));
            }
            Err(e) => ctx2.toast(&t!("Could not take the screenshot: {}", e)),
        },
    );
}

impl Page for MirrorPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn shown(&self) {
        // The module can be loaded while the app is open.
        self.refresh_webcam();
    }
    fn on_event(&self, ev: &Event) {
        match ev {
            Event::Tools | Event::SettingsChanged => self.refresh_tool(),
            Event::Screenshot => {
                if let Some(p) = LAST_SHOT.with(|l| l.borrow().clone()) {
                    self.thumb.set_filename(Some(&p));
                    self.thumb_box.set_visible(true);
                }
            }
            _ => {}
        }
    }
}
