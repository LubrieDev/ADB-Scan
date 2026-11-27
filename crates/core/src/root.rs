//! Root: what a phone needs to be rooted with Magisk, read without root, and
//! the steps that ADB Scan can do from the computer.
//!
//! Reading ([`SCRIPT`]) is read-only: every property, the names of the
//! partitions (`/dev/block/by-name`, readable by the shell user), whether a
//! root manager is installed and the kernel. From that: the bootloader state,
//! A/B slots, the partition Magisk patches (`init_boot` on phones launched
//! with Android 13 or later, otherwise `boot`) and the kernel's GKI branch
//! (for KernelSU).
//!
//! Magisk itself only comes from its official repository
//! (`github.com/topjohnwu/Magisk`): the latest release's `Magisk-v*.apk`,
//! accepted only with the SHA-256 sum GitHub publishes for it. ADB Scan never
//! patches images itself: the Magisk app does it on the phone, and flashing
//! is done in the Flasher with its safeguards. Unlocking the bootloader (also
//! in the Flasher) erases the phone.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::{i18n::tr, t};

/// Read-only.
pub const SCRIPT: &str = "echo @@@PROPS@@@; getprop
echo @@@PARTS@@@; ls /dev/block/by-name/ 2>&1
echo @@@SU@@@; command -v su 2>/dev/null
echo @@@PKGS@@@; pm list packages 2>&1 | grep -iE 'magisk|kernelsu|apatch|ksunext|sukisu'
echo @@@MAGISK@@@; dumpsys package com.topjohnwu.magisk 2>/dev/null | grep -m1 -E 'versionName='
echo @@@UNAME@@@; uname -r
echo @@@END@@@
";

/// Where the patched images are looked for on the phone.
pub const PHONE_DOWNLOADS: &str = "/sdcard/Download";

/// A root manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Magisk,
    KernelSu,
    KernelSuNext,
    SukiSu,
    APatch,
}

impl Manager {
    pub fn name(self) -> &'static str {
        match self {
            Manager::Magisk => "Magisk",
            Manager::KernelSu => "KernelSU",
            Manager::KernelSuNext => "KernelSU Next",
            Manager::SukiSu => "SukiSU",
            Manager::APatch => "APatch",
        }
    }

    fn of_package(p: &str) -> Option<Manager> {
        Some(match p {
            "com.topjohnwu.magisk" | "io.github.huskydg.magisk" | "io.github.vvb2060.magisk" => Manager::Magisk,
            "me.weishu.kernelsu" => Manager::KernelSu,
            "com.rifsxd.ksunext" => Manager::KernelSuNext,
            "com.sukisu.ultra" => Manager::SukiSu,
            "me.bmax.apatch" => Manager::APatch,
            _ => return None,
        })
    }
}

/// The bootloader as Android reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bootloader {
    Locked,
    Unlocked,
    Unknown,
}

/// How hard unlocking the bootloader is for a manufacturer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    /// `fastboot flashing unlock` after allowing it in Developer options.
    Easy,
    /// It needs a code, an account or a waiting period from the manufacturer.
    Medium,
    /// Only some models or regions, with the manufacturer's own tools.
    Hard,
    /// The manufacturer does not allow it.
    NotPossible,
}

impl Difficulty {
    pub fn label(self) -> &'static str {
        match self {
            Difficulty::Easy => tr("Easy"),
            Difficulty::Medium => tr("Possible, with the manufacturer's permission"),
            Difficulty::Hard => tr("Hard: only some models or regions"),
            Difficulty::NotPossible => tr("Not possible: the manufacturer does not allow it"),
        }
    }
}

/// How to unlock the bootloader of a brand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnlockGuide {
    pub brand: &'static str,
    pub difficulty: Difficulty,
    /// What to do, in words.
    pub how: &'static str,
    /// Whether the phone has fastboot (Samsung does not: Odin).
    pub fastboot: bool,
}

/// The guide for a manufacturer (`ro.product.manufacturer` /
/// `ro.product.brand`, any case).
pub fn unlock_guide(manufacturer: &str, brand: &str) -> UnlockGuide {
    let m = format!("{} {}", manufacturer.to_lowercase(), brand.to_lowercase());
    let has = |k: &str| m.contains(k);
    let g = |brand, difficulty, how, fastboot| UnlockGuide { brand, difficulty, how, fastboot };
    if has("google") {
        g("Google Pixel", Difficulty::Easy, tr("Developer options → OEM unlocking, then in the bootloader “fastboot flashing unlock” (Flasher → Unlock the bootloader). Pixels sold by some carriers (Verizon) cannot be unlocked."), true)
    } else if has("nubia") || has("redmagic") || has("zte") {
        g("nubia / REDMAGIC / ZTE", Difficulty::Easy, tr("Developer options → OEM unlocking, then in the bootloader “fastboot flashing unlock” (Flasher → Unlock the bootloader) and confirm on the phone with the volume keys."), true)
    } else if has("oneplus") {
        g("OnePlus", Difficulty::Easy, tr("Developer options → OEM unlocking, then “fastboot flashing unlock” (Flasher → Unlock the bootloader). Some carrier and Chinese models need a token from OnePlus."), true)
    } else if has("xiaomi") || has("redmi") || has("poco") {
        g(
            "Xiaomi / Redmi / POCO",
            Difficulty::Medium,
            tr("Link the phone to a Mi account in Developer options → Mi Unlock status, wait the period Xiaomi sets (days or weeks) and unlock with Xiaomi's Mi Unlock tool (Windows). With HyperOS Xiaomi limits it further, and Chinese ROMs need a level in its community app."),
            true,
        )
    } else if has("motorola") || has("lenovo") {
        g("Motorola", Difficulty::Medium, tr("In the bootloader, Flasher → Unlock data (Motorola) gives the text to paste on Motorola's unlock website; with the code it sends by email, Flasher → Unlock with a code. Carrier models and many cheap models are usually excluded."), true)
    } else if has("sony") {
        g("Sony", Difficulty::Medium, tr("Get the unlock code on Sony's website with the IMEI and use it in Flasher → Unlock with a code (with “0x” in front). The camera's DRM keys are lost for good."), true)
    } else if has("nothing") || has("fairphone") || has("asus") || has("realme") || has("oppo") {
        if has("nothing") || has("fairphone") {
            g("Nothing / Fairphone", Difficulty::Easy, tr("Developer options → OEM unlocking, then “fastboot flashing unlock” (Flasher → Unlock the bootloader). Fairphone asks for a code from its website first."), true)
        } else if has("asus") {
            g("ASUS", Difficulty::Hard, tr("Only with ASUS's own unlock app, which it stopped offering for recent models."), true)
        } else {
            g("OPPO / realme", Difficulty::Hard, tr("Only with the manufacturer's “Deep testing” app for some models and regions; it is often unavailable."), true)
        }
    } else if has("samsung") {
        g(
            "Samsung",
            Difficulty::Hard,
            tr("No fastboot: OEM unlocking in Developer options, then the unlock screen in download mode, and the patched AP file is flashed with Odin (Windows) or Heimdall, not from here. It trips Knox for good (Samsung Pay, Secure Folder). US models with Snapdragon cannot be unlocked."),
            false,
        )
    } else if has("huawei") || has("honor") {
        g("Huawei / Honor", Difficulty::NotPossible, tr("Huawei and Honor stopped giving unlock codes in 2018: their bootloaders cannot be unlocked."), true)
    } else if has("vivo") || has("iqoo") {
        g("vivo / iQOO", Difficulty::NotPossible, tr("vivo and iQOO do not allow unlocking on current models."), true)
    } else {
        g(
            "",
            Difficulty::Medium,
            tr("Usually: Developer options → OEM unlocking, then “fastboot flashing unlock” (Flasher → Unlock the bootloader). Look up this model: some manufacturers ask for a code or do not allow it."),
            true,
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RootReport {
    pub props: BTreeMap<String, String>,
    /// Partition names without the `_a`/`_b` suffix, sorted.
    pub partitions: Vec<String>,
    /// `/system/bin/su` or similar if it is on the path.
    pub su_path: Option<String>,
    /// Root managers installed, with their version if known.
    pub managers: Vec<(Manager, Option<String>)>,
    pub kernel: String,
}

impl RootReport {
    pub fn prop(&self, key: &str) -> &str {
        self.props.get(key).map(String::as_str).unwrap_or("")
    }

    pub fn bootloader(&self) -> Bootloader {
        match (self.prop("ro.boot.flash.locked"), self.prop("ro.boot.vbmeta.device_state")) {
            ("1", _) | (_, "locked") => Bootloader::Locked,
            ("0", _) | (_, "unlocked") => Bootloader::Unlocked,
            _ => match self.prop("ro.boot.verifiedbootstate") {
                "green" => Bootloader::Locked,
                "orange" | "yellow" => Bootloader::Unlocked,
                _ => Bootloader::Unknown,
            },
        }
    }

    /// The “OEM unlocking” switch of Developer options, when the phone
    /// publishes it (`sys.oem_unlock_allowed`; empty on some brands).
    pub fn oem_unlock_allowed(&self) -> Option<bool> {
        match self.prop("sys.oem_unlock_allowed") {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        }
    }

    /// A Unisoc (Spreadtrum) chip: manufacturers rarely give unlock codes for
    /// them.
    pub fn unisoc(&self) -> bool {
        let platform = self.prop("ro.board.platform").to_lowercase();
        let hardware = self.prop("ro.hardware").to_lowercase();
        platform.starts_with("ums") || platform.starts_with("sp9") || hardware.starts_with("ums") || hardware.starts_with("s9863")
    }

    /// 32-bit system (Android Go phones).
    pub fn is_32bit(&self) -> bool {
        matches!(self.prop("ro.product.cpu.abi"), "armeabi-v7a" | "armeabi" | "x86")
    }

    pub fn has_partition(&self, name: &str) -> bool {
        self.partitions.iter().any(|p| p == name)
    }

    pub fn ab(&self) -> bool {
        self.prop("ro.build.ab_update") == "true" || !self.prop("ro.boot.slot_suffix").is_empty()
    }

    /// `a` or `b`.
    pub fn slot(&self) -> Option<&str> {
        self.prop("ro.boot.slot_suffix").strip_prefix('_').filter(|s| !s.is_empty())
    }

    /// The image Magisk patches: `init_boot` where it exists (phones launched
    /// with Android 13 or later), otherwise `boot`.
    pub fn target_partition(&self) -> &'static str {
        if self.has_partition("init_boot") {
            "init_boot"
        } else {
            "boot"
        }
    }

    /// Root seems active (a `su` on the path or a manager installed).
    pub fn rooted(&self) -> bool {
        self.su_path.is_some()
    }

    /// The kernel's GKI branch (`android16-6.12`), from `uname -r`
    /// (`6.12.38-android16-5-g…`).
    pub fn gki(&self) -> Option<String> {
        let (version, rest) = self.kernel.split_once('-')?;
        let android = rest.split('-').next().filter(|a| a.starts_with("android") && a[7..].chars().all(|c| c.is_ascii_digit()))?;
        let mut v = version.split('.');
        Some(format!("{android}-{}.{}", v.next()?, v.next()?))
    }

    /// KernelSU (GKI mode) needs a GKI kernel 5.10 or newer.
    pub fn kernelsu_gki_ok(&self) -> bool {
        let Some(g) = self.gki() else { return false };
        let ver = g.split('-').nth(1).unwrap_or("");
        let mut it = ver.split('.').filter_map(|n| n.parse::<u32>().ok());
        matches!((it.next(), it.next()), (Some(maj), Some(min)) if (maj, min) >= (5, 10))
    }

    pub fn guide(&self) -> UnlockGuide {
        let mut g = unlock_guide(self.prop("ro.product.manufacturer"), self.prop("ro.product.brand"));
        if self.samsung_us_snapdragon() {
            g.difficulty = Difficulty::NotPossible;
            g.how = tr("This is a Samsung model for the US with a Snapdragon chip (model ending in U or U1): Samsung locks its bootloader for good and the OEM unlocking switch does not exist. Root is not possible on it.");
        }
        g
    }

    /// A Samsung sold in the US with a Qualcomm chip (`SM-X218U`,
    /// `SM-S918U1`): its bootloader cannot be unlocked.
    pub fn samsung_us_snapdragon(&self) -> bool {
        let samsung = self.prop("ro.product.manufacturer").eq_ignore_ascii_case("samsung");
        let model = self.prop("ro.product.model").to_uppercase();
        let us = model.starts_with("SM-") && (model.ends_with('U') || model.ends_with("U1"));
        let qualcomm = matches!(self.prop("ro.soc.manufacturer").to_uppercase().as_str(), "QTI" | "QUALCOMM") || self.prop("ro.hardware").eq_ignore_ascii_case("qcom");
        samsung && us && qualcomm
    }

    /// The chip as Android names it: `SM6375 (blair)`, or the platform alone
    /// before Android 12.
    pub fn chip(&self) -> String {
        let (model, platform) = (self.prop("ro.soc.model"), self.prop("ro.board.platform"));
        match (model.is_empty(), platform.is_empty()) {
            (false, false) if !model.eq_ignore_ascii_case(platform) => format!("{model} ({platform})"),
            (false, _) => model.to_string(),
            _ => platform.to_string(),
        }
    }
}

pub fn parse(text: &str) -> RootReport {
    let s = crate::parsers::split_sections(text);
    let get = |k: &str| s.get(k).map(String::as_str).unwrap_or("");
    let props = crate::parsers::parse_getprop(get("PROPS"));
    let mut partitions: Vec<String> = get("PARTS")
        .split_whitespace()
        .filter(|p| !p.contains(':') && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
        .map(|p| p.strip_suffix("_a").or_else(|| p.strip_suffix("_b")).unwrap_or(p).to_string())
        .collect();
    partitions.sort();
    partitions.dedup();
    let su_path = get("SU").lines().map(str::trim).find(|l| l.starts_with('/')).map(str::to_string);
    let magisk_version = get("MAGISK").trim().strip_prefix("versionName=").map(|v| v.trim().to_string());
    let mut managers: Vec<(Manager, Option<String>)> = Vec::new();
    for p in get("PKGS").lines().filter_map(|l| l.trim().strip_prefix("package:")) {
        if let Some(m) = Manager::of_package(p.trim()) {
            if !managers.iter().any(|(x, _)| *x == m) {
                let v = if m == Manager::Magisk { magisk_version.clone() } else { None };
                managers.push((m, v));
            }
        }
    }
    RootReport { props, partitions, su_path, managers, kernel: get("UNAME").trim().to_string() }
}

// ---------------------------------------------------------------------------
// Magisk from its official releases
// ---------------------------------------------------------------------------

/// The latest published release of Magisk (not the pre-releases).
pub const MAGISK_RELEASE_URL: &str = "https://api.github.com/repos/topjohnwu/Magisk/releases/latest";

/// The only place its APK is downloaded from.
pub const MAGISK_DOWNLOADS: &str = "https://github.com/topjohnwu/Magisk/releases/download/";

/// Magisk's page, to read about it.
pub const MAGISK_PAGE: &str = "https://github.com/topjohnwu/Magisk";

/// Magisk's APK is ~11 MB.
const MAX_APK_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

/// A Magisk release ready to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagiskRelease {
    /// `v31.1`
    pub tag: String,
    pub apk: crate::updater::Package,
}

/// The release's `Magisk-v*.apk`: from the official repository, with a plain
/// name, a plausible size and GitHub's SHA-256 sum. `app-debug.apk` and the
/// rest are ignored.
pub fn magisk_from_json(json: &str) -> Option<MagiskRelease> {
    let r: Release = serde_json::from_str(json).ok()?;
    let tag_ok = !r.tag_name.is_empty() && r.tag_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if !tag_ok {
        return None;
    }
    let apk = r.assets.iter().find_map(|a| {
        let name_ok = a.name.starts_with("Magisk-") && a.name.ends_with(".apk") && a.name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
        let url_ok = a.browser_download_url == format!("{MAGISK_DOWNLOADS}{}/{}", r.tag_name, a.name);
        let sha256 = a.digest.as_deref()?.strip_prefix("sha256:")?.to_ascii_lowercase();
        let sum_ok = sha256.len() == 64 && sha256.chars().all(|c| c.is_ascii_hexdigit());
        let size_ok = (1..=MAX_APK_BYTES).contains(&a.size);
        (name_ok && url_ok && sum_ok && size_ok).then(|| crate::updater::Package { name: a.name.clone(), url: a.browser_download_url.clone(), size: a.size, sha256 })
    })?;
    Some(MagiskRelease { tag: r.tag_name, apk })
}

/// Asks GitHub for the latest Magisk (one small request).
pub fn latest_magisk() -> Result<MagiskRelease> {
    let json = crate::updater::fetch_json(MAGISK_RELEASE_URL).map_err(|e| Error::Invalid(t!("could not ask GitHub for Magisk: {}", e)))?;
    magisk_from_json(&json).ok_or_else(|| Error::Invalid(tr("Magisk's latest release has no APK with a published SHA-256 sum").into()))
}

/// Folder where Magisk's APK is kept.
pub fn magisk_dir() -> PathBuf {
    crate::paths::cache_dir().join("magisk")
}

/// Downloads Magisk's APK (or reuses an earlier verified copy of the same
/// file) and returns it.
pub fn download_magisk(release: &MagiskRelease, progress: &dyn Fn(u64, u64), cancel: &AtomicBool) -> Result<PathBuf> {
    let existing = magisk_dir().join(&release.apk.name);
    if existing.is_file() && crate::tools_setup::sha256_file(&existing).is_ok_and(|s| s.eq_ignore_ascii_case(&release.apk.sha256)) {
        return Ok(existing);
    }
    crate::updater::download_into(&release.apk, &magisk_dir(), progress, cancel)
}

/// `magisk_patched-31100_AbCdE.img` (what the Magisk app writes to
/// Download).
pub fn is_patched_name(name: &str) -> bool {
    name.starts_with("magisk_patched") && name.ends_with(".img") && name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// The newest patched image in `ls -t` output of the phone's Download folder.
pub fn newest_patched(ls: &str) -> Option<String> {
    ls.lines().map(str::trim).find(|n| is_patched_name(n)).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    // As a REDMAGIC NX809J (Android 16) answers, shortened.
    const OUT: &str = "@@@PROPS@@@
[ro.product.manufacturer]: [nubia]
[ro.product.brand]: [nubia]
[ro.product.device]: [NX809J]
[ro.boot.flash.locked]: [1]
[ro.boot.vbmeta.device_state]: [locked]
[ro.boot.verifiedbootstate]: [green]
[ro.boot.slot_suffix]: [_a]
[ro.build.ab_update]: [true]
[ro.product.first_api_level]: [36]
@@@PARTS@@@
abl_a
abl_b
boot_a
boot_b
init_boot_a
init_boot_b
vbmeta_a
vbmeta_b
userdata
@@@SU@@@
@@@PKGS@@@
@@@MAGISK@@@
@@@UNAME@@@
6.12.38-android16-5-g665eafb62659-ab14778838-4k
@@@END@@@
";

    #[test]
    fn readiness() {
        let r = parse(OUT);
        assert_eq!(r.bootloader(), Bootloader::Locked);
        assert!(r.ab());
        assert_eq!(r.slot(), Some("a"));
        assert_eq!(r.target_partition(), "init_boot");
        assert!(r.has_partition("vbmeta") && r.has_partition("userdata"));
        assert_eq!(r.gki().as_deref(), Some("android16-6.12"));
        assert!(r.kernelsu_gki_ok());
        assert!(!r.rooted());
        assert!(r.managers.is_empty());
        assert_eq!(r.guide().difficulty, Difficulty::Easy);
    }

    #[test]
    fn rooted_and_old_phones() {
        let out = "@@@PROPS@@@\n[ro.boot.verifiedbootstate]: [orange]\n@@@PARTS@@@\nboot\nrecovery\n@@@SU@@@\n/debug_ramdisk/su\n@@@PKGS@@@\npackage:com.topjohnwu.magisk\npackage:com.example.magiskhelper\n@@@MAGISK@@@\n    versionName=31.1\n@@@UNAME@@@\n4.14.190-perf+\n@@@END@@@\n";
        let r = parse(out);
        assert_eq!(r.bootloader(), Bootloader::Unlocked);
        assert_eq!(r.target_partition(), "boot");
        assert!(!r.ab());
        assert!(r.rooted());
        assert_eq!(r.managers, vec![(Manager::Magisk, Some("31.1".into()))]);
        assert_eq!(r.gki(), None);
        assert!(!r.kernelsu_gki_ok());
    }

    /// Galaxy Tab A9+ 5G (SM-X218U, Snapdragon 695): a US model, no unlock.
    #[test]
    fn samsung_us() {
        let out = "@@@PROPS@@@\n[ro.product.manufacturer]: [samsung]\n[ro.product.brand]: [samsung]\n[ro.product.model]: [SM-X218U]\n[ro.soc.manufacturer]: [QTI]\n[ro.soc.model]: [SM6375]\n[ro.board.platform]: [blair]\n[ro.hardware]: [qcom]\n[ro.boot.flash.locked]: [1]\n@@@PARTS@@@\nboot\ninit_boot\nrecovery\n@@@SU@@@\n@@@PKGS@@@\n@@@MAGISK@@@\n@@@UNAME@@@\n6.1.172-android14-11\n@@@END@@@\n";
        let r = parse(out);
        assert!(r.samsung_us_snapdragon());
        assert_eq!(r.guide().difficulty, Difficulty::NotPossible);
        assert!(!r.guide().fastboot);
        assert_eq!(r.chip(), "SM6375 (blair)");
        assert_eq!(r.target_partition(), "init_boot");
        // The international model (Exynos or “B” suffix) keeps Samsung's guide.
        let intl = parse(&out.replace("SM-X218U", "SM-X216B"));
        assert!(!intl.samsung_us_snapdragon());
        assert_eq!(intl.guide().difficulty, Difficulty::Hard);
    }

    #[test]
    fn moto_e20() {
        // As a moto e20 (Android 11 Go, Unisoc T606) answers, shortened.
        let out = "@@@PROPS@@@\n[ro.product.manufacturer]: [motorola]\n[ro.product.brand]: [motorola]\n[ro.product.cpu.abi]: [armeabi-v7a]\n[ro.board.platform]: [ums9230]\n[ro.hardware]: [ums9230_4h10_go]\n[ro.oem_unlock_supported]: [1]\n[sys.oem_unlock_allowed]: [1]\n[ro.boot.flash.locked]: [1]\n[ro.boot.slot_suffix]: [_a]\n@@@PARTS@@@\nboot_a\nboot_b\nsuper\nvbmeta_a\nvbmeta_b\nvendor_boot_a\nvendor_boot_b\n@@@SU@@@\n@@@PKGS@@@\n@@@MAGISK@@@\n@@@UNAME@@@\n4.14.199+-ab8ddaf\n@@@END@@@\n";
        let r = parse(out);
        assert_eq!(r.oem_unlock_allowed(), Some(true));
        assert!(r.unisoc() && r.is_32bit());
        assert_eq!(r.target_partition(), "boot");
        assert_eq!(r.gki(), None);
        assert_eq!(r.guide().brand, "Motorola");
        assert_eq!(parse(OUT).oem_unlock_allowed(), None);
    }

    #[test]
    fn guides() {
        assert_eq!(unlock_guide("Google", "google").difficulty, Difficulty::Easy);
        assert_eq!(unlock_guide("Xiaomi", "Redmi").difficulty, Difficulty::Medium);
        assert!(!unlock_guide("samsung", "samsung").fastboot);
        assert_eq!(unlock_guide("HUAWEI", "HONOR").difficulty, Difficulty::NotPossible);
        assert_eq!(unlock_guide("motorola", "motorola").brand, "Motorola");
    }

    #[test]
    fn magisk_release() {
        // GitHub's answer for v31.1 (trimmed).
        let json = r#"{"tag_name":"v31.1","assets":[
 {"name":"app-debug.apk","size":37034525,"digest":"sha256:b983c975663feebaaf0bcc63fe57a8c4406914ec2b4a08fa3259bca817a10778","browser_download_url":"https://github.com/topjohnwu/Magisk/releases/download/v31.1/app-debug.apk"},
 {"name":"Magisk-v31.1.apk","size":10679283,"digest":"sha256:89cefac0a318afa39fb3880c45c87b036c05bc1ebf02a7a715b7f8d7c26ac6e7","browser_download_url":"https://github.com/topjohnwu/Magisk/releases/download/v31.1/Magisk-v31.1.apk"}]}"#;
        let m = magisk_from_json(json).unwrap();
        assert_eq!(m.tag, "v31.1");
        assert_eq!(m.apk.name, "Magisk-v31.1.apk");
        assert_eq!(m.apk.size, 10_679_283);
        // From anywhere else, or without its sum: refused.
        assert!(magisk_from_json(&json.replace("github.com/topjohnwu", "github.com/someone")).is_none());
        assert!(magisk_from_json(&json.replace("sha256:89ce", "md5:89ce")).is_none());
        assert!(is_patched_name("magisk_patched-31100_AbCdE.img"));
        assert!(!is_patched_name("magisk_patched;rm.img"));
        assert_eq!(newest_patched("other.zip\nmagisk_patched-31100_x.img\nmagisk_patched-30000_y.img\n").as_deref(), Some("magisk_patched-31100_x.img"));
    }
}
