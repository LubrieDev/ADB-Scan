//! Availability of each item read from the phone.
//!
//! The same command can work on one phone, be blocked by SELinux on another and
//! not exist on an old Android version. Each item is stored as a [`Probe`] so
//! the interface can say *why* it is missing instead of making up a value or
//! showing a “0”.

use crate::i18n::tr;
use serde::{Deserialize, Serialize};

/// Result of reading an item.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub enum Probe<T> {
    /// Read and with a value.
    Ok(T),
    /// The command responded, but this phone does not publish the item.
    #[default]
    Missing,
    /// Android denies it to the `shell` user (SELinux or permissions).
    Restricted(String),
    /// This Android version or this manufacturer does not have the command or
    /// service.
    Unsupported(String),
    /// Unexpected error: unrecognised output, timeout…
    Failed(String),
}

impl<T> Probe<T> {
    pub fn ok(&self) -> Option<&T> {
        match self {
            Probe::Ok(v) => Some(v),
            _ => None,
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, Probe::Ok(_))
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Probe<U> {
        match self {
            Probe::Ok(v) => Probe::Ok(f(v)),
            Probe::Missing => Probe::Missing,
            Probe::Restricted(r) => Probe::Restricted(r),
            Probe::Unsupported(r) => Probe::Unsupported(r),
            Probe::Failed(r) => Probe::Failed(r),
        }
    }

    /// Text for the interface when there is no value; `None` if there is one.
    pub fn status_text(&self) -> Option<String> {
        match self {
            Probe::Ok(_) => None,
            Probe::Missing => Some(tr("This phone does not publish it").into()),
            Probe::Restricted(r) => Some(with_detail(tr("Restricted by Android"), r)),
            Probe::Unsupported(r) => Some(with_detail(tr("Not supported"), r)),
            Probe::Failed(r) => Some(with_detail(tr("Could not be obtained"), r)),
        }
    }

    /// Formatted value or the reason it is missing.
    pub fn display(&self, f: impl FnOnce(&T) -> String) -> String {
        match self {
            Probe::Ok(v) => f(v),
            other => other.status_text().unwrap_or_default(),
        }
    }
}

fn with_detail(head: &str, detail: &str) -> String {
    if detail.is_empty() {
        head.into()
    } else {
        format!("{head} ({detail})")
    }
}

/// Recognises Android's error messages in an output. The checks are textual
/// because `adb shell` does not always propagate the exit code and each tool
/// (toybox, `cmd`, `dumpsys`) reports in its own way.
pub fn classify_failure(text: &str) -> Option<Probe<()>> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    // Only the first lines are checked: a long dump may contain these words as
    // part of its normal content.
    let head: String = t.lines().take(3).collect::<Vec<_>>().join("\n");
    let lower = head.to_lowercase();
    const RESTRICTED: [&str; 6] = ["permission denied", "operation not permitted", "securityexception", "requires android.permission", "not allowed to", "access denied"];
    const UNSUPPORTED: [&str; 8] =
        ["can't find service", "inaccessible or not found", ": not found", "unknown command", "no shell command implementation", "unknown option", "no such file or directory", "not supported"];
    let detail = head.lines().next().unwrap_or("").trim().chars().take(120).collect::<String>();
    if RESTRICTED.iter().any(|k| lower.contains(k)) {
        return Some(Probe::Restricted(detail));
    }
    if UNSUPPORTED.iter().any(|k| lower.contains(k)) {
        return Some(Probe::Unsupported(detail));
    }
    None
}

/// Interprets a section: first the known errors; if there are none, the parser
/// decides whether there is data (`Some`) or it is not published (`None`). A
/// missing section (the script never got to print it) is a failure.
pub fn from_section<T>(section: Option<&String>, parse: impl FnOnce(&str) -> Option<T>) -> Probe<T> {
    let Some(text) = section else {
        return Probe::Failed(tr("no response").into());
    };
    if let Some(p) = classify_failure(text) {
        return p.map_never();
    }
    match parse(text) {
        Some(v) => Probe::Ok(v),
        None => Probe::Missing,
    }
}

impl Probe<()> {
    /// Converts an error from [`classify_failure`] to the requested type.
    pub fn map_never<T>(self) -> Probe<T> {
        match self {
            Probe::Ok(()) | Probe::Missing => Probe::Missing,
            Probe::Restricted(r) => Probe::Restricted(r),
            Probe::Unsupported(r) => Probe::Unsupported(r),
            Probe::Failed(r) => Probe::Failed(r),
        }
    }
}

/// Value of a system property: empty or absent counts as not published.
pub fn prop(props: &std::collections::BTreeMap<String, String>, key: &str) -> Probe<String> {
    match props.get(key).map(|v| v.trim()) {
        Some(v) if !v.is_empty() => Probe::Ok(v.to_string()),
        _ => Probe::Missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_errors() {
        assert!(matches!(classify_failure("cat: /proc/pressure/memory: Permission denied"), Some(Probe::Restricted(_))));
        assert!(matches!(classify_failure("dmesg: klogctl: Permission denied"), Some(Probe::Restricted(_))));
        assert!(matches!(classify_failure("Can't find service: dnsresolver"), Some(Probe::Unsupported(_))));
        assert!(matches!(classify_failure("/system/bin/sh: getenforce: inaccessible or not found"), Some(Probe::Unsupported(_))));
        assert!(matches!(classify_failure("cat: /sys/x: No such file or directory"), Some(Probe::Unsupported(_))));
        assert_eq!(classify_failure("Enforcing"), None);
        assert_eq!(classify_failure(""), None);
    }

    #[test]
    fn only_looks_at_the_head() {
        let long = format!("{}\nPermission denied", "normal line\n".repeat(10));
        assert_eq!(classify_failure(&long), None);
    }

    #[test]
    fn section_states() {
        let ok = "4096".to_string();
        assert_eq!(from_section(Some(&ok), |t| t.trim().parse::<u32>().ok()), Probe::Ok(4096));
        let empty = String::new();
        assert_eq!(from_section(Some(&empty), |t| t.trim().parse::<u32>().ok()), Probe::Missing);
        assert!(matches!(from_section::<u32>(None, |_| None), Probe::Failed(_)));
        let denied = "cat: x: Permission denied".to_string();
        assert!(matches!(from_section(Some(&denied), |t| Some(t.to_string())), Probe::Restricted(_)));
    }
}
