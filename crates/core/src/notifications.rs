//! Phone notifications, to see them on the laptop.
//!
//! **Privacy:** the text of notifications (messages, verification codes…) is
//! among the most sensitive data on the phone. That is why the feature is off
//! by default, it is only read while it is on, nothing is stored on disk or in
//! History and the interface respects private mode. Read-only: it does not
//! dismiss or touch any notification.

/// Read-only. `--noredact` gives the real text (without it, Android only gives
/// the length).
pub const READ_SCRIPT: &str = "dumpsys notification --noredact 2>&1";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PhoneNotification {
    /// Android's unique key (`0|package|id|tag|uid`).
    pub key: String,
    pub package: String,
    pub title: String,
    pub text: String,
    /// Ongoing (music, downloads, services): not forwarded.
    pub ongoing: bool,
    /// Group summary: it repeats the group's notifications, not forwarded.
    pub summary: bool,
}

impl PhoneNotification {
    /// Does it deserve an alert on the laptop?
    pub fn worth_showing(&self) -> bool {
        !self.ongoing && !self.summary && !(self.title.is_empty() && self.text.is_empty())
    }
}

/// Value of an `extras` line: `android.title=String (Hello)`. Without
/// `--noredact` it arrives as `String [length=4]`, which is not text.
fn extra_value(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.strip_prefix('=')?;
    let (_, value) = rest.split_once(" (")?;
    let value = value.strip_suffix(')').unwrap_or(value);
    Some(value.to_string()).filter(|v| !v.is_empty() && v != "null")
}

fn field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let start = line.find(&format!(" {name}="))? + name.len() + 2;
    line[start..].split_whitespace().next()
}

/// Notifications in the “Notification List” of `dumpsys notification`.
pub fn parse(text: &str) -> Vec<PhoneNotification> {
    let mut out: Vec<PhoneNotification> = Vec::new();
    let indent = |l: &str| l.len() - l.trim_start().len();
    // Indentation of “Notification List:”; `None` until it is found.
    let mut list_indent: Option<usize> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("Notification List:") {
            list_indent = Some(indent(raw));
            continue;
        }
        let Some(li) = list_indent else { continue };
        // End of the list: the next section (“Snoozed notifications:”…) has the
        // same indentation or less.
        if !line.is_empty() && indent(raw) <= li {
            break;
        }
        if line.starts_with("NotificationRecord(") {
            let flags = field(line, "flags").unwrap_or("").trim_end_matches(')');
            // With names (`ONGOING_EVENT|NO_CLEAR`) or in hexadecimal (`0x62`): ONGOING_EVENT
            // 0x2, NO_CLEAR 0x20, FOREGROUND_SERVICE 0x40, GROUP_SUMMARY 0x200.
            let bits = flags.strip_prefix("0x").and_then(|h| u32::from_str_radix(h, 16).ok()).unwrap_or(0);
            out.push(PhoneNotification {
                key: field(line, "key").unwrap_or("").to_string(),
                package: field(line, "pkg").unwrap_or("").to_string(),
                ongoing: bits & 0x62 != 0 || flags.contains("ONGOING_EVENT") || flags.contains("FOREGROUND_SERVICE") || flags.contains("NO_CLEAR"),
                summary: bits & 0x200 != 0 || flags.contains("GROUP_SUMMARY"),
                ..Default::default()
            });
            continue;
        }
        let Some(n) = out.last_mut() else { continue };
        if let Some(v) = extra_value(line, "android.title") {
            n.title = v;
        } else if let Some(v) = extra_value(line, "android.bigText").or_else(|| extra_value(line, "android.text")) {
            // The long text, if any, comes after and completes the short one.
            if n.text.len() < v.len() {
                n.text = v;
            }
        }
    }
    out.retain(|n| !n.key.is_empty());
    out
}
