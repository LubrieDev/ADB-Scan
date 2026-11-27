//! Search everywhere (Ctrl+K): type a few letters to open any section or run
//! a common action (screenshot, restart the phone, clear caches…). Each entry
//! has extra words so it is also found by what it does (“cable” finds
//! Connection, “cache” finds Apps and the clean-up).

use adbscan_core::i18n::tr;
use adw::prelude::*;
use gtk::glib;

use crate::util;
use crate::window::page_defs;

struct Entry {
    icon: &'static str,
    title: String,
    /// Where it is or what it does.
    subtitle: String,
    keywords: &'static str,
    action: &'static str,
    target: Option<&'static str>,
}

/// Extra words for each section, so it is found by what is in it.
fn page_keywords(id: &str) -> &'static str {
    match id {
        "dashboard" => tr("summary home status quick"),
        "device" => tr("model android version properties reboot nickname hardware"),
        "performance" => tr("cpu memory ram gpu fps temperature sensors memory per app procstats"),
        "battery" => tr("charge health cycles drain wakelocks alarms wakeups who wakes background tasks jobs"),
        "connection" => tr("usb cable port speed charger wireless wifi adb diagnosis tcpip pair qr server"),
        "network" => tr("wifi ip interfaces routes capture tshark saved networks forget nearby networks channel analyzer scan mobile signal 4g 5g lte band dbm coverage"),
        "apps" => tr("install uninstall cache permissions apk packages pause suspend background restrict data unrestricted battery optimization notifications late game mode fps resolution language smoothness jank"),
        "files" => tr("photos transfer copy storage folders"),
        "mirror" => tr("scrcpy mirror record camera webcam screenshot"),
        "control" => tr("brightness volume do not disturb airplane remote media calls find quick settings automations on connect private dns ad blocking adguard text size accessibility grayscale bold night light"),
        "debloat" => tr("debloat bloatware remove preinstalled system apps restore uninstall"),
        "permissions" => tr("camera microphone location contacts sms accessibility notifications overlay revoke privacy special access"),
        "storage" => tr("space full analyzer large files duplicates whatsapp downloads free up clean"),
        "basic" => tr("without debugging mtp"),
        "data" => tr("imei sim accounts mac public ip contacts vcard call log sms messages export backup sell reset"),
        "security" => tr("audit score permissions risk"),
        "changes" => tr("timeline installed updated uninstalled removed restarts reboots shutdown what changed modifications"),
        "usage" => tr("screen time digital wellbeing hours data mobile wifi consumption megabytes apps used notifications unlocks pickups unused apps forgotten"),
        "stability" => tr("crash closed unexpectedly anr not responding freeze restart reboot reason bug report bugreport errors"),
        "history" => tr("log events sessions charts"),
        "terminal" => tr("shell command console"),
        "logcat" => tr("log errors crash debug"),
        "tools" => tr("processes sensors services developer options port forwarding reverse demo mode resolution density animations show taps layout bounds optimize apps slow after update dexopt"),
        "flasher" => tr("fastboot flash bootloader recovery rom firmware"),
        "root" => tr("root magisk kernelsu apatch su unlock bootloader oem unlocking boot init_boot patch"),
        _ => "",
    }
}

fn entries() -> Vec<Entry> {
    let mut out: Vec<Entry> = page_defs()
        .into_iter()
        .map(|d| Entry { icon: d.icon, title: d.title.to_string(), subtitle: capitalized(d.section), keywords: page_keywords(d.id), action: "win.go", target: Some(d.id) })
        .collect();
    let action = |icon, title: &str, subtitle: &str, keywords, action, target| Entry { icon, title: title.into(), subtitle: subtitle.into(), keywords, action, target };
    out.extend([
        action("adb-file-text-symbolic", tr("Health report"), tr("Battery, storage, bootloader and security in one page"), tr("sell buy used second hand certificate pdf print battery health"), "win.health-report", None),
        action("adb-save-symbolic", tr("Export apps"), tr("Apps"), tr("migrate move new phone backup apk transfer"), "win.bulk", Some("export")),
        action("adb-system-software-install-symbolic", tr("Install from an export"), tr("Apps"), tr("migrate restore new phone apk"), "win.bulk", Some("import")),
        action("adb-camera-photo-symbolic", tr("Take a screenshot"), tr("Saves the phone's screen"), tr("capture picture"), "win.screenshot", None),
        action("adb-video-display-symbolic", tr("Mirror the screen"), "scrcpy", tr("scrcpy cast"), "win.mirror", None),
        action("adb-eye-off-symbolic", tr("Private mode"), tr("Hides serial numbers, IPs and networks"), tr("privacy hide"), "win.privacy", None),
        action("adb-link-symbolic", tr("Ways to connect"), tr("USB, Wi-Fi with QR or code"), tr("wireless pair qr connect"), "win.connect", Some("root")),
        action("adb-star-symbolic", tr("Default apps"), tr("Apps"), tr("default browser launcher home screen dialer phone sms messages assistant role keyboard"), "win.bulk", Some("default-apps")),
        action("adb-circle-x-symbolic", tr("Close open apps"), tr("Apps"), tr("recents clear all kill"), "win.bulk", Some("close")),
        action("adb-media-playback-stop-symbolic", tr("Stop all apps"), tr("Apps"), tr("kill force stop memory free ram"), "win.bulk", Some("stop")),
        action("adb-edit-clear-symbolic", tr("Clear all caches"), tr("Apps"), tr("cache clean free space storage"), "win.bulk", Some("caches")),
        action("adb-system-reboot-symbolic", tr("Restart"), tr("The phone"), tr("reboot restart"), "win.reboot", Some("system")),
        action("adb-system-reboot-symbolic", tr("Reboot into recovery"), tr("The phone"), tr("reboot recovery"), "win.reboot", Some("recovery")),
        action("adb-system-reboot-symbolic", tr("Reboot into the bootloader"), tr("The phone"), tr("reboot fastboot bootloader"), "win.reboot", Some("bootloader")),
        action("adb-settings-symbolic", tr("Preferences"), "Ctrl+,", tr("settings options language theme notifications"), "app.settings", None),
        action("adb-layout-grid-symbolic", tr("Quick panel"), tr("Small window with the essentials"), tr("panel tray"), "app.panel", None),
        action("adb-input-keyboard-symbolic", tr("Keyboard shortcuts"), "", tr("keys shortcuts"), "win.show-help-overlay", None),
    ]);
    out
}

/// “ESTADO” → “Estado”.
fn capitalized(text: &str) -> String {
    let lower = text.to_lowercase();
    let mut chars = lower.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// Lower case without accents, so “camara” finds “Cámara”.
fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

/// Score of an entry for the query (higher is better); `None` if it does not
/// match. Every word must appear; the title counts more than the rest.
fn score(e: &Entry, words: &[String]) -> Option<u32> {
    let title = fold(&e.title);
    let rest = fold(&format!("{} {}", e.subtitle, e.keywords));
    let mut total = 0;
    for w in words {
        total += if title.starts_with(w.as_str()) {
            30
        } else if title.split_whitespace().any(|t| t.starts_with(w.as_str())) {
            20
        } else if title.contains(w.as_str()) {
            10
        } else if rest.split_whitespace().any(|t| t.starts_with(w.as_str())) {
            5
        } else {
            return None;
        };
    }
    Some(total)
}

/// Whether a sidebar page matches what was typed in the sidebar's search box
/// (its title or what is in it, without accents).
pub fn page_matches(id: &str, title: &str, query: &str) -> bool {
    let words: Vec<String> = fold(query).split_whitespace().map(String::from).collect();
    let e = Entry { icon: "", title: title.into(), subtitle: String::new(), keywords: page_keywords(id), action: "", target: None };
    words.is_empty() || score(&e, &words).is_some()
}

/// Opens the search over `window`.
pub fn show(window: &adw::ApplicationWindow) {
    show_with(window, "");
}

/// Opens the search with `query` already typed.
pub fn show_with(window: &adw::ApplicationWindow, query: &str) {
    let entries = std::rc::Rc::new(entries());
    let dialog = adw::Dialog::builder().title(tr("Search")).content_width(520).content_height(460).build();
    let entry = gtk::SearchEntry::builder().placeholder_text(tr("Sections and actions: “cable”, “cache”, “restart”…")).hexpand(true).build();
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    list.set_selection_mode(gtk::SelectionMode::Browse);
    let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
    let body = util::vbox(8);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    body.append(&entry);
    body.append(&scroll);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&body));
    dialog.set_child(Some(&view));

    // Shown rows, in order, with the index of their entry.
    let shown: std::rc::Rc<std::cell::RefCell<Vec<usize>>> = Default::default();
    let fill = {
        let (entries, list, shown) = (entries.clone(), list.clone(), shown.clone());
        move |query: &str| {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let words: Vec<String> = fold(query).split_whitespace().map(String::from).collect();
            let mut hits: Vec<(u32, usize)> = entries.iter().enumerate().filter_map(|(i, e)| if words.is_empty() { Some((0, i)) } else { score(e, &words).map(|s| (s, i)) }).collect();
            hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            let mut order = Vec::new();
            for (_, i) in hits {
                let e = &entries[i];
                let row = adw::ActionRow::builder().title(glib::markup_escape_text(&e.title)).subtitle(glib::markup_escape_text(&e.subtitle)).activatable(true).build();
                row.add_prefix(&util::icon(e.icon));
                list.append(&row);
                order.push(i);
            }
            if order.is_empty() {
                let row = adw::ActionRow::builder().title(tr("Nothing found")).build();
                row.set_activatable(false);
                list.append(&row);
            }
            *shown.borrow_mut() = order;
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
        }
    };
    entry.set_text(query);
    fill(query);
    let run = {
        let (entries, shown, dialog, window) = (entries.clone(), shown.clone(), dialog.clone(), window.clone());
        move |index: usize| {
            let Some(&i) = shown.borrow().get(index) else { return };
            let e = &entries[i];
            dialog.close();
            let target = e.target.map(|t| t.to_variant());
            if let Err(err) = gtk::prelude::WidgetExt::activate_action(&window, e.action, target.as_ref()) {
                log::warn!("search: {} could not run: {err}", e.action);
            }
        }
    };
    {
        let fill = fill.clone();
        entry.connect_search_changed(move |e| fill(&e.text()));
    }
    {
        let (run, list) = (run.clone(), list.clone());
        entry.connect_activate(move |_| run(list.selected_row().map(|r| r.index()).unwrap_or(0).max(0) as usize));
    }
    {
        let run = run.clone();
        list.connect_row_activated(move |_, row| run(row.index().max(0) as usize));
    }
    {
        let d = dialog.clone();
        entry.connect_stop_search(move |_| {
            d.close();
        });
    }
    // Up and down move through the results without leaving the search box.
    let keys = gtk::EventControllerKey::new();
    {
        let list = list.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            let step = match key {
                gtk::gdk::Key::Down => 1,
                gtk::gdk::Key::Up => -1,
                _ => return glib::Propagation::Proceed,
            };
            let current = list.selected_row().map(|r| r.index()).unwrap_or(-1);
            if let Some(row) = list.row_at_index((current + step).max(0)) {
                list.select_row(Some(&row));
                // Keeps it in view; the typing stays in the search box.
                if let Some(adj) = list.parent().and_then(|v| v.parent()).and_then(|s| s.downcast::<gtk::ScrolledWindow>().ok()).map(|s| s.vadjustment()) {
                    if let Some(b) = row.compute_bounds(&list) {
                        let (top, bottom) = (b.y() as f64, (b.y() + b.height()) as f64);
                        if top < adj.value() {
                            adj.set_value(top);
                        } else if bottom > adj.value() + adj.page_size() {
                            adj.set_value(bottom - adj.page_size());
                        }
                    }
                }
            }
            glib::Propagation::Stop
        });
    }
    entry.add_controller(keys);
    dialog.present(Some(window));
    entry.grab_focus();
    entry.set_position(-1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(title: &str, keywords: &'static str) -> Entry {
        Entry { icon: "", title: title.into(), subtitle: String::new(), keywords, action: "", target: None }
    }

    #[test]
    fn sidebar_filter_uses_what_is_in_each_page() {
        assert!(page_matches("control", "Control", "calls"));
        assert!(page_matches("control", "Control", ""));
        assert!(!page_matches("battery", "Battery", "calls"));
        assert!(page_matches("connection", "Connection", "conn"));
        assert!(page_matches("connection", "Conexión", "conexion cable"));
    }

    #[test]
    fn matches_titles_and_keywords() {
        let words = |q: &str| fold(q).split_whitespace().map(String::from).collect::<Vec<_>>();
        let conn = e("Conexión", "usb cable puerto");
        assert_eq!(score(&conn, &words("conex")), Some(30));
        assert_eq!(score(&conn, &words("CABLE")), Some(5));
        assert_eq!(score(&conn, &words("cable usb")), Some(10));
        assert_eq!(score(&conn, &words("cable wifi")), None);
        assert_eq!(score(&e("Cámara", ""), &words("camara")), Some(30));
        assert!(score(&e("Borrar la caché de todas", ""), &words("cache")).unwrap() > score(&e("Aplicaciones", "cache"), &words("cache")).unwrap());
    }
}
