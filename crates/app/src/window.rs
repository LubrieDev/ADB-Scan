//! Main window: sidebar, header with a device selector and pages built on
//! demand (fast startup).

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adbscan_core::models::DeviceState;

use crate::ctx::{Ctx, Event};
use crate::pages;
use crate::util;
use crate::widgets::StatusPill;

pub trait Page {
    fn widget(&self) -> gtk::Widget;
    fn on_event(&self, _ev: &Event) {}
    /// Called every time the page becomes visible.
    fn shown(&self) {}
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Requires an authorised ADB device.
    Adb,
    /// Always available (it handles its own states).
    Any,
}

pub struct PageDef {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    pub section: &'static str,
    pub need: Need,
    pub build: fn(&Rc<Ctx>) -> Rc<dyn Page>,
}

pub fn page_defs() -> Vec<PageDef> {
    // The first nine have the Alt+1…9 shortcuts.
    vec![
        PageDef { id: "dashboard", title: tr("Overview"), icon: "adb-layout-dashboard-symbolic", section: tr("STATUS"), need: Need::Any, build: pages::dashboard::build },
        PageDef { id: "device", title: tr("Device"), icon: "adb-smartphone-symbolic", section: tr("STATUS"), need: Need::Adb, build: pages::device::build },
        PageDef { id: "performance", title: tr("Performance"), icon: "adb-activity-symbolic", section: tr("STATUS"), need: Need::Adb, build: pages::performance::build },
        PageDef { id: "battery", title: tr("Battery"), icon: "adb-battery-medium-symbolic", section: tr("STATUS"), need: Need::Adb, build: pages::battery::build },
        // Without ADB too: the PC's side of the cable and the diagnosis.
        PageDef { id: "connection", title: tr("Connection"), icon: "adb-link-symbolic", section: tr("STATUS"), need: Need::Any, build: pages::connection::build },
        PageDef { id: "network", title: tr("Network"), icon: "adb-wifi-symbolic", section: tr("STATUS"), need: Need::Adb, build: pages::network::build },
        PageDef { id: "apps", title: tr("Apps"), icon: "adb-package-symbolic", section: tr("MANAGE"), need: Need::Adb, build: pages::apps::build },
        PageDef { id: "files", title: tr("Files"), icon: "adb-folder-symbolic", section: tr("MANAGE"), need: Need::Adb, build: pages::files::build },
        PageDef { id: "mirror", title: tr("Screen"), icon: "adb-monitor-symbolic", section: tr("MANAGE"), need: Need::Adb, build: pages::mirror::build },
        PageDef { id: "control", title: tr("Control"), icon: "adb-sliders-horizontal-symbolic", section: tr("MANAGE"), need: Need::Adb, build: pages::control::build },
        PageDef { id: "debloat", title: tr("Preinstalled apps"), icon: "adb-eraser-symbolic", section: tr("MANAGE"), need: Need::Adb, build: pages::debloat::build },
        PageDef { id: "permissions", title: tr("Permissions"), icon: "adb-key-round-symbolic", section: tr("MANAGE"), need: Need::Adb, build: pages::permissions::build },
        PageDef { id: "storage", title: tr("Storage"), icon: "adb-hard-drive-symbolic", section: tr("MANAGE"), need: Need::Adb, build: pages::storage::build },
        // Without ADB: always available.
        PageDef { id: "basic", title: tr("Basic analysis"), icon: "adb-search-symbolic", section: tr("DATA"), need: Need::Any, build: pages::basic::build },
        PageDef { id: "data", title: tr("Phone data"), icon: "adb-smartphone-lock-symbolic", section: tr("DATA"), need: Need::Adb, build: pages::data::build },
        PageDef { id: "security", title: tr("Security"), icon: "adb-shield-symbolic", section: tr("DATA"), need: Need::Adb, build: pages::security::build },
        PageDef { id: "changes", title: tr("Phone changes"), icon: "adb-document-open-recent-symbolic", section: tr("DATA"), need: Need::Adb, build: pages::changes::build },
        PageDef { id: "usage", title: tr("Usage"), icon: "adb-chart-line-symbolic", section: tr("DATA"), need: Need::Adb, build: pages::usage::build },
        PageDef { id: "stability", title: tr("Stability"), icon: "adb-heart-pulse-symbolic", section: tr("DATA"), need: Need::Adb, build: pages::stability::build },
        PageDef { id: "history", title: tr("History"), icon: "adb-history-symbolic", section: tr("DATA"), need: Need::Any, build: pages::history::build },
        PageDef { id: "terminal", title: "Terminal", icon: "adb-square-terminal-symbolic", section: tr("ADVANCED"), need: Need::Any, build: pages::terminal::build },
        PageDef { id: "logcat", title: "Logcat", icon: "adb-scroll-symbolic", section: tr("ADVANCED"), need: Need::Adb, build: pages::logcat::build },
        PageDef { id: "tools", title: tr("Tools"), icon: "adb-wrench-symbolic", section: tr("ADVANCED"), need: Need::Adb, build: pages::tools::build },
        PageDef { id: "flasher", title: "Flasher", icon: "adb-usb-symbolic", section: tr("ADVANCED"), need: Need::Any, build: pages::flasher::build },
        PageDef { id: "root", title: "Root", icon: "adb-key-round-symbolic", section: tr("ADVANCED"), need: Need::Adb, build: pages::root::build },
    ]
}

/// Identifier of a page that was renamed or merged (saved settings, links
/// and `--page` keep working).
pub fn page_alias(id: &str) -> &str {
    match id {
        "controls" | "remote" => "control",
        other => other,
    }
}

struct Slot {
    def_index: usize,
    gate: gtk::Stack,
    empty: adw::StatusPage,
    /// “View basic analysis” in the notice of the sections that use ADB.
    basic_btn: gtk::Button,
    page: Option<Rc<dyn Page>>,
}

pub struct MainWindow {
    pub window: adw::ApplicationWindow,
    ctx: Rc<Ctx>,
    defs: Vec<PageDef>,
    stack: gtk::Stack,
    slots: RefCell<HashMap<&'static str, Slot>>,
    sidebar: gtk::ListBox,
    side: Rc<crate::sidebar::Sidebar>,
    title: adw::WindowTitle,
    device_name: gtk::Label,
    device_sub: gtk::Label,
    pill: StatusPill,
    device_menu: gtk::MenuButton,
    device_list: gtk::ListBox,
    current: RefCell<&'static str>,
    /// Pages that were hidden when private mode changed: they redraw their
    /// texts when shown, so toggling it (Ctrl+Shift+P) only redraws the
    /// visible page and does not freeze the window.
    privacy_stale: RefCell<std::collections::HashSet<&'static str>>,
    split: adw::OverlaySplitView,
    quick_buttons: Vec<gtk::Button>,
    /// Sidebar icons that reflect the open section.
    live_icons: SidebarIcons,
    /// “A new version is available”, under the header bar (hidden until then).
    update_banner: adw::Banner,
}

/// The Apps box opens and the Terminal cursor blinks while their section is
/// selected; the others play their gesture once when opened. Nothing else
/// animates them.
struct SidebarIcons {
    apps: crate::anim::PackageIcon,
    terminal: crate::anim::TerminalIcon,
    sections: Vec<(&'static str, crate::anim::SectionIcon)>,
}

impl SidebarIcons {
    fn select(&self, id: &str) {
        self.apps.set_open(id == "apps", true);
        self.terminal.set_active(id == "terminal");
        for (sid, icon) in &self.sections {
            icon.set_active(*sid == id);
        }
    }
}

impl MainWindow {
    pub fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let defs = page_defs();
        let s = ctx.settings.borrow().clone();
        let window = adw::ApplicationWindow::builder()
            .application(&ctx.app)
            .title("ADB Scan")
            .default_width(s.window_width)
            .default_height(s.window_height)
            .width_request(360)
            .height_request(480)
            .icon_name("adb-scan")
            .build();
        if s.window_maximized {
            window.maximize();
        }

        // -- sidebar --------------------------------------------------
        let sidebar = gtk::ListBox::new();
        sidebar.add_css_class("navigation-sidebar");
        sidebar.set_selection_mode(gtk::SelectionMode::Single);
        let mut live_icons = SidebarIcons { apps: crate::anim::PackageIcon::new(16), terminal: crate::anim::TerminalIcon::new(), sections: Vec::new() };
        for d in defs.iter() {
            let row = gtk::ListBoxRow::new();
            let b = util::hbox(12);
            b.set_margin_top(2);
            b.set_margin_bottom(2);
            b.set_margin_start(8);
            match d.id {
                "apps" => b.append(&live_icons.apps.widget),
                "terminal" => b.append(&live_icons.terminal.widget),
                _ => {
                    let icon = crate::anim::SectionIcon::new(d.id, d.icon);
                    b.append(&icon.widget);
                    live_icons.sections.push((d.id, icon));
                }
            }
            b.append(&util::label(d.title, &[]));
            row.set_child(Some(&b));
            row.set_widget_name(d.id);
            row.set_tooltip_text(Some(d.title));
            sidebar.append(&row);
        }

        let device_name = util::label(tr("No device"), &["title"]);
        device_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let device_sub = util::label(tr("Connect a phone over USB"), &["dim", "caption"]);
        device_sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let pill = StatusPill::new();
        let card = util::vbox(6);
        card.add_css_class("sidebar-device");
        card.append(&device_name);
        card.append(&device_sub);
        card.append(&pill.root);
        pill.root.set_halign(gtk::Align::Start);

        // Filters the sections as you type: by name or by what is in them
        // (“calls” leaves Control). Enter opens the first one; with none,
        // it searches the actions (Ctrl+K) with the same text.
        let side_search = gtk::SearchEntry::builder().placeholder_text(tr("Search sections…")).build();
        side_search.add_css_class("sidebar-search");
        side_search.set_margin_start(8);
        side_search.set_margin_end(8);
        side_search.set_margin_top(4);
        side_search.set_margin_bottom(4);
        let no_match = util::label(tr("No section matches. Press Enter to search the actions."), &["dim", "caption"]);
        no_match.set_wrap(true);
        no_match.set_margin_top(12);
        no_match.set_margin_start(12);
        no_match.set_margin_end(12);
        sidebar.set_placeholder(Some(&no_match));
        // Folding, pinning and filtering (see `sidebar`).
        let side = crate::sidebar::Sidebar::new(ctx, &sidebar, &side_search, &defs);

        let side_box = util::vbox(0);
        side_box.append(&card);
        side_box.append(&side_search);
        // The scrollbar gets its own column at the panel's edge instead of
        // floating over the rows (it covered the fold chevrons and the
        // selected row). The panel is widened by that column (see
        // `min_sidebar_width`), so the rows keep their width.
        let side_scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).overlay_scrolling(false).vexpand(true).child(&sidebar).build();
        side_scroll.add_css_class("sidebar-scroll");
        // If there are pages below, a gradient with an arrow says so (and
        // clicking it scrolls to the end). Without it, the cut-off list seemed
        // to end there and Flasher and Tools went unnoticed.
        let more = gtk::Button::builder().icon_name("adb-chevron-down-symbolic").tooltip_text(tr("More sections below")).valign(gtk::Align::End).build();
        more.add_css_class("sidebar-more");
        more.set_visible(false);
        let side_overlay = gtk::Overlay::builder().child(&side_scroll).vexpand(true).build();
        side_overlay.add_overlay(&more);
        {
            let adj = side_scroll.vadjustment();
            let more2 = more.clone();
            let update = move |a: &gtk::Adjustment| more2.set_visible(a.value() + a.page_size() < a.upper() - 4.0);
            let u = update.clone();
            adj.connect_value_changed(move |a| u(a));
            adj.connect_changed(move |a| update(a));
            more.connect_clicked(move |_| adj.set_value(adj.upper() - adj.page_size()));
        }
        side_box.append(&side_overlay);
        // Preferences, with the same look and alignment as the rows above.
        let settings_content = util::hbox(12);
        settings_content.set_halign(gtk::Align::Start);
        settings_content.append(&util::icon("adb-settings-symbolic"));
        settings_content.append(&util::label(tr("Preferences"), &[]));
        let settings_btn = gtk::Button::builder().child(&settings_content).tooltip_text(tr("Preferences (Ctrl+,)")).build();
        settings_btn.add_css_class("flat");
        settings_btn.add_css_class("sidebar-settings");
        settings_btn.set_action_name(Some("app.settings"));
        side_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        side_box.append(&settings_btn);

        let side_header = adw::HeaderBar::builder().title_widget(&adw::WindowTitle::new("ADB Scan", "")).build();
        let side_view = adw::ToolbarView::new();
        side_view.add_top_bar(&side_header);
        side_view.set_content(Some(&side_box));

        // -- content ------------------------------------------------------
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).transition_duration(200).hhomogeneous(false).vhomogeneous(false).build();
        let toast = adw::ToastOverlay::new();
        toast.set_child(Some(&stack));
        ctx.set_toast_overlay(&toast);

        let title = adw::WindowTitle::new(tr("Overview"), "");
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        let toggle = gtk::ToggleButton::builder().icon_name("adb-panel-left-symbolic").tooltip_text(tr("Sidebar")).build();
        header.pack_start(&toggle);

        // device selector
        let device_list = gtk::ListBox::new();
        device_list.add_css_class("boxed-list");
        device_list.set_selection_mode(gtk::SelectionMode::None);
        let pop_box = util::vbox(10);
        pop_box.set_margin_top(10);
        pop_box.set_margin_bottom(10);
        pop_box.set_margin_start(10);
        pop_box.set_margin_end(10);
        pop_box.set_width_request(320);
        pop_box.append(&util::label(tr("Devices"), &["heading"]));
        pop_box.append(&device_list);
        let pop_actions = util::hbox(6);
        // The three modes: basic, USB and wireless (QR or manual).
        let wifi_btn = util::pill_button("adb-link-symbolic", tr("Ways to connect…"));
        wifi_btn.set_action_name(Some("win.connect"));
        wifi_btn.set_action_target_value(Some(&"root".to_variant()));
        wifi_btn.set_hexpand(true);
        let refresh_btn = util::icon_button("adb-refresh-cw-symbolic", tr("Refresh"));
        refresh_btn.set_action_name(Some("win.refresh-devices"));
        pop_actions.append(&wifi_btn);
        pop_actions.append(&refresh_btn);
        pop_box.append(&pop_actions);
        let popover = gtk::Popover::builder().child(&pop_box).build();
        // Opening the connection dialog closes the menu: otherwise it stayed
        // on top and the first click in the dialog only dismissed it.
        {
            let p = popover.clone();
            wifi_btn.connect_clicked(move |_| p.popdown());
        }
        let device_menu = gtk::MenuButton::builder().icon_name("adb-smartphone-symbolic").tooltip_text(tr("Choose device")).popover(&popover).build();

        // Private mode: hides serials, IPs, MACs, networks and nicknames on
        // screen.
        let privacy = gtk::ToggleButton::builder()
            .icon_name("adb-eye-off-symbolic")
            .tooltip_text(tr("Private mode: hides serial numbers, IPs, Wi-Fi networks and nicknames (Ctrl+Shift+P)"))
            .active(ctx.private_mode())
            .build();
        privacy.add_css_class("privacy-toggle");
        privacy.set_action_name(Some("win.privacy"));
        let shot = util::icon_button("adb-camera-photo-symbolic", tr("Screenshot (Ctrl+Shift+S)"));
        shot.set_action_name(Some("win.screenshot"));
        let mirror = util::icon_button("adb-video-display-symbolic", tr("Mirror the screen with scrcpy (Ctrl+M)"));
        mirror.set_action_name(Some("win.mirror"));
        let menu = gtk::gio::Menu::new();
        let power = gtk::gio::Menu::new();
        for (label, mode) in [(tr("Restart"), "system"), (tr("Reboot into recovery"), "recovery"), (tr("Reboot into the bootloader"), "bootloader"), (tr("Reboot into fastbootd"), "fastbootd")] {
            power.append(Some(label), Some(&format!("win.reboot::{mode}")));
        }
        menu.append_section(None, &power);
        let general = gtk::gio::Menu::new();
        general.append(Some(tr("Health report…")), Some("win.health-report"));
        general.append(Some(tr("Quick panel")), Some("app.panel"));
        general.append(Some(tr("Preferences")), Some("app.settings"));
        general.append(Some(tr("Keyboard shortcuts")), Some("win.show-help-overlay"));
        general.append(Some(tr("About ADB Scan")), Some("app.about"));
        general.append(Some(tr("Quit")), Some("app.quit"));
        menu.append_section(None, &general);
        let menu_btn = gtk::MenuButton::builder().icon_name("adb-ellipsis-vertical-symbolic").menu_model(&menu).tooltip_text(tr("Menu")).build();
        header.pack_end(&menu_btn);
        header.pack_end(&device_menu);
        header.pack_end(&mirror);
        header.pack_end(&shot);
        header.pack_end(&privacy);

        let update_banner = adw::Banner::new("");
        let content_view = adw::ToolbarView::new();
        content_view.add_top_bar(&header);
        content_view.add_top_bar(&update_banner);
        content_view.set_content(Some(&toast));

        let split = adw::OverlaySplitView::builder()
            .sidebar(&side_view)
            .content(&content_view)
            .min_sidebar_width(222.0)
            .max_sidebar_width(272.0)
            .sidebar_width_fraction(0.22)
            .show_sidebar(s.sidebar_visible)
            .build();
        split.bind_property("show-sidebar", &toggle, "active").bidirectional().sync_create().build();
        window.set_content(Some(&split));

        let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(adw::BreakpointConditionLengthType::MaxWidth, 820.0, adw::LengthUnit::Sp));
        bp.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(bp);

        let this = Rc::new(Self {
            window,
            ctx: ctx.clone(),
            defs,
            stack,
            slots: RefCell::new(HashMap::new()),
            sidebar,
            side,
            title,
            device_name,
            device_sub,
            pill,
            device_menu,
            device_list,
            // Nothing is open yet: with “dashboard” here, selecting its row at
            // startup looked like “already open” and the page was never built
            // (a blank Overview when it was the last section used).
            current: RefCell::new(""),
            privacy_stale: RefCell::new(std::collections::HashSet::new()),
            split,
            quick_buttons: vec![shot, mirror],
            live_icons,
            update_banner,
        });

        {
            let weak = Rc::downgrade(&this);
            this.sidebar.connect_row_selected(move |_, row| {
                let (Some(this), Some(row)) = (weak.upgrade(), row) else {
                    return;
                };
                let Some(id) = this.side.id_of(row) else {
                    return;
                };
                // `show_page` selects the row itself when the page was opened
                // another way (shortcut, Ctrl+K, a link): showing it again
                // would call the page's `shown()` twice.
                let already = *this.current.borrow() == id;
                if !already {
                    this.show_page(id);
                }
                if this.split.is_collapsed() {
                    this.split.set_show_sidebar(false);
                }
            });
        }
        {
            let weak = Rc::downgrade(&this);
            side_search.connect_activate(move |e| {
                let Some(this) = weak.upgrade() else { return };
                let query = e.text().to_string();
                let first = this.side.first_visible();
                e.set_text("");
                match first {
                    Some(id) if !query.trim().is_empty() => {
                        this.show_page(id);
                        if this.split.is_collapsed() {
                            this.split.set_show_sidebar(false);
                        }
                    }
                    Some(_) => {}
                    None => crate::search::show_with(&this.window, &query),
                }
            });
            side_search.connect_stop_search(|e| e.set_text(""));
        }
        {
            let weak = Rc::downgrade(&this);
            ctx.subscribe(move |ev| {
                if let Some(this) = weak.upgrade() {
                    this.on_event(ev);
                }
            });
        }
        this.install_actions();
        this.track_visibility();
        this.save_geometry_on_close();
        let start = ctx.settings.borrow().last_module.clone();
        let start = this.defs.iter().find(|d| d.id == page_alias(&start)).map_or("dashboard", |d| d.id);
        if let Some(row) = this.side.row(start) {
            this.sidebar.select_row(Some(&row));
        }
        // The row may already have been selected (no signal): open it anyway.
        if this.current.borrow().is_empty() {
            this.show_page(start);
        }
        this.refresh_device_ui();
        this
    }

    pub fn present(&self) {
        self.window.present();
    }

    /// Shows that a newer version was published. Its button downloads and
    /// installs it when this copy can (see `crate::update`); otherwise it
    /// opens the release page in the browser.
    pub fn show_update(&self, update: &adbscan_core::updater::Update) {
        let banner = &self.update_banner;
        banner.set_title(&t!("ADB Scan {} is available", update.version));
        if crate::update::available(update) {
            banner.set_button_label(Some(tr("Update now")));
            let (ctx, window, update) = (self.ctx.clone(), self.window.clone(), update.clone());
            banner.connect_button_clicked(move |b| {
                b.set_revealed(false);
                crate::update::start(&ctx, window.upcast_ref(), &update);
            });
            banner.set_revealed(true);
            return;
        }
        banner.set_button_label(Some(tr("View release")));
        let url = update.url.clone();
        banner.connect_button_clicked(move |b| {
            if let Err(e) = gtk::gio::AppInfo::launch_default_for_uri(&url, gtk::gio::AppLaunchContext::NONE) {
                log::warn!("could not open {url}: {e}");
            }
            b.set_revealed(false);
        });
        banner.set_revealed(true);
    }

    pub fn show_page(self: &Rc<Self>, id: &str) {
        let id = page_alias(id);
        let Some(idx) = self.defs.iter().position(|d| d.id == id) else {
            return;
        };
        let def = &self.defs[idx];
        let id = def.id;
        {
            let mut slots = self.slots.borrow_mut();
            if !slots.contains_key(id) {
                let gate = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).transition_duration(180).build();
                let empty = adw::StatusPage::builder().icon_name("adb-smartphone-symbolic").title(tr("Connect a device")).build();
                let basic_btn = gtk::Button::with_label(tr("View basic analysis"));
                basic_btn.add_css_class("pill");
                basic_btn.add_css_class("suggested-action");
                basic_btn.set_action_name(Some("win.go"));
                basic_btn.set_action_target_value(Some(&"basic".to_variant()));
                let ways = gtk::Button::with_label(tr("Ways to connect"));
                ways.add_css_class("pill");
                ways.set_action_name(Some("win.connect"));
                ways.set_action_target_value(Some(&"root".to_variant()));
                let buttons = util::hbox(8);
                buttons.set_halign(gtk::Align::Center);
                buttons.append(&basic_btn);
                buttons.append(&ways);
                empty.set_child(Some(&buttons));
                gate.add_named(&empty, Some("empty"));
                self.stack.add_named(&gate, Some(id));
                slots.insert(id, Slot { def_index: idx, gate, empty, basic_btn, page: None });
            }
        }
        *self.current.borrow_mut() = id;
        self.live_icons.select(id);
        self.ctx.settings.borrow_mut().last_module = id.to_string();
        self.title.set_title(def.title);
        self.update_gate(id);
        self.stack.set_visible_child_name(id);
        if let Some(row) = self.side.row(id) {
            if !row.is_selected() {
                self.sidebar.select_row(Some(&row));
            }
        }
        // The open page stays in view even inside a folded section.
        self.side.refresh();
        let page = self.slots.borrow().get(id).and_then(|s| s.page.clone());
        if let Some(p) = page {
            if self.privacy_stale.borrow_mut().remove(id) {
                let t0 = std::time::Instant::now();
                p.on_event(&Event::Privacy);
                crate::util::log_slow(&format!("event Privacy (deferred) in {id}"), t0);
            }
            let t0 = std::time::Instant::now();
            p.shown();
            crate::util::log_slow(&format!("show {id}"), t0);
            crate::util::log_widget_count(id, &p.widget());
        }
    }

    /// Builds the page the first time it can be shown.
    fn update_gate(&self, id: &str) {
        let (need, build) = {
            let slots = self.slots.borrow();
            let Some(slot) = slots.get(id) else { return };
            let def = &self.defs[slot.def_index];
            (def.need, def.build)
        };
        let ready = self.ctx.ready_serial().is_some();
        let show_content = need == Need::Any || ready;
        if show_content {
            let needs_build = self.slots.borrow().get(id).is_some_and(|s| s.page.is_none());
            if needs_build {
                let t0 = std::time::Instant::now();
                let page = build(&self.ctx);
                crate::util::log_slow(&format!("build page {id}"), t0);
                let mut slots = self.slots.borrow_mut();
                let slot = slots.get_mut(id).unwrap();
                slot.gate.add_named(&page.widget(), Some("content"));
                slot.page = Some(page);
            }
            if let Some(slot) = self.slots.borrow().get(id) {
                slot.gate.set_visible_child_name("content");
            }
        } else if let Some(slot) = self.slots.borrow().get(id) {
            let (title, desc, icon) = self.empty_message();
            slot.empty.set_title(&title);
            slot.empty.set_description(Some(&desc));
            slot.empty.set_icon_name(Some(icon));
            // Without debugging (or without a phone) the basic analysis is
            // always there.
            let usb = self.ctx.usb_devices();
            // The basic analysis does not exist on Windows (no direct USB access).
            let offer_basic = adbscan_core::usb::HOST_USB && self.ctx.selected_device().is_none() && (usb.is_empty() || usb.iter().any(|u| u.mode == adbscan_core::usb::UsbMode::Other));
            slot.basic_btn.set_visible(offer_basic);
            slot.gate.set_visible_child_name("empty");
        }
    }

    fn empty_message(&self) -> (String, String, &'static str) {
        if self.ctx.adb().is_none() {
            let how = if cfg!(windows) {
                tr("adb.exe should be in the platform-tools folder next to adb-scan.exe: reinstall ADB Scan, or set the path in Preferences → Tools.")
            } else {
                tr("Install the Android tools (sudo apt install adb) or set the path in Preferences → Tools.")
            };
            return (tr("ADB was not found").into(), how.into(), "adb-triangle-alert-symbolic");
        }
        match self.ctx.selected_device().map(|d| d.state) {
            Some(DeviceState::Unauthorized) => {
                (tr("Waiting for authorisation").into(), tr("Unlock the phone and accept “Allow USB debugging?”. Tick “Always allow from this computer”.").into(), "adb-key-round-symbolic")
            }
            Some(DeviceState::NoPermissions) => {
                let how = if cfg!(windows) {
                    tr("Windows does not let adb open the phone: install its USB driver (Google USB Driver or the manufacturer's).")
                } else {
                    tr("The system does not allow access to the device. Install the package's udev rules or add your user to the plugdev group.")
                };
                (tr("No USB permissions").into(), how.into(), "adb-triangle-alert-symbolic")
            }
            Some(DeviceState::Offline) => {
                (tr("Device offline").into(), tr("Unplug and replug the cable, or restart the ADB server from Tools.").into(), "adb-triangle-alert-symbolic")
            }
            Some(s) if s.is_fastboot() => {
                (tr("The device is in fastboot mode").into(), tr("This section needs Android running. Use the Flasher or reboot into the system.").into(), "adb-system-software-install-symbolic")
            }
            Some(DeviceState::Recovery) | Some(DeviceState::Sideload) => {
                (tr("The device is in recovery").into(), tr("You can send a ZIP from the Flasher (sideload) or reboot into the system.").into(), "adb-system-reboot-symbolic")
            }
            _ if self.ctx.usb_devices().iter().any(|u| u.has_adb_interface()) => {
                (tr("Phone detected over USB").into(), tr("USB debugging is on: accept the “Allow USB debugging?” prompt on the phone.").into(), "adb-key-round-symbolic")
            }
            _ if !self.ctx.usb_devices().is_empty() => (
                tr("Advanced analysis section").into(),
                tr("This section uses ADB and needs USB debugging, which is optional. The basic analysis already shows what your phone shares without it.").into(),
                "adb-smartphone-symbolic",
            ),
            _ if !adbscan_core::usb::HOST_USB => (
                tr("Connect a device").into(),
                tr("This section uses ADB: connect a phone with USB debugging turned on or over Wi-Fi with wireless debugging. If Windows does not see the phone, install its USB driver.").into(),
                "adb-smartphone-symbolic",
            ),
            _ => (
                tr("Connect a device").into(),
                tr("This section uses ADB: connect a phone with USB debugging turned on (optional) or over Wi-Fi with wireless debugging. Without debugging you get the basic analysis.").into(),
                "adb-smartphone-symbolic",
            ),
        }
    }

    fn on_event(self: &Rc<Self>, ev: &Event) {
        if matches!(ev, Event::Privacy) {
            if let Some(a) = self.window.lookup_action("privacy").and_downcast::<gtk::gio::SimpleAction>() {
                a.set_state(&self.ctx.private_mode().to_variant());
            }
        }
        if matches!(ev, Event::Devices | Event::SettingsChanged | Event::Privacy) {
            self.refresh_device_ui();
            let ids: Vec<&'static str> = self.slots.borrow().keys().copied().collect();
            for id in ids {
                self.update_gate(id);
            }
        }
        if matches!(ev, Event::Snapshot) {
            self.refresh_device_card();
        }
        if matches!(ev, Event::ShowApp(_)) {
            self.show_page("apps");
        }
        if let Event::ShowPage(id) = ev {
            self.show_page(id);
        }
        if matches!(ev, Event::PrepareFlash(..)) {
            self.show_page("flasher");
        }
        // Telemetry (every 500 ms) is only processed by the visible page and
        // only if the window is visible; the others catch up in `shown()`.
        // Previously every page already opened processed it, and the cost grew
        // with each section visited.
        let current = *self.current.borrow();
        let visible = self.window.is_visible() && !self.window.is_suspended();
        if matches!(ev, Event::Privacy) {
            let mut stale = self.privacy_stale.borrow_mut();
            stale.extend(self.slots.borrow().iter().filter(|(id, s)| **id != current && s.page.is_some()).map(|(id, _)| *id));
        }
        let pages: Vec<(&'static str, Rc<dyn Page>)> = self
            .slots
            .borrow()
            .iter()
            .filter(|(id, _)| match ev {
                Event::Snapshot => visible && **id == current,
                Event::Privacy => **id == current,
                _ => true,
            })
            .filter_map(|(id, s)| Some((*id, s.page.clone()?)))
            .collect();
        for (id, p) in pages {
            let t0 = std::time::Instant::now();
            p.on_event(ev);
            crate::util::log_slow(&format!("event {ev:?} in {id}"), t0);
        }
    }

    fn refresh_device_card(&self) {
        let Some(d) = self.ctx.selected_device() else {
            return;
        };
        let mut sub = self.ctx.shown_value(&d.serial);
        if let Some(s) = self.ctx.snapshot() {
            let android = s.android_version();
            let batt = s.battery.percentage().map(|p| format!(" · {p:.0} %")).unwrap_or_default();
            sub = format!("{android}{batt}");
        }
        self.device_sub.set_text(&sub);
    }

    fn refresh_device_ui(self: &Rc<Self>) {
        let devices = self.ctx.devices();
        let selected = self.ctx.selected_device();
        match &selected {
            Some(d) => {
                let name = self.ctx.device_label(d);
                self.device_name.set_text(&name);
                self.title.set_subtitle(&format!("{name} · {}", d.state_label()));
                let kind = match d.state {
                    DeviceState::Device => "ok",
                    DeviceState::Unauthorized | DeviceState::Authorizing => "warn",
                    s if s.is_fastboot() || s == DeviceState::Recovery || s == DeviceState::Sideload => "warn",
                    _ => "bad",
                };
                let via = if d.is_wireless() { " · Wi-Fi" } else { "" };
                self.pill.set(&format!("{}{via}", d.state_label()), kind);
                self.refresh_device_card();
                if self.ctx.snapshot().is_none() {
                    self.device_sub.set_text(&self.ctx.shown_value(&d.serial));
                }
            }
            None => {
                let usb = self.ctx.usb_devices();
                self.device_name.set_text(if usb.is_empty() { tr("No device") } else { tr("Phone over USB") });
                self.device_sub.set_text(&usb.first().map(|u| self.ctx.shown(&u.display_name())).unwrap_or_else(|| tr("Connect a phone over USB").into()));
                // With debugging on, authorisation is missing; without it, the
                // basic analysis already works: it is not an error.
                let (text, kind) = if usb.is_empty() {
                    (tr("Disconnected"), "")
                } else if usb.iter().any(|u| u.has_adb_interface()) {
                    (tr("Not authorised"), "warn")
                } else {
                    (tr("Basic analysis"), "accent")
                };
                self.pill.set(text, kind);
                self.title.set_subtitle(if self.ctx.adb().is_none() { tr("ADB not found") } else { tr("No device") });
            }
        }
        let ready = self.ctx.ready_serial().is_some();
        for b in &self.quick_buttons {
            b.set_sensitive(ready);
        }
        // Selector list
        while let Some(child) = self.device_list.first_child() {
            self.device_list.remove(&child);
        }
        if devices.is_empty() {
            let row = adw::ActionRow::builder().title(tr("No devices")).subtitle(tr("Connect over USB or Wi-Fi")).build();
            self.device_list.append(&row);
        }
        for d in devices {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&self.ctx.device_label(&d)))
                .subtitle(format!("{} · {}{}", self.ctx.shown_value(&d.serial), d.state_label(), if d.is_wireless() { " · Wi-Fi" } else { "" }))
                .activatable(true)
                .build();
            let icon = if d.is_wireless() { "adb-wifi-symbolic" } else { "adb-smartphone-symbolic" };
            row.add_prefix(&util::icon(icon));
            if selected.as_ref().is_some_and(|s| s.serial == d.serial) {
                row.add_suffix(&util::icon("adb-emblem-ok-symbolic"));
            }
            if d.is_wireless() {
                let disc = util::icon_button("adb-circle-x-symbolic", tr("Disconnect"));
                disc.add_css_class("flat");
                disc.set_valign(gtk::Align::Center);
                let ctx = self.ctx.clone();
                let serial = d.serial.clone();
                disc.connect_clicked(move |_| {
                    if let Some(adb) = ctx.adb() {
                        let s = serial.clone();
                        util::run_bg(move || adb.disconnect(&s), |_| {});
                    }
                });
                row.add_suffix(&disc);
            }
            let ctx = self.ctx.clone();
            let serial = d.serial.clone();
            let menu = self.device_menu.clone();
            row.connect_activated(move |_| {
                ctx.select(&serial);
                menu.popdown();
            });
            self.device_list.append(&row);
        }
        // Current section: refreshes the empty message if the state changes.
        let current = *self.current.borrow();
        self.update_gate(current);
    }

    fn install_actions(self: &Rc<Self>) {
        let win = &self.window;
        let add = |name: &str, f: Box<dyn Fn()>| {
            let a = gtk::gio::SimpleAction::new(name, None);
            a.connect_activate(move |_, _| f());
            win.add_action(&a);
        };
        {
            let ctx = self.ctx.clone();
            add("refresh-devices", Box::new(move || ctx.refresh_devices()));
        }
        {
            // Stateful action: the bar button reflects it by itself.
            let a = gtk::gio::SimpleAction::new_stateful("privacy", None, &self.ctx.private_mode().to_variant());
            let ctx = self.ctx.clone();
            a.connect_activate(move |a, _| {
                let on = !a.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
                ctx.set_private_mode(on);
            });
            win.add_action(&a);
        }
        {
            let ctx = self.ctx.clone();
            let w = self.window.clone();
            add("screenshot", Box::new(move || crate::pages::mirror::quick_screenshot(&ctx, w.upcast_ref())));
        }
        {
            let ctx = self.ctx.clone();
            let w = self.window.clone();
            add("health-report", Box::new(move || crate::pages::dashboard::save_health_report(&ctx, w.upcast_ref())));
        }
        {
            let ctx = self.ctx.clone();
            let w = self.window.clone();
            add("mirror", Box::new(move || crate::pages::mirror::quick_mirror(&ctx, w.upcast_ref())));
        }
        {
            let w = self.window.clone();
            add("search", Box::new(move || crate::search::show(&w)));
        }
        {
            // `win.bulk::close|stop|caches`: actions on all apps.
            let ctx = self.ctx.clone();
            let w = self.window.clone();
            let a = gtk::gio::SimpleAction::new("bulk", Some(glib::VariantTy::STRING));
            a.connect_activate(move |_, p| {
                if let Some(which) = p.and_then(|v| v.str().map(String::from)) {
                    crate::pages::bulk::run(&ctx, w.upcast_ref(), &which);
                }
            });
            win.add_action(&a);
        }
        {
            let ctx = self.ctx.clone();
            let w = self.window.clone();
            add("wireless", Box::new(move || crate::wireless::show(&ctx, w.upcast_ref())));
        }
        {
            // `win.connect::<page>`: root, basic, usb, wireless, qr, manual.
            let ctx = self.ctx.clone();
            let w = self.window.clone();
            let a = gtk::gio::SimpleAction::new("connect", Some(glib::VariantTy::STRING));
            a.connect_activate(move |_, p| {
                let page = p.and_then(|v| v.str().map(String::from)).unwrap_or_else(|| "root".into());
                crate::wireless::show_at(&ctx, w.upcast_ref(), &page);
            });
            win.add_action(&a);
        }
        {
            let weak = Rc::downgrade(self);
            let a = gtk::gio::SimpleAction::new("go", Some(glib::VariantTy::STRING));
            a.connect_activate(move |_, p| {
                if let (Some(this), Some(id)) = (weak.upgrade(), p.and_then(|v| v.str().map(String::from))) {
                    this.show_page(&id);
                }
            });
            win.add_action(&a);
        }
        {
            let ctx = self.ctx.clone();
            let w = self.window.clone();
            let a = gtk::gio::SimpleAction::new("reboot", Some(glib::VariantTy::STRING));
            a.connect_activate(move |_, p| {
                let Some(mode) = p.and_then(|v| v.str().map(String::from)) else {
                    return;
                };
                crate::pages::device::confirm_reboot(&ctx, w.upcast_ref(), &mode);
            });
            win.add_action(&a);
        }
        // Shortcuts (not in capture mode: a key pressed while its window has
        // the focus must not act on the phone; see `app::block_real_input`).
        let overlay = crate::help::shortcuts_window(&self.defs);
        self.window.set_help_overlay(Some(&overlay));
        if crate::app::capture_mode() {
            return;
        }
        let app = &self.ctx.app;
        app.set_accels_for_action("win.search", &["<Control>k"]);
        app.set_accels_for_action("win.screenshot", &["<Control><Shift>s"]);
        app.set_accels_for_action("win.privacy", &["<Control><Shift>p"]);
        app.set_accels_for_action("win.mirror", &["<Control>m"]);
        app.set_accels_for_action("win.refresh-devices", &["F5"]);
        app.set_accels_for_action("app.settings", &["<Control>comma"]);
        app.set_accels_for_action("app.quit", &["<Control>q"]);
        app.set_accels_for_action("window.close", &["<Control>w"]);
        for (i, d) in self.defs.iter().enumerate().take(9) {
            app.set_accels_for_action(&format!("win.go::{}", d.id), &[&format!("<Alt>{}", i + 1)]);
        }
    }

    fn track_visibility(self: &Rc<Self>) {
        let ctx = self.ctx.clone();
        let weak = Rc::downgrade(self);
        self.window.connect_map(move |_| {
            ctx.surface_visible(true);
            // It received no telemetry while hidden: catch up.
            if let Some(this) = weak.upgrade() {
                let id = *this.current.borrow();
                let page = this.slots.borrow().get(id).and_then(|s| s.page.clone());
                if let Some(p) = page {
                    p.shown();
                }
            }
        });
        let ctx = self.ctx.clone();
        self.window.connect_unmap(move |_| ctx.surface_visible(false));
    }

    fn save_geometry_on_close(self: &Rc<Self>) {
        let ctx = self.ctx.clone();
        let split = self.split.clone();
        self.window.connect_close_request(move |w| {
            {
                let mut s = ctx.settings.borrow_mut();
                s.window_maximized = w.is_maximized();
                if !w.is_maximized() {
                    s.window_width = w.width();
                    s.window_height = w.height();
                }
                if !split.is_collapsed() {
                    s.sidebar_visible = split.shows_sidebar();
                }
            }
            ctx.save_settings();
            if ctx.settings.borrow().close_to_tray && crate::app::tray_available() {
                w.set_visible(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
}
