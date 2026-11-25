//! Fastboot: information, validated flashing, A/B slots and ROM plans.
//!
//! Safeguards:
//! - Validated partition names (no paths or operators).
//! - Critical partitions (bootloaders, modem, persist…) require explicit
//!   confirmation: [`Confirm::Critical`].
//! - Erasing data, unlocking or locking the bootloader requires
//!   [`Confirm::Destructive`], which the interface only grants after the
//!   device's serial number is typed.

use crate::{i18n::tr, t};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::adb::{find_tool, Tool};
use crate::error::{Error, Result};
use crate::exec::{self, CommandResult, StreamEvent, StreamHandle, StreamOptions};
use crate::models::AdbDevice;
use crate::parsers;

/// Partitions whose incorrect flashing can brick the phone.
pub const CRITICAL_PARTITIONS: [&str; 26] = [
    "xbl",
    "xbl_config",
    "abl",
    "aboot",
    "sbl1",
    "tz",
    "hyp",
    "rpm",
    "pmic",
    "keymaster",
    "cmnlib",
    "cmnlib64",
    "devcfg",
    "modem",
    "modemst1",
    "modemst2",
    "fsg",
    "persist",
    "frp",
    "devinfo",
    "bootloader",
    "radio",
    "preloader",
    "lk",
    "uefi",
    "qupfw",
];

/// Usual partitions in ROMs (recommended flashing order).
pub const COMMON_PARTITIONS: [&str; 18] = [
    "boot",
    "init_boot",
    "vendor_boot",
    "dtbo",
    "vbmeta",
    "vbmeta_system",
    "vbmeta_vendor",
    "recovery",
    "super",
    "system",
    "system_ext",
    "product",
    "vendor",
    "odm",
    "vendor_dlkm",
    "odm_dlkm",
    "system_dlkm",
    "userdata",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    None,
    Critical,
    Destructive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    A,
    B,
    All,
}

impl Slot {
    pub fn arg(self) -> &'static str {
        match self {
            Slot::A => "a",
            Slot::B => "b",
            Slot::All => "all",
        }
    }
}

pub fn base_partition(name: &str) -> &str {
    name.strip_suffix("_ab").or_else(|| name.strip_suffix("_a")).or_else(|| name.strip_suffix("_b")).unwrap_or(name)
}

pub fn validate_partition(name: &str) -> Result<()> {
    let ok = !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') && name.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(t!("invalid partition name: “{}”", name)))
    }
}

pub fn is_critical(name: &str) -> bool {
    CRITICAL_PARTITIONS.contains(&base_partition(name))
}

pub fn required_confirmation(partition: &str) -> Confirm {
    match base_partition(partition) {
        "userdata" | "metadata" | "cache" => Confirm::Destructive,
        p if CRITICAL_PARTITIONS.contains(&p) => Confirm::Critical,
        _ => Confirm::None,
    }
}

fn check_confirm(needed: Confirm, given: Confirm) -> Result<()> {
    let rank = |c: Confirm| match c {
        Confirm::None => 0,
        Confirm::Critical => 1,
        Confirm::Destructive => 2,
    };
    if rank(given) >= rank(needed) {
        Ok(())
    } else {
        Err(Error::Guarded(tr("this operation requires explicit confirmation").into()))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FastbootInfo {
    pub vars: BTreeMap<String, String>,
}

impl FastbootInfo {
    pub fn get(&self, key: &str) -> &str {
        self.vars.get(key).map(String::as_str).unwrap_or("")
    }
    pub fn unlocked(&self) -> Option<bool> {
        match self.get("unlocked") {
            "yes" => Some(true),
            "no" => Some(false),
            _ => None,
        }
    }
    pub fn current_slot(&self) -> Option<String> {
        let s = self.get("current-slot").trim().trim_start_matches('_');
        (!s.is_empty()).then(|| s.to_string())
    }
    pub fn has_slots(&self) -> bool {
        self.get("slot-count").parse::<u32>().is_ok_and(|n| n >= 2) || self.current_slot().is_some()
    }
    pub fn is_userspace(&self) -> bool {
        self.get("is-userspace") == "yes"
    }
    pub fn product(&self) -> &str {
        self.get("product")
    }
    /// Size in bytes of a partition (`partition-size:boot_a: 0x6000000`).
    pub fn partition_size(&self, partition: &str) -> Option<u64> {
        self.vars.iter().find_map(|(k, v)| {
            if k != "partition-size" {
                return None;
            }
            let (name, size) = v.split_once(':')?;
            (name.trim() == partition).then(|| u64::from_str_radix(size.trim().trim_start_matches("0x"), 16).ok())?
        })
    }
}

#[derive(Debug, Clone)]
pub struct Fastboot {
    path: PathBuf,
}

impl Fastboot {
    pub fn locate(configured: Option<&str>) -> Result<Self> {
        find_tool(Tool::Fastboot, configured).map(|path| Self { path }).ok_or(Error::ToolNotFound { tool: "fastboot".into() })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn run(&self, serial: Option<&str>, args: &[&str], timeout: Duration) -> Result<CommandResult> {
        let mut v: Vec<String> = Vec::new();
        if let Some(s) = serial {
            v.push("-s".into());
            v.push(s.into());
        }
        v.extend(args.iter().map(|a| a.to_string()));
        exec::run(&self.path, &v, timeout, None)
    }

    pub fn stream<F>(&self, serial: &str, args: &[String], on_event: F) -> Result<StreamHandle>
    where
        F: Fn(StreamEvent) + Send + Sync + 'static,
    {
        let mut v = vec!["-s".to_string(), serial.to_string()];
        v.extend(args.iter().cloned());
        exec::stream(&self.path, &v, StreamOptions::default(), on_event)
    }

    pub fn devices(&self) -> Result<Vec<AdbDevice>> {
        let r = self.run(None, &["devices", "-l"], Duration::from_secs(5))?;
        Ok(parsers::parse_fastboot_devices(&r.combined()))
    }

    pub fn info(&self, serial: &str) -> Result<FastbootInfo> {
        let r = self.run(Some(serial), &["getvar", "all"], Duration::from_secs(20))?;
        let vars = parsers::parse_fastboot_vars(&r.combined());
        if vars.is_empty() {
            return Err(Error::Command { summary: r.summary(), code: r.code });
        }
        Ok(FastbootInfo { vars })
    }

    pub fn getvar(&self, serial: &str, var: &str) -> Result<String> {
        if !var.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':') {
            return Err(Error::Invalid(var.into()));
        }
        let r = self.run(Some(serial), &["getvar", var], Duration::from_secs(10))?;
        let vars = parsers::parse_fastboot_vars(&r.combined());
        vars.get(var).cloned().ok_or(Error::Command { summary: r.summary(), code: r.code })
    }

    /// Arguments of `fastboot flash` after validating everything. The vector is
    /// returned so the interface can show it before running.
    pub fn flash_args(&self, partition: &str, image: &Path, slot: Option<Slot>, confirm: Confirm) -> Result<Vec<String>> {
        validate_partition(partition)?;
        check_confirm(required_confirmation(partition), confirm)?;
        if !image.is_file() {
            return Err(Error::Invalid(t!("the image {} does not exist", image.display())));
        }
        let mut args = Vec::new();
        if let Some(s) = slot {
            args.push(format!("--slot={}", s.arg()));
        }
        args.extend(["flash".to_string(), partition.to_string(), image.display().to_string()]);
        Ok(args)
    }

    pub fn flash<F>(&self, serial: &str, partition: &str, image: &Path, slot: Option<Slot>, confirm: Confirm, on_event: F) -> Result<StreamHandle>
    where
        F: Fn(StreamEvent) + Send + Sync + 'static,
    {
        let args = self.flash_args(partition, image, slot, confirm)?;
        self.stream(serial, &args, on_event)
    }

    /// Boots an image without flashing it (`fastboot boot`).
    pub fn boot<F>(&self, serial: &str, image: &Path, on_event: F) -> Result<StreamHandle>
    where
        F: Fn(StreamEvent) + Send + Sync + 'static,
    {
        if !image.is_file() {
            return Err(Error::Invalid(t!("the image {} does not exist", image.display())));
        }
        self.stream(serial, &["boot".into(), image.display().to_string()], on_event)
    }

    pub fn set_active(&self, serial: &str, slot: Slot) -> Result<CommandResult> {
        if slot == Slot::All {
            return Err(Error::Invalid(tr("choose slot a or b").into()));
        }
        self.run(Some(serial), &["set_active", slot.arg()], Duration::from_secs(15))?.into_result()
    }

    pub fn erase(&self, serial: &str, partition: &str, confirm: Confirm) -> Result<CommandResult> {
        validate_partition(partition)?;
        check_confirm(Confirm::Destructive, confirm)?;
        self.run(Some(serial), &["erase", partition], Duration::from_secs(300))?.into_result()
    }

    /// Erases user data (`fastboot -w`).
    pub fn wipe_data(&self, serial: &str, confirm: Confirm) -> Result<CommandResult> {
        check_confirm(Confirm::Destructive, confirm)?;
        self.run(Some(serial), &["-w"], Duration::from_secs(300))?.into_result()
    }

    /// `reboot`, `reboot bootloader`, `reboot recovery`, `reboot fastboot`.
    pub fn reboot(&self, serial: &str, target: Option<&str>) -> Result<CommandResult> {
        let mut args = vec!["reboot"];
        match target {
            None => {}
            Some(t @ ("bootloader" | "recovery" | "fastboot")) => args.push(t),
            Some(other) => return Err(Error::Invalid(other.into())),
        }
        self.run(Some(serial), &args, Duration::from_secs(20))?.into_result()
    }

    /// Continues the normal boot from the bootloader.
    pub fn continue_boot(&self, serial: &str) -> Result<CommandResult> {
        self.run(Some(serial), &["continue"], Duration::from_secs(20))?.into_result()
    }

    /// `flashing unlock|lock`. Erases all the data on the phone.
    pub fn flashing(&self, serial: &str, unlock: bool, confirm: Confirm) -> Result<CommandResult> {
        check_confirm(Confirm::Destructive, confirm)?;
        let verb = if unlock { "unlock" } else { "lock" };
        self.run(Some(serial), &["flashing", verb], Duration::from_secs(120))
    }

    /// Read-only `oem <cmd>` (device-info, get_unlock_ability…).
    pub fn oem_info(&self, serial: &str, cmd: &str) -> Result<CommandResult> {
        const ALLOWED: [&str; 4] = ["device-info", "get_unlock_ability", "get_unlock_data", "lks"];
        if !ALLOWED.contains(&cmd) {
            return Err(Error::Guarded(t!("oem command not allowed: {}", cmd)));
        }
        self.run(Some(serial), &["oem", cmd], Duration::from_secs(15))
    }

    /// Runs free-form arguments from the fastboot terminal, blocking shell
    /// operators and unconfirmed destructive operations.
    pub fn validate_free_args(args: &str) -> Result<Vec<String>> {
        let list: Vec<String> = args.split_whitespace().map(String::from).collect();
        if list.is_empty() {
            return Err(Error::Invalid(tr("type a command").into()));
        }
        if list.iter().any(|a| a.chars().any(|c| "&|;><`$".contains(c))) {
            return Err(Error::Invalid(tr("shell operators are not allowed").into()));
        }
        let first = list.iter().find(|a| !a.starts_with('-')).map(String::as_str).unwrap_or("");
        if matches!(first, "erase" | "format" | "flashing" | "oem" | "flash" | "-w" | "update" | "flashall") || list.iter().any(|a| a == "-w") {
            return Err(Error::Guarded(t!("“{}” must be done from the Flasher, which validates and asks for confirmation", first)));
        }
        Ok(list)
    }
}

// ---------------------------------------------------------------------------
// Flashing plan for a ROM folder
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct FlashStep {
    pub partition: String,
    pub image: PathBuf,
    pub size: u64,
    pub exists: bool,
    pub critical: bool,
    pub confirm: Confirm,
    /// Extra options from the original script (e.g. `--disable-verity`).
    pub extra_args: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlashPlan {
    pub source: String,
    /// Models the ROM requires (`getvar product | grep "^product: *X"` in the
    /// script or `require board=`/`require product=` in android-info.txt).
    pub required_products: Vec<String>,
    pub steps: Vec<FlashStep>,
    pub warnings: Vec<String>,
    pub wipes_data: bool,
    pub set_active: Option<String>,
}

impl FlashPlan {
    pub fn total_bytes(&self) -> u64 {
        self.steps.iter().map(|s| s.size).sum()
    }
    pub fn needs(&self) -> Confirm {
        if self.wipes_data || self.steps.iter().any(|s| s.confirm == Confirm::Destructive) {
            Confirm::Destructive
        } else if self.steps.iter().any(|s| s.critical) {
            Confirm::Critical
        } else {
            Confirm::None
        }
    }
}

/// Reads `flash_all.sh` / `flash-all.sh` / `flash_all.bat` and extracts the
/// `fastboot flash` commands in order. It never runs the script.
pub fn parse_flash_script(text: &str, base: &Path) -> FlashPlan {
    let mut plan = FlashPlan::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') || line.starts_with("::") || line.to_lowercase().starts_with("rem ") {
            continue;
        }
        if let Some(req) = required_product_in_line(line) {
            if !plan.required_products.contains(&req) {
                plan.required_products.push(req);
            }
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(pos) = tokens.iter().position(|t| t.ends_with("fastboot") || t.ends_with("fastboot.exe") || t.ends_with("fastboot\"")) else {
            continue;
        };
        let rest: Vec<&str> = tokens[pos + 1..]
            .iter()
            .copied()
            .take_while(|t| !["||", "&&", "|", ">", "2>&1", ";"].contains(t))
            .filter(|t| *t != "%*" && !t.starts_with("$*") && !t.starts_with("\"$@\"") && *t != "$@")
            .collect();
        let mut extra = Vec::new();
        let mut words = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            let t = rest[i];
            if t == "-s" || t == "--slot" {
                i += 2;
                continue;
            }
            if t.starts_with("--disable-") || t.starts_with("--skip-") {
                extra.push(t.to_string());
            } else if t == "-w" {
                plan.wipes_data = true;
            } else if !t.starts_with('-') {
                words.push(t.trim_matches('"'));
            }
            i += 1;
        }
        match words.as_slice() {
            ["flash", part, file, ..] => {
                let file = file.replace("%~dp0", "").replace("$(dirname $0)/", "");
                let image = resolve_image(base, &file);
                let meta = image.metadata().ok();
                plan.steps.push(FlashStep {
                    partition: part.to_string(),
                    exists: meta.is_some(),
                    size: meta.map(|m| m.len()).unwrap_or(0),
                    critical: is_critical(part),
                    confirm: required_confirmation(part),
                    extra_args: extra,
                    image,
                });
            }
            ["erase", part, ..] | ["format", part, ..] => {
                if matches!(base_partition(part), "userdata" | "metadata") {
                    plan.wipes_data = true;
                }
            }
            ["set_active", slot, ..] | ["--set-active", slot, ..] => plan.set_active = Some(slot.to_string()),
            _ => {}
        }
    }
    plan
}

/// `fastboot getvar product 2>&1 | grep "^product: *NX809J"` → “NX809J”.
fn required_product_in_line(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    if !lower.contains("getvar product") {
        return None;
    }
    let idx = lower.find("product:")?;
    let after = line[idx + "product:".len()..].trim_start_matches([' ', '*']);
    let id: String = after.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')).collect();
    (!id.is_empty()).then_some(id)
}

/// `android-info.txt` of factory images: `require board=a|b` or `require
/// product=x`.
pub fn parse_android_info(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        for key in ["require board=", "require product="] {
            if let Some(v) = line.strip_prefix(key) {
                out.extend(v.split('|').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()));
            }
        }
    }
    out
}

/// Does fastboot's `product` match any of the known identifiers?
/// Case-insensitive and allowing region suffixes (`NX809J` ↔ `NX809J-UN`).
pub fn product_matches(product: &str, ids: &[String]) -> bool {
    let p = product.trim().to_ascii_lowercase();
    if p.is_empty() {
        return false;
    }
    ids.iter().any(|id| {
        let id = id.trim().to_ascii_lowercase();
        if id.is_empty() {
            return false;
        }
        let prefix = |a: &str, b: &str| a.len() > b.len() && a.starts_with(b) && matches!(a.as_bytes()[b.len()], b'-' | b'_');
        id == p || prefix(&id, &p) || prefix(&p, &id)
    })
}

fn resolve_image(base: &Path, file: &str) -> PathBuf {
    let candidate = base.join(file.trim_start_matches("./"));
    if candidate.exists() {
        return candidate;
    }
    let name = Path::new(file).file_name().map(|n| n.to_os_string()).unwrap_or_default();
    for sub in ["", "images"] {
        let c = base.join(sub).join(&name);
        if c.exists() {
            return c;
        }
    }
    candidate
}

/// Analyses a ROM folder: uses the official script if it exists and, if not,
/// proposes a plan from the known `.img` files (excluding critical ones).
pub fn scan_rom_folder(dir: &Path) -> Result<FlashPlan> {
    if !dir.is_dir() {
        return Err(Error::Invalid(t!("{} is not a folder", dir.display())));
    }
    for name in ["flash_all.sh", "flash-all.sh", "flash_all_except_storage.sh", "flash_all.bat", "flash-all.bat"] {
        let p = dir.join(name);
        if let Ok(text) = std::fs::read_to_string(&p) {
            let mut plan = parse_flash_script(&text, dir);
            if !plan.steps.is_empty() {
                plan.source = name.into();
                for s in plan.steps.iter().filter(|s| !s.exists) {
                    plan.warnings.push(t!("{} is missing for “{}”", s.image.display(), s.partition));
                }
                return Ok(plan);
            }
        }
    }
    let mut plan = FlashPlan { source: tr("images in the folder").into(), ..Default::default() };
    if dir.join("payload.bin").exists() {
        plan.warnings.push(tr("The ROM is a payload.bin (OTA). The images must be extracted before flashing them, or use “adb sideload”.").into());
    }
    let mut images: BTreeMap<String, PathBuf> = BTreeMap::new();
    for sub in [dir.to_path_buf(), dir.join("images")] {
        if let Ok(rd) = std::fs::read_dir(&sub) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "img") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        images.entry(stem.to_string()).or_insert(p);
                    }
                }
            }
        }
    }
    for part in COMMON_PARTITIONS {
        if part == "userdata" {
            continue;
        }
        if let Some(img) = images.remove(part) {
            let size = img.metadata().map(|m| m.len()).unwrap_or(0);
            plan.steps.push(FlashStep { partition: part.into(), image: img, size, exists: true, critical: false, confirm: Confirm::None, extra_args: Vec::new() });
        }
    }
    let skipped: Vec<String> = images.keys().cloned().collect();
    if !skipped.is_empty() {
        plan.warnings.push(t!("Not included automatically: {}. Flash them one by one if you know what they do.", skipped.join(", ")));
    }
    if plan.steps.is_empty() && plan.warnings.is_empty() {
        plan.warnings.push(tr("No recognisable .img images were found.").into());
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_rules() {
        assert!(validate_partition("boot_a").is_ok());
        assert!(validate_partition("../boot").is_err());
        assert!(validate_partition("Boot").is_err());
        assert!(validate_partition("").is_err());
        assert!(is_critical("xbl_a"));
        assert!(!is_critical("boot"));
        assert_eq!(required_confirmation("userdata"), Confirm::Destructive);
        assert_eq!(required_confirmation("modem_b"), Confirm::Critical);
        assert_eq!(required_confirmation("boot"), Confirm::None);
        assert!(check_confirm(Confirm::Critical, Confirm::None).is_err());
        assert!(check_confirm(Confirm::Critical, Confirm::Destructive).is_ok());
    }

    #[test]
    fn free_args_guard() {
        assert!(Fastboot::validate_free_args("getvar all").is_ok());
        assert!(Fastboot::validate_free_args("devices; rm -rf /").is_err());
        assert!(Fastboot::validate_free_args("erase userdata").is_err());
        assert!(Fastboot::validate_free_args("-s X -w").is_err());
        assert!(Fastboot::validate_free_args("flashing unlock").is_err());
    }

    #[test]
    fn info_helpers() {
        let vars = parsers::parse_fastboot_vars(
            "(bootloader) current-slot:b\n(bootloader) slot-count:2\n(bootloader) unlocked:yes\n(bootloader) partition-size:boot_b: 0x6000000\n(bootloader) is-userspace:no\n",
        );
        let info = FastbootInfo { vars };
        assert_eq!(info.current_slot().as_deref(), Some("b"));
        assert!(info.has_slots());
        assert_eq!(info.unlocked(), Some(true));
        assert_eq!(info.partition_size("boot_b"), Some(0x6000000));
        assert!(!info.is_userspace());
    }

    #[test]
    fn flash_script_plan() {
        let dir = std::env::temp_dir().join(format!("adbscan-rom-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::write(dir.join("images/boot.img"), vec![0u8; 10]).unwrap();
        std::fs::write(dir.join("images/xbl.img"), vec![0u8; 5]).unwrap();
        let script = "#!/bin/sh\n\
            fastboot $* getvar product 2>&1 | grep \"^product: *star\"\n\
            fastboot $* flash xbl_ab images/xbl.img || exit 1\n\
            fastboot $* flash boot_ab images/boot.img\n\
            fastboot $* --disable-verity flash vbmeta_ab images/vbmeta.img\n\
            fastboot $* erase userdata\n\
            fastboot $* set_active a\n";
        let plan = parse_flash_script(script, &dir);
        let parts: Vec<_> = plan.steps.iter().map(|s| s.partition.as_str()).collect();
        assert_eq!(parts, vec!["xbl_ab", "boot_ab", "vbmeta_ab"]);
        assert!(plan.steps[0].critical);
        assert!(plan.steps[1].exists);
        assert!(!plan.steps[2].exists);
        assert_eq!(plan.steps[2].extra_args, vec!["--disable-verity"]);
        assert!(plan.wipes_data);
        assert_eq!(plan.set_active.as_deref(), Some("a"));
        assert_eq!(plan.needs(), Confirm::Destructive);
        assert_eq!(plan.total_bytes(), 15);
        assert_eq!(plan.required_products, vec!["star".to_string()]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn folder_plan_without_script() {
        let dir = std::env::temp_dir().join(format!("adbscan-rom2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["boot.img", "dtbo.img", "abl.img", "vendor_boot.img"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let plan = scan_rom_folder(&dir).unwrap();
        let parts: Vec<_> = plan.steps.iter().map(|s| s.partition.as_str()).collect();
        assert_eq!(parts, vec!["boot", "vendor_boot", "dtbo"]);
        assert!(plan.warnings.iter().any(|w| w.contains("abl")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn product_identity() {
        let ids = vec!["NX809J".to_string(), "NX809J-UN".to_string()];
        assert!(product_matches("NX809J", &ids));
        assert!(product_matches("nx809j", &ids));
        assert!(product_matches("NX809J-EEA", &["NX809J".into()]));
        assert!(!product_matches("NX809", &ids));
        assert!(!product_matches("NX769J", &ids));
        assert!(!product_matches("", &ids));
        assert_eq!(parse_android_info("require board=shiba|husky\nrequire version-bootloader=x\n"), vec!["shiba", "husky"]);
    }
}
