//! Update check: version comparison and GitHub's answer, without the network.

use adbscan_core::updater::*;
use semver::Version;

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}

/// Trimmed answer of `GET /repos/{owner}/{repo}/releases/latest`.
fn release(tag: &str, url: &str) -> String {
    format!(
        r#"{{
  "url": "https://api.github.com/repos/LubrieDev/ADB-Scan/releases/1",
  "html_url": "{url}",
  "id": 1,
  "tag_name": "{tag}",
  "target_commitish": "main",
  "name": "ADB Scan {tag}",
  "draft": false,
  "prerelease": false,
  "created_at": "2026-11-24T10:00:00Z",
  "published_at": "2026-11-24T10:05:00Z",
  "assets": [
    {{
      "name": "adb-scan_1.1.0-1_amd64.deb",
      "size": 4200000,
      "browser_download_url": "https://github.com/LubrieDev/ADB-Scan/releases/download/{tag}/adb-scan_1.1.0-1_amd64.deb"
    }}
  ],
  "body": "Notes"
}}"#
    )
}

const PAGE: &str = "https://github.com/LubrieDev/ADB-Scan/releases/tag/v1.1.0";

#[test]
fn tags_with_and_without_v() {
    assert_eq!(tag_version("v1.2.3"), Some(v("1.2.3")));
    assert_eq!(tag_version("1.2.3"), Some(v("1.2.3")));
    assert_eq!(tag_version(" V2.0.0 "), Some(v("2.0.0")));
    assert_eq!(tag_version("v1.2.3-beta.1"), Some(v("1.2.3-beta.1")));
}

#[test]
fn tags_that_are_not_versions() {
    for tag in ["", "v", "latest", "v1.2", "1", "vv1.2.3", "release-1.2.3", "v1.2.3.4"] {
        assert_eq!(tag_version(tag), None, "{tag:?}");
    }
}

#[test]
fn only_newer_versions_count() {
    assert_eq!(newer_version("1.0.0", "v1.0.1"), Some(v("1.0.1")));
    assert_eq!(newer_version("1.0.0", "v1.1.0"), Some(v("1.1.0")));
    assert_eq!(newer_version("1.9.0", "v1.10.0"), Some(v("1.10.0")), "numeric, not text order");
    assert_eq!(newer_version("1.0.0", "v2.0.0"), Some(v("2.0.0")));
    assert_eq!(newer_version("1.0.0", "v1.0.0"), None, "same version");
    assert_eq!(newer_version("1.1.0", "v1.0.9"), None, "older release");
}

#[test]
fn pre_releases_follow_semver() {
    // A final version is newer than its pre-releases, and the other way round.
    assert_eq!(newer_version("1.1.0-beta.1", "v1.1.0"), Some(v("1.1.0")));
    assert_eq!(newer_version("1.1.0", "v1.1.0-beta.2"), None);
    assert_eq!(newer_version("1.0.0", "v1.1.0-rc.1"), Some(v("1.1.0-rc.1")));
}

#[test]
fn a_bad_current_version_means_no_update() {
    assert_eq!(newer_version("dev", "v9.9.9"), None);
    assert_eq!(newer_version("", "v9.9.9"), None);
}

#[test]
fn newer_release_from_json() {
    let update = update_from_json(&release("v1.1.0", PAGE), "1.0.0").unwrap();
    assert_eq!(update.version, v("1.1.0"));
    assert_eq!(update.url, PAGE);
}

#[test]
fn same_or_older_release_from_json() {
    assert_eq!(update_from_json(&release("v1.0.0", PAGE), "1.0.0"), None);
    assert_eq!(update_from_json(&release("v0.9.0", PAGE), "1.0.0"), None);
}

#[test]
fn only_the_fields_used_are_needed() {
    let json = format!(r#"{{"tag_name": "v1.1.0", "html_url": "{PAGE}"}}"#);
    assert!(update_from_json(&json, "1.0.0").is_some());
}

#[test]
fn unexpected_answers_mean_no_update() {
    let cases = [
        String::new(),
        "not json".into(),
        "[]".into(),
        "null".into(),
        // GitHub's answer when the repository has no release yet (404).
        r#"{"message": "Not Found", "documentation_url": "https://docs.github.com/rest/releases/releases#get-the-latest-release", "status": "404"}"#.into(),
        // Rate limit (403).
        r#"{"message": "API rate limit exceeded for 203.0.113.7."}"#.into(),
        format!(r#"{{"tag_name": 110, "html_url": "{PAGE}"}}"#),
        format!(r#"{{"tag_name": null, "html_url": "{PAGE}"}}"#),
        r#"{"tag_name": "v1.1.0"}"#.into(),
        release("nightly", PAGE),
        release("v1.1", PAGE),
        // Cut short.
        release("v1.1.0", PAGE)[..120].to_string(),
    ];
    for json in &cases {
        assert_eq!(update_from_json(json, "1.0.0"), None, "{json}");
    }
}

#[test]
fn links_outside_the_repository_are_ignored() {
    for url in ["https://example.com/LubrieDev/ADB-Scan/releases/tag/v1.1.0", "http://github.com/LubrieDev/ADB-Scan/releases/tag/v1.1.0", "https://github.com/someone/ADB-Scan/releases/tag/v1.1.0", "javascript:alert(1)", ""] {
        assert_eq!(update_from_json(&release("v1.1.0", url), "1.0.0"), None, "{url}");
    }
}

const SUM: &str = "b12a2c4ee8be317422451fc7dcf8ee20a71b5ea7ef9ad73ddd825a25316227a5";

/// A release of `v1.2.0` with the given files: (name, url, size, digest).
fn release_with(assets: &[(&str, &str, u64, Option<&str>)]) -> String {
    let files: Vec<String> = assets
        .iter()
        .map(|(name, url, size, digest)| {
            let digest = digest.map(|d| format!(r#", "digest": "{d}""#)).unwrap_or_default();
            format!(r#"{{"name": "{name}", "size": {size}, "browser_download_url": "{url}"{digest}}}"#)
        })
        .collect();
    format!(r#"{{"tag_name": "v1.2.0", "html_url": "https://github.com/LubrieDev/ADB-Scan/releases/tag/v1.2.0", "assets": [{}]}}"#, files.join(", "))
}

const DEB: &str = "adb-scan_1.2.0-1_amd64.deb";
const DEB_URL: &str = "https://github.com/LubrieDev/ADB-Scan/releases/download/v1.2.0/adb-scan_1.2.0-1_amd64.deb";
const SETUP: &str = "ADB-Scan-1.2.0-setup-x64.exe";
const SETUP_URL: &str = "https://github.com/LubrieDev/ADB-Scan/releases/download/v1.2.0/ADB-Scan-1.2.0-setup-x64.exe";

#[test]
fn each_system_gets_its_package() {
    let digest = format!("sha256:{}", SUM.to_uppercase());
    let json = release_with(&[(SETUP, SETUP_URL, 35_000_000, Some(&digest)), (DEB, DEB_URL, 5_000_000, Some(&digest))]);
    let linux = update_from_json_for(&json, "1.1.0", Some(Platform::LinuxDeb)).unwrap().package.unwrap();
    assert_eq!((linux.name.as_str(), linux.url.as_str(), linux.size, linux.sha256.as_str()), (DEB, DEB_URL, 5_000_000, SUM));
    let windows = update_from_json_for(&json, "1.1.0", Some(Platform::WindowsSetup)).unwrap().package.unwrap();
    assert_eq!((windows.name.as_str(), windows.size), (SETUP, 35_000_000));
    // Another system: the notice, without a package.
    let other = update_from_json_for(&json, "1.1.0", None).unwrap();
    assert_eq!(other.package, None);
}

#[test]
fn packages_without_a_trusted_sum_or_source_are_not_offered() {
    let sum = format!("sha256:{SUM}");
    let cases: Vec<(&str, String, u64, Option<String>)> = vec![
        // No sum (files uploaded before GitHub computed them), or another kind.
        (DEB, DEB_URL.into(), 5_000_000, None),
        (DEB, DEB_URL.into(), 5_000_000, Some(format!("sha1:{}", &SUM[..40]))),
        (DEB, DEB_URL.into(), 5_000_000, Some("sha256:1234".into())),
        // Another repository, another tag, plain HTTP, a URL with another name.
        (DEB, DEB_URL.replace("LubrieDev", "someone"), 5_000_000, Some(sum.clone())),
        (DEB, DEB_URL.replace("v1.2.0/", "v1.1.0/"), 5_000_000, Some(sum.clone())),
        (DEB, DEB_URL.replace("https:", "http:"), 5_000_000, Some(sum.clone())),
        (DEB, DEB_URL.replace(DEB, "other.deb"), 5_000_000, Some(sum.clone())),
        // Empty or absurd sizes.
        (DEB, DEB_URL.into(), 0, Some(sum.clone())),
        (DEB, DEB_URL.into(), 900_000_000, Some(sum.clone())),
        // Names that are not the package or not a plain file name.
        ("adb-scan_1.2.0-1_arm64.deb", DEB_URL.replace("amd64", "arm64"), 5_000_000, Some(sum.clone())),
        ("adb-scan_../x_amd64.deb", "https://github.com/LubrieDev/ADB-Scan/releases/download/v1.2.0/adb-scan_../x_amd64.deb".into(), 5_000_000, Some(sum.clone())),
    ];
    for (name, url, size, digest) in &cases {
        let json = release_with(&[(name, url, *size, digest.as_deref())]);
        let update = update_from_json_for(&json, "1.1.0", Some(Platform::LinuxDeb)).expect("the notice is still shown");
        assert_eq!(update.package, None, "{name} {url} {size} {digest:?}");
    }
}

/// Against GitHub: the latest release offers this system's package, and it
/// downloads with its progress and the SHA-256 sum GitHub publishes. Not run
/// by default (network, a few MB); run it with
/// `XDG_CACHE_HOME=$(mktemp -d) cargo test -p adbscan-core --test updater -- --ignored`.
#[test]
#[ignore]
fn downloads_the_latest_release() {
    use std::sync::atomic::AtomicBool;
    let update = check("0.0.1").expect("the latest release");
    let pkg = update.package.expect("a package with its sum");
    let calls = std::cell::Cell::new(0u32);
    let last = std::cell::Cell::new((0u64, 0u64));
    let file = download(&pkg, &|got, total| {
        calls.set(calls.get() + 1);
        last.set((got, total));
    }, &AtomicBool::new(false))
    .expect("downloaded and checked");
    assert_eq!(std::fs::metadata(&file).unwrap().len(), pkg.size);
    assert_eq!(last.get(), (pkg.size, pkg.size));
    assert!(calls.get() > 2, "progress is reported as it goes");
    // A wrong sum: nothing is kept.
    let bad = Package { sha256: "0".repeat(64), ..pkg.clone() };
    assert!(download(&bad, &|_, _| {}, &AtomicBool::new(false)).is_err());
    assert!(!download_dir().join(format!("{}.part", pkg.name)).exists());
    clean_downloads();
    assert!(!file.exists());
}
