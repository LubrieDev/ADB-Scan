//! The keyboard (input method) the phone uses, among the ones enabled in
//! Settings → System → Keyboard. Reading is read-only (`ime list -s`,
//! `settings get secure default_input_method`); choosing another one
//! (`ime set <id>`) **changes the phone** and is undone by choosing the
//! previous one. Keyboards that are installed but not enabled are not offered:
//! enabling one is done on the phone, where Android warns about what a
//! keyboard can read.

/// Read-only.
pub const SCRIPT: &str = "echo @@@LIST@@@; ime list -s 2>&1
echo @@@NOW@@@; settings get secure default_input_method 2>&1
echo @@@END@@@
";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keyboards {
    /// `package/.Service` of the keyboard in use.
    pub current: Option<String>,
    /// Enabled keyboards, in Android's order, without repeats.
    pub enabled: Vec<String>,
}

/// An input method id: `package/class` with the characters Android uses.
pub fn is_valid_id(id: &str) -> bool {
    let Some((pkg, class)) = id.split_once('/') else { return false };
    crate::adb::is_valid_package_name(pkg) && !class.is_empty() && class.len() <= 200 && class.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'$')
}

/// The app of an input method id.
pub fn package_of(id: &str) -> &str {
    id.split('/').next().unwrap_or(id)
}

pub fn parse(text: &str) -> Keyboards {
    let s = crate::parsers::split_sections(text);
    let mut enabled: Vec<String> = Vec::new();
    for l in s.get("LIST").map(String::as_str).unwrap_or("").lines().map(str::trim) {
        if is_valid_id(l) && !enabled.iter().any(|e| e == l) {
            enabled.push(l.to_string());
        }
    }
    let current = s.get("NOW").map(|v| v.trim().to_string()).filter(|v| is_valid_id(v));
    Keyboards { current, enabled }
}

/// Arguments that make `id` the keyboard in use.
pub fn set_command(id: &str) -> Option<Vec<String>> {
    is_valid_id(id).then(|| vec!["ime".into(), "set".into(), id.into()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboards() {
        // As Android 16 prints it (a keyboard may appear twice).
        let k = parse("@@@LIST@@@\ncom.google.android.inputmethod.latin/com.android.inputmethod.latin.LatinIME\ndev.patrickgold.florisboard/.FlorisImeService\ndev.patrickgold.florisboard/.FlorisImeService\n@@@NOW@@@\ndev.patrickgold.florisboard/.FlorisImeService\n@@@END@@@\n");
        assert_eq!(k.enabled.len(), 2);
        assert_eq!(k.current.as_deref(), Some("dev.patrickgold.florisboard/.FlorisImeService"));
        assert_eq!(package_of(&k.enabled[0]), "com.google.android.inputmethod.latin");
        assert_eq!(set_command(&k.enabled[1]).unwrap().join(" "), "ime set dev.patrickgold.florisboard/.FlorisImeService");
        assert!(set_command("a/b; reboot").is_none());
        assert_eq!(parse("@@@LIST@@@\n@@@NOW@@@\nnull\n@@@END@@@\n").current, None);
    }
}
