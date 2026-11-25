//! Security: read-only audit without root, with a score, recommendations,
//! export and history.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adbscan_core::format;
use adbscan_core::models::now_secs;
use adbscan_core::security::{self, Report, Severity};

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::widgets::{Gauge, Level};
use crate::window::Page;

struct SecurityPage {
    me: std::rc::Weak<Self>,
    ctx: Rc<Ctx>,
    root: gtk::ScrolledWindow,
    gauge: Gauge,
    grade: gtk::Label,
    summary: gtk::Label,
    run_btn: gtk::Button,
    spinner: gtk::Spinner,
    results: gtk::Box,
    export_box: gtk::Box,
    history: gtk::ListBox,
    /// “Delete reports”: only visible if there is any saved.
    clear_btn: gtk::Button,
    report: RefCell<Option<Report>>,
}

/// Header text when no analysis is in view.
const INTRO: &str = "Analyses the phone's configuration without root and without modifying anything: verified boot, patches, encryption, SELinux, signs of root, network ADB, apps with special permissions and more.";

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    SecurityPage::new(ctx)
}

fn sev_icon(s: Severity) -> (&'static str, &'static str) {
    match s {
        Severity::Ok => ("adb-emblem-ok-symbolic", "severity-ok"),
        Severity::Info => ("adb-info-symbolic", "dim"),
        Severity::Low => ("adb-info-symbolic", "severity-low"),
        Severity::Medium => ("adb-dialog-warning-symbolic", "severity-medium"),
        Severity::High => ("adb-triangle-alert-symbolic", "severity-high"),
    }
}

impl SecurityPage {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let content = util::vbox(18);
        let hero = util::hbox(28);
        hero.add_css_class("hero");
        let gauge = Gauge::new(170, 15.0);
        let text = util::vbox(8);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let grade = util::label(tr("Security audit"), &["hero-title"]);
        let summary = util::label(tr(INTRO), &["hero-sub"]);
        summary.set_wrap(true);
        let actions = util::hbox(8);
        let run_btn = util::pill_button("adb-security-high-symbolic", tr("Analyse now"));
        run_btn.add_css_class("suggested-action");
        run_btn.add_css_class("pill");
        let spinner = gtk::Spinner::new();
        actions.append(&run_btn);
        actions.append(&spinner);
        let export_box = util::hbox(6);
        let md = util::pill_button("adb-file-text-symbolic", tr("Export report"));
        let json = util::pill_button("adb-save-symbolic", "JSON");
        export_box.append(&md);
        export_box.append(&json);
        export_box.set_visible(false);
        actions.append(&export_box);
        text.append(&grade);
        text.append(&summary);
        text.append(&actions);
        hero.append(&gauge.root);
        hero.append(&text);
        content.append(&hero);

        let results = util::vbox(18);
        content.append(&results);

        let hist_group = adw::PreferencesGroup::builder().title(tr("Previous reports")).build();
        let clear_btn = util::pill_button("adb-edit-delete-symbolic", tr("Delete reports"));
        // Like “Clear history”: red but not filled, and with confirmation.
        clear_btn.add_css_class("destructive-action");
        clear_btn.add_css_class("flat");
        clear_btn.set_valign(gtk::Align::Center);
        clear_btn.set_visible(false);
        hist_group.set_header_suffix(Some(&clear_btn));
        let history = gtk::ListBox::new();
        history.add_css_class("boxed-list");
        history.set_selection_mode(gtk::SelectionMode::None);
        hist_group.add(&history);
        content.append(&hist_group);

        let root = util::page_scroller(&content, 1000);
        let this = Rc::new_cyclic(|me| Self { me: me.clone(), ctx: ctx.clone(), root, gauge, grade, summary, run_btn, spinner, results, export_box, history, clear_btn, report: RefCell::new(None) });
        this.gauge.set(None, "—", tr("Score"), Level::Normal);
        {
            let w = this.me.clone();
            this.run_btn.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    t.run();
                }
            });
        }
        {
            let w = this.me.clone();
            md.connect_clicked(move |b| {
                let Some(t) = w.upgrade() else { return };
                let Some(r) = t.report.borrow().clone() else {
                    return;
                };
                let name = format!("security-{}-{}.md", r.serial, format::file_stamp(r.timestamp as f64));
                let c = t.ctx.clone();
                util::save_file(b, tr("Export report"), &name, move |p| match std::fs::write(&p, r.to_markdown()) {
                    Ok(_) => c.toast_action(tr("Report saved"), tr("Open"), move || util::open_path(&p)),
                    Err(e) => c.toast(&format!("Error: {e}")),
                });
            });
        }
        {
            let w = this.me.clone();
            json.connect_clicked(move |b| {
                let Some(t) = w.upgrade() else { return };
                let Some(r) = t.report.borrow().clone() else {
                    return;
                };
                let name = format!("security-{}-{}.json", r.serial, format::file_stamp(r.timestamp as f64));
                let c = t.ctx.clone();
                util::save_file(b, tr("Export JSON"), &name, move |p| match serde_json::to_string_pretty(&r).map(|j| std::fs::write(&p, j)) {
                    Ok(Ok(_)) => c.toast(tr("JSON saved")),
                    _ => c.toast(tr("Could not save")),
                });
            });
        }
        {
            let w = this.me.clone();
            this.clear_btn.connect_clicked(move |b| {
                let Some(t) = w.upgrade() else { return };
                t.confirm_clear(b);
            });
        }
        this.load_history();
        this
    }

    /// Asks for confirmation and deletes the saved reports of the connected
    /// phone. Exported files are not touched.
    fn confirm_clear(&self, button: &gtk::Button) {
        let (Some(h), Some(serial)) = (self.ctx.history.clone(), self.ctx.ready_serial()) else { return };
        let device = self.ctx.selected_device().map(|d| self.ctx.device_label(&d)).unwrap_or_else(|| tr("this phone").into());
        let w = self.me.clone();
        util::confirm(
            button,
            tr("Delete the security reports?"),
            &t!("The saved reports of {} will be deleted. Those you exported to files are kept.", device),
            tr("Clear"),
            true,
            move || {
                let Some(t) = w.upgrade() else { return };
                match h.delete_security_reports(&serial) {
                    Ok(n) => {
                        t.ctx.toast(&t!("{} {} deleted", n, if n == 1 { tr("report") } else { tr("reports") }));
                        // The analysis in view was one of those deleted: back to the start.
                        t.report.borrow_mut().take();
                        t.gauge.set(None, "—", tr("Score"), Level::Normal);
                        t.grade.set_text(tr("Security audit"));
                        t.summary.set_text(tr(INTRO));
                        while let Some(c) = t.results.first_child() {
                            t.results.remove(&c);
                        }
                        t.export_box.set_visible(false);
                    }
                    Err(e) => t.ctx.toast(&t!("Could not delete them: {}", e)),
                }
                t.load_history();
            },
        );
    }

    fn run(&self) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        self.run_btn.set_sensitive(false);
        self.spinner.start();
        self.summary.set_text(tr("Analysing… it may take a few seconds if many apps are installed."));
        let w = self.me.clone();
        let s2 = serial.clone();
        util::run_bg(
            move || {
                let out = adb.shell_script(&s2, security::SCRIPT, Duration::from_secs(180)).map(|r| r.stdout);
                let now = now_secs();
                let offset = format::local_offset_secs();
                let today = ((now as i64) + offset).div_euclid(86_400);
                out.map(|o| security::evaluate(&s2, &o, today, now as i64))
            },
            move |r| {
                let Some(t) = w.upgrade() else { return };
                t.run_btn.set_sensitive(true);
                t.spinner.stop();
                match r {
                    Ok(report) => {
                        if let Some(h) = &t.ctx.history {
                            if let Ok(json) = serde_json::to_string(&report) {
                                let _ = h.save_security_report(&report.serial, report.score, &json);
                            }
                        }
                        t.ctx.audit("security_audit", Some(&format!("{}/100", report.score)), true, None);
                        t.show(&report);
                        *t.report.borrow_mut() = Some(report);
                        t.load_history();
                    }
                    Err(e) => {
                        t.summary.set_text(&t!("Could not complete the analysis: {}", e));
                    }
                }
            },
        );
    }

    fn show(&self, r: &Report) {
        let level = match r.score {
            s if s >= 75 => Level::Normal,
            s if s >= 55 => Level::High,
            _ => Level::Critical,
        };
        self.gauge.set(Some(r.score as f64), &r.score.to_string(), tr("out of 100"), level);
        self.grade.set_text(&t!("Security {}", r.grade().to_lowercase()));
        self.summary.set_text(&t!("{} · {} · patch {}. {} high risks, {} medium, {} low and {} checks passed.", self.ctx.shown_device_name(&r.model), r.android, r.patch, r.count(Severity::High), r.count(Severity::Medium), r.count(Severity::Low), r.count(Severity::Ok)));
        self.export_box.set_visible(true);
        while let Some(c) = self.results.first_child() {
            self.results.remove(&c);
        }
        let mut cats: Vec<String> = Vec::new();
        for f in &r.findings {
            if !cats.contains(&f.category) {
                cats.push(f.category.clone());
            }
        }
        let mut cards = Vec::new();
        let mut name_rows: Vec<(String, adw::ActionRow)> = Vec::new();
        for cat in cats {
            let group = adw::PreferencesGroup::builder().title(&cat).build();
            let mut items: Vec<_> = r.findings.iter().filter(|f| f.category == cat).collect();
            items.sort_by_key(|f| std::cmp::Reverse(f.severity));
            for f in items {
                let (icon, class) = sev_icon(f.severity);
                let row = adw::ExpanderRow::builder().title(glib::markup_escape_text(&self.ctx.shown(&f.title))).subtitle(f.severity.label()).build();
                let img = util::icon(icon);
                img.add_css_class(class);
                row.add_prefix(&img);
                if !f.packages.is_empty() {
                    // One row per app, with its name and actions.
                    for pkg in &f.packages {
                        row.add_row(&self.package_row(&f.id, pkg, &mut name_rows));
                    }
                } else if !f.detail.is_empty() {
                    let d = adw::ActionRow::builder().title(glib::markup_escape_text(&self.ctx.shown(&f.detail))).build();
                    d.set_title_selectable(true);
                    d.add_css_class("property");
                    row.add_row(&d);
                }
                if f.id == "adb_usb" {
                    row.add_row(&self.disable_adb_row());
                }
                if !f.recommendation.is_empty() {
                    let rec = adw::ActionRow::builder().title(glib::markup_escape_text(&self.ctx.shown(&f.recommendation))).subtitle(tr("Recommendation")).build();
                    rec.add_prefix(&util::icon("adb-wrench-symbolic"));
                    row.add_row(&rec);
                }
                row.set_enable_expansion(!f.detail.is_empty() || !f.recommendation.is_empty());
                if f.severity >= Severity::Medium {
                    row.set_expanded(true);
                }
                group.add(&row);
            }
            self.results.append(&group);
            cards.push(group.upcast::<gtk::Widget>());
        }
        crate::widgets::reveal_staggered(&cards);
        self.resolve_names(name_rows);
    }

    /// Row for an app in a finding: display name, package and actions.
    fn package_row(&self, finding: &str, pkg: &str, name_rows: &mut Vec<(String, adw::ActionRow)>) -> adw::ActionRow {
        let labels = crate::pages::apps::cached_labels();
        let name = labels.get(pkg).cloned().unwrap_or_else(|| pkg.to_string());
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(&name)).subtitle(pkg).build();
        row.set_subtitle_selectable(true);
        if !labels.contains_key(pkg) {
            name_rows.push((pkg.to_string(), row.clone()));
        }
        if finding == "unknown_sources" {
            let revoke = gtk::Button::with_label(tr("Revoke"));
            revoke.add_css_class("pill");
            revoke.add_css_class("destructive-action");
            revoke.set_valign(gtk::Align::Center);
            revoke.set_tooltip_text(Some(tr("Remove the “Install unknown apps” permission")));
            let (w, pkg2, row2) = (self.me.clone(), pkg.to_string(), row.clone());
            revoke.connect_clicked(move |b| {
                if let Some(t) = w.upgrade() {
                    t.revoke_installer(b, &pkg2, &row2);
                }
            });
            row.add_suffix(&revoke);
        }
        let open = util::icon_button("adb-package-symbolic", tr("View in Apps"));
        open.add_css_class("flat");
        open.set_valign(gtk::Align::Center);
        let (ctx, pkg2) = (self.ctx.clone(), pkg.to_string());
        open.connect_clicked(move |_| ctx.emit(Event::ShowApp(pkg2.clone())));
        row.add_suffix(&open);
        row
    }

    fn revoke_installer(&self, button: &gtk::Button, pkg: &str, row: &adw::ActionRow) {
        let name = row.title().to_string();
        let (ctx, pkg, button, row) = (self.ctx.clone(), pkg.to_string(), button.clone(), row.clone());
        util::confirm(
            &self.root,
            &t!("Revoke {}'s permission to install apps?", name),
            &t!("{} ({}) will no longer be able to install APKs. If you need it later, Android will ask you again (Settings → Apps → Special app access → Install unknown apps).", name, pkg),
            tr("Revoke"),
            true,
            move || {
                let (b2, r2, p2) = (button.clone(), row.clone(), pkg.clone());
                ctx.device_op(
                    "revoke_install",
                    &pkg,
                    &t!("Permission removed from {}", r2.title()),
                    move |adb, s| adb.deny_install_packages(&s, &p2).map(|r| r.summary()),
                    move |ok| {
                        if ok {
                            b2.set_sensitive(false);
                            b2.set_label(tr("Revoked"));
                            b2.remove_css_class("destructive-action");
                            r2.add_css_class("dim-label");
                        }
                    },
                );
            },
        );
    }

    fn disable_adb_row(&self) -> adw::ActionRow {
        let row = adw::ActionRow::builder().title(tr("Turn off USB debugging now")).subtitle(tr("ADB Scan will lose the connection until you turn it back on on the phone")).build();
        let b = gtk::Button::with_label(tr("Turn off"));
        b.add_css_class("pill");
        b.add_css_class("destructive-action");
        b.set_valign(gtk::Align::Center);
        let ctx = self.ctx.clone();
        let root = self.root.clone();
        b.connect_clicked(move |_| {
            let ctx = ctx.clone();
            util::confirm(
                &root,
                tr("Turn off USB debugging?"),
                tr("The connection with ADB Scan will be cut immediately. To use it again: Settings → Developer options → USB debugging."),
                tr("Turn off"),
                true,
                move || ctx.device_op("disable_adb", "adb_enabled=0", tr("USB debugging turned off"), |adb, s| adb.disable_usb_debugging(&s).map(|r| r.summary()), |_| {}),
            );
        });
        row.add_suffix(&b);
        row
    }

    /// Names of apps that are not in the cache (e.g. system apps): read from
    /// the APK on the phone in the background.
    fn resolve_names(&self, rows: Vec<(String, adw::ActionRow)>) {
        let (Some(adb), Some(serial)) = (self.ctx.adb(), self.ctx.ready_serial()) else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        let pkgs: Vec<String> = rows.iter().map(|(p, _)| p.clone()).collect();
        util::run_bg(
            move || {
                pkgs.iter()
                    .filter_map(|p| {
                        let path = adb.package_apk_paths(&serial, p).ok()?.into_iter().find(|x| x.ends_with("base.apk") || x.ends_with(".apk"))?;
                        Some((p.clone(), adb.package_label(&serial, &path)?))
                    })
                    .collect::<Vec<_>>()
            },
            move |found: Vec<(String, String)>| {
                for (pkg, label) in found {
                    for (p, row) in &rows {
                        if *p == pkg {
                            row.set_title(&glib::markup_escape_text(&label));
                        }
                    }
                }
            },
        );
    }

    fn load_history(&self) {
        let (Some(h), Some(serial)) = (self.ctx.history.clone(), self.ctx.ready_serial()) else {
            return;
        };
        let w = self.me.clone();
        util::run_bg(
            move || h.security_reports(&serial, 10).unwrap_or_default(),
            move |list| {
                let Some(t) = w.upgrade() else { return };
                while let Some(c) = t.history.first_child() {
                    t.history.remove(&c);
                }
                t.clear_btn.set_visible(!list.is_empty());
                if list.is_empty() {
                    t.history.append(&adw::ActionRow::builder().title(tr("There are no reports for this device yet")).build());
                }
                // Without an analysis in view (or with another phone's), the last
                // saved one is shown instead of leaving the gauge empty.
                let current = t.ctx.ready_serial();
                let stale = t.report.borrow().as_ref().map_or(true, |r| Some(&r.serial) != current.as_ref());
                if let (true, Some((_, ts, _, json))) = (stale, list.first()) {
                    if let Ok(r) = serde_json::from_str::<Report>(json) {
                        t.show(&r);
                        t.summary.set_text(&t!("{} Last analysis: {}.", t.summary.text(), format::timestamp(Some(*ts))));
                        *t.report.borrow_mut() = Some(r);
                    }
                }
                for (_, ts, score, json) in list {
                    let row = adw::ActionRow::builder().title(format!("{score} / 100")).subtitle(format::timestamp(Some(ts))).activatable(true).build();
                    row.add_suffix(&util::icon("adb-chevron-down-symbolic"));
                    let w2 = t.me.clone();
                    row.connect_activated(move |_| {
                        if let (Some(t), Ok(r)) = (w2.upgrade(), serde_json::from_str::<Report>(&json)) {
                            t.show(&r);
                            *t.report.borrow_mut() = Some(r);
                        }
                    });
                    t.history.append(&row);
                }
            },
        );
    }
}

impl Page for SecurityPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn on_event(&self, ev: &Event) {
        if matches!(ev, Event::Devices) {
            self.load_history();
        }
        if matches!(ev, Event::Privacy) {
            let report = self.report.borrow().clone();
            if let Some(r) = report {
                self.show(&r);
            }
        }
    }
}
