//! Storage analyzer: what takes up the shared storage (`/sdcard`).
//!
//! One read-only walk (`du -a -k`, [`SCAN_SCRIPT`]) lists every file and
//! folder with its size; everything else is computed here: the folder tree
//! to browse, the largest files, the space by file type, well-known places
//! (WhatsApp media, downloads, thumbnails…) and possible duplicates (same
//! name and size in different folders). Deleting is file by file and only
//! inside `/sdcard/` ([`crate::adb::Adb::delete_shared_file`]).

use std::collections::{HashMap, HashSet};

use crate::i18n::tr;

/// The folder that is scanned.
pub const ROOT: &str = "/sdcard";

/// Read-only: sizes in KiB of every file and folder under [`ROOT`].
pub const SCAN_SCRIPT: &str = "du -a -k /sdcard/ 2>/dev/null";

/// A file or folder in the scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub path: String,
    pub name: String,
    pub kib: u64,
    pub is_dir: bool,
}

/// File types, for the space by type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileType {
    Images,
    Videos,
    Audio,
    Documents,
    Archives,
    Other,
}

impl FileType {
    pub const ALL: [FileType; 6] = [FileType::Images, FileType::Videos, FileType::Audio, FileType::Documents, FileType::Archives, FileType::Other];

    pub fn of(name: &str) -> Self {
        // Lowercased into a stack buffer: this runs once per file (100 000+).
        let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        let mut buf = [0u8; 6];
        if ext.len() > buf.len() {
            return FileType::Other;
        }
        for (b, c) in buf.iter_mut().zip(ext.bytes()) {
            *b = c.to_ascii_lowercase();
        }
        match std::str::from_utf8(&buf[..ext.len()]).unwrap_or("") {
            "jpg" | "jpeg" | "png" | "heic" | "heif" | "webp" | "gif" | "bmp" | "dng" | "raw" | "avif" => FileType::Images,
            "mp4" | "mkv" | "mov" | "webm" | "3gp" | "avi" | "m4v" | "ts" => FileType::Videos,
            "mp3" | "m4a" | "aac" | "opus" | "ogg" | "wav" | "flac" | "amr" | "mid" => FileType::Audio,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp" | "txt" | "md" | "csv" | "epub" => FileType::Documents,
            "zip" | "rar" | "7z" | "tar" | "gz" | "xz" | "apk" | "apks" | "xapk" | "img" | "iso" | "qcow2" | "bin" => FileType::Archives,
            _ => FileType::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FileType::Images => tr("Images"),
            FileType::Videos => tr("Videos"),
            FileType::Audio => tr("Audio"),
            FileType::Documents => tr("Documents"),
            FileType::Archives => tr("Archives, APKs and disk images"),
            FileType::Other => tr("Other"),
        }
    }
}

/// A well-known place that usually grows without anyone noticing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub title: &'static str,
    pub hint: &'static str,
    /// Folder to open in the browser (the largest of the matches).
    pub path: String,
    pub kib: u64,
}

/// Files with the same name and size in different folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Duplicate {
    pub name: String,
    pub kib: u64,
    pub paths: Vec<String>,
}

impl Duplicate {
    /// What deleting every copy but one would free.
    pub fn reclaimable(&self) -> u64 {
        self.kib * (self.paths.len() as u64 - 1)
    }
}

/// Largest files listed by [`Scan::report`].
pub const LARGEST: usize = 500;

/// Smallest file considered for duplicates (1 MiB): smaller ones are not
/// worth the time.
pub const DUPLICATE_MIN_KIB: u64 = 1024;

/// Everything the analyzer shows that needs a pass over every file,
/// computed at once (off the interface thread) by [`Scan::report`].
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub total_kib: u64,
    pub file_count: usize,
    pub by_type: Vec<(FileType, u64, usize)>,
    pub places: Vec<Place>,
    pub largest: Vec<Node>,
    pub duplicates: Vec<Duplicate>,
}

impl Report {
    /// What deleting every duplicate copy but one would free.
    pub fn freeable_kib(&self) -> u64 {
        self.duplicates.iter().map(Duplicate::reclaimable).sum()
    }
}

/// The result of a scan.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    size: HashMap<String, u64>,
    children: HashMap<String, Vec<String>>,
    dirs: HashSet<String>,
}

fn parent(path: &str) -> Option<&str> {
    let i = path.rfind('/')?;
    (i > 0).then(|| &path[..i])
}

fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl Scan {
    /// Parses `du -a -k` output. Lines are `KiB<TAB>path`; the root may
    /// come with a trailing `/`.
    pub fn parse(du: &str) -> Self {
        let mut size = HashMap::new();
        for line in du.lines() {
            let Some((kib, path)) = line.split_once('\t') else { continue };
            let Ok(kib) = kib.trim().parse::<u64>() else { continue };
            let path = path.trim_end_matches('/');
            if path != ROOT && !path.starts_with("/sdcard/") {
                continue;
            }
            size.insert(path.to_string(), kib);
        }
        let mut children: HashMap<String, Vec<String>> = HashMap::new();
        let mut dirs = HashSet::new();
        dirs.insert(ROOT.to_string());
        for path in size.keys() {
            if let Some(p) = parent(path) {
                children.entry(p.to_string()).or_default().push(path.clone());
                dirs.insert(p.to_string());
            }
        }
        Scan { size, children, dirs }
    }

    pub fn is_empty(&self) -> bool {
        self.size.is_empty()
    }

    /// Totals, types, places, largest files and duplicates. It walks every
    /// file several times (tens of milliseconds with 100 000 files): call it
    /// in the background.
    pub fn report(&self) -> Report {
        Report { total_kib: self.total_kib(), file_count: self.file_count(), by_type: self.by_type(), places: self.places(), largest: self.largest_files(LARGEST), duplicates: self.duplicates(DUPLICATE_MIN_KIB) }
    }

    pub fn total_kib(&self) -> u64 {
        self.size.get(ROOT).copied().unwrap_or(0)
    }

    pub fn file_count(&self) -> usize {
        self.size.keys().filter(|p| !self.dirs.contains(*p)).count()
    }

    fn node(&self, path: &str) -> Node {
        Node { path: path.to_string(), name: name(path).to_string(), kib: self.size.get(path).copied().unwrap_or(0), is_dir: self.dirs.contains(path) }
    }

    /// What a folder contains, largest first.
    pub fn children(&self, dir: &str) -> Vec<Node> {
        let mut v: Vec<Node> = self.children.get(dir).map(|c| c.iter().map(|p| self.node(p)).collect()).unwrap_or_default();
        v.sort_by(|a, b| b.kib.cmp(&a.kib).then(a.name.cmp(&b.name)));
        v
    }

    pub fn size_of(&self, path: &str) -> Option<u64> {
        self.size.get(path).copied()
    }

    fn files(&self) -> impl Iterator<Item = (&String, u64)> {
        self.size.iter().filter(|(p, _)| !self.dirs.contains(*p)).map(|(p, k)| (p, *k))
    }

    /// The `n` largest files.
    pub fn largest_files(&self, n: usize) -> Vec<Node> {
        let mut v: Vec<(&String, u64)> = self.files().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        v.into_iter().take(n).map(|(p, _)| self.node(p)).collect()
    }

    /// Space by file type, largest first.
    pub fn by_type(&self) -> Vec<(FileType, u64, usize)> {
        let mut acc: HashMap<FileType, (u64, usize)> = HashMap::new();
        for (p, k) in self.files() {
            let e = acc.entry(FileType::of(name(p))).or_default();
            e.0 += k;
            e.1 += 1;
        }
        let mut v: Vec<_> = FileType::ALL.iter().map(|t| (*t, acc.get(t).map(|a| a.0).unwrap_or(0), acc.get(t).map(|a| a.1).unwrap_or(0))).filter(|(_, k, _)| *k > 0).collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
        v
    }

    /// Well-known places that exist on this phone, largest first.
    pub fn places(&self) -> Vec<Place> {
        let defs: [(&'static str, &'static str, &[&str]); 8] = [
            (tr("WhatsApp media"), tr("Photos, videos, voice notes and documents received in chats. Delete them from WhatsApp → Settings → Storage to keep the chats tidy."), &["/sdcard/Android/media/com.whatsapp/WhatsApp/Media", "/sdcard/Android/media/com.whatsapp.w4b/WhatsApp Business/Media", "/sdcard/WhatsApp/Media"]),
            (tr("Telegram"), tr("Files downloaded by Telegram. Telegram can clear its cache in Settings → Data and storage."), &["/sdcard/Android/media/org.telegram.messenger", "/sdcard/Telegram", "/sdcard/Android/data/org.telegram.messenger"]),
            (tr("Downloads"), tr("Everything downloaded from the browser and other apps."), &["/sdcard/Download"]),
            (tr("Camera"), tr("Photos and videos taken with the phone."), &["/sdcard/DCIM"]),
            (tr("Screenshots"), tr("Screenshots and screen recordings."), &["/sdcard/Pictures/Screenshots", "/sdcard/DCIM/Screenshots", "/sdcard/Movies/Screenrecorder"]),
            (tr("Recordings"), tr("Voice and call recordings."), &["/sdcard/Recordings", "/sdcard/Music/Recordings", "/sdcard/Sounds"]),
            (tr("App files"), tr("Files apps keep in shared storage. Clearing an app's data in Apps deletes its folder here."), &["/sdcard/Android/data"]),
            (tr("Game data (OBB)"), tr("Extra data of installed games. Uninstalling the game deletes it."), &["/sdcard/Android/obb"]),
        ];
        let mut out: Vec<Place> = defs
            .iter()
            .filter_map(|(title, hint, paths)| {
                let found: Vec<(&str, u64)> = paths.iter().filter_map(|p| self.size.get(*p).map(|k| (*p, *k))).collect();
                let kib: u64 = found.iter().map(|f| f.1).sum();
                let path = found.iter().max_by_key(|f| f.1)?.0.to_string();
                (kib > 0).then_some(Place { title, hint, path, kib })
            })
            .collect();
        // Thumbnail caches, wherever they are.
        let thumbs: Vec<(&String, u64)> = self.size.iter().filter(|(p, _)| name(p) == ".thumbnails" && self.dirs.contains(*p)).map(|(p, k)| (p, *k)).collect();
        let kib: u64 = thumbs.iter().map(|t| t.1).sum();
        if let Some((path, _)) = thumbs.iter().max_by_key(|t| t.1) {
            if kib > 0 {
                out.push(Place { title: tr("Thumbnail caches"), hint: tr("Small copies of your photos made by galleries. They are made again when needed."), path: (*path).clone(), kib });
            }
        }
        out.sort_by_key(|p| std::cmp::Reverse(p.kib));
        out
    }

    /// Files of at least `min_kib` with the same name and size in more than
    /// one folder, by space that could be freed.
    pub fn duplicates(&self, min_kib: u64) -> Vec<Duplicate> {
        let mut groups: HashMap<(&str, u64), Vec<String>> = HashMap::new();
        // Caches are made of blocks with repeated names (`data_1`, `index`):
        // files without an extension or inside a cache folder are not
        // documents anyone copied twice.
        let cache = |p: &str| p.split('/').any(|c| matches!(c.to_ascii_lowercase().as_str(), "cache" | ".cache" | "caches" | "code_cache" | "gpucache" | "shadercache"));
        for (p, k) in self.files().filter(|(p, k)| *k >= min_kib && name(p).contains('.') && !cache(p)) {
            groups.entry((name(p), k)).or_default().push(p.clone());
        }
        let mut v: Vec<Duplicate> = groups
            .into_iter()
            .filter(|(_, ps)| ps.len() > 1)
            .map(|((n, k), mut paths)| {
                paths.sort();
                Duplicate { name: n.to_string(), kib: k, paths }
            })
            .collect();
        v.sort_by(|a, b| b.reclaimable().cmp(&a.reclaimable()).then(a.name.cmp(&b.name)));
        v
    }

    /// Forgets a deleted file and subtracts its size from its folders, so
    /// the view stays right without scanning again.
    pub fn remove_file(&mut self, path: &str) {
        let Some(kib) = self.size.remove(path) else { return };
        if let Some(p) = parent(path) {
            if let Some(c) = self.children.get_mut(p) {
                c.retain(|x| x != path);
            }
        }
        let mut cur = parent(path);
        while let Some(dir) = cur {
            if let Some(s) = self.size.get_mut(dir) {
                *s = s.saturating_sub(kib);
            }
            cur = parent(dir);
        }
    }
}

/// Whether a path may be deleted from the analyzer: a file inside
/// `/sdcard/`, without `..`.
pub fn deletable(path: &str) -> bool {
    path.starts_with("/sdcard/") && !path.split('/').any(|c| c == ".." || c == ".") && !crate::adb::is_protected_remote_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DU: &str = "4\t/sdcard/Download/a.pdf\n2048\t/sdcard/Download/movie.mp4\n2056\t/sdcard/Download\n2048\t/sdcard/DCIM/Camera/movie.mp4\n8\t/sdcard/DCIM/Camera/.thumbnails/x.jpg\n8\t/sdcard/DCIM/Camera/.thumbnails\n2060\t/sdcard/DCIM/Camera\n2060\t/sdcard/DCIM\n4120\t/sdcard/\n";

    #[test]
    fn tree_and_sizes() {
        let s = Scan::parse(DU);
        assert_eq!(s.total_kib(), 4120);
        assert_eq!(s.file_count(), 4);
        let root = s.children(ROOT);
        assert_eq!(root.iter().map(|n| (n.name.as_str(), n.is_dir)).collect::<Vec<_>>(), [("DCIM", true), ("Download", true)]);
        assert_eq!(s.largest_files(1)[0].path, "/sdcard/DCIM/Camera/movie.mp4");
    }

    #[test]
    fn types_places_and_duplicates() {
        let s = Scan::parse(DU);
        let types = s.by_type();
        assert_eq!(types[0], (FileType::Videos, 4096, 2));
        let places = s.places();
        assert!(places.iter().any(|p| p.path == "/sdcard/Download" && p.kib == 2056));
        assert!(places.iter().any(|p| p.path == "/sdcard/DCIM/Camera/.thumbnails"));
        let d = s.duplicates(1024);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].paths, ["/sdcard/DCIM/Camera/movie.mp4", "/sdcard/Download/movie.mp4"]);
        assert_eq!(d[0].reclaimable(), 2048);
    }

    #[test]
    fn removing_updates_parents() {
        let mut s = Scan::parse(DU);
        s.remove_file("/sdcard/Download/movie.mp4");
        assert_eq!(s.size_of("/sdcard/Download"), Some(8));
        assert_eq!(s.total_kib(), 2072);
        assert_eq!(s.children("/sdcard/Download").len(), 1);
    }

    #[test]
    fn report_matches_the_parts() {
        let s = Scan::parse(DU);
        let r = s.report();
        assert_eq!((r.total_kib, r.file_count), (4120, 4));
        assert_eq!(r.by_type, s.by_type());
        assert_eq!(r.largest, s.largest_files(LARGEST));
        assert_eq!(r.freeable_kib(), 2048);
    }

    #[test]
    fn file_type_ignores_case_and_long_extensions() {
        assert_eq!(FileType::of("IMG_0001.JPG"), FileType::Images);
        assert_eq!(FileType::of("disk.QCOW2"), FileType::Archives);
        assert_eq!(FileType::of("notes"), FileType::Other);
        assert_eq!(FileType::of("x.verylongext"), FileType::Other);
        assert_eq!(FileType::of("canción.ñp3"), FileType::Other);
    }

    #[test]
    fn only_shared_files_are_deletable() {
        assert!(deletable("/sdcard/Download/a.pdf"));
        assert!(!deletable("/sdcard"));
        assert!(!deletable("/sdcard/../data/x"));
        assert!(!deletable("/data/local/tmp/x"));
    }
}
