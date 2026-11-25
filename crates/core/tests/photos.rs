//! Photo backup: what gets copied and where.

use adbscan_core::photos::*;
use std::path::{Path, PathBuf};

const LIST: &str = "4100|1763900000|/sdcard/DCIM/Camera/IMG_1.jpg
9000|1763950000|/sdcard/DCIM/Camera/VID_2.mp4
120|1763800000|/sdcard/Pictures/Screenshots/Shot 3.PNG
55|1763700000|/sdcard/DCIM/Camera/notes.txt
roto
";

#[test]
fn list_keeps_media_only() {
    let l = parse_list(LIST);
    assert_eq!(l.len(), 3);
    assert_eq!(l[2].path, "/sdcard/Pictures/Screenshots/Shot 3.PNG");
    assert_eq!(l[0].size, 4100);
}

#[test]
fn local_paths_stay_inside_destination() {
    let d = Path::new("/home/u/Photos");
    assert_eq!(local_path(d, "/sdcard/DCIM/Camera/a.jpg"), Some(PathBuf::from("/home/u/Photos/DCIM/Camera/a.jpg")));
    assert_eq!(local_path(d, "/storage/emulated/0/DCIM/b.jpg"), Some(PathBuf::from("/home/u/Photos/DCIM/b.jpg")));
    assert_eq!(local_path(d, "/sdcard/DCIM/../../etc/x.jpg"), None);
    assert_eq!(local_path(d, "/sdcard//etc/x.jpg"), None); // absolute after removing the root
    assert_eq!(local_path(d, "/data/x.jpg"), None);
}

#[test]
fn plan_copies_only_new_newest_first() {
    let d = Path::new("/f");
    let l = parse_list(LIST);
    // IMG_1 is already there with the same size; the screenshot is incomplete.
    let p = plan(&l, d, |p| match p.to_str().unwrap() {
        "/f/DCIM/Camera/IMG_1.jpg" => Some(4100),
        "/f/Pictures/Screenshots/Shot 3.PNG" => Some(10),
        _ => None,
    });
    let names: Vec<&str> = p.iter().map(|(r, _, _)| r.as_str()).collect();
    assert_eq!(names, vec!["/sdcard/DCIM/Camera/VID_2.mp4", "/sdcard/Pictures/Screenshots/Shot 3.PNG"]);
}
