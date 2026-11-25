//! Every literal passed to `tr`, `trc`, `t!` or `tc!` in the core and the
//! interface must have a Spanish translation in `i18n/es.po`, so that no
//! English text slips into the Spanish interface.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Value of a Rust string literal starting at `s[0]` (`"…"` or `r#"…"#`) and
/// the number of bytes it takes.
fn literal(s: &str) -> Option<(String, usize)> {
    if let Some(rest) = s.strip_prefix('r') {
        let hashes = rest.len() - rest.trim_start_matches('#').len();
        let body = rest[hashes..].strip_prefix('"')?;
        let close = format!("\"{}", "#".repeat(hashes));
        let end = body.find(&close)?;
        return Some((body[..end].to_string(), 1 + hashes + 1 + end + close.len()));
    }
    let body = s.strip_prefix('"')?;
    let (mut out, mut chars) = (String::new(), body.char_indices());
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, i + 2)),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                'x' => {
                    let hex: String = (0..2).filter_map(|_| chars.next().map(|c| c.1)).collect();
                    out.push(u8::from_str_radix(&hex, 16).ok()? as char);
                }
                'u' => {
                    let mut hex = String::new();
                    for (_, c) in chars.by_ref() {
                        match c {
                            '{' => {}
                            '}' => break,
                            c => hex.push(c),
                        }
                    }
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                '\n' => {
                    while chars.clone().next().is_some_and(|(_, c)| c.is_whitespace()) {
                        chars.next();
                    }
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

#[test]
fn every_interface_string_has_a_spanish_translation() {
    let core = Path::new(env!("CARGO_MANIFEST_DIR"));
    let catalog = adbscan_core::i18n::parse_po(&std::fs::read_to_string(core.join("i18n/es.po")).unwrap());
    let mut files = Vec::new();
    rust_files(&core.join("src"), &mut files);
    rust_files(&core.join("../app/src"), &mut files);
    let mut missing = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        // Tests are not interface code.
        let text = text.split("#[cfg(test)]").next().unwrap();
        for call in ["tr(", "trc(", "t!(", "tc!("] {
            for (pos, _) in text.match_indices(call) {
                let before = text[..pos].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ':' && !text[..pos].ends_with("::")) {
                    continue;
                }
                let mut rest = text[pos + call.len()..].trim_start();
                let mut context = None;
                if call.starts_with("trc") || call.starts_with("tc") {
                    let Some((ctx, len)) = literal(rest) else { continue };
                    context = Some(ctx);
                    rest = rest[len..].trim_start().strip_prefix(',').unwrap_or("").trim_start();
                }
                let Some((english, _)) = literal(rest) else { continue };
                let key = match &context {
                    Some(c) => format!("{c}\u{4}{english}"),
                    None => english.clone(),
                };
                if !catalog.contains_key(&key) {
                    missing.push(format!("{}: “{english}”", file.strip_prefix(core).unwrap_or(&file).display()));
                }
            }
        }
    }
    assert!(missing.is_empty(), "{} strings without a Spanish translation:\n{}", missing.len(), missing.join("\n"));
}
