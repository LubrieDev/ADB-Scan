//! Phone data: public sheet (to share) and private identifiers (IMEI, SIM,
//! accounts, MAC, public IP).
//!
//! Private identifiers never go through the telemetry, the history or the logs:
//! they are read on demand and live only in memory while the view is open.
//!
//! `service call iphonesubinfo` changes its numbering between Android versions
//! (the ICCID is code 12 on Android 13 and 14 on Android 16). That is why each
//! value is accepted **by its shape** (Luhn, carrier prefix, length) and not by
//! the code that returned it: if the shape does not fit, “—” is shown instead
//! of wrong data. Only low `IPhoneSubInfo` codes are used, which are reads in
//! every version.

use crate::{i18n::tr, t};
use std::collections::HashMap;
use std::net::IpAddr;

use crate::format;
use crate::models::Snapshot;
use crate::parsers;

// ---------------------------------------------------------------------------
// Public sheet
// ---------------------------------------------------------------------------

/// Group of the sheet: title and rows (label, value).
#[derive(Debug, Clone, PartialEq)]
pub struct SheetGroup {
    pub title: &'static str,
    pub rows: Vec<(&'static str, String)>,
}

fn non_empty(values: &[&str]) -> String {
    values.iter().filter(|v| !v.is_empty()).cloned().collect::<Vec<_>>().join(" · ")
}

/// What identifies the **model**, not the owner: it can be shared on a forum or
/// with technical support. No serial number, IMEI, accounts or the nickname the
/// user gave the phone.
///
/// `commercial_hint`: marketing name obtained another way (the USB product) for
/// phones that do not publish it in their properties.
pub fn public_sheet(s: &Snapshot, commercial_hint: &str) -> Vec<SheetGroup> {
    let p = |k: &str| s.prop(k).to_string();
    let dash = |v: String| {
        if v.trim().is_empty() {
            "—".to_string()
        } else {
            v
        }
    };
    let mut commercial = s.commercial_name();
    if commercial.is_empty() && commercial_hint != s.model() && commercial_hint != s.serial {
        commercial = commercial_hint.trim().to_string();
    }
    let model = SheetGroup {
        title: tr("Model"),
        rows: vec![(tr("Manufacturer"), s.manufacturer()), (tr("Brand"), s.brand()), (tr("Model"), s.model()), (tr("Marketing name"), dash(commercial)), (tr("Codename"), dash(p("ro.product.device")))],
    };
    let system = SheetGroup {
        title: tr("System"),
        rows: vec![("Android", s.android_version()), (tr("Security patch"), s.security_patch()), (tr("Build"), dash(p("ro.build.display.id"))), (tr("Build type"), dash(p("ro.build.type")))],
    };
    let cpu = &s.cpu;
    let cores = match (cpu.core_count, cpu.max_freq_khz()) {
        (Some(n), Some(_)) => t!("{} · up to {}", n, format::frequency_khz(cpu.max_freq_khz())),
        (Some(n), None) => n.to_string(),
        _ => String::new(),
    };
    let refresh_max = s.display.supported_refresh.iter().cloned().reduce(f64::max).or(s.display.refresh_hz);
    let screen = match (s.display.width, refresh_max) {
        (Some(_), Some(hz)) => t!("{} · {} dpi · up to {} Hz", s.display.resolution(), s.display.density_dpi.unwrap_or(0), format!("{:.0}", hz)),
        (Some(_), None) => format!("{} · {} dpi", s.display.resolution(), s.display.density_dpi.unwrap_or(0)),
        _ => String::new(),
    };
    let hardware = SheetGroup {
        title: "Hardware",
        rows: vec![
            (tr("Processor"), dash(non_empty(&[s.prop("ro.soc.manufacturer"), s.prop("ro.soc.model"), s.prop("ro.board.platform")]))),
            (tr("Cores"), dash(cores)),
            (tr("Architecture"), dash(p("ro.product.cpu.abi"))),
            ("RAM", format::human_kib(s.memory.total_kib)),
            (tr("Storage"), format::human_kib(s.data_storage.total_kib)),
            (tr("Screen"), dash(screen)),
        ],
    };
    let h = &s.health;
    let mut battery_rows = vec![(tr("Design capacity"), h.battery_design_mah.map(|d| format!("{} mAh", format::number(Some(d as f64), 0))).unwrap_or("—".into()))];
    battery_rows.push((tr("Health"), h.battery_percent().map(|p| format!("{} %", format::number(Some(p), 1))).unwrap_or("—".into())));
    battery_rows.push((tr("Charge cycles"), h.battery_cycles.map(|c| c.to_string()).unwrap_or("—".into())));
    let battery = SheetGroup { title: tr("Battery"), rows: battery_rows };
    vec![model, system, hardware, battery]
}

/// Sheet as plain text to paste into a forum or an email.
pub fn sheet_text(title: &str, groups: &[SheetGroup]) -> String {
    let mut out = format!("{title}\n");
    for g in groups {
        out.push_str(&format!("\n{}\n", g.title));
        for (label, value) in &g.rows {
            out.push_str(&format!("  {label}: {value}\n"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Private identifiers
// ---------------------------------------------------------------------------

/// A SIM slot according to the `gsm.*` properties (one entry per slot).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimSlot {
    pub state: String,
    pub operator: String,
    pub numeric: String,
    pub country: String,
}

impl SimSlot {
    pub fn present(&self) -> bool {
        !matches!(self.state.as_str(), "" | "ABSENT" | "NOT_READY" | "UNKNOWN")
    }

    pub fn state_label(&self) -> &'static str {
        match self.state.as_str() {
            "LOADED" | "READY" => tr("Active"),
            "ABSENT" => tr("Empty"),
            "PIN_REQUIRED" => tr("Locked (PIN)"),
            "PUK_REQUIRED" => tr("Locked (PUK)"),
            "NETWORK_LOCKED" => tr("Locked by the carrier"),
            "NOT_READY" => tr("Not ready"),
            "" => "—",
            _ => tr("Unknown"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub kind: String,
}

impl Account {
    /// Readable name of the account type (`com.google` → “Google”).
    pub fn kind_label(&self) -> String {
        let known = match self.kind.as_str() {
            "com.google" => "Google",
            "com.google.android.apps.tachyon" => "Google Meet",
            "com.whatsapp" => "WhatsApp",
            "com.whatsapp.w4b" => "WhatsApp Business",
            "com.github.android" => "GitHub",
            "com.twitter.android.auth.login" => "X (Twitter)",
            "com.facebook.auth.login" => "Facebook",
            "com.microsoft.workaccount" | "com.microsoft.office" => "Microsoft",
            "com.xiaomi" => "Xiaomi",
            "com.samsung.android.mobileservice" | "com.osp.app.signin" => "Samsung",
            "com.android.exchange" | "com.google.android.gm.exchange" => "Exchange",
            _ => "",
        };
        if !known.is_empty() {
            return known.to_string();
        }
        // “org.telegram.messenger” → “Telegram”: the first part that is neither
        // a domain nor a generic word.
        const GENERIC: [&str; 16] = ["com", "org", "net", "www", "ai", "io", "x", "android", "auth", "login", "account", "accounts", "app", "apps", "messenger", "mobile"];
        let name = self.kind.split('.').find(|p| p.len() > 1 && !GENERIC.contains(p)).unwrap_or(&self.kind);
        let mut c = name.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    }
}

/// Phone identifiers. Each field is `None` if Android does not provide it or if
/// the value does not have the expected shape.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivateInfo {
    /// IMEI per slot (1 and 2), validated with Luhn.
    pub imei: [Option<String>; 2],
    pub serial: Option<String>,
    pub android_id: Option<String>,
    pub device_name: Option<String>,
    pub bluetooth_name: Option<String>,
    pub bluetooth_mac: Option<String>,
    pub imsi: Option<String>,
    pub iccid: Option<String>,
    pub phone_number: Option<String>,
    pub sims: Vec<SimSlot>,
    pub accounts: Vec<Account>,
}

/// Read-only script. It interpolates nothing: it is fixed.
pub const PRIVATE_SCRIPT: &str = r#"
echo '@@@IMEI0@@@'; service call iphonesubinfo 4 i32 0 s16 com.android.shell s16 null
echo '@@@IMEI1@@@'; service call iphonesubinfo 4 i32 1 s16 com.android.shell s16 null
echo '@@@IMEI@@@'; service call iphonesubinfo 1 s16 com.android.shell
echo '@@@IMSI@@@'; service call iphonesubinfo 9 s16 com.android.shell s16 null
echo '@@@ICCID@@@'; service call iphonesubinfo 14 s16 com.android.shell s16 null
echo '@@@LINE1@@@'; service call iphonesubinfo 16 s16 com.android.shell s16 null
echo '@@@SERIAL@@@'; getprop ro.serialno
echo '@@@ANDROIDID@@@'; settings get secure android_id
echo '@@@DEVNAME@@@'; settings get global device_name
echo '@@@BTNAME@@@'; settings get secure bluetooth_name
echo '@@@BTMAC@@@'; settings get secure bluetooth_address
echo '@@@SIM@@@'
getprop gsm.sim.state
getprop gsm.sim.operator.alpha
getprop gsm.sim.operator.numeric
getprop gsm.sim.operator.iso-country
echo '@@@ACCOUNTS@@@'; dumpsys account 2>/dev/null | grep -E '^ +Account [{]'
echo '@@@END@@@'
"#;

/// String from a `service call` reply (Parcel with status, length and UTF-16
/// characters). `None` if it is an exception or a null string.
pub fn parse_parcel_string(text: &str) -> Option<String> {
    let mut words: Vec<u32> = Vec::new();
    for line in text.lines() {
        // What is between quotes is the ASCII view: it is not data.
        let data = line.split('\'').next().unwrap_or("");
        let data = data.trim_start_matches("Result: Parcel(");
        for token in data.split_whitespace() {
            if token.len() == 8 && token.chars().all(|c| c.is_ascii_hexdigit()) {
                if let Ok(w) = u32::from_str_radix(token, 16) {
                    words.push(w);
                }
            }
        }
    }
    let (&status, rest) = words.split_first()?;
    if status != 0 {
        return None;
    }
    let (&len, chars) = rest.split_first()?;
    if len == u32::MAX || len > 256 {
        return None;
    }
    let units: Vec<u16> = chars.iter().flat_map(|w| [(*w & 0xffff) as u16, (*w >> 16) as u16]).take(len as usize).collect();
    if units.len() < len as usize {
        return None;
    }
    String::from_utf16(&units).ok()
}

/// Luhn check digit (IMEI, ICCID).
pub fn luhn_ok(digits: &str) -> bool {
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let sum: u32 = digits
        .chars()
        .rev()
        .enumerate()
        .map(|(i, c)| {
            let d = c.to_digit(10).unwrap();
            if i % 2 == 1 {
                let x = d * 2;
                if x > 9 {
                    x - 9
                } else {
                    x
                }
            } else {
                d
            }
        })
        .sum();
    sum % 10 == 0
}

/// IMEI: 15 digits passing Luhn that do not start with the carrier code (an
/// IMSI also has 15 digits and would pass Luhn one time in ten).
pub fn is_imei(value: &str, operator_codes: &[String]) -> bool {
    value.len() == 15 && luhn_ok(value) && !operator_codes.iter().any(|c| c.len() >= 5 && value.starts_with(c.as_str()))
}

/// IMSI: 14–15 digits starting with the MCC+MNC of a present SIM.
pub fn is_imsi(value: &str, operator_codes: &[String]) -> bool {
    (14..=15).contains(&value.len()) && value.chars().all(|c| c.is_ascii_digit()) && operator_codes.iter().any(|c| c.len() >= 5 && value.starts_with(c.as_str()))
}

/// ICCID: 18–20 digits starting with 89 (telecommunications).
pub fn is_iccid(value: &str) -> bool {
    (18..=20).contains(&value.len()) && value.starts_with("89") && value.chars().all(|c| c.is_ascii_digit())
}

/// MAC “aa:bb:cc:dd:ee:ff”, without Android's “hidden” placeholder.
pub fn valid_mac(value: &str) -> Option<String> {
    let v = value.trim().to_uppercase();
    let parts: Vec<&str> = v.split(':').collect();
    let ok = parts.len() == 6 && parts.iter().all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()));
    (ok && v != "02:00:00:00:00:00" && v != "00:00:00:00:00:00").then_some(v)
}

/// Random MAC (“locally administered” bit): Android uses one per Wi-Fi network
/// since version 10 to avoid being tracked.
pub fn is_randomized_mac(mac: &str) -> bool {
    u8::from_str_radix(mac.get(..2).unwrap_or(""), 16).is_ok_and(|b| b & 0x02 != 0)
}

fn clean_setting(value: &str) -> Option<String> {
    let v = value.trim();
    (!v.is_empty() && v != "null").then(|| v.to_string())
}

/// Splits “LOADED,ABSENT” and the like into slots.
pub fn parse_sim_props(text: &str) -> Vec<SimSlot> {
    let lines: Vec<Vec<String>> = text.lines().take(4).map(|l| l.split(',').map(|v| v.trim().to_string()).collect()).collect();
    let get = |row: usize, i: usize| lines.get(row).and_then(|r| r.get(i)).cloned().unwrap_or_default();
    let slots = lines.first().map(|r| r.len()).unwrap_or(0);
    (0..slots).map(|i| SimSlot { state: get(0, i), operator: get(1, i), numeric: get(2, i), country: get(3, i).to_uppercase() }).filter(|s| !s.state.is_empty()).collect()
}

/// Accounts from `dumpsys account`, without repeats.
pub fn parse_accounts(text: &str) -> Vec<Account> {
    let mut out: Vec<Account> = Vec::new();
    for line in text.lines() {
        let Some(inner) = line.trim().strip_prefix("Account {").and_then(|r| r.strip_suffix('}')) else {
            continue;
        };
        let Some((name_part, type_part)) = inner.rsplit_once(", type=") else {
            continue;
        };
        let Some(name) = name_part.strip_prefix("name=") else {
            continue;
        };
        let account = Account { name: name.to_string(), kind: type_part.to_string() };
        if !account.name.is_empty() && !out.contains(&account) {
            out.push(account);
        }
    }
    out
}

pub fn parse_private(text: &str) -> PrivateInfo {
    let sections: HashMap<String, String> = parsers::split_sections(text);
    let get = |k: &str| sections.get(k).map(String::as_str).unwrap_or("");
    let parcel = |k: &str| parse_parcel_string(get(k)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

    let sims = parse_sim_props(get("SIM"));
    let codes: Vec<String> = sims.iter().map(|s| s.numeric.clone()).filter(|c| c.len() >= 5).collect();

    let imei1 = parcel("IMEI0").filter(|v| is_imei(v, &codes)).or_else(|| parcel("IMEI").filter(|v| is_imei(v, &codes)));
    let imei2 = parcel("IMEI1").filter(|v| is_imei(v, &codes) && Some(v) != imei1.as_ref());
    let phone_number = parcel("LINE1").filter(|v| {
        let digits = v.trim_start_matches('+');
        (8..=15).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit())
    });
    PrivateInfo {
        imei: [imei1, imei2],
        serial: clean_setting(get("SERIAL")),
        android_id: clean_setting(get("ANDROIDID")).filter(|v| (8..=16).contains(&v.len()) && v.chars().all(|c| c.is_ascii_hexdigit())),
        device_name: clean_setting(get("DEVNAME")),
        bluetooth_name: clean_setting(get("BTNAME")),
        bluetooth_mac: valid_mac(get("BTMAC")),
        imsi: parcel("IMSI").filter(|v| is_imsi(v, &codes)),
        iccid: parcel("ICCID").filter(|v| is_iccid(v)),
        phone_number,
        sims,
        accounts: parse_accounts(get("ACCOUNTS")),
    }
}

// ---------------------------------------------------------------------------
// Public IP
// ---------------------------------------------------------------------------

/// Service that returns the IP as plain text. It is only queried when the
/// button is pressed.
pub const PUBLIC_IP_SERVICE: &str = "https://api.ipify.org";

/// It runs **on the phone**: that way the IP is that of its network (Wi-Fi or
/// mobile data), which may not be the PC's.
pub fn public_ip_script() -> String {
    format!("command -v curl >/dev/null || {{ echo 'NO_CURL'; exit 0; }}; curl -s -m 8 {PUBLIC_IP_SERVICE}")
}

pub fn parse_public_ip(text: &str) -> std::result::Result<IpAddr, String> {
    let t = text.trim();
    if t.contains("NO_CURL") {
        return Err(tr("The phone does not have curl to query the IP").into());
    }
    if t.is_empty() {
        return Err(tr("No response: the phone has no Internet connection").into());
    }
    t.parse::<IpAddr>().map_err(|_| tr("Invalid response from the service").to_string())
}
