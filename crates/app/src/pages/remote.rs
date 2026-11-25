//! Remote: control the phone from the laptop. Media, calls, sending links and
//! messages, and finding the phone (see `adbscan_core::remote`).

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adbscan_core::adb::Adb;
use adbscan_core::models::now_secs;
use adbscan_core::remote::{CallState, MediaKey, NowPlaying};
use adbscan_core::schedule::{Scheduled, ScheduledAction, DAY_LETTERS};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

/// How often the state (call and what is playing) is read again while the page is in
/// view.
const REFRESH_SECS: f64 = 3.0;

struct RemotePage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    media_row: adw::ActionRow,
    play_button: gtk::Button,
    call_row: adw::ActionRow,
    answer_button: gtk::Button,
    hang_button: gtk::Button,
    number: adw::EntryRow,
    url: adw::EntryRow,
    sms_number: adw::EntryRow,
    sms_text: adw::EntryRow,
    reading: Cell<bool>,
    last_read: Cell<f64>,
    call_state: Cell<CallState>,
    playing: RefCell<Option<NowPlaying>>,
    sched_group: adw::PreferencesGroup,
    /// Rows of the saved actions (rebuilt when the list changes).
    sched_rows: RefCell<Vec<adw::ActionRow>>,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    RemotePage::new(ctx)
}

fn button_row(buttons: &[&gtk::Button]) -> gtk::Box {
    let b = util::hbox(8);
    b.set_margin_top(8);
    b.set_margin_bottom(8);
    for x in buttons {
        b.append(*x);
    }
    b
}

impl RemotePage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);
        let head =
            adw::PreferencesGroup::builder().title(tr("Remote")).description(tr("Control the phone from the laptop. Each action runs immediately on the phone and is recorded in History.")).build();
        content.append(&head);

        // Media
        let media = adw::PreferencesGroup::builder().title(tr("Media")).description(tr("The keys go to the app that is playing, like the buttons on a pair of headphones")).build();
        let media_row = adw::ActionRow::builder().title(tr("Nothing playing")).subtitle("—").build();
        media_row.add_prefix(&util::icon("adb-audio-volume-high-symbolic"));
        media.add(&media_row);
        let prev = util::icon_button("adb-media-skip-backward-symbolic", tr("Previous"));
        let play_button = util::icon_button("adb-media-playback-start-symbolic", tr("Play or pause"));
        play_button.add_css_class("suggested-action");
        let next = util::icon_button("adb-media-skip-forward-symbolic", tr("Next"));
        let vol_down = util::icon_button("adb-audio-volume-low-symbolic", tr("Volume down"));
        let vol_up = util::icon_button("adb-audio-volume-high-symbolic", tr("Volume up"));
        for b in [&prev, &play_button, &next, &vol_down, &vol_up] {
            b.add_css_class("circular");
        }
        media.add(&button_row(&[&prev, &play_button, &next, &vol_down, &vol_up]));
        content.append(&media);

        // Calls
        let calls = adw::PreferencesGroup::builder().title(tr("Calls")).description(tr("The call is made with the phone: the sound comes out of it (or its headphones)")).build();
        let call_row = adw::ActionRow::builder().title(tr("Status")).subtitle("—").build();
        call_row.add_prefix(&util::icon("adb-call-start-symbolic"));
        let answer_button = util::pill_button("adb-call-start-symbolic", tr("Answer"));
        answer_button.add_css_class("suggested-action");
        let hang_button = util::pill_button("adb-call-stop-symbolic", tr("Hang up"));
        hang_button.add_css_class("destructive-action");
        for b in [&answer_button, &hang_button] {
            b.set_valign(gtk::Align::Center);
            b.set_sensitive(false);
            call_row.add_suffix(b);
        }
        calls.add(&call_row);
        let number = adw::EntryRow::builder().title(tr("Number")).input_purpose(gtk::InputPurpose::Phone).build();
        let dial = util::pill_button("adb-call-start-symbolic", tr("Call"));
        dial.set_valign(gtk::Align::Center);
        number.add_suffix(&dial);
        calls.add(&number);
        content.append(&calls);

        // Send to the phone
        let send = adw::PreferencesGroup::builder()
            .title(tr("Send to the phone"))
            .description(tr("The SMS opens already written in the messaging app: Android does not allow sending it over ADB, so you send it on the phone"))
            .build();
        let url = adw::EntryRow::builder().title(tr("Link (https://…)")).input_purpose(gtk::InputPurpose::Url).build();
        let open = util::pill_button("adb-send-to-symbolic", tr("Open on the phone"));
        open.set_valign(gtk::Align::Center);
        url.add_suffix(&open);
        send.add(&url);
        let sms_number = adw::EntryRow::builder().title(tr("SMS: number")).input_purpose(gtk::InputPurpose::Phone).build();
        send.add(&sms_number);
        let sms_text = adw::EntryRow::builder().title(tr("SMS: message")).build();
        let compose = util::pill_button("adb-send-to-symbolic", tr("Prepare SMS"));
        compose.set_valign(gtk::Align::Center);
        sms_text.add_suffix(&compose);
        send.add(&sms_text);
        content.append(&send);

        // Find the phone
        let find = adw::PreferencesGroup::builder().title(tr("Find the phone")).build();
        let find_row = adw::ActionRow::builder()
            .title(tr("Make the phone ring"))
            .subtitle(tr("Sets a 1-second timer on its clock: it rings at alarm volume, even when silenced, until you stop it on the phone"))
            .build();
        find_row.add_prefix(&util::icon("adb-bell-ring-symbolic"));
        let ring = util::pill_button("adb-bell-ring-symbolic", tr("Ring"));
        ring.set_valign(gtk::Align::Center);
        find_row.add_suffix(&ring);
        find.add(&find_row);
        content.append(&find);

        // Phone notifications
        let notif = adw::PreferencesGroup::builder()
            .title(tr("Phone notifications"))
            .description(tr("Their text is private (messages, verification codes…): it is not stored anywhere and private mode hides it. They are read every 5 s only while this is on."))
            .build();
        let mirror = adw::SwitchRow::builder()
            .title(tr("Show on the laptop"))
            .subtitle(tr("Desktop notifications for new notifications (not persistent ones, such as music)"))
            .active(ctx.settings.borrow().mirror_notifications)
            .build();
        mirror.add_prefix(&util::icon("adb-bell-ring-symbolic"));
        {
            let ctx = ctx.clone();
            mirror.connect_active_notify(move |r| {
                ctx.settings.borrow_mut().mirror_notifications = r.is_active();
                ctx.save_settings();
                if r.is_active() && !ctx.settings.borrow().notifications_enabled {
                    ctx.toast(tr("ADB Scan notifications are turned off in Preferences › Alerts"));
                }
            });
        }
        notif.add(&mirror);
        content.append(&notif);

        // Scheduled actions
        let sched_group = adw::PreferencesGroup::builder()
            .title(tr("Scheduled actions"))
            .description(tr("ADB Scan runs them while it is open (also when hidden in the tray) and the phone is connected. You will get a notification each time."))
            .build();
        let add = adw::ExpanderRow::builder().title(tr("Add an action")).build();
        add.add_prefix(&util::icon("adb-alarm-symbolic"));
        let actions = gtk::StringList::new(&ScheduledAction::ALL.map(|a| a.label()));
        let action_row = adw::ComboRow::builder().title(tr("Action")).model(&actions).build();
        add.add_row(&action_row);
        let time_row = adw::ActionRow::builder().title(tr("Time")).build();
        let hour = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
        let minute = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
        for (sb, v) in [(&hour, 23.0), (&minute, 0.0)] {
            sb.set_value(v);
            sb.set_valign(gtk::Align::Center);
            sb.set_wrap(true);
            // Always two digits: 07, 00…
            sb.connect_output(|sb| {
                sb.set_text(&format!("{:02}", sb.value() as i64));
                glib::Propagation::Stop
            });
        }
        time_row.add_suffix(&hour);
        time_row.add_suffix(&util::label(":", &["heading"]));
        time_row.add_suffix(&minute);
        add.add_row(&time_row);
        let days_row = adw::ActionRow::builder().title(tr("Days")).build();
        let day_buttons: Vec<gtk::ToggleButton> = DAY_LETTERS
            .iter()
            .map(|d| {
                let b = gtk::ToggleButton::builder().label(*d).active(true).valign(gtk::Align::Center).build();
                b.add_css_class("circular");
                days_row.add_suffix(&b);
                b
            })
            .collect();
        add.add_row(&days_row);
        let save_row = adw::ActionRow::new();
        let save = util::pill_button("adb-list-add-symbolic", tr("Add"));
        save.add_css_class("suggested-action");
        save.set_valign(gtk::Align::Center);
        save_row.add_suffix(&save);
        add.add_row(&save_row);
        sched_group.add(&add);
        content.append(&sched_group);

        let root = util::page_scroller(&content, 900);
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            ctx: ctx.clone(),
            root,
            media_row,
            play_button,
            call_row,
            answer_button,
            hang_button,
            number,
            url,
            sms_number,
            sms_text,
            reading: Cell::new(false),
            last_read: Cell::new(0.0),
            call_state: Cell::new(CallState::Unknown),
            playing: RefCell::new(None),
            sched_group,
            sched_rows: RefCell::new(Vec::new()),
        });
        this.render_schedules();
        {
            let w = this.me.clone();
            save.connect_clicked(move |_| {
                let Some(t) = w.upgrade() else { return };
                let mut days = [false; 7];
                for (i, b) in day_buttons.iter().enumerate() {
                    days[i] = b.is_active();
                }
                if !days.contains(&true) {
                    t.ctx.toast(tr("Choose at least one day"));
                    return;
                }
                let action = ScheduledAction::ALL[action_row.selected() as usize % ScheduledAction::ALL.len()];
                let item = Scheduled { id: (now_secs() * 1000.0) as u64, enabled: true, minute: (hour.value() as u16) * 60 + minute.value() as u16, days, action };
                t.ctx.toast(&t!("Scheduled: {} at {} ({})", action.label(), item.time_text(), item.days_text().to_lowercase()));
                t.ctx.settings.borrow_mut().scheduled.push(item);
                t.ctx.save_settings();
                t.render_schedules();
            });
        }

        let keys: [(&gtk::Button, &'static str, &'static str); 5] = [
            (&prev, "KEYCODE_MEDIA_PREVIOUS", tr("Previous")),
            (&this.play_button, MediaKey::PlayPause.keycode(), tr("Play or pause")),
            (&next, "KEYCODE_MEDIA_NEXT", tr("Next")),
            (&vol_down, "KEYCODE_VOLUME_DOWN", tr("Volume −")),
            (&vol_up, "KEYCODE_VOLUME_UP", tr("Volume +")),
        ];
        for (b, key, what) in keys {
            let w = this.me.clone();
            b.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    t.act("media_key", what, move |adb, s| adb.keyevent(&s, key).map(|_| String::new()), true);
                }
            });
        }
        let w = this.me.clone();
        this.answer_button.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.act("answer_call", "", |adb, s| adb.answer_call(&s).map(|_| tr("Call answered").into()), true);
            }
        });
        let w = this.me.clone();
        this.hang_button.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.act("end_call", "", |adb, s| adb.end_call(&s).map(|_| tr("Call ended").into()), true);
            }
        });
        let w = this.me.clone();
        let on_dial = move || {
            let Some(t) = w.upgrade() else { return };
            let n = t.number.text().to_string();
            let Some(clean) = adbscan_core::remote::clean_number(&n) else {
                t.ctx.toast(tr("Type a valid phone number"));
                return;
            };
            let target = t.ctx.shown(&clean);
            t.act(
                "call",
                &target,
                move |adb, s| {
                    adb.dial(&s, &clean).map(|started| {
                        if started {
                            t!("Calling {}…", clean)
                        } else {
                            tr("Android does not allow calling over ADB: the dialer is open with the number, tap Call on the phone").into()
                        }
                    })
                },
                true,
            );
        };
        let d = on_dial.clone();
        dial.connect_clicked(move |_| d());
        this.number.connect_entry_activated(move |_| on_dial());

        let w = this.me.clone();
        let on_open = move || {
            let Some(t) = w.upgrade() else { return };
            let Some(url) = adbscan_core::remote::clean_url(&t.url.text()) else {
                t.ctx.toast(tr("Type a web link (https://…)"));
                return;
            };
            t.act("open_url", &url.clone(), move |adb, s| adb.open_url(&s, &url).map(|_| tr("Link opened on the phone").into()), false);
        };
        let o = on_open.clone();
        open.connect_clicked(move |_| o());
        this.url.connect_entry_activated(move |_| on_open());

        let w = this.me.clone();
        compose.connect_clicked(move |_| {
            let Some(t) = w.upgrade() else { return };
            let (n, text) = (t.sms_number.text().to_string(), t.sms_text.text().to_string());
            let Some(clean) = adbscan_core::remote::clean_number(&n) else {
                t.ctx.toast(tr("Type a valid phone number"));
                return;
            };
            let target = t.ctx.shown(&clean);
            t.act("compose_sms", &target, move |adb, s| adb.compose_sms(&s, &clean, &text).map(|_| tr("SMS prepared: send it on the phone").into()), false);
        });

        let w = this.me.clone();
        ring.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.act("find_phone", "", |adb, s| adb.find_phone(&s).map(|_| tr("The phone is ringing: stop the timer on it").into()), false);
            }
        });
        this
    }

    /// Runs an action in the background, records it in History and shows the
    /// result (`work` returns the toast text; empty, none).
    fn act<W>(&self, action: &'static str, target: &str, work: W, refresh: bool)
    where
        W: FnOnce(Adb, String) -> adbscan_core::Result<String> + Send + 'static,
    {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            self.ctx.toast(tr("No device is ready"));
            return;
        };
        let target = target.to_string();
        let w = self.me.clone();
        util::run_bg(
            move || work(adb, serial),
            move |res| {
                let Some(t) = w.upgrade() else { return };
                match res {
                    Ok(msg) => {
                        t.ctx.audit(action, Some(&target).filter(|s| !s.is_empty()).map(String::as_str), true, Some(&msg));
                        if !msg.is_empty() {
                            t.ctx.toast(&msg);
                        }
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        t.ctx.audit(action, Some(&target).filter(|s| !s.is_empty()).map(String::as_str), false, Some(&msg));
                        t.ctx.toast(&format!("Error: {msg}"));
                    }
                }
                if refresh {
                    // What is playing or the call state change right away.
                    t.last_read.set(0.0);
                    glib::timeout_add_local_once(std::time::Duration::from_millis(600), {
                        let w = t.me.clone();
                        move || {
                            if let Some(t) = w.upgrade() {
                                t.read_status();
                            }
                        }
                    });
                }
            },
        );
    }

    /// One row per saved action: a switch to pause it and a button to delete it.
    fn render_schedules(&self) {
        for r in self.sched_rows.borrow_mut().drain(..) {
            self.sched_group.remove(&r);
        }
        let mut items = self.ctx.settings.borrow().scheduled.clone();
        items.sort_by_key(|s| s.minute);
        for item in items {
            let row = adw::ActionRow::builder().title(format!("{} · {}", item.time_text(), item.action.label())).subtitle(item.days_text()).build();
            let switch = gtk::Switch::builder().active(item.enabled).valign(gtk::Align::Center).tooltip_text(tr("Enable or pause this action")).build();
            let delete = util::icon_button("adb-edit-delete-symbolic", tr("Clear"));
            delete.add_css_class("flat");
            delete.set_valign(gtk::Align::Center);
            row.add_suffix(&switch);
            row.add_suffix(&delete);
            let (ctx, id) = (self.ctx.clone(), item.id);
            switch.connect_active_notify(move |sw| {
                if let Some(s) = ctx.settings.borrow_mut().scheduled.iter_mut().find(|s| s.id == id) {
                    s.enabled = sw.is_active();
                }
                ctx.save_settings();
            });
            let w = self.me.clone();
            delete.connect_clicked(move |_| {
                let Some(t) = w.upgrade() else { return };
                t.ctx.settings.borrow_mut().scheduled.retain(|s| s.id != id);
                t.ctx.save_settings();
                t.render_schedules();
            });
            self.sched_group.add(&row);
            self.sched_rows.borrow_mut().push(row);
        }
    }

    fn read_status(&self) {
        if self.reading.get() {
            return;
        }
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        self.reading.set(true);
        self.last_read.set(now_secs());
        let w = self.me.clone();
        util::run_bg(
            move || adb.remote_status(&serial),
            move |res| {
                let Some(t) = w.upgrade() else { return };
                t.reading.set(false);
                if let Ok((call, playing)) = res {
                    t.call_state.set(call);
                    *t.playing.borrow_mut() = playing;
                    t.render();
                }
            },
        );
    }

    fn render(&self) {
        let call = self.call_state.get();
        self.call_row.set_subtitle(call.label());
        self.answer_button.set_sensitive(call == CallState::Ringing);
        self.hang_button.set_sensitive(matches!(call, CallState::Ringing | CallState::InCall));
        match self.playing.borrow().as_ref() {
            Some(p) => {
                self.media_row.set_title(&glib::markup_escape_text(&p.title));
                let mut sub: Vec<&str> = [p.artist.as_str(), p.album.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
                let app = super::apps::cached_labels().get(&p.package).cloned().unwrap_or_else(|| p.package.clone());
                let state = if p.playing { t!("Playing in {}", app) } else { t!("Paused · {}", app) };
                sub.push(&state);
                self.media_row.set_subtitle(&glib::markup_escape_text(&sub.join(" · ")));
                self.play_button.set_icon_name(if p.playing { "adb-media-playback-pause-symbolic" } else { "adb-media-playback-start-symbolic" });
            }
            None => {
                self.media_row.set_title(tr("Nothing playing"));
                self.media_row.set_subtitle(tr("Open a music or video app on the phone"));
                self.play_button.set_icon_name("adb-media-playback-start-symbolic");
            }
        }
    }
}

use gtk::glib;

impl Page for RemotePage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn on_event(&self, ev: &Event) {
        match ev {
            Event::Devices => {
                self.playing.borrow_mut().take();
                self.call_state.set(CallState::Unknown);
                self.render();
                self.read_status();
            }
            // Telemetry serves as a clock: it only arrives while the page is in
            // view.
            Event::Snapshot if now_secs() - self.last_read.get() > REFRESH_SECS => self.read_status(),
            _ => {}
        }
    }

    fn shown(&self) {
        self.read_status();
    }
}
