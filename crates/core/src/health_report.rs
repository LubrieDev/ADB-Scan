//! Phone health report: one self-contained HTML page (printable to PDF from
//! the browser) with what matters when buying, selling or taking a phone to
//! a repair shop: battery wear, storage life, bootloader and verified boot,
//! the latest security audit and the public sheet.
//!
//! It is made to be shared: like the public sheet, it has no serial number,
//! IMEI, accounts or nickname. Everything comes from data already read
//! (telemetry snapshot and the saved audit); nothing is asked of the phone.

use crate::format;
use crate::i18n::tr;
use crate::models::{HealthLevel, Snapshot};
use crate::security::{Report, Severity};
use crate::t;

pub struct Input<'a> {
    pub snapshot: &'a Snapshot,
    /// Latest audit of this phone, if any.
    pub security: Option<&'a Report>,
    /// Marketing name obtained another way (see `identity::public_sheet`).
    pub commercial_hint: &'a str,
    /// When the report is made (seconds since the epoch).
    pub generated: f64,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Verdict colour: `good`, `fair`, `poor` or `none`.
fn class(level: Option<HealthLevel>) -> &'static str {
    match level {
        Some(HealthLevel::Good) => "good",
        Some(HealthLevel::Fair) => "fair",
        Some(HealthLevel::Poor) => "poor",
        None => "none",
    }
}

/// Bootloader lock from the boot properties.
fn bootloader(s: &Snapshot) -> (String, &'static str) {
    let state = s.prop("ro.boot.vbmeta.device_state");
    let locked = s.prop("ro.boot.flash.locked");
    match (state, locked) {
        ("locked", _) | (_, "1") => (tr("Locked").into(), "good"),
        ("unlocked", _) | (_, "0") => (tr("Unlocked").into(), "fair"),
        _ => ("—".into(), "none"),
    }
}

/// Verified boot (`green` = original, signed system).
fn verified_boot(s: &Snapshot) -> (String, &'static str) {
    match s.prop("ro.boot.verifiedbootstate") {
        "green" => (tr("Original system (green)").into(), "good"),
        "yellow" => (tr("Signed with its own key (yellow)").into(), "fair"),
        "orange" => (tr("Unverified system (orange)").into(), "poor"),
        "red" => (tr("Verification failed (red)").into(), "poor"),
        _ => ("—".into(), "none"),
    }
}

fn card(title: &str, value: &str, detail: &str, class: &str) -> String {
    format!("<div class=\"card {class}\"><div class=\"k\">{}</div><div class=\"v\">{}</div><div class=\"d\">{}</div></div>", esc(title), esc(value), esc(detail))
}

/// The report as an HTML document.
pub fn to_html(input: &Input) -> String {
    let s = input.snapshot;
    let h = &s.health;
    let title = t!("Phone health report — {}", s.model());
    let mut cards = String::new();
    let batt = h.battery_percent().map(|p| format!("{} %", format::number(Some(p), 0))).unwrap_or_else(|| "—".into());
    let batt_detail = match (h.battery_cycles, h.battery_full_mah, h.battery_design_mah) {
        (c, Some(full), Some(design)) => t!("{} of {} mAh{}", full, design, c.map(|c| t!(" · {} cycles", c)).unwrap_or_default()),
        (Some(c), _, _) => t!("{} cycles", c),
        _ => tr("The phone does not publish its capacity").to_string(),
    };
    cards.push_str(&card(tr("Battery health"), &batt, &batt_detail, class(h.battery_level())));
    cards.push_str(&card(tr("Storage life"), &h.storage_label(), &h.storage_detail(), class(h.storage_level())));
    let (bl, bl_class) = bootloader(s);
    cards.push_str(&card(tr("Bootloader"), &bl, tr("Unlocked means the system can be modified"), bl_class));
    let (vb, vb_class) = verified_boot(s);
    cards.push_str(&card(tr("Verified boot"), &vb, &t!("Security patch: {}", s.security_patch()), vb_class));
    match input.security {
        Some(r) => {
            let c = match r.score {
                75..=100 => "good",
                55..=74 => "fair",
                _ => "poor",
            };
            cards.push_str(&card(tr("Security"), &format!("{} / 100", r.score), &t!("{} · audit of {}", r.grade(), format::timestamp(Some(r.timestamp as f64))), c));
        }
        None => cards.push_str(&card(tr("Security"), "—", tr("No audit yet: run one in Security"), "none")),
    }
    let used = match (s.data_storage.used_kib, s.data_storage.total_kib) {
        (Some(u), Some(t)) if t > 0 => format!("{} / {}", format::human_kib(Some(u)), format::human_kib(Some(t))),
        _ => "—".into(),
    };
    cards.push_str(&card(tr("Storage in use"), &used, "", "none"));

    let mut sheet = String::new();
    for g in crate::identity::public_sheet(s, input.commercial_hint) {
        sheet.push_str(&format!("<h2>{}</h2><table>", esc(g.title)));
        for (k, v) in &g.rows {
            sheet.push_str(&format!("<tr><th>{}</th><td>{}</td></tr>", esc(k), esc(v)));
        }
        sheet.push_str("</table>");
    }

    let mut audit = String::new();
    if let Some(r) = input.security {
        let mut findings = r.findings.clone();
        findings.sort_by_key(|f| std::cmp::Reverse(f.severity));
        let findings: Vec<_> = findings.into_iter().filter(|f| f.severity > Severity::Info).collect();
        audit.push_str(&format!("<h2>{}</h2>", esc(tr("Security findings"))));
        if findings.is_empty() {
            audit.push_str(&format!("<p>{}</p>", esc(tr("The audit found nothing to fix."))));
        } else {
            audit.push_str("<ul>");
            for f in findings {
                audit.push_str(&format!("<li><b>[{}]</b> {}</li>", esc(f.severity.label()), esc(&f.title)));
            }
            audit.push_str("</ul>");
        }
    }

    format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{title}</title>
<style>
body{{font-family:system-ui,-apple-system,"Segoe UI",Roboto,Cantarell,sans-serif;max-width:860px;margin:32px auto;padding:0 20px;color:#1d1d20;line-height:1.45}}
h1{{font-size:1.6em;margin:0 0 4px}} h2{{font-size:1.1em;margin:28px 0 8px}} .meta{{color:#666;margin-bottom:22px}}
.cards{{display:grid;grid-template-columns:repeat(auto-fill,minmax(250px,1fr));gap:12px}}
.card{{border:1px solid #ddd;border-left:6px solid #bbb;border-radius:10px;padding:12px 14px}}
.card .k{{font-size:.85em;color:#555}} .card .v{{font-size:1.4em;font-weight:700;margin:2px 0}} .card .d{{font-size:.85em;color:#666}}
.good{{border-left-color:#2e9d57}} .fair{{border-left-color:#d08a00}} .poor{{border-left-color:#c62828}}
table{{border-collapse:collapse;width:100%}} th,td{{text-align:left;padding:5px 8px;border-bottom:1px solid #eee;vertical-align:top}} th{{width:38%;font-weight:600;color:#444}}
footer{{margin-top:32px;color:#777;font-size:.85em;border-top:1px solid #ddd;padding-top:10px}}
@media print{{body{{margin:0}} .card{{break-inside:avoid}}}}
</style></head><body>
<h1>{title}</h1>
<div class="meta">{generated}</div>
<div class="cards">{cards}</div>
{sheet}
{audit}
<footer>{footer}</footer>
</body></html>
"#,
        title = esc(&title),
        generated = esc(&t!("Made on {} with ADB Scan", format::timestamp(Some(input.generated)))),
        footer = esc(tr("Read from the phone over ADB, without root and without changing anything. It has no serial number, IMEI or accounts, so it can be shared. Battery health is the capacity the battery reports compared with its design capacity; storage life is the wear the memory chip reports, in steps of 10 %.")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_the_essentials_and_no_serial() {
        let mut s = Snapshot::new("SERIAL123");
        s.props.insert("ro.product.model".into(), "NX809J".into());
        s.props.insert("ro.boot.verifiedbootstate".into(), "green".into());
        s.props.insert("ro.boot.flash.locked".into(), "1".into());
        s.health.battery_design_mah = Some(7500);
        s.health.battery_full_mah = Some(7360);
        s.health.battery_cycles = Some(54);
        let html = to_html(&Input { snapshot: &s, security: None, commercial_hint: "", generated: 0.0 });
        assert!(html.contains("NX809J"));
        assert!(html.contains("98 %"));
        assert!(html.contains("7360 of 7500 mAh"));
        assert!(html.contains("class=\"card good\""));
        assert!(!html.contains("SERIAL123"));
    }

    #[test]
    fn escapes() {
        assert_eq!(esc("<a & \"b\">"), "&lt;a &amp; &quot;b&quot;&gt;");
    }
}
