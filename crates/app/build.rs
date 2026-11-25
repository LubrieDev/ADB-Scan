//! Embeds the SVG icons from `assets/icons` into the binary.
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
}
