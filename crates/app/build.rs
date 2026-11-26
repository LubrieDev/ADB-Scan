//! Embeds the SVG icons from `assets/icons` into the binary and, on Windows,
//! the icon and version information of `adb-scan.exe`.
use std::fmt::Write;
use std::path::Path;

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, String)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else if p.extension().is_some_and(|x| x == "svg") {
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            out.push((rel, p.canonicalize().unwrap().display().to_string()));
        }
    }
}

fn main() {
    // Resolved when the script runs (not env!): survives moving the folder.
    let root = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../assets/icons");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    walk(&root, &root, &mut files);
    let mut code = String::from("pub static ICONS: &[(&str, &[u8])] = &[\n");
    for (rel, abs) in &files {
        writeln!(code, "    ({rel:?}, include_bytes!({abs:?})),").unwrap();
    }
    code.push_str("];\n");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("icons.rs");
    std::fs::write(out, code).unwrap();
    windows_resources();
}

/// Icon (`assets/windows/adb-scan.ico`, made from `adb-scan.svg`) and version
/// information of the .exe. `winresource` is a build dependency only when
/// building on Windows (the CI), and it is only used for a Windows target.
#[cfg(windows)]
fn windows_resources() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    // Without canonicalize: a `\\?\` path is not understood by windres.
    let ico = Path::new(&manifest).parent().and_then(Path::parent).unwrap().join("assets").join("windows").join("adb-scan.ico");
    println!("cargo:rerun-if-changed={}", ico.display());
    let mut res = winresource::WindowsResource::new();
    res.set_icon(&ico.display().to_string());
    res.set("ProductName", "ADB Scan");
    res.set("FileDescription", "ADB Scan");
    res.set("LegalCopyright", "2026, LubrieDev");
    res.compile().expect("could not embed the Windows icon (windres)");
}

#[cfg(not(windows))]
fn windows_resources() {}
