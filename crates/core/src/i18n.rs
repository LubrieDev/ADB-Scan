//! Interface language.
//!
//! English is the source language: every user-facing string is written in
//! English in the code and goes through [`tr`] (plain text) or [`t!`] (text
//! with `{}` placeholders). Other languages are gettext-style catalogs in
//! `crates/core/i18n/<code>.po`, embedded in the binary, that map each
//! English string to its translation. A string missing from a catalog is
//! shown in English.
//!
//! The language is chosen once at startup with [`set_language`]; changing it
//! in Preferences takes effect the next time ADB Scan starts.

use std::collections::HashMap;
use std::fmt::{Display, Write};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Languages ADB Scan can be shown in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "es")]
    Spanish,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::Spanish];

    /// ISO 639-1 code (`en`, `es`).
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Spanish => "es",
        }
    }

    /// Accepts a bare code (`es`) or a locale (`es_MX.UTF-8`).
    pub fn from_code(code: &str) -> Option<Self> {
        let base = code.split(['_', '.', '-', '@']).next().unwrap_or("").to_ascii_lowercase();
        Self::ALL.into_iter().find(|l| l.code() == base)
    }

    /// Name of the language written in that language.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Spanish => "Español",
        }
    }

    fn catalog_source(self) -> Option<&'static str> {
        match self {
            Language::English => None,
            Language::Spanish => Some(include_str!("../i18n/es.po")),
        }
    }
}

static LANGUAGE: OnceLock<Language> = OnceLock::new();
static CATALOG: OnceLock<HashMap<String, &'static str>> = OnceLock::new();

/// Chooses the interface language. Only the first call counts: it must run
/// before any text is translated (at the start of `main`). Returns `false`
/// if the language had already been fixed.
pub fn set_language(language: Language) -> bool {
    let first = LANGUAGE.set(language).is_ok();
    let _ = catalog();
    first
}

/// Current interface language (English until [`set_language`] is called).
pub fn language() -> Language {
    LANGUAGE.get().copied().unwrap_or_default()
}

/// The catalog of the chosen language. Until [`set_language`] runs (loading
/// the settings that name the language already translates default values)
/// texts stay in English without fixing the catalog, so the language chosen
/// afterwards still applies.
fn catalog() -> &'static HashMap<String, &'static str> {
    static EMPTY: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    match LANGUAGE.get() {
        Some(language) => CATALOG.get_or_init(|| language.catalog_source().map(parse_po).unwrap_or_default()),
        None => EMPTY.get_or_init(HashMap::new),
    }
}

/// Translation of an English interface string.
pub fn tr(english: &'static str) -> &'static str {
    catalog().get(english).copied().unwrap_or(english)
}

/// Like [`tr`], for an English string that is translated differently
/// depending on `context` (`msgctxt` in the catalog): for example a plural
/// or a feminine form in Spanish where English has a single word.
pub fn trc(context: &'static str, english: &'static str) -> &'static str {
    catalog().get(&format!("{context}\u{4}{english}")).copied().unwrap_or(english)
}

/// Fills the placeholders of a translated template: `{}` takes the next
/// argument and `{N}` argument N (translations may reorder them); `{{` and
/// `}}` are literal braces. Used by [`t!`].
pub fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16 * args.len());
    let mut next = 0;
    let mut chars = template.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '{' if chars.peek().map(|p| p.1) == Some('{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek().map(|p| p.1) == Some('}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let Some(end) = template[i..].find('}') else {
                    out.push_str(&template[i..]);
                    break;
                };
                let inner = &template[i + 1..i + end];
                let index = if inner.is_empty() {
                    next += 1;
                    Some(next - 1)
                } else {
                    inner.parse::<usize>().ok()
                };
                match index.and_then(|n| args.get(n)) {
                    Some(arg) => {
                        let _ = write!(out, "{arg}");
                    }
                    None => out.push_str(&template[i..=i + end]),
                }
                while chars.peek().is_some_and(|p| p.0 <= i + end) {
                    chars.next();
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Translates an English template and fills its `{}` placeholders, like
/// `format!` but through the catalog: `t!("{} apps stopped", n)`.
/// Placeholders take no format specs: arguments needing one are formatted
/// beforehand (`format!("{x:.1}")`).
#[macro_export]
macro_rules! t {
    ($text:literal $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr($text), &[])
    };
    ($text:literal, $($arg:expr),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr($text), &[$(&$arg as &dyn ::std::fmt::Display),+])
    };
}

/// [`t!`] with a translation context (see [`trc`]).
#[macro_export]
macro_rules! tc {
    ($context:literal, $text:literal $(,)?) => {
        $crate::i18n::fill($crate::i18n::trc($context, $text), &[])
    };
    ($context:literal, $text:literal, $($arg:expr),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::trc($context, $text), &[$(&$arg as &dyn ::std::fmt::Display),+])
    };
}

/// Reads a `.po` catalog: `msgctxt`, `msgid` and `msgstr`, each possibly
/// continued on following `"…"` lines. Entries with an empty `msgstr` are
/// skipped (they stay in English).
pub fn parse_po(text: &str) -> HashMap<String, &'static str> {
    #[derive(Clone, Copy, PartialEq)]
    enum Field {
        None,
        Ctx,
        Id,
        Str,
    }
    let mut map = HashMap::new();
    let (mut ctx, mut id, mut value) = (None::<String>, String::new(), String::new());
    let mut field = Field::None;
    let mut flush = |ctx: &mut Option<String>, id: &mut String, value: &mut String| {
        if !id.is_empty() && !value.is_empty() {
            let key = match ctx.take() {
                Some(c) => format!("{c}\u{4}{id}"),
                None => std::mem::take(id),
            };
            map.insert(key, &*Box::leak(std::mem::take(value).into_boxed_str()));
        }
        *ctx = None;
        id.clear();
        value.clear();
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (next, rest) = if let Some(r) = line.strip_prefix("msgctxt ") {
            (Field::Ctx, r)
        } else if let Some(r) = line.strip_prefix("msgid ") {
            (Field::Id, r)
        } else if let Some(r) = line.strip_prefix("msgstr ") {
            (Field::Str, r)
        } else {
            (field, line)
        };
        if next != field && (next == Field::Ctx || (next == Field::Id && field != Field::Ctx)) {
            flush(&mut ctx, &mut id, &mut value);
        }
        field = next;
        let piece = unquote(rest);
        match field {
            Field::Ctx => ctx.get_or_insert_with(String::new).push_str(&piece),
            Field::Id => id.push_str(&piece),
            Field::Str => value.push_str(&piece),
            Field::None => {}
        }
    }
    flush(&mut ctx, &mut id, &mut value);
    map
}

/// Content of a `"…"` PO string with its escapes resolved.
fn unquote(s: &str) -> String {
    let s = s.trim();
    let inner = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(s);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_placeholders() {
        assert_eq!(fill("{} of {}", &[&3, &"ten"]), "3 of ten");
        assert_eq!(fill("{1} de {0}", &[&3, &"diez"]), "diez de 3");
        assert_eq!(fill("{{literal}} {}", &[&1]), "{literal} 1");
        assert_eq!(fill("missing {}", &[]), "missing {}");
        assert_eq!(fill("ñandú {} °C", &[&"36.6"]), "ñandú 36.6 °C");
    }

    #[test]
    fn parses_po() {
        let po = "# comment\nmsgid \"\"\nmsgstr \"\"\n\"Content-Type: text/plain\\n\"\n\n\
                  msgid \"Open\"\nmsgstr \"Abrir\"\n\n\
                  msgctxt \"state\"\nmsgid \"Open\"\nmsgstr \"Abierto\"\n\n\
                  msgid \"Line one\\n\"\n\"line two\"\nmsgstr \"Línea uno\\n\"\n\"línea dos\"\n\n\
                  msgid \"Untranslated\"\nmsgstr \"\"\n";
        let map = parse_po(po);
        assert_eq!(map.get("Open"), Some(&"Abrir"));
        assert_eq!(map.get("state\u{4}Open"), Some(&"Abierto"));
        assert_eq!(map.get("Line one\nline two"), Some(&"Línea uno\nlínea dos"));
        assert!(!map.contains_key("Untranslated"));
        assert!(!map.contains_key(""));
    }

    #[test]
    fn language_codes() {
        assert_eq!(Language::from_code("es_MX.UTF-8"), Some(Language::Spanish));
        assert_eq!(Language::from_code("en"), Some(Language::English));
        assert_eq!(Language::from_code("fr"), None);
        assert_eq!(serde_json::to_string(&Language::Spanish).unwrap(), "\"es\"");
    }

    /// Every catalog entry keeps the placeholders of its English text.
    #[test]
    fn catalog_placeholders_match() {
        let count = |s: &str| fill(s, &[&"\u{1}", &"\u{1}", &"\u{1}", &"\u{1}", &"\u{1}", &"\u{1}", &"\u{1}", &"\u{1}"]).matches('\u{1}').count();
        for lang in Language::ALL {
            let Some(src) = lang.catalog_source() else { continue };
            for (english, translated) in parse_po(src) {
                let english = english.rsplit('\u{4}').next().unwrap_or(&english).to_string();
                assert_eq!(count(&english), count(translated), "{lang:?}: “{english}” → “{translated}”");
            }
        }
    }
}
