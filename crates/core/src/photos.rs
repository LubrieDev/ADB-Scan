//! Backup of photos and videos from the phone to the laptop: only what is new.
//!
//! `DCIM`, `Pictures` and `Movies` are listed (without hidden folders such as
//! `.thumbnails`) and every media file that is not already at the destination
//! with the same size is copied. Subfolders are kept (`DCIM/Camera`,
//! `Pictures/Screenshots`…). The listing is read-only; the copy does not delete
//! or change anything on the phone.

use std::path::{Component, Path, PathBuf};

/// Read-only listing: `size|seconds|path` per file.
pub const LIST_SCRIPT: &str = "for d in /sdcard/DCIM /sdcard/Pictures /sdcard/Movies; do \
[ -d \"$d\" ] && find \"$d\" -path '*/.*' -prune -o -type f -exec stat -c '%s|%Y|%n' {} + 2>/dev/null; \
done; true";

/// Root of the shared storage, which is not repeated at the destination.
const ROOTS: [&str; 2] = ["/sdcard/", "/storage/emulated/0/"];

const MEDIA_EXT: [&str; 14] = ["jpg", "jpeg", "png", "heic", "heif", "webp", "gif", "dng", "mp4", "mov", "3gp", "mkv", "webm", "avif"];

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteMedia {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
}

pub fn parse_list(text: &str) -> Vec<RemoteMedia> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '|');
            let size = it.next()?.trim().parse().ok()?;
            let mtime = it.next()?.trim().parse().ok()?;
            let path = it.next()?.to_string();
            let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
            MEDIA_EXT.contains(&ext.as_str()).then_some(RemoteMedia { path, size, mtime })
        })
        .collect()
}

/// Where `remote` is saved inside `dest`, or `None` if the path is not safe
/// (`..`, absolute after removing the root…): nothing is ever written outside.
pub fn local_path(dest: &Path, remote: &str) -> Option<PathBuf> {
    let rel = ROOTS.iter().find_map(|r| remote.strip_prefix(r))?;
    let rel = Path::new(rel);
    if rel.as_os_str().is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(dest.join(rel))
}

/// What to copy: (path on the phone, size, destination), most recent first.
/// `local_size` gives the size of a local file (if it exists).
pub fn plan(remote: &[RemoteMedia], dest: &Path, local_size: impl Fn(&Path) -> Option<u64>) -> Vec<(String, u64, PathBuf)> {
    let mut items: Vec<&RemoteMedia> = remote.iter().collect();
    items.sort_by_key(|m| std::cmp::Reverse(m.mtime));
    items
        .into_iter()
        .filter_map(|m| {
            let local = local_path(dest, &m.path)?;
            (local_size(&local) != Some(m.size)).then(|| (m.path.clone(), m.size, local))
        })
        .collect()
}
