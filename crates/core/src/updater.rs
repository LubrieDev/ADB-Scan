//! Checks GitHub Releases for a newer version of ADB Scan and, when the user
//! asks, downloads and installs it.
//!
//! The check is one request to the latest release of the repository. Anything
//! that goes wrong (no release yet, no Internet, timeout, unexpected JSON, a
//! tag that is not a version) means “no update”: it never fails loudly.
//!
//! Nothing is downloaded until the user presses “Update now”. Then the
//! release's file for this system (the `.deb` on Linux, the installer on
//! Windows) is downloaded from this repository's releases only, and it is
//! installed only if its SHA-256 sum is the one GitHub publishes for it.
//! Without that sum, or for a copy that was not installed by those packages
//! (a build of your own, the portable zip), only the release page is offered.

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::{i18n::tr, t};

/// Latest published release (drafts and pre-releases are not included).
pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/LubrieDev/ADB-Scan/releases/latest";

/// Pages a release link may point to (the link is opened in the browser).
pub const RELEASES_PAGE: &str = "https://github.com/LubrieDev/ADB-Scan/releases/";

/// The whole request, connection included.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// Where a release's files may be downloaded from.
pub const DOWNLOADS: &str = "https://github.com/LubrieDev/ADB-Scan/releases/download/";

/// The answer is a few KiB; anything much bigger is not a release.
const MAX_BODY_BYTES: u64 = 1024 * 1024;

/// A package bigger than this is not ADB Scan's (the installer is ~35 MB).
const MAX_PACKAGE_BYTES: u64 = 500 * 1024 * 1024;

/// A newer version than the one running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: Version,
    /// Release page on GitHub.
    pub url: String,
    /// The file that installs it on this system, if the release has one
    /// with its SHA-256 sum.
    pub package: Option<Package>,
}

/// A release file that installs ADB Scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// File name, also used for the downloaded copy.
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hexadecimal.
    pub sha256: String,
}

/// The kind of package each system installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// `adb-scan_1.2.0-1_amd64.deb`, installed with apt.
    LinuxDeb,
    /// `ADB-Scan-1.2.0-setup-x64.exe`, the NSIS installer.
    WindowsSetup,
}

impl Platform {
    /// The platform of this build, if its packages are published.
    pub fn current() -> Option<Platform> {
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Some(Platform::LinuxDeb)
        } else if cfg!(all(windows, target_arch = "x86_64")) {
            Some(Platform::WindowsSetup)
        } else {
            None
        }
    }

    fn matches(self, name: &str) -> bool {
        match self {
            Platform::LinuxDeb => name.starts_with("adb-scan_") && name.ends_with("_amd64.deb"),
            Platform::WindowsSetup => name.starts_with("ADB-Scan-") && name.ends_with("-setup-x64.exe"),
        }
    }
}

/// The fields used from GitHub's answer.
#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    /// `sha256:<hex>`, computed by GitHub when the file was uploaded.
    #[serde(default)]
    digest: Option<String>,
}

/// The package for `platform` among a release's files: from this
/// repository's releases of `tag`, with a plain file name, a plausible size
/// and a SHA-256 sum.
fn package_for(assets: &[Asset], tag: &str, platform: Platform) -> Option<Package> {
    assets.iter().find_map(|a| {
        let plain = !a.name.is_empty() && a.name.chars().all(|c| c.is_ascii_alphanumeric() || "._-+".contains(c)) && !a.name.starts_with('.');
        let url_ok = a.browser_download_url == format!("{DOWNLOADS}{tag}/{}", a.name);
        let sha256 = a.digest.as_deref()?.strip_prefix("sha256:")?.to_ascii_lowercase();
        let sum_ok = sha256.len() == 64 && sha256.chars().all(|c| c.is_ascii_hexdigit());
        let size_ok = (1..=MAX_PACKAGE_BYTES).contains(&a.size);
        (plain && platform.matches(&a.name) && url_ok && sum_ok && size_ok).then(|| Package { name: a.name.clone(), url: a.browser_download_url.clone(), size: a.size, sha256 })
    })
}

/// Version of a tag: `v1.2.3` or `1.2.3`. `None` if it is not semver.
pub fn tag_version(tag: &str) -> Option<Version> {
    let tag = tag.trim();
    Version::parse(tag.strip_prefix(['v', 'V']).unwrap_or(tag)).ok()
}

/// The release's version if it is newer than `current`.
pub fn newer_version(current: &str, tag: &str) -> Option<Version> {
    let current = Version::parse(current.trim()).ok()?;
    tag_version(tag).filter(|latest| *latest > current)
}

/// Reads GitHub's answer for the latest release and compares it with
/// `current`. `None` if it cannot be read, is not newer, or its link does not
/// point to this repository's releases.
pub fn update_from_json(json: &str, current: &str) -> Option<Update> {
    update_from_json_for(json, current, Platform::current())
}

/// [`update_from_json`] for a given platform (`None`: no package).
pub fn update_from_json_for(json: &str, current: &str, platform: Option<Platform>) -> Option<Update> {
    let release: Release = serde_json::from_str(json).ok()?;
    if !release.html_url.starts_with(RELEASES_PAGE) {
        return None;
    }
    let version = newer_version(current, &release.tag_name)?;
    let package = platform.and_then(|p| package_for(&release.assets, &release.tag_name, p));
    Some(Update { version, url: release.html_url, package })
}

/// Asks GitHub for the latest release (blocking, at most `TIMEOUT`): run it
/// off the interface thread. `None` when there is no newer version or the
/// check did not work.
pub fn check(current: &str) -> Option<Update> {
    match fetch_latest() {
        Ok(json) => update_from_json(&json, current),
        Err(e) => {
            log::debug!("update check skipped: {e}");
            None
        }
    }
}

fn fetch_latest() -> std::result::Result<String, ureq::Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(concat!("ADB-Scan/", env!("CARGO_PKG_VERSION"), " (+https://github.com/LubrieDev/ADB-Scan)"))
        .build()
        .into();
    // A 404 (no release yet) or any other error status is an `Err`.
    let mut response = agent.get(LATEST_RELEASE_URL).header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28").call()?;
    response.body_mut().with_config().limit(MAX_BODY_BYTES).read_to_string()
}

// -- download ----------------------------------------------------------------------

/// Folder for the downloaded package.
pub fn download_dir() -> PathBuf {
    crate::paths::cache_dir().join("update")
}

/// Deletes the files left in [`download_dir`] by an earlier update (the
/// installer is not needed once it has run). Only files, by name.
pub fn clean_downloads() {
    let Ok(entries) = std::fs::read_dir(download_dir()) else { return };
    for e in entries.flatten() {
        if e.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Downloads `pkg` into [`download_dir`], reporting `(bytes, total)`, and
/// checks its size and SHA-256 sum. Returns the file only if both match;
/// otherwise nothing is left behind. `cancel` stops it between blocks.
pub fn download(pkg: &Package, progress: &dyn Fn(u64, u64), cancel: &AtomicBool) -> Result<PathBuf> {
    let dir = download_dir();
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(&pkg.name);
    let part = dir.join(format!("{}.part", pkg.name));
    let r = fetch_to(pkg, &part, progress, cancel);
    match r {
        Ok(()) => {
            std::fs::rename(&part, &target)?;
            Ok(target)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

fn fetch_to(pkg: &Package, part: &Path, progress: &dyn Fn(u64, u64), cancel: &AtomicBool) -> Result<()> {
    let net = |e: ureq::Error| Error::Invalid(t!("could not download {}: {}", pkg.name, e));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(30 * 60)))
        .user_agent(concat!("ADB-Scan/", env!("CARGO_PKG_VERSION"), " (+https://github.com/LubrieDev/ADB-Scan)"))
        .build()
        .into();
    let mut response = agent.get(&pkg.url).call().map_err(net)?;
    let mut reader = response.body_mut().with_config().limit(pkg.size + 1).reader();
    let mut file = std::fs::File::create(part)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut got = 0u64;
    progress(0, pkg.size);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Invalid(t!("the download of {} was cancelled", pkg.name)));
        }
        let n = reader.read(&mut buf).map_err(|e| Error::Invalid(t!("could not download {}: {}", pkg.name, e)))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        got += n as u64;
        progress(got.min(pkg.size), pkg.size);
    }
    file.sync_all()?;
    if got != pkg.size {
        return Err(Error::Invalid(t!("{} arrived incomplete ({} of {} bytes); nothing was installed", pkg.name, got, pkg.size)));
    }
    let sum: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if sum != pkg.sha256 {
        return Err(Error::Invalid(t!("the SHA-256 sum of {} does not match (expected {}, got {}); nothing was installed", pkg.name, pkg.sha256, sum)));
    }
    Ok(())
}

// -- install -----------------------------------------------------------------------

/// Where the `.deb` puts the program.
#[cfg(unix)]
const DEB_EXE: &str = "/usr/bin/adb-scan";

/// The running executable. On Linux, once the package is replaced, the
/// kernel reports the old file as “… (deleted)”.
pub fn running_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let text = exe.to_string_lossy();
    Some(match text.strip_suffix(" (deleted)") {
        Some(clean) => PathBuf::from(clean),
        None => exe,
    })
}

/// Whether this copy can update itself: on Linux, the one installed from the
/// `.deb` (with `pkexec` to ask for the password and apt to install); on
/// Windows, one installed with the installer (its `uninstall.exe` next to it).
pub fn can_install() -> bool {
    #[cfg(unix)]
    {
        running_exe().is_some_and(|e| e == Path::new(DEB_EXE)) && Path::new("/usr/bin/pkexec").exists() && Path::new("/usr/bin/apt-get").exists()
    }
    #[cfg(windows)]
    {
        installed_dir().is_some()
    }
}

/// Folder of a copy installed with the Windows installer.
#[cfg(windows)]
pub fn installed_dir() -> Option<PathBuf> {
    let dir = running_exe()?.parent()?.to_path_buf();
    dir.join("uninstall.exe").is_file().then_some(dir)
}

/// Installs the downloaded `.deb` with apt, as administrator: `pkexec` asks
/// for the password in a window of the desktop. Blocks until apt finishes.
#[cfg(unix)]
pub fn install_deb(deb: &Path) -> Result<()> {
    let out = std::process::Command::new("/usr/bin/pkexec")
        .args(["/usr/bin/apt-get", "install", "-y"])
        // Absolute (it starts with “/”): apt reads it as a file, not a package
        // name or an option.
        .arg(deb)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| Error::Spawn { program: "pkexec".into(), source: e })?;
    match out.status.code() {
        Some(0) => Ok(()),
        // pkexec: the password window was closed, or the user is not allowed.
        Some(126) | Some(127) => Err(Error::Invalid(tr("the administrator password was not given; nothing was installed").into())),
        code => {
            let err = String::from_utf8_lossy(&out.stderr);
            let last: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty() && !l.starts_with("N:") && !l.starts_with("W:")).collect();
            let summary = last.iter().rev().take(3).rev().copied().collect::<Vec<_>>().join(" · ");
            Err(Error::Command { summary: if summary.is_empty() { t!("apt failed (code {})", code.unwrap_or(-1)) } else { summary }, code })
        }
    }
}

/// Starts the downloaded installer silently over the installed copy. It waits
/// for ADB Scan to close, installs, and opens it again (`/RUN`): the caller
/// quits right after this.
#[cfg(windows)]
pub fn start_installer(setup: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    let dir = installed_dir().ok_or_else(|| Error::Invalid(tr("this copy of ADB Scan was not installed with the installer").into()))?;
    // NSIS wants /D= last and unquoted, even with spaces.
    std::process::Command::new(setup)
        .args(["/S", "/RUN"])
        .raw_arg(format!("/D={}", dir.display()))
        .spawn()
        .map(|_| ())
        .map_err(|e| Error::Spawn { program: setup.display().to_string(), source: e })
}
