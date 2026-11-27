//! Wi-Fi networks saved on the phone (`cmd wifi list-networks`, Android 11+),
//! read-only, and forgetting one (`cmd wifi forget-network <id>`), which
//! **changes the phone**: it will not join that network again until its
//! password is entered on the phone. Passwords are never readable without
//! root.
//!
//! Format from Android 16's `WifiShellCommand`: a header, then
//! `%-12d %-32s %-4s` (id, SSID, security type). Android 16 prints one line
//! per security type of a network (`open` and `owe^`, `wpa2-psk` and
//! `wpa3-sae^`), all with the same id: they are joined here.

/// A saved network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedNetwork {
    pub id: u32,
    pub ssid: String,
    /// `wpa2-psk`, `open`, `wpa2-psk/wpa3-sae^`… (as Android names them;
    /// `^`: added by Android itself for an upgrade).
    pub security: String,
}

impl SavedNetwork {
    /// The security in words: `WPA2 / WPA3`, `Open`…
    pub fn security_label(&self) -> String {
        let mut names: Vec<&str> = Vec::new();
        for part in self.security.split('/') {
            let name = match part.trim_end_matches('^') {
                "open" => crate::i18n::trc("wifi", "Open"),
                "owe" => crate::i18n::tr("Enhanced open (OWE)"),
                "wep" => "WEP",
                "wpa2-psk" | "psk" | "wpa2" => "WPA2",
                "wpa3-sae" | "sae" | "wpa3" => "WPA3",
                p if p.contains("eap") || p.contains("enterprise") => "WPA-Enterprise",
                "" => continue,
                p => p,
            };
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names.join(" / ")
    }
}

pub fn parse(text: &str) -> Option<Vec<SavedNetwork>> {
    let t = text.trim();
    if t.starts_with("No networks") {
        return Some(Vec::new());
    }
    if !t.lines().next()?.starts_with("Network Id") {
        return None;
    }
    let mut out = Vec::new();
    for line in t.lines().skip(1) {
        let line = line.trim_end();
        let Some((id, rest)) = line.split_once(' ') else { continue };
        let Ok(id) = id.trim().parse::<u32>() else { continue };
        let rest = rest.trim_start();
        // The security type is the last word; the SSID may have spaces.
        let (ssid, security) = match rest.rsplit_once(' ') {
            Some((s, sec)) => (s.trim_end(), sec),
            None => (rest, ""),
        };
        match out.iter_mut().find(|n: &&mut SavedNetwork| n.id == id) {
            Some(n) if !security.is_empty() && !n.security.split('/').any(|s| s == security) => {
                if !n.security.is_empty() {
                    n.security.push('/');
                }
                n.security.push_str(security);
            }
            Some(_) => {}
            None => out.push(SavedNetwork { id, ssid: ssid.to_string(), security: security.to_string() }),
        }
    }
    Some(out)
}

/// Arguments to forget a network.
pub fn forget_command(id: u32) -> Vec<String> {
    vec!["cmd".into(), "wifi".into(), "forget-network".into(), id.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_networks() {
        let out = "Network Id      SSID                         Security type\n\
0            Home                             wpa2-psk\n\
3            Coffee shop guest                open\n\
7            Office 5G                        wpa2-psk/sae^\n";
        let n = parse(out).unwrap();
        assert_eq!(n.len(), 3);
        assert_eq!(n[1], SavedNetwork { id: 3, ssid: "Coffee shop guest".into(), security: "open".into() });
        assert_eq!(n[2].security, "wpa2-psk/sae^");
        assert_eq!(parse("No networks\n"), Some(vec![]));
        assert_eq!(parse("Unknown command: list-networks"), None);
        // Android 16: one line per security type.
        let out = "Network Id      SSID                         Security type\n\
8            Casa                             wpa2-psk\n\
8            Casa                             wpa3-sae^\n\
9            Cafe libre                       open\n\
9            Cafe libre                       owe^\n";
        let n = parse(out).unwrap();
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].security, "wpa2-psk/wpa3-sae^");
        assert_eq!(n[0].security_label(), "WPA2 / WPA3");
        // Android 11 (moto e20): plain names.
        let old = parse("Network Id      SSID                         Security type\n0            home                       wpa2\n1            cafe                      open\n").unwrap();
        assert_eq!(old.len(), 2);
        assert_eq!(old[0].security_label(), "WPA2");
        assert_eq!(n[1], SavedNetwork { id: 9, ssid: "Cafe libre".into(), security: "open/owe^".into() });
        assert_eq!(forget_command(7).join(" "), "cmd wifi forget-network 7");
    }
}
