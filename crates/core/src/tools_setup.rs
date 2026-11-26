//! Managed tools: Google's official platform-tools (adb and fastboot) and
//! scrcpy, downloaded to the user's data folder
//! (`~/.local/share/adb-scan/tools`), without root and without touching the
//! system ones.
//!
//! Each package has its version and SHA-256 sum pinned here: whatever does not
//! match is not extracted. The download is done by `curl` (HTTPS only) and the
//! installation is atomic: the previous version stays in place until the new
//! one is complete. [`crate::adb::find_tool`] looks in this folder after the
//! configured path and the PATH.

use crate::{i18n::tr, t};
use crate::adb::{tool_version, Tool};
use crate::error::{Error, Result};
use crate::exec::{self, StreamEvent, StreamOptions};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archive {
    Zip,
    TarGz,
}

/// A downloadable package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Package {
    pub id: &'static str,
    pub title: &'static str,
    pub version: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    /// Size of the compressed file (for the progress).
    pub size: u64,
    pub archive: Archive,
    /// Top-level folder inside the archive.
    pub top_dir: &'static str,
    /// Folder inside [`crate::paths::managed_tools_dir`] (the one `find_tool` looks in).
    pub dir: &'static str,
    pub provides: &'static [Tool],
}

/// Google's adb and fastboot (version checked by hand: sum computed over the
/// official archive).
pub const PLATFORM_TOOLS: Package = Package {
    id: "platform-tools",
    title: "Google platform-tools",
    version: "37.0.1",
    url: "https://dl.google.com/android/repository/platform-tools_r37.0.1-linux.zip",
    sha256: "d230f13842f60f782a8645f9c813f8f845bf36089ea7289f28c48f17979313f1",
    size: 9_054_187,
    archive: Archive::Zip,
    top_dir: "platform-tools",
    dir: "platform-tools",
    provides: &[Tool::Adb, Tool::Fastboot],
};

/// Official scrcpy (Genymobile), statically built with its `scrcpy-server`.
pub const SCRCPY: Package = Package {
    id: "scrcpy",
    title: "scrcpy",
    version: "5.0.1",
    url: "https://github.com/Genymobile/scrcpy/releases/download/v5.0.1/scrcpy-linux-x86_64-v5.0.1.tar.gz",
    sha256: "9f969d30cc574816077edecda65719c1c70b0aee1df1ba5fa00779ac07b8fd6e",
    size: 17_883_742,
    archive: Archive::TarGz,
    top_dir: "scrcpy-linux-x86_64-v5.0.1",
    dir: "scrcpy",
    provides: &[Tool::Scrcpy],
};

pub const PACKAGES: [Package; 2] = [PLATFORM_TOOLS, SCRCPY];

/// Google and Genymobile only publish these tools for Linux x86-64.
pub fn supported() -> bool {
    cfg!(all(target_os = "linux", target_arch = "x86_64"))
}

pub fn package_for(tool: Tool) -> Package {
    match tool {
        Tool::Scrcpy => SCRCPY,
        _ => PLATFORM_TOOLS,
    }
}

/// File with the installed version inside the package folder.
const VERSION_FILE: &str = ".adb-scan-version";

/// Folder where a package is installed.
pub fn install_dir(p: &Package) -> PathBuf {
    crate::paths::managed_tools_dir().join(p.dir)
}

/// Installed managed version, if any.
pub fn installed_version(p: &Package) -> Option<String> {
    std::fs::read_to_string(install_dir(p).join(VERSION_FILE)).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Path of a tool inside the managed copy (whether it exists or not).
pub fn managed_path(tool: Tool) -> PathBuf {
    install_dir(&package_for(tool)).join(exec::exe_name(tool.name()))
}

/// Where a found tool is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Path chosen in Preferences.
    Configured,
    /// Downloaded by ADB Scan.
    Managed,
    /// From the system (PATH or a usual location).
    System,
    /// Next to ADB Scan's executable (the Windows zip).
    Bundled,
}

/// State of a tool for the assistant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolState {
    pub tool: Tool,
    /// Path and where it comes from; `None` if it is not there.
    pub found: Option<(PathBuf, Origin)>,
    pub version: Option<String>,
}

/// Looks for a tool the way the app does and says where it comes from.
pub fn tool_state(tool: Tool, configured: &str) -> ToolState {
    let path = crate::adb::find_tool(tool, Some(configured));
    let bundled = crate::adb::bundled_tool(tool);
    let origin = path.as_ref().map(|p| {
        if bundled.as_ref() == Some(p) {
            Origin::Bundled
        } else if *p == managed_path(tool) {
            Origin::Managed
        } else if !configured.trim().is_empty() {
            Origin::Configured
        } else {
            Origin::System
        }
    });
    let version = path.as_ref().and_then(|p| tool_version(p, tool));
    ToolState { tool, found: path.zip(origin), version }
}

/// Phase of an installation, to show the progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stage {
    /// Bytes received and expected total.
    Downloading(u64, u64),
    Verifying,
    Extracting,
}

/// Downloads, verifies and installs `p`. `progress` is called from this thread;
/// `cancel` stops the download. Returns the installed folder. Blocking: call it
/// in the background.
pub fn install(p: &Package, progress: &dyn Fn(Stage), cancel: &AtomicBool) -> Result<PathBuf> {
    if !supported() {
        return Err(Error::Invalid(tr("Google and scrcpy only publish these tools for 64-bit Linux (x86-64)").into()));
    }
    let root = crate::paths::managed_tools_dir();
    let staging = root.join(format!(".download-{}", p.id));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let result = install_in(p, &root, &staging, progress, cancel);
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn install_in(p: &Package, root: &Path, staging: &Path, progress: &dyn Fn(Stage), cancel: &AtomicBool) -> Result<PathBuf> {
    let archive = staging.join(tr("file"));
    download(p.url, &archive, p.size, progress, cancel)?;
    progress(Stage::Verifying);
    let sum = sha256_file(&archive)?;
    if !sum.eq_ignore_ascii_case(p.sha256) {
        return Err(Error::Invalid(t!("the SHA-256 sum of {} does not match (expected {}, got {}); nothing was installed", p.title, p.sha256, sum)));
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(cancelled());
    }
    progress(Stage::Extracting);
    let out = staging.join("x");
    std::fs::create_dir_all(&out)?;
    match p.archive {
        Archive::Zip => extract_zip(&archive, &out)?,
        Archive::TarGz => extract_tar_gz(&archive, &out)?,
    }
    let fresh = out.join(p.top_dir);
    for tool in p.provides {
        if !exec::is_executable(&fresh.join(exec::exe_name(tool.name()))) {
            return Err(Error::Invalid(t!("the package does not include an executable {}", tool.name())));
        }
    }
    std::fs::write(fresh.join(VERSION_FILE), p.version)?;
    // Atomic switch: the previous copy is moved aside and only deleted at the
    // end.
    let dest = root.join(p.dir);
    let old = root.join(format!(".anterior-{}", p.id));
    let _ = std::fs::remove_dir_all(&old);
    let had_old = dest.exists();
    if had_old {
        std::fs::rename(&dest, &old)?;
    }
    if let Err(e) = std::fs::rename(&fresh, &dest) {
        if had_old {
            let _ = std::fs::rename(&old, &dest);
        }
        return Err(e.into());
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(dest)
}

fn cancelled() -> Error {
    Error::Invalid(tr("download cancelled").into())
}

/// Download with `curl` (HTTPS only, follows redirects, fails on HTTP errors)
/// and reports progress by the file's size.
fn download(url: &str, to: &Path, expected: u64, progress: &dyn Fn(Stage), cancel: &AtomicBool) -> Result<()> {
    let curl = exec::which("curl").ok_or(Error::ToolNotFound { tool: "curl".into() })?;
    let args: Vec<String> = ["-fsSL", "--proto", "=https", "--tlsv1.2", "--retry", "2", "--connect-timeout", "20", "-o"]
        .iter()
        .map(|s| s.to_string())
        .chain([to.display().to_string(), url.to_string()])
        .collect();
    let (tx, rx) = mpsc::channel();
    let errors = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let errs = errors.clone();
    let handle = exec::stream(&curl, &args, StreamOptions::default(), move |ev| match ev {
        StreamEvent::Stderr(l) => {
            let mut e = errs.lock().unwrap();
            e.push_str(l.trim());
            e.push(' ');
        }
        StreamEvent::Exit { code, cancelled } => {
            let _ = tx.send((code, cancelled));
        }
        StreamEvent::Stdout(_) => {}
    })?;
    loop {
        match rx.recv_timeout(Duration::from_millis(150)) {
            Ok((code, was_cancelled)) => {
                if was_cancelled || cancel.load(Ordering::SeqCst) {
                    return Err(cancelled());
                }
                if code != Some(0) {
                    let e = errors.lock().unwrap().trim().to_string();
                    return Err(Error::Command { summary: t!("could not download: {}", if e.is_empty() { tr("network error") } else { &e }), code });
                }
                progress(Stage::Downloading(expected, expected));
                return Ok(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if cancel.load(Ordering::SeqCst) {
                    handle.cancel();
                }
                let got = std::fs::metadata(to).map(|m| m.len()).unwrap_or(0);
                progress(Stage::Downloading(got.min(expected), expected));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(cancelled()),
        }
    }
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Safe relative path (no `..` and not absolute) inside `dest`.
fn safe_join(dest: &Path, rel: &Path) -> Option<PathBuf> {
    let ok = rel.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    (ok && rel.components().next().is_some()).then(|| dest.join(rel))
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let bad = |e: zip::result::ZipError| Error::Invalid(t!("invalid ZIP archive: {}", e));
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(std::fs::File::open(archive)?)).map_err(bad)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(bad)?;
        let Some(out) = entry.enclosed_name().and_then(|n| safe_join(dest, &n)) else { continue };
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut f)?;
        // Keeps the executable bit (adb, fastboot…); no setuid.
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o755))?;
        }
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<()> {
    let gz = flate2::read::GzDecoder::new(std::io::BufReader::new(std::fs::File::open(archive)?));
    let mut tar = tar::Archive::new(gz);
    tar.set_preserve_permissions(false);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        // Only folders and regular files: no links or devices.
        if !(kind.is_dir() || kind.is_file()) {
            continue;
        }
        let rel = entry.path()?.into_owned();
        if safe_join(dest, &rel).is_none() {
            continue;
        }
        // `unpack_in` checks again that nothing escapes `dest`.
        entry.unpack_in(dest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("adbscan-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn packages_are_pinned() {
        for p in PACKAGES {
            assert!(p.url.starts_with("https://"), "{}", p.id);
            assert_eq!(p.sha256.len(), 64, "{}", p.id);
            assert!(p.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", p.id);
            assert!(p.url.contains(p.version), "{}", p.id);
        }
        assert_eq!(package_for(Tool::Fastboot), PLATFORM_TOOLS);
        assert_eq!(package_for(Tool::Scrcpy), SCRCPY);
    }

    #[test]
    fn hashes_files() {
        let d = scratch("hash");
        let f = d.join("a");
        std::fs::write(&f, b"abc").unwrap();
        assert_eq!(sha256_file(&f).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rejects_escaping_paths() {
        let d = Path::new("/x");
        assert!(safe_join(d, Path::new("../etc/passwd")).is_none());
        assert!(safe_join(d, Path::new("/etc/passwd")).is_none());
        assert!(safe_join(d, Path::new("")).is_none());
        assert_eq!(safe_join(d, Path::new("a/b")), Some(PathBuf::from("/x/a/b")));
    }

    #[test]
    #[cfg(unix)]
    fn zip_keeps_exec_bit_and_skips_traversal() {
        let d = scratch("zip");
        let archive = d.join("t.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            let exe = zip::write::SimpleFileOptions::default().unix_permissions(0o4755);
            z.start_file("pt/adb", exe).unwrap();
            z.write_all(b"#!/bin/sh\n").unwrap();
            z.start_file("../outside", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        let out = d.join("out");
        std::fs::create_dir_all(&out).unwrap();
        extract_zip(&archive, &out).unwrap();
        let mode = std::fs::metadata(out.join("pt/adb")).unwrap().permissions().mode() & 0o7777;
        // Executable, but without setuid.
        assert_eq!(mode, 0o755);
        assert!(!d.join("outside").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tar_gz_extracts_regular_files_only() {
        let d = scratch("tar");
        let archive = d.join("t.tar.gz");
        {
            let gz = flate2::write::GzEncoder::new(std::fs::File::create(&archive).unwrap(), flate2::Compression::fast());
            let mut t = tar::Builder::new(gz);
            let mut h = tar::Header::new_gnu();
            h.set_size(3);
            h.set_mode(0o755);
            h.set_cksum();
            t.append_data(&mut h, "s/scrcpy", &b"abc"[..]).unwrap();
            let mut l = tar::Header::new_gnu();
            l.set_entry_type(tar::EntryType::Symlink);
            l.set_size(0);
            l.set_cksum();
            t.append_link(&mut l, "s/link", "/etc/passwd").unwrap();
            t.into_inner().unwrap().finish().unwrap();
        }
        let out = d.join("out");
        std::fs::create_dir_all(&out).unwrap();
        extract_tar_gz(&archive, &out).unwrap();
        assert_eq!(std::fs::read(out.join("s/scrcpy")).unwrap(), b"abc");
        assert!(std::fs::symlink_metadata(out.join("s/link")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
