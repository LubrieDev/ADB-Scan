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
