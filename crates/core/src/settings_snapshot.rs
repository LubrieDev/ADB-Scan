//! Snapshots of the phone's settings (`settings list global|system|secure`),
//! kept on the computer, to see what changed since then: after a system
//! update, after installing an app, after lending the phone. Android itself
//! keeps no dated history of its settings.
//!
//! Reading is read-only. Restoring a key (`settings put`, or `settings
//! delete` if it did not exist) **changes the phone**, one key at a time and
//! after confirming; keys that would cut ADB or undo the phone's setup are
//! never restored. Identifiers (`android_id`, the Bluetooth address and
//! name) are not stored.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::tr;

/// Snapshots kept per phone (the oldest are dropped).
pub const MAX_SNAPSHOTS: usize = 12;

pub const NAMESPACES: [&str; 3] = ["global", "system", "secure"];

/// Read-only.
pub const SCRIPT: &str = "echo @@@SDK@@@; getprop ro.build.version.sdk
echo @@@GLOBAL@@@; settings list global 2>&1
echo @@@SYSTEM@@@; settings list system 2>&1
echo @@@SECURE@@@; settings list secure 2>&1
echo @@@END@@@
";

/// Keys that identify the phone or its owner: never stored.
fn is_identifier(key: &str) -> bool {
    matches!(key, "android_id" | "bluetooth_address" | "bluetooth_name" | "device_name" | "lock_screen_owner_info" | "advertising_id")
}

/// Counters that change on their own and say nothing.
fn is_noise(key: &str) -> bool {
    key == "boot_count" || key.contains("timestamp")
}

/// Keys never restored: they would cut ADB, the phone's setup or its lock.
pub fn is_protected(namespace: &str, key: &str) -> bool {
    matches!(key, "adb_enabled" | "adb_wifi_enabled" | "development_settings_enabled" | "device_provisioned" | "user_setup_complete" | "secure_frp_mode" | "boot_count")
        || key.starts_with("lock")
        || (namespace == "secure" && key.starts_with("enabled_accessibility"))
}

/// A key name `settings` accepts (letters, digits, `_`, `.`, `-`).
pub fn is_valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 200 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Unix time, seconds.
    pub taken_at: f64,
    pub sdk: Option<i64>,
    /// Namespace → key → value.
    pub values: BTreeMap<String, BTreeMap<String, String>>,
}

impl Snapshot {
    pub fn key_count(&self) -> usize {
        self.values.values().map(BTreeMap::len).sum()
    }
}

pub fn parse(text: &str) -> Snapshot {
    let s = crate::parsers::split_sections(text);
    let mut values = BTreeMap::new();
    for ns in NAMESPACES {
        let map = crate::controls::parse_settings_list(s.get(&ns.to_uppercase()).map(String::as_str).unwrap_or(""));
        let map: BTreeMap<String, String> = map.into_iter().filter(|(k, _)| !is_identifier(k)).collect();
        if !map.is_empty() {
            values.insert(ns.to_string(), map);
        }
    }
    Snapshot { taken_at: crate::models::now_secs(), sdk: s.get("SDK").and_then(|v| v.trim().parse().ok()), values }
}

/// A key that differs between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub namespace: String,
    pub key: String,
    /// `None`: the key did not exist.
    pub before: Option<String>,
    pub after: Option<String>,
}

impl Change {
    pub fn restorable(&self) -> bool {
        !is_protected(&self.namespace, &self.key) && is_valid_key(&self.key)
    }

    /// Arguments that put the old value back.
    pub fn restore_command(&self) -> Option<Vec<String>> {
        if !self.restorable() {
            return None;
        }
        let base = |verb: &str| vec!["settings".to_string(), verb.into(), self.namespace.clone(), self.key.clone()];
        Some(match &self.before {
            Some(v) => {
                let mut c = base("put");
                c.push(v.clone());
                c
            }
            None => base("delete"),
        })
    }
}

/// What changed from `old` to `new`, by namespace and key. Counters that
/// change by themselves are left out; a namespace missing from either side
/// (not read) is not compared.
pub fn diff(old: &Snapshot, new: &Snapshot) -> Vec<Change> {
    let mut out = Vec::new();
    for ns in NAMESPACES {
        let (Some(a), Some(b)) = (old.values.get(ns), new.values.get(ns)) else { continue };
        let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
        for key in keys {
            if is_noise(key) {
                continue;
            }
            let (before, after) = (a.get(key), b.get(key));
            if before != after {
                out.push(Change { namespace: ns.to_string(), key: key.clone(), before: before.cloned(), after: after.cloned() });
            }
        }
    }
    out
}

/// File of a phone's snapshots (one per phone, in the app's data folder).
fn file(serial: &str) -> PathBuf {
    let safe: String = serial.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    crate::paths::data_dir().join("settings-snapshots").join(format!("{safe}.json"))
}

/// Saved snapshots, oldest first.
pub fn load(serial: &str) -> Vec<Snapshot> {
    std::fs::read_to_string(file(serial)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Adds a snapshot and keeps the newest [`MAX_SNAPSHOTS`].
pub fn save(serial: &str, snapshot: Snapshot) -> std::io::Result<Vec<Snapshot>> {
    let mut all = load(serial);
    all.push(snapshot);
    let excess = all.len().saturating_sub(MAX_SNAPSHOTS);
    all.drain(..excess);
    let path = file(serial);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&all).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(all)
}

/// Readable namespace.
pub fn namespace_label(ns: &str) -> &'static str {
    match ns {
        "global" => tr("Global (whole phone)"),
        "system" => tr("System (screen and sound)"),
        _ => tr("Secure (privacy and security)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "@@@SDK@@@\n36\n@@@GLOBAL@@@\nairplane_mode_on=0\nboot_count=10\nwifi_on=1\n@@@SYSTEM@@@\nscreen_brightness=100\nfont_scale=1.0\n@@@SECURE@@@\nandroid_id=abcdef0123456789\nlocation_mode=3\n@@@END@@@\n";
    const B: &str = "@@@SDK@@@\n36\n@@@GLOBAL@@@\nairplane_mode_on=0\nboot_count=11\nwifi_on=0\nprivate_dns_mode=hostname\n@@@SYSTEM@@@\nscreen_brightness=40\n@@@SECURE@@@\nandroid_id=abcdef0123456789\nlocation_mode=3\nadb_enabled=1\n@@@END@@@\n";

    #[test]
    fn identifiers_are_not_kept() {
        let s = parse(A);
        assert_eq!(s.sdk, Some(36));
        assert!(!s.values["secure"].contains_key("android_id"));
        assert_eq!(s.key_count(), 6);
    }

    #[test]
    fn differences() {
        let d = diff(&parse(A), &parse(B));
        let keys: Vec<(&str, &str)> = d.iter().map(|c| (c.namespace.as_str(), c.key.as_str())).collect();
        assert_eq!(keys, [("global", "private_dns_mode"), ("global", "wifi_on"), ("system", "font_scale"), ("system", "screen_brightness"), ("secure", "adb_enabled")]);
        let brightness = d.iter().find(|c| c.key == "screen_brightness").unwrap();
        assert_eq!(brightness.restore_command().unwrap().join(" "), "settings put system screen_brightness 100");
        // Added since: deleted to restore.
        let dns = d.iter().find(|c| c.key == "private_dns_mode").unwrap();
        assert_eq!(dns.restore_command().unwrap().join(" "), "settings delete global private_dns_mode");
        // Removed since: put back.
        assert_eq!(d.iter().find(|c| c.key == "font_scale").unwrap().after, None);
        // Never restored.
        assert!(d.iter().find(|c| c.key == "adb_enabled").unwrap().restore_command().is_none());
    }

    #[test]
    fn unread_namespaces_are_not_compared() {
        let partial = parse("@@@SDK@@@\n36\n@@@GLOBAL@@@\nwifi_on=1\n@@@SYSTEM@@@\nPermission denial\n@@@SECURE@@@\n@@@END@@@\n");
        let d = diff(&parse(A), &partial);
        assert!(d.iter().all(|c| c.namespace == "global"));
        assert!(!is_valid_key("a;b"));
    }
}
