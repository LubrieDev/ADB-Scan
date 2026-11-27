//! Radio around the phone, read-only and without root:
//!
//! - **Nearby Wi-Fi networks** (`cmd wifi start-scan`, then `cmd wifi
//!   list-scan-results`, Android 11+): name, band, channel and signal, and how
//!   crowded each channel is, to choose the router's channel. A scan is what
//!   the phone does by itself every few minutes; Android may throttle it, and
//!   then the last results are shown with their age. Access point addresses
//!   (BSSID) are read but never shown.
//! - **Mobile signal** (`dumpsys telephony.registry`): technology (4G, 5G),
//!   band, strength (RSRP, RSRQ, SNR) and bandwidth of the SIM in service.
//!
//! Formats from Android 16 (`WifiShellCommand`, `SignalStrength`,
//! `ServiceState`), checked on a REDMAGIC NX809J.

use crate::i18n::tr;
use crate::t;

/// Read-only (a scan changes nothing). `sleep`: the scan takes 2–4 s.
pub const WIFI_SCRIPT: &str = "cmd wifi start-scan >/dev/null 2>&1; sleep 4; cmd wifi list-scan-results 2>&1";

/// Read-only.
pub const MOBILE_SCRIPT: &str = "dumpsys telephony.registry 2>&1 | grep -E '^ +(mServiceState|mSignalStrength)='";

/// A network heard by the phone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Network {
    /// Empty: hidden network.
    pub ssid: String,
    pub bssid: String,
    pub freq_mhz: u32,
    pub rssi: i32,
    /// Seconds since it was heard (`None`: more than 1000 s).
    pub age_s: Option<f64>,
    /// `WPA2`, `WPA3`, `Open`…
    pub security: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Band {
    G2,
    G5,
    G6,
}

impl Band {
    pub fn label(self) -> &'static str {
        match self {
            Band::G2 => "2.4 GHz",
            Band::G5 => "5 GHz",
            Band::G6 => "6 GHz",
        }
    }
}

impl Network {
    pub fn band(&self) -> Option<Band> {
        match self.freq_mhz {
            2400..=2500 => Some(Band::G2),
            4900..=5900 => Some(Band::G5),
            5925..=7125 => Some(Band::G6),
            _ => None,
        }
    }

    pub fn channel(&self) -> Option<u32> {
        let f = self.freq_mhz;
        Some(match self.band()? {
            Band::G2 if f == 2484 => 14,
            Band::G2 => (f - 2407) / 5,
            Band::G5 => (f - 5000) / 5,
            Band::G6 => (f - 5950) / 5,
        })
    }

    /// 0 (none) to 4 (excellent), like Android's bars.
    pub fn bars(&self) -> u8 {
        match self.rssi {
            r if r >= -55 => 4,
            r if r >= -67 => 3,
            r if r >= -75 => 2,
            r if r >= -85 => 1,
            _ => 0,
        }
    }
}

/// `[WPA2-PSK-CCMP-128][RSN-PSK-CCMP-128][ESS][WPS]` → `WPA2`.
fn security_of(flags: &str) -> String {
    let f = flags.to_uppercase();
    let mut out: Vec<&str> = Vec::new();
    if f.contains("SAE") {
        out.push("WPA3");
    }
    if f.contains("WPA2") || (f.contains("RSN") && f.contains("PSK")) {
        out.push("WPA2");
    } else if f.contains("WPA-") {
        out.push("WPA");
    }
    if f.contains("EAP") {
        out.push("Enterprise");
    }
    if f.contains("OWE") {
        out.push("OWE");
    }
    if f.contains("WEP") {
        out.push("WEP");
    }
    if out.is_empty() {
        return crate::i18n::trc("wifi", "Open").to_string();
    }
    out.join("/")
}

/// `cmd wifi list-scan-results`:
///
/// ```text
///     BSSID              Frequency      RSSI           Age(sec)     SSID                                 Flags
///   aa:bb:cc:dd:ee:ff       2462    -53(0:-54/1:-59)   >1000.0    Home network                      [WPA2-PSK-CCMP-128][ESS]
/// ```
pub fn parse_scan(text: &str) -> Option<Vec<Network>> {
    if !text.contains("BSSID") {
        return None;
    }
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        let mut it = l.split_whitespace();
        let (Some(bssid), Some(freq), Some(rssi), Some(age)) = (it.next(), it.next(), it.next(), it.next()) else { continue };
        if bssid.len() != 17 || bssid.matches(':').count() != 5 {
            continue;
        }
        let Ok(freq_mhz) = freq.parse::<u32>() else { continue };
        let Ok(rssi) = rssi.split('(').next().unwrap_or(rssi).parse::<i32>() else { continue };
        let age_s = age.parse::<f64>().ok();
        // The rest: SSID (may have spaces, may be empty) and the flags.
        let after_age = l.find(age).map(|i| &l[i + age.len()..]).unwrap_or("");
        let (ssid, flags) = match after_age.find('[') {
            Some(i) => (after_age[..i].trim(), &after_age[i..]),
            None => (after_age.trim(), ""),
        };
        out.push(Network { ssid: ssid.to_string(), bssid: bssid.to_lowercase(), freq_mhz, rssi, age_s, security: security_of(flags) });
    }
    out.sort_by_key(|n| std::cmp::Reverse(n.rssi));
    Some(out)
}

/// How crowded a channel is: networks heard on it or overlapping it, weighed
/// by how loud they are.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelLoad {
    pub band: Band,
    pub channel: u32,
    pub networks: usize,
    /// 0 = free; one strong network next to the phone is about 1.
    pub load: f64,
}

/// Weight of a network for congestion: strong ones count more.
fn weight(rssi: i32) -> f64 {
    (((rssi + 95) as f64) / 40.0).clamp(0.05, 1.5)
}

/// Load of the channels routers should use: 1, 6 and 11 at 2.4 GHz (they do
/// not overlap) and every channel heard at 5 and 6 GHz. In 2.4 GHz a network
/// overlaps channels up to 4 away.
pub fn channel_loads(nets: &[Network]) -> Vec<ChannelLoad> {
    let mut out: Vec<ChannelLoad> = Vec::new();
    for ch in [1u32, 6, 11] {
        let near: Vec<&Network> = nets.iter().filter(|n| n.band() == Some(Band::G2) && n.channel().is_some_and(|c| c.abs_diff(ch) <= 4)).collect();
        let load = near.iter().map(|n| weight(n.rssi) * (1.0 - n.channel().unwrap_or(ch).abs_diff(ch) as f64 * 0.2)).sum();
        out.push(ChannelLoad { band: Band::G2, channel: ch, networks: near.len(), load });
    }
    let mut wide: Vec<(Band, u32)> = nets.iter().filter_map(|n| Some((n.band()?, n.channel()?))).filter(|(b, _)| *b != Band::G2).collect();
    wide.sort();
    wide.dedup();
    for (band, ch) in wide {
        let on: Vec<&Network> = nets.iter().filter(|n| n.band() == Some(band) && n.channel() == Some(ch)).collect();
        out.push(ChannelLoad { band, channel: ch, networks: on.len(), load: on.iter().map(|n| weight(n.rssi)).sum() });
    }
    out
}

/// The least crowded 2.4 GHz channel among 1, 6 and 11.
pub fn best_24(nets: &[Network]) -> Option<u32> {
    let loads = channel_loads(nets);
    loads.iter().filter(|l| l.band == Band::G2).min_by(|a, b| a.load.total_cmp(&b.load)).map(|l| l.channel)
}

// ---------------------------------------------------------------------------
// Mobile signal
// ---------------------------------------------------------------------------

/// The mobile signal of the SIM in service.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MobileSignal {
    /// `4G`, `5G`, `3G`…
    pub generation: String,
    /// Radio technology as Android names it (`LTE`, `NR`…).
    pub technology: String,
    pub band: Option<u32>,
    /// dBm.
    pub rsrp: Option<i32>,
    /// dB.
    pub rsrq: Option<i32>,
    /// dB.
    pub snr: Option<i32>,
    /// Android's 0–4 bars.
    pub level: Option<u8>,
    pub bandwidth_mhz: Option<u32>,
    pub carrier_aggregation: bool,
    /// 5G next to 4G (non-standalone) is shown as `5G` with `nr_attached`.
    pub nr_attached: bool,
}

const UNAVAILABLE: i64 = 2147483647;

/// Value of `key=` (or `key = `) in a long line, if it is not “unavailable”.
fn value(text: &str, key: &str) -> Option<i64> {
    let mut rest = text;
    while let Some(i) = rest.find(key) {
        let before_ok = i == 0 || !rest.as_bytes()[i - 1].is_ascii_alphanumeric();
        let after = rest[i + key.len()..].trim_start();
        rest = &rest[i + key.len()..];
        if !before_ok {
            continue;
        }
        let Some(v) = after.strip_prefix('=') else { continue };
        let v = v.trim_start();
        let end = v.find(|c: char| !(c.is_ascii_digit() || c == '-')).unwrap_or(v.len());
        let n: i64 = v[..end].parse().ok()?;
        return (n != UNAVAILABLE && n != -UNAVAILABLE).then_some(n);
    }
    None
}

/// `getRilDataRadioTechnology=14(LTE)` → (14, LTE).
fn radio_tech(state: &str) -> Option<(i64, String)> {
    let (_, rest) = state.split_once("getRilDataRadioTechnology=")?;
    let (num, rest) = rest.split_once('(')?;
    let name = rest.split(')').next()?;
    Some((num.trim().parse().ok()?, name.to_string()))
}

fn generation(tech: i64) -> &'static str {
    match tech {
        20 => "5G",
        13 | 14 | 19 => "4G",
        3 | 8 | 9 | 10 | 11 | 15 | 17 => "3G",
        1 | 2 | 4 | 5 | 6 | 7 | 12 | 16 => "2G",
        _ => "",
    }
}

/// `mServiceState=…` and `mSignalStrength=…` lines, one pair per SIM slot.
pub fn parse_mobile(text: &str) -> Option<MobileSignal> {
    let states: Vec<&str> = text.lines().filter_map(|l| l.trim().strip_prefix("mServiceState=")).collect();
    let signals: Vec<&str> = text.lines().filter_map(|l| l.trim().strip_prefix("mSignalStrength=")).collect();
    // The slot whose data is in service.
    let slot = states.iter().position(|s| s.contains("mDataRegState=0(IN_SERVICE)")).or_else(|| states.iter().position(|s| s.contains("mVoiceRegState=0(IN_SERVICE)")))?;
    let state = states[slot];
    let signal = signals.get(slot).copied().unwrap_or("");
    let (tech, technology) = radio_tech(state).unwrap_or((0, String::new()));
    let lte = signal.split_once("mLte=").map(|(_, r)| r.split(",mNr=").next().unwrap_or(r)).unwrap_or("");
    let nr = signal.split_once("mNr=").map(|(_, r)| r).unwrap_or("");
    let nr_rsrp = value(nr, "ssRsrp");
    let nr_attached = state.contains("nrState=CONNECTED") || nr_rsrp.is_some();
    let mut m = MobileSignal { technology, nr_attached, ..Default::default() };
    if tech == 20 {
        m.rsrp = nr_rsrp.map(|v| v as i32);
        m.rsrq = value(nr, "ssRsrq").map(|v| v as i32);
        m.snr = value(nr, "ssSinr").map(|v| v as i32);
        m.level = value(nr, "level").map(|v| v as u8);
    } else {
        m.rsrp = value(lte, "rsrp").map(|v| v as i32);
        m.rsrq = value(lte, "rsrq").map(|v| v as i32);
        m.snr = value(lte, "rssnr").map(|v| v as i32);
        m.level = value(lte, "level").map(|v| v as u8);
        m.band = value(lte, "lteBand").filter(|b| *b > 0).map(|b| b as u32);
    }
    m.generation = if nr_attached && tech != 20 && generation(tech) == "4G" { "5G".into() } else { generation(tech).into() };
    // `mCellBandwidths=[20000]` (kHz), added up with carrier aggregation.
    if let Some(list) = state.split_once("mCellBandwidths=[").and_then(|(_, r)| r.split(']').next()) {
        let total: u32 = list.split(',').filter_map(|v| v.trim().parse::<u32>().ok()).sum();
        m.bandwidth_mhz = (total > 0).then_some(total / 1000);
    }
    m.carrier_aggregation = state.contains("isUsingCarrierAggregation=true");
    Some(m)
}

impl MobileSignal {
    /// RSRP in words (3GPP ranges as carriers use them).
    pub fn quality(&self) -> &'static str {
        match self.rsrp {
            Some(r) if r >= -80 => tr("Excellent"),
            Some(r) if r >= -90 => tr("Good"),
            Some(r) if r >= -100 => tr("Fair"),
            Some(r) if r >= -110 => tr("Weak"),
            Some(_) => tr("Very weak"),
            None => tr("Unknown"),
        }
    }

    /// `4G (LTE) · band 66 · 20 MHz`
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        let tech = match (self.generation.as_str(), self.technology.as_str()) {
            ("", t) => t.to_string(),
            (g, "") => g.to_string(),
            (g, t) if self.nr_attached && t == "LTE" => format!("{g} (LTE + NR)"),
            (g, t) => format!("{g} ({t})"),
        };
        if !tech.is_empty() {
            parts.push(tech);
        }
        if let Some(b) = self.band {
            parts.push(t!("band {}", b));
        }
        if let Some(w) = self.bandwidth_mhz {
            parts.push(format!("{w} MHz"));
        }
        if self.carrier_aggregation {
            parts.push(tr("carrier aggregation").into());
        }
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real output with sample names and addresses.
    const SCAN: &str = "    BSSID              Frequency      RSSI           Age(sec)     SSID                                 Flags
  02:00:00:00:00:01       2462    -82(0:-87/1:-84)   >1000.0    Vecino 1                          [WPA2-PSK+FT/PSK-CCMP-128][RSN-PSK+FT/PSK-CCMP-128][ESS][WPS]
  02:00:00:00:00:02       2462    -53(0:-54/1:-59)   12.5       Casa                              [WPA2-PSK-CCMP-128][RSN-PSK-CCMP-128][ESS]
  02:00:00:00:00:03       2412    -80(0:-82/1:-84)   >1000.0    Vecino 2                          [WPA2-PSK-CCMP-128][RSN-PSK-CCMP-128][ESS][WPS]
  02:00:00:00:00:04       5180    -70(0:-70/1:-72)   3.0                                          [RSN-SAE-CCMP-128][ESS]
  02:00:00:00:00:05       2437    -90(0:-90/1:-91)   3.0        Cafe libre                        [ESS]
";

    #[test]
    fn scan() {
        let n = parse_scan(SCAN).unwrap();
        assert_eq!(n.len(), 5);
        assert_eq!((n[0].ssid.as_str(), n[0].rssi, n[0].channel(), n[0].age_s), ("Casa", -53, Some(11), Some(12.5)));
        assert_eq!(n[0].security, "WPA2");
        let hidden = n.iter().find(|x| x.freq_mhz == 5180).unwrap();
        assert_eq!((hidden.ssid.as_str(), hidden.channel(), hidden.band(), hidden.security.as_str()), ("", Some(36), Some(Band::G5), "WPA3"));
        assert_eq!(n.iter().find(|x| x.ssid == "Vecino 1").unwrap().age_s, None);
        assert_eq!(n.iter().find(|x| x.ssid == "Cafe libre").unwrap().security, "Open");
        // 11 has the strong network, 1 a weak one, 6 only a very weak one.
        assert_eq!(best_24(&n), Some(6));
        assert!(parse_scan("Unknown command").is_none());
        assert_eq!(parse_scan("    BSSID   Frequency  RSSI  Age(sec)  SSID  Flags\n"), Some(vec![]));
    }

    #[test]
    fn mobile() {
        // As Android 16 prints it (two slots, the second without service).
        let text = "    mServiceState={mVoiceRegState=0(IN_SERVICE), mDataRegState=0(IN_SERVICE), mChannelNumber=67036, duplexMode()=1, mCellBandwidths=[20000], mOperatorAlphaLong=X, isManualNetworkSelection=false(automatic), getRilVoiceRadioTechnology=14(LTE), getRilDataRadioTechnology=14(LTE), isUsingCarrierAggregation=false, nrState=****}
    mSignalStrength=SignalStrength:{mCdma=CellSignalStrengthCdma: cdmaDbm=2147483647 level=0,mLte=CellSignalStrengthLte: rssi=-77 rsrp=-110 rsrq=-12 rssnr=0 cqiTableIndex=2147483647 cqi=2147483647 ta=2147483647 level=2 parametersUseForLevel=0 operatorLevel=3 lteBand=66,mNr=CellSignalStrengthNr:{ csiRsrp = 2147483647 csiRsrq = 2147483647 ssRsrp = 2147483647 ssRsrq = 2147483647 ssSinr = 2147483647 level = 0 }}
    mServiceState={mVoiceRegState=1(OUT_OF_SERVICE), mDataRegState=1(OUT_OF_SERVICE), mCellBandwidths=[], getRilDataRadioTechnology=0(Unknown)}
    mSignalStrength=SignalStrength:{mLte=CellSignalStrengthLte: rssi=2147483647 rsrp=2147483647 level=0 lteBand=0,mNr=CellSignalStrengthNr:{ ssRsrp = 2147483647 level = 0 }}
";
        let m = parse_mobile(text).unwrap();
        assert_eq!((m.generation.as_str(), m.technology.as_str(), m.band), ("4G", "LTE", Some(66)));
        assert_eq!((m.rsrp, m.rsrq, m.snr, m.level, m.bandwidth_mhz), (Some(-110), Some(-12), Some(0), Some(2), Some(20)));
        assert!(!m.nr_attached);
        assert_eq!(m.quality(), "Weak");
        assert_eq!(m.summary(), "4G (LTE) · band 66 · 20 MHz");
        assert!(parse_mobile("    mServiceState={mVoiceRegState=1(OUT_OF_SERVICE), mDataRegState=1(OUT_OF_SERVICE)}\n").is_none());
        // `rsrp` must not match `ssRsrp`; `level` inside the NR block.
        assert_eq!(value("ssRsrp = -95 level = 3", "level"), Some(3));
        assert_eq!(value("csiRsrp = 2147483647", "Rsrp"), None);
    }
}
