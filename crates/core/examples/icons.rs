//! Real icons of the user apps against a real phone (read-only).
//!
//! `cargo run -p adbscan-core --example icons -- [folder]` says which icon it
//! finds for each app and, if a folder is given, saves the images.

use adbscan_core::adb::Adb;
use std::time::Instant;

fn main() {
    let adb = Adb::locate(None).expect("adb");
    let Some(dev) = adb.devices().expect("adb devices").into_iter().find(|d| d.connected()) else {
        println!("No authorised device");
        return;
    };
    let out = std::env::args().nth(1).map(std::path::PathBuf::from);
    let apps: Vec<_> = adb.packages(&dev.serial).expect("packages").into_iter().filter(|p| !p.is_system).collect();
    let (mut ok, mut adaptive, mut svg, mut failed) = (0, 0, 0, 0);
    for p in &apps {
        let t = Instant::now();
        match adb.app_icon(&dev.serial, &p.apk_path) {
            Ok(Some(icon)) => {
                ok += 1;
                adaptive += usize::from(icon.adaptive);
                let is_svg = |b: &[u8]| b.starts_with(b"<svg");
                let vector = is_svg(&icon.image) || icon.background_image.as_deref().is_some_and(is_svg);
                svg += usize::from(vector);
                let kind = if icon.adaptive {
                    format!(
                        "adaptive, background {}",
                        match (icon.background_color, &icon.background_image) {
                            (Some(c), _) => format!("#{c:08x}"),
                            (_, Some(b)) => format!("image {} B", b.len()),
                            _ => "none".into(),
                        }
                    )
                } else {
                    "image".into()
                };
                println!("{:45} {:>7} B  {kind}{}  ({:?})", p.package, icon.image.len(), if vector { " · vector" } else { "" }, t.elapsed());
                if let Some(dir) = &out {
                    let ext = if is_svg(&icon.image) { "svg" } else { "img" };
                    let _ = std::fs::write(dir.join(format!("{}.{ext}", p.package)), &icon.image);
                    if let Some(b) = &icon.background_image {
                        let _ = std::fs::write(dir.join(format!("{}.background.{}", p.package, if is_svg(b) { "svg" } else { "img" })), b);
                    }
                }
            }
            Ok(None) => println!("{:45} no icon  ({:?})", p.package, t.elapsed()),
            Err(e) => {
                failed += 1;
                println!("{:45} FAILED: {e}  ({:?})", p.package, t.elapsed());
            }
        }
    }
    println!("\n{ok} of {} with an icon ({adaptive} adaptive, {svg} vectors converted, {failed} connection failures)", apps.len());
}
