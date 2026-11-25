//! Private mode: **display-only** hiding of persistent identifiers (serial
//! numbers, IP and MAC addresses, Wi-Fi network names).
//!
//! The [`Redactor`] receives text already prepared for display and returns it
//! with that data replaced by [`MASK`]. It is never applied to what is stored,
//! exported, copied or sent to ADB: the data does not change.
//!
//! - Serial numbers and Wi-Fi networks: looked for as literal text (learned
//!   when seen: connected devices, history, telemetry).
//! - IPs and MACs: recognised by their shape with a dedicated parser, so as not
//!   to confuse times (“23:12:24”), dates, versions (“6.12.38”) or logcat
//!   symbols (“android::base”).
//! - Nicknames: not looked for as text (a short nickname would mask common
//!   words); the interface shows the marketing name instead.

use std::borrow::Cow;

pub const MASK: &str = "••••••";

/// Minimum length of a learned literal: a shorter one would mask single letters
/// in any text.
const MIN_LITERAL: usize = 4;

#[derive(Debug, Clone, Default)]
pub struct Redactor {
    enabled: bool,
    /// Sorted from longest to shortest: “ABC123:5555” before “ABC123”.
    literals: Vec<String>,
}

impl Redactor {
    pub fn new(enabled: bool) -> Self {
        Self { enabled, literals: Vec::new() }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    /// Remembers a sensitive value (serial number, Wi-Fi network) to hide it
    /// wherever it appears. Returns `true` if it is new.
    pub fn learn(&mut self, value: &str) -> bool {
        let v = value.trim();
        if v.chars().count() < MIN_LITERAL || self.literals.iter().any(|l| l == v) {
            return false;
        }
        self.literals.push(v.to_string());
        self.literals.sort_by_key(|l| std::cmp::Reverse(l.len()));
        true
    }

    /// Value that is entirely sensitive data (the “Serial number” row, the
    /// network name): hidden completely, whatever its shape.
    pub fn value<'a>(&self, value: &'a str) -> Cow<'a, str> {
        if self.enabled && !value.trim().is_empty() {
            Cow::Borrowed(MASK)
        } else {
            Cow::Borrowed(value)
        }
    }

    /// Free text: hides the learned literals, IPs and MACs.
    pub fn text<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if !self.enabled || text.is_empty() {
            return Cow::Borrowed(text);
        }
        let mut out: Cow<'a, str> = Cow::Borrowed(text);
        for lit in &self.literals {
            if out.contains(lit.as_str()) {
                out = Cow::Owned(out.replace(lit.as_str(), MASK));
            }
        }
        match mask_addresses(&out) {
            Some(masked) => Cow::Owned(masked),
            None => out,
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_addr_char(c: char) -> bool {
    c.is_ascii_hexdigit() || matches!(c, ':' | '.' | '/')
}

/// Hides IPs and MACs. `None` if there were none (without copying the text).
fn mask_addresses(text: &str) -> Option<String> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = String::new();
    let mut last = 0usize;
    let mut changed = false;
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        let left_ok = i == 0 || !is_word(chars[i - 1].1);
        if !is_addr_char(c) || !left_ok {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < chars.len() && is_addr_char(chars[j].1) {
            j += 1;
        }
        // Final punctuation (“… is 10.0.0.1.”): not part of the address.
        let mut k = j;
        while k > i && matches!(chars[k - 1].1, '.' | ':' | '/') {
            k -= 1;
        }
        let right_ok = j == chars.len() || !is_word(chars[j].1) || chars[j].1 == '%';
        let end = chars.get(k).map(|(b, _)| *b).unwrap_or(text.len());
        let token = &text[start..end];
        if right_ok && k > i && is_address(token) {
            out.push_str(&text[last..start]);
            out.push_str(MASK);
            last = end;
            changed = true;
        }
        i = j.max(i + 1);
    }
    if !changed {
        return None;
    }
    out.push_str(&text[last..]);
    Some(out)
}

fn is_address(token: &str) -> bool {
    // Network prefix (“/24”) kept apart.
    let (body, prefix) = match token.split_once('/') {
        Some((b, p)) => (b, Some(p)),
        None => (token, None),
    };
    if prefix.is_some_and(|p| p.is_empty() || p.len() > 3 || !p.chars().all(|c| c.is_ascii_digit())) {
        return false;
    }
    is_ipv4_with_port(body) || is_mac(body) || is_ipv6(body)
}

fn is_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()) && p.parse::<u16>().is_ok_and(|n| n <= 255))
}

fn is_ipv4_with_port(s: &str) -> bool {
    match s.split_once(':') {
        Some((host, port)) => is_ipv4(host) && !port.is_empty() && port.len() <= 5 && port.chars().all(|c| c.is_ascii_digit()),
        None => is_ipv4(s),
    }
}

fn is_mac(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() == 6 && parts.iter().all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// IPv6 with “::” or with its 8 groups (times have 2 or 3 groups and no “::”).
/// The last group may be an IPv4 (`::ffff:1.2.3.4`).
fn is_ipv6(s: &str) -> bool {
    if !s.contains(':') || s.matches("::").count() > 1 {
        return false;
    }
    let groups: Vec<&str> = s.split(':').collect();
    let compressed = s.contains("::");
    let n = groups.len();
    if !(compressed || n == 8) || !(3..=9).contains(&n) {
        return false;
    }
    let mut hex_groups = 0;
    let mut hex_digits = 0;
    for (idx, g) in groups.iter().enumerate() {
        if g.is_empty() {
            continue;
        }
        if idx == n - 1 && g.contains('.') {
            if !is_ipv4(g) {
                return false;
            }
            continue;
        }
        if g.len() > 4 || !g.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
        hex_groups += 1;
        hex_digits += g.len();
    }
    // “::”, “::1” or “a::b” (C++ symbols in logcat) are not enough: at least
    // two groups and four digits, like the shortest real one (“fe80::1”).
    (hex_groups >= 2 && hex_digits >= 4) || (compressed && groups.last().is_some_and(|g| g.contains('.')))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Redactor {
        let mut r = Redactor::new(true);
        r.learn("A1B2C3D4E5F6");
        r.learn("Hogar");
        r
    }

    #[test]
    fn disabled_changes_nothing() {
        let r = Redactor::new(false);
        let t = "A1B2C3D4E5F6 on 192.168.1.228";
        assert!(matches!(r.text(t), Cow::Borrowed(_)));
        assert_eq!(r.value("A1B2C3D4E5F6"), "A1B2C3D4E5F6");
    }

    #[test]
    fn serials_ssids_and_addresses_are_masked() {
        let r = on();
        assert_eq!(r.text("NX809J · A1B2C3D4E5F6"), format!("NX809J · {MASK}"));
        assert_eq!(r.text("A1B2C3D4E5F6-20261122-2312.log"), format!("{MASK}-20261122-2312.log"));
        assert_eq!(r.text("Wi-Fi “Hogar” · -51 dBm"), format!("Wi-Fi “{MASK}” · -51 dBm"));
        assert_eq!(r.text("192.168.1.228"), MASK);
        assert_eq!(r.text("Connected to 192.168.1.228:5555."), format!("Connected to {MASK}."));
        assert_eq!(r.text("default via 192.168.1.1 dev wlan0"), format!("default via {MASK} dev wlan0"));
        assert_eq!(r.text("192.168.1.0/24 dev wlan0"), format!("{MASK} dev wlan0"));
        assert_eq!(r.text("inet6 fe80::a2b1:c2ff:fe3d:4e5f/64 scope link"), format!("inet6 {MASK} scope link"));
        assert_eq!(r.text("2001:db8:85a3:0:0:8a2e:370:7334"), MASK);
        assert_eq!(r.text("::ffff:10.0.0.1"), MASK);
        assert_eq!(r.text("MAC dc:f0:12:34:ac:56"), format!("MAC {MASK}"));
        assert_eq!(r.text("fe80::1%wlan0"), format!("{MASK}%wlan0"));
        assert_eq!(r.value("Hogar"), MASK);
    }

    #[test]
    fn times_dates_versions_and_symbols_are_left_alone() {
        let r = on();
        for t in [
            "22/11/2026 23:12:24 · 3 min 36 s",
            "12:05",
            "Linux 6.12.38-android16-5 #1 SMP",
            "REDMAGICOS11.5.9_GB",
            "version 24.45.31.1.2",
            "android::base::LogMessage",
            "E/libc: a::b",
            "deadbeef",
            "Android 16 (API 36) · 4.74 GHz · 1216 × 2688",
            "0x00000000: 00000000",
        ] {
            assert_eq!(r.text(t), t, "{t}");
        }
    }

    #[test]
    fn short_literals_are_not_learned() {
        let mut r = Redactor::new(true);
        assert!(!r.learn("ab"));
        assert!(!r.learn("   "));
        assert!(r.learn("casa"));
        assert!(!r.learn("casa"), "no repeats");
        assert_eq!(r.text("ab cd"), "ab cd");
    }
}
