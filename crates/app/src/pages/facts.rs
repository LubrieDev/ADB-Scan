//! Technical sheets: presentation of [`adbscan_core::platform`] (Device page)
//! and [`adbscan_core::app_inspect`] (Apps page).
//!
//! Each row says where the data comes from (tooltip) and, if it is missing,
//! why: not published, restricted by Android, not supported or an error.

use adbscan_core::{i18n::{tr, trc}, t};
use adw::prelude::*;
use gtk::glib;
use std::rc::Rc;

use adbscan_core::apk::{ComponentKind, ManifestSummary};
use adbscan_core::app_inspect::{self as ai, AppInspection};
use adbscan_core::format;
use adbscan_core::platform::{self, PlatformReport};
use adbscan_core::probe::Probe;

use crate::ctx::Ctx;
use crate::util;

/// “title · value” row. If the data is missing, the reason in italics.
pub fn probe_row<T>(ctx: &Ctx, title: &str, value: &Probe<T>, source: &str, fmt: impl FnOnce(&T) -> String) -> adw::ActionRow {
    let (row, label) = util::info_row(title);
    match value {
        Probe::Ok(v) => {
            let text = fmt(v);
            if text.is_empty() {
                label.set_text("—");
            } else {
                ctx.set_shown_text(&label, &text);
            }
        }
        other => {
            label.set_text(&other.status_text().unwrap_or_default());
            label.add_css_class("probe-missing");
        }
    }
    if !source.is_empty() {
        row.set_tooltip_text(Some(&t!("Source: {}", source)));
    }
    row
}

fn text_row(ctx: &Ctx, title: &str, value: &str, source: &str) -> adw::ActionRow {
    probe_row(ctx, title, &Probe::Ok(value.to_string()), source, |v| v.clone())
}

fn esc(text: &str) -> String {
    glib::markup_escape_text(text).to_string()
}

fn ago(secs: f64) -> String {
    t!("{} ago", format::human_duration(Some(secs)))
}

// ---------------------------------------------------------------------------
// Device: platform
// ---------------------------------------------------------------------------

/// Groups of the platform sheet. `report` is `None` while it is being read.
pub fn platform_groups(ctx: &Ctx, props: &std::collections::BTreeMap<String, String>, report: Option<&PlatformReport>) -> Vec<adw::PreferencesGroup> {
    let mut out = Vec::new();
    let pending = Probe::Failed(tr("reading…").to_string());
    let get = |f: fn(&PlatformReport) -> Probe<String>| report.map(f).unwrap_or_else(|| pending.clone());

    // System and boot
    let g = adw::PreferencesGroup::builder().title(tr("Platform and boot")).description(tr("How the system is built and how it boots. Comes from getprop except where noted.")).build();
    g.add(&probe_row(ctx, "SELinux", &get(|r| r.selinux.clone()), "getenforce", |v| platform::selinux_label(v)));
    for f in platform::boot_facts(props).iter().chain(platform::platform_facts(props).iter()) {
        g.add(&probe_row(ctx, f.label, &f.value, f.source, |v| v.clone()));
    }
    g.add(&probe_row(ctx, tr("Kernel build"), &report.map(|r| r.kernel.clone()).unwrap_or_else(|| Probe::Failed(tr("reading…").into())), "/proc/version", |k| {
        let mut parts = vec![k.compiler.clone(), k.built.clone()];
        if let Some(g) = &k.gki {
            parts.push(t!("GKI generic kernel {}", g));
        }
        parts.retain(|p| !p.is_empty());
        parts.join(" · ")
    }));
    g.add(&probe_row(ctx, tr("Memory page size"), &report.map(|r| r.page_size.clone()).unwrap_or_else(|| Probe::Failed(tr("reading…").into())), "getconf PAGESIZE", |v| {
        format!("{} KB", v / 1024)
    }));
    g.add(&probe_row(ctx, tr("Google Play system"), &get(|r| r.mainline.clone()), "dumpsys package com.google.android.modulemetadata", |v| v.clone()));
    out.push(g);

    // Graphics
    let g = adw::PreferencesGroup::builder().title(tr("Graphics")).build();
    let gles = report.map(|r| r.gles.clone()).unwrap_or_else(|| Probe::Failed(tr("reading…").into()));
    g.add(&probe_row(ctx, "GPU", &gles, "dumpsys SurfaceFlinger (GLES)", |v| format!("{} · {}", v.renderer, v.vendor)));
    match (&gles, platform::opengles_from_prop(props)) {
        (Probe::Ok(v), _) => g.add(&text_row(ctx, "OpenGL ES", &v.version, "dumpsys SurfaceFlinger (GLES)")),
        (_, Some(v)) => g.add(&text_row(ctx, "OpenGL ES", &v, "ro.opengles.version")),
        _ => {}
    }
    let vk = report.map(|r| r.vulkan.clone()).unwrap_or_else(|| Probe::Failed(tr("reading…").into()));
    g.add(&probe_row(ctx, "Vulkan", &vk, "cmd gpu vkjson (Android 10+)", |v| t!("{} · driver {}", v.api, v.driver_version)));
    if let Probe::Ok(v) = &vk {
        if !v.driver_name.is_empty() {
            g.add(&text_row(ctx, tr("Vulkan driver"), &v.driver_name, "cmd gpu vkjson"));
        }
    }
    if let Some(Probe::Ok(m)) = report.map(|r| &r.gpu_model) {
        g.add(&text_row(ctx, tr("Graphics chip model"), m, "/sys/class/kgsl/kgsl-3d0/gpu_model (Adreno)"));
    }
    out.push(g);

    // CPU by cluster
    let g = adw::PreferencesGroup::builder().title(tr("Processor by cluster")).description(tr("Limits the kernel allows each group of cores. The current frequency is in Performance.")).build();
    match report.map(|r| &r.clusters) {
        Some(Probe::Ok(clusters)) => {
            for c in clusters {
                let row = adw::ActionRow::builder().title(c.cpus_label()).build();
                let mut parts = vec![format!("{} – {}", format::frequency_khz(c.min_khz), format::frequency_khz(c.max_khz))];
                parts.push(c.governor.display(|g| t!("governor {}", g)));
                if let Probe::Ok(n) = c.steps {
                    parts.push(t!("{} frequency steps", n));
                }
                row.set_subtitle(&esc(&parts.join(" · ")));
                row.set_tooltip_text(Some(tr("Source: /sys/devices/system/cpu/cpufreq/policy*")));
                g.add(&row);
            }
        }
        Some(other) => g.add(&probe_row(ctx, tr("Clusters"), other, "/sys/devices/system/cpu/cpufreq", |_| String::new())),
        None => g.add(&probe_row(ctx, tr("Clusters"), &pending, "", |_: &String| String::new())),
    }
    out.push(g);

    // Thermal limits
    let g = adw::PreferencesGroup::builder()
        .title(tr("Thermal limits"))
        .description(tr("Thresholds declared by the manufacturer, not measurements: beyond them the system lowers performance or shuts down. Current temperatures are in Performance."))
        .build();
    match report.map(|r| &r.thermal_limits) {
        Some(Probe::Ok(limits)) => {
            let range = |r: Option<(f64, f64)>| match r {
                Some((a, b)) if (a - b).abs() < 0.05 => format::temperature(Some(a)),
                Some((a, b)) => format!("{} – {}", format::temperature(Some(a)), format::temperature(Some(b))),
                None => "—".into(),
            };
            for l in limits {
                let row = adw::ActionRow::builder().title(esc(&l.kind)).subtitle(esc(&t!("throttles at {} · shuts down at {}", range(l.throttling), range(l.shutdown)))).build();
                row.set_tooltip_text(Some(tr("Source: dumpsys hardware_properties")));
                g.add(&row);
            }
        }
        Some(other) => g.add(&probe_row(ctx, tr("Thresholds"), other, "dumpsys hardware_properties", |_| String::new())),
        None => g.add(&probe_row(ctx, tr("Thresholds"), &pending, "", |_: &String| String::new())),
    }
    out.push(g);

    // Hardware features
    let g = adw::PreferencesGroup::builder().title(tr("Hardware features")).description(tr("What the manufacturer declares to Android (pm list features). “No” means it is not declared.")).build();
    match report.map(|r| &r.features) {
        Some(Probe::Ok(features)) => {
            for (label, present, detail) in platform::feature_highlights(features) {
                let (row, l) = util::info_row(label);
                l.set_text(&match (present, detail.is_empty()) {
                    (true, true) => tr("Yes").to_string(),
                    (true, false) => t!("Yes · {}", detail),
                    (false, _) => tr("No").to_string(),
                });
                if !present {
                    l.add_css_class("probe-missing");
                }
                g.add(&row);
            }
            let all = adw::ExpanderRow::builder().title(t!("All declared features ({})", features.len())).build();
            let features = features.clone();
            lazy_rows(&all, move |exp| {
                for f in &features {
                    let r = adw::ActionRow::builder().title(esc(&f.name)).build();
                    r.set_title_selectable(true);
                    if let Some(v) = &f.version {
                        r.set_subtitle(&esc(&t!("version {}", v)));
                    }
                    exp.add_row(&r);
                }
            });
            g.add(&all);
        }
        Some(other) => g.add(&probe_row(ctx, tr("Features"), other, "pm list features", |_| String::new())),
        None => g.add(&probe_row(ctx, tr("Features"), &pending, "", |_: &String| String::new())),
    }
    out.push(g);
    out
}

/// Creates an expander's rows the first time it opens: an app like Google Play
/// Services declares more than a thousand components. Even when closed, GTK
/// measures and lays out the text of every row it contains, and that blocks the
/// interface (~10 ms per row on a long page).
pub(crate) fn lazy_rows(exp: &adw::ExpanderRow, build: impl Fn(&adw::ExpanderRow) + 'static) {
    let done = std::cell::Cell::new(false);
    exp.connect_expanded_notify(move |e| {
        if e.is_expanded() && !done.replace(true) {
            build(e);
            util::cap_wrapping(e);
        }
    });
}

// ---------------------------------------------------------------------------
// Apps: technical details of an app
// ---------------------------------------------------------------------------

/// Maximum rows per component type (the rest is summarised).
const MAX_COMPONENT_ROWS: usize = 200;
/// Recorded accesses: the first ones in view, up to the maximum in total.
const VISIBLE_OPS: usize = 8;
const MAX_OPS: usize = 40;

/// “Recorded accesses” row.
fn op_row(o: &ai::AppOpUse) -> adw::ActionRow {
    let mut parts = Vec::new();
    if let Some(t) = o.last_access_ago {
        parts.push(t!("Allowed {}", ago(t)));
    }
    if let Some(d) = o.duration.filter(|d| *d >= 1.0) {
        parts.push(t!("for {}", format::human_duration(Some(d))));
    }
    if let Some(t) = o.last_reject_ago {
        parts.push(t!("Denied {}", ago(t)));
    }
    let r = adw::ActionRow::builder().title(esc(&ai::op_label(&o.op))).subtitle(esc(&parts.join(" · "))).build();
    r.set_tooltip_text(Some(&t!("{} · mode {}", o.op, o.mode)));
    if ai::is_sensitive_op(&o.op) {
        r.add_prefix(&util::icon("adb-view-reveal-symbolic"));
    }
    r
}

pub fn inspection_groups(ctx: &Rc<Ctx>, a: &AppInspection) -> Vec<adw::PreferencesGroup> {
    let mut out = Vec::new();
    let pkg = a.package.as_str();

    // -- Status according to Android
    // ------------------------------------------------
    let g = adw::PreferencesGroup::builder().title(tr("Status according to Android")).description(t!("Read at {}. Read-only: it changes nothing on the phone.", format::clock(a.read_at))).build();
    g.add(&probe_row(ctx, tr("Processes"), &a.processes, "dumpsys activity lru", |procs| {
        if procs.is_empty() {
            return tr("Not running").into();
        }
        procs
            .iter()
            .map(|p| {
                let sub = p.process.strip_prefix(pkg).unwrap_or("").trim_start_matches(':');
                let state = ai::proc_state_label(&p.state);
                if sub.is_empty() {
                    state.to_string()
                } else {
                    format!("{state} ({sub})")
                }
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }));
    g.add(&probe_row(ctx, tr("Standby bucket"), &a.standby_bucket, "am get-standby-bucket (Android 9+)", |b| {
        let (name, what) = ai::standby_bucket_label(*b);
        if what.is_empty() {
            name.to_string()
        } else {
            format!("{name}: {what}")
        }
    }));
    g.add(&probe_row(ctx, tr("Battery optimisation"), &a.battery_exemption, "dumpsys deviceidle whitelist", |e| match e {
        Some(kind) => ai::exemption_label(kind).to_string(),
        None => tr("Optimised (no exemption)").to_string(),
    }));
    if let Probe::Ok(e) = &a.extras {
        let u = &e.user;
        let mut notes = Vec::new();
        if u.stopped == Some(true) {
            notes.push(tr("Stopped: it receives no notifications or alarms until it is opened"));
        }
        if u.not_launched == Some(true) {
            notes.push(tr("Never opened"));
        }
        if u.suspended == Some(true) {
            notes.push(tr("Suspended"));
        }
        if u.hidden == Some(true) {
            notes.push(tr("Hidden"));
        }
        if u.quarantined == Some(true) {
            notes.push(tr("Quarantined"));
        }
        if u.installed == Some(false) {
            notes.push(tr("Not installed for user 0"));
        }
        let text = if notes.is_empty() { "Normal".to_string() } else { notes.join(" · ") };
        g.add(&text_row(ctx, tr("Status for the user"), &text, "dumpsys package (User 0)"));
    }
    out.push(g);

    // -- Package ----------------------------------------------------------------
    let g = adw::PreferencesGroup::builder().title(tr("Package")).build();
    match &a.extras {
        Probe::Ok(e) => {
            let abi = match (&e.primary_abi, &e.secondary_abi) {
                (Some(p), Some(s)) => t!("{} · also {}", p, s),
                (Some(p), None) => p.clone(),
                _ => tr("No native code").into(),
            };
            g.add(&text_row(ctx, tr("Architecture (ABI)"), &abi, "dumpsys package (primaryCpuAbi)"));
            if !e.splits.is_empty() {
                g.add(&text_row(ctx, tr("APK parts"), &format!("{} · {}", e.splits.len(), e.splits.join(", ")), "dumpsys package (splits)"));
            }
            g.add(&probe_row(ctx, tr("APK signature"), &e.signing_scheme.map(Probe::Ok).unwrap_or_default(), "dumpsys package (apkSigningVersion)", |v| t!("Scheme v{}", v)));
            if let Some((status, reason)) = &e.dexopt {
                g.add(&text_row(ctx, tr("Compilation (ART)"), &format!("{} · {status}, {reason}", ai::dexopt_label(status)), "dumpsys package (Dexopt state)"));
            }
            if let Some((version, path)) = &e.factory_version {
                g.add(&text_row(ctx, tr("Factory version"), &t!("{} in {} (the installed one is an update)", version, path), "dumpsys package (Hidden system packages)"));
            }
            if let Some(i) = e.initiating_installer.as_ref().filter(|i| Some(*i) != e.installer.as_ref()) {
                g.add(&text_row(ctx, tr("Installation initiated by"), i, "dumpsys package (initiatingPackageName)"));
            }
            if let Some(s) = &e.shared_user {
                g.add(&text_row(ctx, tr("Shared UID"), s, "dumpsys package (sharedUser)"));
            }
            if let Some(d) = &e.data_dir {
                g.add(&text_row(ctx, tr("Data folder"), d, "dumpsys package (dataDir)"));
            }
        }
        other => g.add(&probe_row(ctx, tr("Details"), other, "dumpsys package", |_| String::new())),
    }
    let cert = a.links.clone().map(|l| l.signatures.join("\n")).and_then_empty();
    let cert_row = probe_row(ctx, tr("Signing certificate (SHA-256)"), &cert, "pm get-app-links (Android 12+)", |v| v.clone());
    if !cert.is_ok() {
        cert_row.set_subtitle(tr("Android only publishes it for apps with web links"));
    }
    g.add(&cert_row);
    if let Probe::Ok(m) = &a.manifest {
        g.add(&text_row(ctx, tr("Unencrypted traffic (HTTP)"), &cleartext_text(m), "AndroidManifest.xml"));
    }
    out.push(g);

    // -- Components --------------------------------------------------------------
    let g = adw::PreferencesGroup::builder()
        .title(tr("Components"))
        .description(
            tr("Read from the manifest of the installed APK. “Open”: exported and without a permission, any app on the phone can invoke it (normal for the main screen and share menus)."),
        )
        .build();
    match &a.manifest {
        Probe::Ok(m) => {
            for kind in ComponentKind::ALL {
                let items: Vec<_> = m.of_kind(kind).cloned().collect();
                if items.is_empty() {
                    continue;
                }
                let exported = items.iter().filter(|c| c.is_exported(m.target_sdk)).count();
                let open = items.iter().filter(|c| c.is_open(m.target_sdk)).count();
                let exp = adw::ExpanderRow::builder().title(kind.plural()).subtitle(t!("{} · {} exported · {} open", items.len(), exported, open)).build();
                let (target, package) = (m.target_sdk, m.package.clone());
                let disabled: Vec<String> = a.extras.ok().map(|e| e.disabled_components.clone()).unwrap_or_default();
                lazy_rows(&exp, move |exp| {
                    let mut sorted = items.clone();
                    sorted.sort_by_key(|c| (!c.is_open(target), !c.is_exported(target), c.name.clone()));
                    for c in sorted.iter().take(MAX_COMPONENT_ROWS) {
                        let mut tags = Vec::new();
                        if c.is_open(target) {
                            tags.push(trc("state", "Open").to_string());
                        } else if c.is_exported(target) {
                            tags.push(t!("Exported · requires {}", c.permission.as_deref().unwrap_or("")));
                        } else {
                            tags.push(tr("Internal").to_string());
                        }
                        if c.exported_attr.is_none() && c.is_exported(target) {
                            tags.push(tr("implicitly exported").into());
                        }
                        if c.has_intent_filter {
                            tags.push(tr("with intent filters").into());
                        }
                        if c.disabled_in_manifest || disabled.contains(&c.name) {
                            tags.push(tr("disabled").into());
                        }
                        if let Some(auth) = &c.authorities {
                            tags.push(format!("content://{auth}"));
                        }
                        let r = adw::ActionRow::builder().title(esc(&c.short_name(&package))).subtitle(esc(&tags.join(" · "))).build();
                        r.set_title_selectable(true);
                        if c.is_open(target) {
                            r.add_css_class("open-component");
                        }
                        exp.add_row(&r);
                    }
                    if sorted.len() > MAX_COMPONENT_ROWS {
                        exp.add_row(&adw::ActionRow::builder().title(t!("… and {} more", sorted.len() - MAX_COMPONENT_ROWS)).build());
                    }
                });
                g.add(&exp);
            }
            if m.components.is_empty() {
                g.add(&adw::ActionRow::builder().title(tr("Declares no components")).build());
            }
        }
        other => g.add(&probe_row(ctx, tr("Manifest"), other, "unzip -p base.apk AndroidManifest.xml", |_| String::new())),
    }
    if let Probe::Ok(e) = &a.extras {
        if !e.declared_permissions.is_empty() {
            let exp = adw::ExpanderRow::builder().title(t!("Permissions it declares ({})", e.declared_permissions.len())).subtitle(tr("Its own permissions that other apps can request")).build();
            let perms = e.declared_permissions.clone();
            lazy_rows(&exp, move |exp| {
                for (name, prot) in &perms {
                    let r = adw::ActionRow::builder().title(esc(name.rsplit('.').next().unwrap_or(name))).subtitle(esc(&t!("{} · protection {}", name, prot))).build();
                    exp.add_row(&r);
                }
            });
            g.add(&exp);
        }
    }
    out.push(g);

    // -- App Ops -----------------------------------------------------------------
    let g = adw::PreferencesGroup::builder().title(tr("Recorded accesses")).description(tr("Last use of each protected operation according to Android (App Ops). Most recent first.")).build();
    match &a.appops {
        Probe::Ok(ops) if ops.is_empty() => g.add(&adw::ActionRow::builder().title(tr("No recorded accesses")).build()),
        Probe::Ok(ops) => {
            // The most recent in view; the rest when the expander opens.
            for o in ops.iter().take(VISIBLE_OPS) {
                g.add(&op_row(o));
            }
            let rest: Vec<ai::AppOpUse> = ops.iter().skip(VISIBLE_OPS).take(MAX_OPS - VISIBLE_OPS).cloned().collect();
            if !rest.is_empty() {
                let exp = adw::ExpanderRow::builder().title(t!("{} more accesses", rest.len())).build();
                lazy_rows(&exp, move |exp| {
                    for o in &rest {
                        exp.add_row(&op_row(o));
                    }
                });
                g.add(&exp);
            }
        }
        other => g.add(&probe_row(ctx, "App Ops", other, "cmd appops get", |_| String::new())),
    }
    out.push(g);

    // -- Exits -------------------------------------------------------------------
    let g = adw::PreferencesGroup::builder().title(tr("Recent exits")).build();
    match &a.exits {
        Probe::Ok(exits) => {
            let failures = exits.iter().filter(|e| e.is_failure()).count();
            g.set_description(Some(&t!("{} recorded by Android · {} {}. Includes normal exits (the user swiped it away, low memory…).", exits.len(), failures, if failures == 1 { tr("crash or ANR") } else { tr("crashes or ANRs") })));
            if exits.is_empty() {
                g.add(&adw::ActionRow::builder().title(tr("No recorded exits")).build());
            }
            for e in exits.iter().take(12) {
                let mut title = e.reason_label();
                let sub_reason = ai::exit_subreason_label(&e.subreason);
                if !sub_reason.is_empty() {
                    title.push_str(&format!(" · {sub_reason}"));
                }
                let mut sub = vec![e.timestamp.split('.').next().unwrap_or(&e.timestamp).to_string()];
                let proc_suffix = e.process.strip_prefix(pkg).unwrap_or(&e.process).trim_start_matches(':');
                if !proc_suffix.is_empty() {
                    sub.push(t!("process {}", proc_suffix));
                }
                if let Some(i) = e.importance.map(ai::importance_label).filter(|l| !l.is_empty()) {
                    sub.push(i.to_string());
                }
                if !e.rss.is_empty() && e.rss.trim_end_matches(|c: char| c.is_ascii_alphabetic()).parse::<f64>().map_or(true, |v| v > 0.0) {
                    sub.push(t!("{} in memory", e.rss));
                }
                if !e.description.is_empty() && e.is_failure() {
                    sub.push(e.description.clone());
                }
                let r = adw::ActionRow::builder().title(esc(&title)).subtitle(esc(&ctx.shown(&sub.join(" · ")))).build();
                if e.is_failure() {
                    r.add_prefix(&util::icon("adb-dialog-warning-symbolic"));
                }
                g.add(&r);
            }
        }
        other => g.add(&probe_row(ctx, tr("Exit history"), other, "dumpsys activity exit-info", |_| String::new())),
    }
    out.push(g);

    // -- Web links ---------------------------------------------------------------
    if let Probe::Ok(l) = &a.links {
        if !l.domains.is_empty() {
            let g = adw::PreferencesGroup::builder().title(tr("Web links")).description(tr("Domains the app opens directly (App Links)")).build();
            let exp = adw::ExpanderRow::builder().title(t!("{} domains", l.domains.len())).build();
            let domains = l.domains.clone();
            lazy_rows(&exp, move |exp| {
                for (d, state) in &domains {
                    exp.add_row(&adw::ActionRow::builder().title(esc(d)).subtitle(ai::domain_state_label(state)).build());
                }
            });
            g.add(&exp);
            out.push(g);
        }
    }
    out
}

fn cleartext_text(m: &ManifestSummary) -> String {
    match (m.uses_cleartext, m.network_security_config) {
        (Some(true), _) => tr("Allowed by the manifest").into(),
        (Some(false), _) => tr("Blocked by the manifest").into(),
        (None, true) => tr("According to its network security configuration").into(),
        // Since API 28 Android blocks HTTP by default.
        (None, false) if m.target_sdk.is_some_and(|t| t >= 28) => tr("Blocked (default since API 28)").into(),
        (None, false) => tr("Allowed (default before API 28)").into(),
    }
}

/// An empty text in `Ok` counts as “not published”.
trait EmptyAsMissing {
    fn and_then_empty(self) -> Self;
}

impl EmptyAsMissing for Probe<String> {
    fn and_then_empty(self) -> Self {
        match self {
            Probe::Ok(v) if v.trim().is_empty() => Probe::Missing,
            other => other,
        }
    }
}
