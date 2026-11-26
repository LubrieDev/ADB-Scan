//! Formatters shared by all modules, following the interface language.

use crate::{
    i18n::{tr, Language},
    t,
};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DASH: &str = "—";

pub fn human_bytes(value: Option<f64>) -> String {
    human_bytes_dec(value, 1)
}

pub fn human_bytes_dec(value: Option<f64>, decimals: usize) -> String {
    let Some(mut number) = value.filter(|v| v.is_finite()) else {
        return DASH.into();
    };
    if number.abs() < 1024.0 {
        return format!("{} B", number as i64);
    }
    const UNITS: [&str; 5] = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut index = 0usize;
    number /= 1024.0;
    while number.abs() >= 1024.0 && index < UNITS.len() - 1 {
        number /= 1024.0;
        index += 1;
    }
    format!("{number:.decimals$} {}", UNITS[index])
}

pub fn human_kib(value: Option<i64>) -> String {
    human_bytes(value.map(|v| v as f64 * 1024.0))
}

pub fn human_duration(seconds: Option<f64>) -> String {
    let Some(total) = seconds.filter(|s| s.is_finite() && *s >= 0.0) else {
        return DASH.into();
    };
    let total = total as i64;
    let (days, rem) = (total / 86_400, total % 86_400);
    let (hours, rem) = (rem / 3600, rem % 3600);
    let (minutes, secs) = (rem / 60, rem % 60);
    if days > 0 {
        format!("{days} d {hours} h {minutes} min")
    } else if hours > 0 {
        format!("{hours} h {minutes} min")
    } else if minutes > 0 {
        format!("{minutes} min {secs} s")
    } else {
        format!("{secs} s")
    }
}

pub fn temperature(celsius: Option<f64>) -> String {
    celsius.map_or_else(|| DASH.into(), |c| format!("{c:.1} °C"))
}

pub fn voltage_mv(millivolts: Option<i64>) -> String {
    millivolts.map_or_else(|| DASH.into(), |mv| format!("{:.3} V", mv as f64 / 1000.0))
}

pub fn frequency_khz(khz: Option<i64>) -> String {
    let Some(khz) = khz else { return DASH.into() };
    let mhz = khz as f64 / 1000.0;
    if mhz >= 1000.0 {
        format!("{:.2} GHz", mhz / 1000.0)
    } else {
        format!("{mhz:.0} MHz")
    }
}

pub fn percent(value: Option<f64>) -> String {
    value.map_or_else(|| DASH.into(), |v| format!("{v:.0} %"))
}

/// Number with the thousands and decimal separators of the interface language
/// (`1,234.5` in English, `1.234,5` in Spanish).
pub fn number(value: Option<f64>, decimals: usize) -> String {
    let Some(value) = value.filter(|v| v.is_finite()) else {
        return DASH.into();
    };
    let (thousands, decimal) = match crate::i18n::language() {
        Language::English => (',', '.'),
        Language::Spanish => ('.', ','),
    };
    let formatted = format!("{:.*}", decimals, value.abs());
    let (int_part, frac_part) = match formatted.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (formatted.clone(), None),
    };
    let mut grouped = String::new();
    for (i, ch) in int_part.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            grouped.push(thousands);
        }
        grouped.push(ch);
    }
    let mut out: String = grouped.chars().rev().collect();
    if value < 0.0 {
        out.insert(0, '-');
    }
    if let Some(frac) = frac_part {
        out.push(decimal);
        out.push_str(&frac);
    }
    out
}

pub fn truncate(text: &str, length: usize) -> String {
    if text.chars().count() <= length {
        return text.to_string();
    }
    let mut out: String = text.chars().take(length.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

pub fn relative_time(ts: Option<f64>) -> String {
    let Some(ts) = ts.filter(|t| *t > 0.0) else {
        return DASH.into();
    };
    let diff = (now() - ts).max(0.0) as i64;
    match diff {
        d if d < 5 => tr("just now").into(),
        d if d < 60 => t!("{} s ago", d),
        d if d < 3600 => t!("{} min ago", d / 60),
        d if d < 86_400 => t!("{} h ago", d / 3600),
        d => t!("{} d ago", d / 86_400),
    }
}

/// Converts UNIX seconds into a civil date (UTC shifted by `offset_secs`).
/// Depending on `chrono` is avoided: the algorithm is Howard Hinnant's.
pub fn civil_from_unix(ts: i64, offset_secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let t = ts + offset_secs;
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, (secs / 3600) as u32, ((secs % 3600) / 60) as u32, (secs % 60) as u32)
}

/// Local offset from UTC. Read from `TZ_OFFSET_SECS` if set (useful in tests)
/// and, otherwise, from the system only once.
pub fn local_offset_secs() -> i64 {
    use std::sync::OnceLock;
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        if let Ok(v) = std::env::var("TZ_OFFSET_SECS") {
            if let Ok(n) = v.parse() {
                return n;
            }
        }
        system_offset_secs()
    })
}

/// `date +%z`.
#[cfg(unix)]
fn system_offset_secs() -> i64 {
    let out = std::process::Command::new("date").arg("+%z").output();
    let Ok(out) = out else { return 0 };
    parse_tz_offset(String::from_utf8_lossy(&out.stdout).trim()).unwrap_or(0)
}

/// The time zone's bias, with daylight saving time if it is in effect.
#[cfg(windows)]
fn system_offset_secs() -> i64 {
    use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    // SAFETY: it only writes to `tz`, which lives on this stack.
    let mut tz: TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
    let id = unsafe { GetTimeZoneInformation(&mut tz) };
    // UTC = local time + bias, in minutes. 1: standard time, 2: daylight
    // saving time, 0: the zone has no daylight saving time.
    let bias = match id {
        0 => tz.Bias,
        1 => tz.Bias + tz.StandardBias,
        2 => tz.Bias + tz.DaylightBias,
        _ => return 0,
    };
    -i64::from(bias) * 60
}

pub fn parse_tz_offset(text: &str) -> Option<i64> {
    if text.len() != 5 {
        return None;
    }
    let sign = match &text[..1] {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let hours: i64 = text[1..3].parse().ok()?;
    let minutes: i64 = text[3..5].parse().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

pub fn timestamp(ts: Option<f64>) -> String {
    let Some(ts) = ts.filter(|t| *t > 0.0) else {
        return DASH.into();
    };
    let (y, m, d, hh, mm, ss) = civil_from_unix(ts as i64, local_offset_secs());
    format!("{} {hh:02}:{mm:02}:{ss:02}", civil_date(y, m, d))
}

pub fn date(ts: Option<f64>) -> String {
    let Some(ts) = ts.filter(|t| *t > 0.0) else {
        return DASH.into();
    };
    let (y, m, d, ..) = civil_from_unix(ts as i64, local_offset_secs());
    civil_date(y, m, d)
}

/// Date in the order of the interface language: `2026-11-21` (ISO 8601,
/// unambiguous in English) or `21/11/2026` in Spanish.
/// Days since 1970-01-01 of a civil date: the inverse of
/// [`civil_from_unix`] (Howard Hinnant's `days_from_civil`).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_date(y: i64, m: u32, d: u32) -> String {
    match crate::i18n::language() {
        Language::English => format!("{y}-{m:02}-{d:02}"),
        Language::Spanish => format!("{d:02}/{m:02}/{y}"),
    }
}

/// Month name in the interface language (1 = January).
pub fn month_name(month: u32) -> Option<&'static str> {
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    MONTHS.get((month as usize).checked_sub(1)?).map(|m| tr(m))
}

/// Local time `HH:MM`.
pub fn clock(ts: f64) -> String {
    let (_, _, _, hh, mm, _) = civil_from_unix(ts as i64, local_offset_secs());
    format!("{hh:02}:{mm:02}")
}

/// Timestamp suitable for file names: `2026-11-21_20-15-03`.
pub fn file_stamp(ts: f64) -> String {
    let (y, m, d, hh, mm, ss) = civil_from_unix(ts as i64, local_offset_secs());
    format!("{y}-{m:02}-{d:02}_{hh:02}-{mm:02}-{ss:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes() {
        assert_eq!(human_bytes(None), "—");
        assert_eq!(human_bytes(Some(512.0)), "512 B");
        assert_eq!(human_bytes(Some(2048.0)), "2.0 KiB");
        assert_eq!(human_bytes(Some(5.0 * 1024.0 * 1024.0 * 1024.0)), "5.0 GiB");
        assert_eq!(human_kib(Some(1024)), "1.0 MiB");
    }

    #[test]
    fn durations() {
        assert_eq!(human_duration(Some(5.0)), "5 s");
        assert_eq!(human_duration(Some(65.0)), "1 min 5 s");
        assert_eq!(human_duration(Some(3700.0)), "1 h 1 min");
        assert_eq!(human_duration(Some(90_000.0)), "1 d 1 h 0 min");
        assert_eq!(human_duration(Some(-1.0)), "—");
    }

    #[test]
    fn numbers_english() {
        assert_eq!(number(Some(1234567.0), 0), "1,234,567");
        assert_eq!(number(Some(1234.5), 1), "1,234.5");
        assert_eq!(number(Some(-12.0), 0), "-12");
        assert_eq!(civil_date(2026, 3, 7), "2026-03-07");
        assert_eq!(month_name(4), Some("April"));
        assert_eq!(month_name(13), None);
    }

    #[test]
    fn freq() {
        assert_eq!(frequency_khz(Some(787_200)), "787 MHz");
        assert_eq!(frequency_khz(Some(3_628_800)), "3.63 GHz");
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_unix(0, 0), (1970, 1, 1, 0, 0, 0));
        // 2026-11-21 20:00:00 UTC
        assert_eq!(civil_from_unix(1_795_291_200, 0), (2026, 11, 21, 20, 0, 0));
        assert_eq!(parse_tz_offset("-0600"), Some(-21_600));
        assert_eq!(parse_tz_offset("+0530"), Some(19_800));
    }

    #[test]
    fn days_from_civil_is_the_inverse() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 11, 21), 1_795_291_200 / 86_400);
        assert_eq!(days_from_civil(2000, 2, 29), 11_016);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        for day in (-800_000..800_000).step_by(997) {
            let (y, m, d, ..) = civil_from_unix(day * 86_400, 0);
            assert_eq!(days_from_civil(y, m.into(), d.into()), day);
        }
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate("hola", 10), "hola");
        assert_eq!(truncate("abcdefghij", 5), "abcd…");
    }
}
