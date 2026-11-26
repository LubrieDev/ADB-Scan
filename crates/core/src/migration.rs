//! App migration: exports the user's apps (their APKs, splits included) to a
//! folder on the PC, with a manifest, and installs them on another phone.
//!
//! Only the apps travel, not their data: without root Android does not let
//! ADB read another app's data (and `adb backup` no longer copies it on
//! modern Android). Each app restores its own data from its own backup
//! (WhatsApp, Telegram…) or from the Google account.
//!
//! The manifest is read back from a folder the user chooses, so every path
//! in it is checked ([`safe_relative`]) before any file is used: a tampered
//! manifest cannot point outside its folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::i18n::tr;
use crate::t;

/// File name of the manifest inside the export folder.
pub const MANIFEST: &str = "adb-scan-apps.json";
/// Current manifest format.
pub const FORMAT: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportedApp {
    pub package: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub version_code: Option<i64>,
    /// APK files relative to the export folder (`<package>/base.apk`…).
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    /// Seconds since the epoch.
    pub created: i64,
    #[serde(default)]
    pub source_model: String,
    #[serde(default)]
    pub source_android: String,
    pub apps: Vec<ExportedApp>,
}

/// A relative path made only of safe components (no `..`, no absolute path,
/// no hidden or odd characters).
pub fn safe_relative(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && p.split('/').all(|c| !c.is_empty() && c != "." && c != ".." && !c.starts_with('.') && c.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')))
}

/// Name of an APK on the phone (`base.apk`, `split_config.arm64_v8a.apk`),
/// usable as a file name.
pub fn apk_file_name(remote: &str) -> Option<String> {
    let name = remote.rsplit('/').next()?;
    (name.ends_with(".apk") && safe_relative(name)).then(|| name.to_string())
}

impl ExportedApp {
    /// Absolute paths of its files inside `dir`, only if every one is safe
    /// and exists.
    pub fn paths(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        if self.files.is_empty() {
            return Err(Error::Invalid(t!("{} has no APK in the export", self.package)));
        }
        self.files
            .iter()
            .map(|f| {
                if !safe_relative(f) || !f.ends_with(".apk") {
                    return Err(Error::Invalid(t!("unsafe path in the manifest: {}", f)));
                }
                let p = dir.join(f);
                if p.is_file() {
                    Ok(p)
                } else {
                    Err(Error::Invalid(t!("{} is missing from the export", f)))
                }
            })
            .collect()
    }
}

/// `package:com.x versionCode:123` lines of `pm list packages -3
/// --show-versioncode`.
pub fn parse_user_packages(text: &str) -> Vec<(String, Option<i64>)> {
    let mut v: Vec<(String, Option<i64>)> = text
        .lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("package:")?;
            let mut parts = rest.split_whitespace();
            let pkg = parts.next()?.to_string();
            let code = parts.find_map(|p| p.strip_prefix("versionCode:")).and_then(|c| c.parse().ok());
            crate::adb::is_valid_package_name(&pkg).then_some((pkg, code))
        })
        .collect();
    v.sort();
    v
}

pub fn save(dir: &Path, m: &Manifest) -> Result<()> {
    let json = serde_json::to_string_pretty(m).map_err(|e| Error::Invalid(e.to_string()))?;
    std::fs::write(dir.join(MANIFEST), json)?;
    Ok(())
}

/// Reads and checks the manifest of an export folder.
pub fn load(dir: &Path) -> Result<Manifest> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).map_err(|_| Error::Invalid(t!("{} has no {}: choose a folder exported by ADB Scan", dir.display(), MANIFEST)))?;
    let m: Manifest = serde_json::from_str(&text).map_err(|e| Error::Invalid(t!("the manifest cannot be read: {}", e)))?;
    if m.format > FORMAT {
        return Err(Error::Invalid(tr("the export was made by a newer ADB Scan").into()));
    }
    if let Some(bad) = m.apps.iter().find(|a| !crate::adb::is_valid_package_name(&a.package)) {
        return Err(Error::Invalid(t!("invalid package in the manifest: {}", bad.package)));
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        assert!(safe_relative("com.x/base.apk"));
        assert!(safe_relative("com.x/split_config.arm64_v8a.apk"));
        for bad in ["/etc/passwd", "../x.apk", "com.x/../../y.apk", "com.x/.hidden.apk", "com x/a.apk", ""] {
            assert!(!safe_relative(bad), "{bad}");
        }
        assert_eq!(apk_file_name("/data/app/~~a==/com.x-b==/base.apk").as_deref(), Some("base.apk"));
        assert_eq!(apk_file_name("/data/app/x/lib.so"), None);
    }

    #[test]
    fn user_packages() {
        let v = parse_user_packages("package:org.b versionCode:12\npackage:com.a versionCode:7\njunk\n");
        assert_eq!(v, vec![("com.a".into(), Some(7)), ("org.b".into(), Some(12))]);
    }

    #[test]
    fn manifest_round_trip_and_checks() {
        let dir = std::env::temp_dir().join(format!("adbscan-migration-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("com.x")).unwrap();
        std::fs::write(dir.join("com.x/base.apk"), b"PK").unwrap();
        let m = Manifest { format: FORMAT, created: 1, source_model: "M".into(), source_android: "16".into(), apps: vec![ExportedApp { package: "com.x".into(), label: "X".into(), version_code: Some(1), files: vec!["com.x/base.apk".into()] }] };
        save(&dir, &m).unwrap();
        let back = load(&dir).unwrap();
        assert_eq!(back, m);
        assert_eq!(back.apps[0].paths(&dir).unwrap(), vec![dir.join("com.x/base.apk")]);
        let evil = ExportedApp { package: "com.y".into(), files: vec!["../../etc/x.apk".into()], ..Default::default() };
        assert!(evil.paths(&dir).is_err());
        std::fs::remove_file(dir.join("com.x/base.apk")).unwrap();
        std::fs::remove_file(dir.join(MANIFEST)).unwrap();
        std::fs::remove_dir(dir.join("com.x")).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
