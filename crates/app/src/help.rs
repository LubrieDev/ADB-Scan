//! About, keyboard shortcuts and verification screenshots.

use adbscan_core::i18n::tr;
use adw::prelude::*;
use gtk::glib;

use crate::window::PageDef;

pub fn about(parent: &gtk::Widget) {
    let about = adw::AboutDialog::builder()
        .application_name("ADB Scan")
        .application_icon("adb-scan")
        .version(adbscan_core::paths::VERSION)
        .developer_name(tr("Monitor, terminal and flashing for Android"))
        .license_type(gtk::License::Gpl30)
        .comments(
            tr("Monitors battery, performance, network and temperature live; manages apps and files; opens a real adb shell terminal; mirrors the screen with scrcpy and flashes with fastboot behind safeguards. Everything is processed on your computer: no telemetry, no accounts and no root."),
        )
        .release_notes(
            tr("<p>1.4.0 — root, more control without root, and more phones</p><ul>\
             <li>Root: what rooting needs to know about the phone, how to unlock each brand, and Magisk from its official site, checked before installing.</li>\
             <li>Flasher: Motorola's unlock data and unlocking with a code (Motorola, Sony).</li>\
             <li>What wakes the phone and works in the background, unrestricted battery per app, and per-app tuning (smoothness, language, game mode).</li>\
             <li>Copy contacts, calls and messages to the computer, and a checklist before selling or resetting the phone.</li>\
             <li>Usage: notifications, unlocks and apps not used in months. Also the keyboard, night light, mobile signal, nearby Wi-Fi networks and memory per app.</li>\
             <li>Works with Android 11, Samsung and several devices connected at once.</li></ul>")
                .to_string()
                + tr("<p>1.3.0 — favorites, colours and phone changes</p><ul>\
             <li>Favorites: the star next to each section adds it to the top of the sidebar.</li>\
             <li>Any accent colour of your own, and its intensity, in Preferences.</li>\
             <li>Phone changes: apps installed, updated and uninstalled, and restarts, by day.</li></ul>")
                + tr("<p>1.2.1</p><ul><li>Publishes the Windows installer, which version 1.2.0 could not build.</li></ul>")
                + tr("<p>1.2.0 — updates from the app</p><ul>\
             <li>Update now: ADB Scan downloads the new version, showing the percentage, checks it and installs it.</li>\
             <li>Installer for Windows, instead of the zip: Start menu, desktop shortcut and uninstalling from Settings.</li></ul>")
                + tr("<p>1.1.0 — Windows and control of the phone</p><ul>\
             <li>Version for Windows 10 and 11, as a portable zip.</li>\
             <li>Preinstalled apps: remove and restore manufacturer and carrier apps without root.</li>\
             <li>Permissions: which apps can use the camera, microphone, location and other accesses, and take them away.</li>\
             <li>Storage: folders, largest files and duplicates.</li>\
             <li>Developer tools: touches, animations, resolution and density, demo mode and port forwarding.</li>\
             <li>Health report to save and share, export apps and install them on another phone, and automations when a phone connects.</li></ul>")
                + tr("<p>1.0.0 — first stable release</p><ul>\
             <li>Basic analysis without developer options: identification over USB and, with “File transfer”, battery and storage over MTP.</li>\
             <li>Ways to connect: USB and wireless debugging, pairing with a QR code or manually.</li>\
             <li>Live telemetry, battery and storage health, per-app usage, performance with GPU and FPS.</li>\
             <li>Apps with technical details, files, photo backup, real terminal, logcat and mirroring with scrcpy.</li>\
             <li>Phone controls and remote, scheduled actions, security audit and a Flasher with safeguards.</li>\
             <li>Private mode (Ctrl+Shift+P) and phone data hidden until requested.</li></ul>"),
        )
        .build();
    about.add_credit_section(Some(tr("Inspired by")), &["ADBKit"]);
    about.add_credit_section(Some(tr("Components")), &[tr("GTK 4, libadwaita and VTE"), "Android platform-tools (adb, fastboot)", "scrcpy (Genymobile)", tr("Lucide icons (ISC) and Adwaita (CC BY-SA)")]);
    about.present(Some(parent));
}

pub fn shortcuts_window(defs: &[PageDef]) -> gtk::ShortcutsWindow {
    let mut nav = String::new();
    for (i, d) in defs.iter().enumerate().take(9) {
        nav.push_str(&format!(
            r#"<child><object class="GtkShortcutsShortcut"><property name="title">{}</property><property name="accelerator">&lt;Alt&gt;{}</property></object></child>"#,
            glib::markup_escape_text(d.title),
            i + 1
        ));
    }
    let shortcut = |title: &str, accel: &str| {
        format!(
            r#"<child><object class="GtkShortcutsShortcut"><property name="title">{}</property><property name="accelerator">{accel}</property></object></child>"#,
            glib::markup_escape_text(title)
        )
    };
    let general = [
        (tr("Search sections and actions"), "&lt;Control&gt;k"),
        (tr("Screenshot"), "&lt;Control&gt;&lt;Shift&gt;s"),
        (tr("Private mode"), "&lt;Control&gt;&lt;Shift&gt;p"),
        (tr("Mirror screen"), "&lt;Control&gt;m"),
        (tr("Refresh devices"), "F5"),
        (tr("Preferences"), "&lt;Control&gt;comma"),
        (tr("Close window"), "&lt;Control&gt;w"),
        (tr("Quit"), "&lt;Control&gt;q"),
    ]
    .iter()
    .map(|(title, accel)| shortcut(title, accel))
    .collect::<String>();
    let ui = format!(
        r#"<interface>
  <object class="GtkShortcutsWindow" id="w">
    <property name="modal">1</property>
    <child><object class="GtkShortcutsSection"><property name="section-name">main</property>
      <child><object class="GtkShortcutsGroup"><property name="title">{}</property>{general}</object></child>
      <child><object class="GtkShortcutsGroup"><property name="title">{}</property>{nav}</object></child>
    </object></child>
  </object>
</interface>"#,
        glib::markup_escape_text(tr("General")),
        glib::markup_escape_text(tr("Sections")),
    );
    let builder = gtk::Builder::from_string(&ui);
    builder.object("w").expect("shortcuts window")
}

/// Renders a widget to PNG (to check the interface without capturing the screen).
pub fn capture(widget: &gtk::Widget, path: &str) -> bool {
    let (w, h) = (widget.width(), widget.height());
    if w <= 0 || h <= 0 {
        log::warn!("capture {path}: window has no size ({w}×{h})");
        return false;
    }
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, w as f64, h as f64);
    let Some(node) = snapshot.to_node() else {
        log::debug!("capture {path}: the window produced nothing to paint");
        return false;
    };
    let Some(renderer) = widget.native().and_then(|n| n.renderer()) else {
        log::warn!("capture {path}: no renderer");
        return false;
    };
    let texture = renderer.render_texture(&node, None);
    if let Err(e) = texture.save_to_png(path) {
        log::error!("capture {path}: {e}");
        return false;
    }
    true
}
