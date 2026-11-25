//! Checks GitHub Releases for a newer version of ADB Scan.
//!
//! One request to the latest release of the repository. Anything that goes
//! wrong (no release yet, no Internet, timeout, unexpected JSON, a tag that
//! is not a version) means “no update”: it never fails loudly and never
//! downloads or installs anything. The caller only shows a link to the
//! release page.

use semver::Version;
use serde::Deserialize;
use std::time::Duration;

/// Latest published release (drafts and pre-releases are not included).
pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/LubrieDev/ADB-Scan/releases/latest";

/// Pages a release link may point to (the link is opened in the browser).
pub const RELEASES_PAGE: &str = "https://github.com/LubrieDev/ADB-Scan/releases/";

/// The whole request, connection included.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// The answer is a few KiB; anything much bigger is not a release.
const MAX_BODY_BYTES: u64 = 1024 * 1024;

/// A newer version than the one running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: Version,
    /// Release page on GitHub.
    pub url: String,
}

/// The fields used from GitHub's answer.
#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
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
    let release: Release = serde_json::from_str(json).ok()?;
    if !release.html_url.starts_with(RELEASES_PAGE) {
        return None;
    }
    let version = newer_version(current, &release.tag_name)?;
    Some(Update { version, url: release.html_url })
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

fn fetch_latest() -> Result<String, ureq::Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(concat!("ADB-Scan/", env!("CARGO_PKG_VERSION"), " (+https://github.com/LubrieDev/ADB-Scan)"))
        .build()
        .into();
    // A 404 (no release yet) or any other error status is an `Err`.
    let mut response = agent.get(LATEST_RELEASE_URL).header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28").call()?;
    response.body_mut().with_config().limit(MAX_BODY_BYTES).read_to_string()
}
