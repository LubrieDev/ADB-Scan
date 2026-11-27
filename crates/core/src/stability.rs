//! Stability (read-only): apps that closed unexpectedly or froze (ANR),
//! native crashes, system restarts and the reason of the last start, from
//! Android's own drop box (`dumpsys dropbox`, kept for a few days).
//!
//! The list (`dumpsys dropbox`) gives every entry with its date and tag. For
//! crashes and freezes only the identifying lines of each report are brought
//! (`--print <tag>` filtered on the phone: the date line, `Process:`,
//! `Subject:`, the exception and the native signal), not the whole report.
//! Format from Android 16's `DropBoxManagerService.dump`:
//! `YYYY-MM-DD HH:MM:SS <tag> (text, <n> bytes)`.

use crate::i18n::tr;
use crate::t;

/// Drop box tags with a report worth reading.
pub const DETAIL_TAGS: [&str; 10] = [
    "data_app_crash",
    "system_app_crash",
    "system_server_crash",
    "data_app_anr",
    "system_app_anr",
    "system_server_anr",
    "data_app_native_crash",
    "system_app_native_crash",
    "system_server_watchdog",
    "SYSTEM_TOMBSTONE",
];

/// Signal debuggerd uses to dump a process that goes on running (fdsan and
/// other warnings): not a crash.
const DUMP_SIGNAL: &str = "signal 35 ";

/// Read-only. Only fixed tags; no user data.
pub fn script() -> String {
    format!(
        "echo @@@LIST@@@; dumpsys dropbox 2>&1
echo @@@DETAIL@@@; for t in {}; do dumpsys dropbox --print \"$t\" 2>/dev/null | grep -E '^(=====|[0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}} [0-9:]{{8}} |Process: |pid: [0-9]+, tid: |Subject: |signal [0-9]+|Abort message: |[A-Za-z0-9_.$]+(Exception|Error)(: |$))'; done
echo @@@BOOT@@@; getprop sys.boot.reason.last
echo @@@END@@@
",
        DETAIL_TAGS.join(" ")
    )
}

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// An app closed unexpectedly (Java/Kotlin exception).
    Crash,
    /// Native crash (C/C++ code, a signal).
    NativeCrash,
    /// The app stopped responding (ANR).
    Freeze,
    /// The system itself crashed or froze and restarted its interface.
    System,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Crash => tr("Closed unexpectedly"),
            Kind::NativeCrash => tr("Native crash"),
            Kind::Freeze => tr("Stopped responding"),
            Kind::System => tr("System failure"),
        }
    }

    fn of(tag: &str) -> Option<Kind> {
        Some(match tag {
            "system_server_crash" | "system_server_anr" | "system_server_watchdog" | "SYSTEM_RESTART" => Kind::System,
            t if t.ends_with("_native_crash") || t == "SYSTEM_TOMBSTONE" => Kind::NativeCrash,
            t if t.ends_with("_app_crash") => Kind::Crash,
            t if t.ends_with("_app_anr") => Kind::Freeze,
            _ => return None,
        })
    }
}

/// One crash or freeze.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Incident {
    /// `YYYY-MM-DD HH:MM:SS`, the phone's local time.
    pub time: String,
    pub tag: String,
    /// Process (usually the package), when the report says it.
    pub process: String,
    /// The reason: exception, ANR subject or native signal.
    pub detail: String,
}

impl Incident {
    pub fn kind(&self) -> Option<Kind> {
        Kind::of(&self.tag)
    }
}

/// Incidents of one app, for the summary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppStability {
    pub process: String,
    pub crashes: u32,
    pub freezes: u32,
    pub last: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stability {
    /// Newest first.
    pub incidents: Vec<Incident>,
    /// Oldest entry in the drop box (how far back it goes).
    pub since: Option<String>,
    /// System restarts of the interface (`SYSTEM_RESTART`) in the period.
    pub system_restarts: u32,
    /// Reason of the last start (`sys.boot.reason.last`), raw.
    pub last_boot_reason: String,
    /// Whether the drop box could be read at all.
    pub readable: bool,
}

impl Stability {
    /// Apps with incidents, most incidents first.
    pub fn by_app(&self) -> Vec<AppStability> {
        let mut out: Vec<AppStability> = Vec::new();
        for i in &self.incidents {
            let Some(kind) = i.kind() else { continue };
            if kind == Kind::System || i.process.is_empty() {
                continue;
            }
            let a = match out.iter_mut().position(|a| a.process == i.process) {
                Some(p) => &mut out[p],
                None => {
                    out.push(AppStability { process: i.process.clone(), ..Default::default() });
                    out.last_mut().unwrap()
                }
            };
            if kind == Kind::Freeze {
                a.freezes += 1;
            } else {
                a.crashes += 1;
            }
            if i.time > a.last {
                a.last = i.time.clone();
            }
        }
        out.sort_by(|a, b| (b.crashes + b.freezes).cmp(&(a.crashes + a.freezes)).then(b.last.cmp(&a.last)));
        out
    }
}

/// `2026-11-26 10:11:12 data_app_crash (text, 1234 bytes)` → (time, tag).
fn entry_line(line: &str) -> Option<(String, String)> {
    let l = line.trim();
    let bytes = l.as_bytes();
    let dated = l.len() > 20 && bytes[4] == b'-' && bytes[7] == b'-' && bytes[10] == b' ' && bytes[13] == b':' && bytes[16] == b':' && bytes[19] == b' ';
    if !dated || !bytes[..4].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let tag = l[20..].split_whitespace().next()?;
    Some((l[..19].to_string(), tag.to_string()))
}

/// Process name: `Process: com.app` (also `Process: com.app:remote`), or
/// in a tombstone `pid: 1, tid: 1, name: x  >>> com.app <<<` (a program is
/// given by its file name: `/system/bin/sh` → `sh`).
fn process_of(line: &str) -> Option<String> {
    let line = line.trim();
    if let Some(p) = line.strip_prefix("Process: ") {
        let p = p.trim();
        return Some(p.split_whitespace().next().unwrap_or(p).to_string());
    }
    if !line.starts_with("pid: ") {
        return None;
    }
    let (_, rest) = line.split_once(">>> ")?;
    let cmd = rest.split(" <<<").next()?.trim();
    let cmd = cmd.split_whitespace().next().unwrap_or(cmd);
    Some(cmd.rsplit('/').next().unwrap_or(cmd).to_string())
}

pub fn parse(text: &str) -> Stability {
    let s = crate::parsers::split_sections(text);
    let empty = String::new();
    let list = s.get("LIST").unwrap_or(&empty);
    let mut st = Stability { readable: list.contains("Drop box contents"), ..Default::default() };
    let entries: Vec<(String, String)> = list.lines().filter_map(entry_line).collect();
    st.since = entries.iter().map(|(t, _)| t.clone()).min();
    st.system_restarts = entries.iter().filter(|(_, tag)| tag == "SYSTEM_RESTART").count() as u32;

    // Reports: a dated line opens each one.
    let mut current: Option<Incident> = None;
    for line in s.get("DETAIL").unwrap_or(&empty).lines() {
        if let Some((time, tag)) = entry_line(line) {
            if let Some(i) = current.take() {
                st.incidents.push(i);
            }
            current = Some(Incident { time, tag, ..Default::default() });
            continue;
        }
        let Some(i) = current.as_mut() else { continue };
        let l = line.trim();
        if let Some(p) = process_of(l) {
            if i.process.is_empty() {
                i.process = p;
            }
        } else if i.detail.is_empty() && !l.starts_with("=====") && !l.is_empty() {
            let d = l.strip_prefix("Subject: ").unwrap_or(l);
            i.detail = d.chars().take(200).collect();
        }
    }
    if let Some(i) = current.take() {
        st.incidents.push(i);
    }
    let reported: std::collections::HashSet<(String, String)> = st.incidents.iter().map(|i| (i.time.clone(), i.tag.clone())).collect();
    // A process dumped for a warning went on running.
    st.incidents.retain(|i| !i.detail.starts_with(DUMP_SIGNAL));
    // Entries listed without a readable report (a restart, a report too big to
    // print) still count.
    for (time, tag) in &entries {
        if Kind::of(tag).is_some() && !reported.contains(&(time.clone(), tag.clone())) {
            st.incidents.push(Incident { time: time.clone(), tag: tag.clone(), ..Default::default() });
        }
    }
    st.incidents.sort_by(|a, b| b.time.cmp(&a.time));
    st.last_boot_reason = s.get("BOOT").map(|b| b.trim().to_string()).unwrap_or_default();
    st
}

/// The reason of a start in words (`reboot,userrequested`, `kernel_panic`…).
pub fn boot_reason_text(reason: &str) -> String {
    let r = reason.trim().to_lowercase();
    if r.is_empty() {
        return tr("Not given").into();
    }
    let detail = r.split(',').nth(1).unwrap_or("");
    if r.contains("panic") || r.contains("watchdog") || detail.contains("crash") {
        return t!("After an error ({})", reason.trim());
    }
    if r.contains("thermal") || r.contains("battery") || r.contains("undervoltage") {
        return t!("For safety ({})", reason.trim());
    }
    if r.contains("ota") || detail.contains("update") {
        return tr("To install a system update").into();
    }
    if detail == "userrequested" || detail == "shell" || detail == "adb" || r == "reboot" || r.starts_with("shutdown,userrequested") {
        return tr("Restarted by the user").into();
    }
    if r.starts_with("cold") || r.starts_with("hard") || r.starts_with("warm") {
        return t!("Unexpected restart ({})", reason.trim());
    }
    reason.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "@@@LIST@@@
Drop box contents: 6 entries
Max entries: 1000
Low priority rate limit period: 2000 ms
Low priority tags: {data_app_wtf}

2026-11-24 08:00:01 SYSTEM_BOOT (text, 120 bytes)
2026-11-25 10:11:12 data_app_crash (compressed text, 2034 bytes)
2026-11-25 12:00:00 data_app_anr (compressed text, 90000 bytes)
2026-11-25 13:00:00 SYSTEM_TOMBSTONE (compressed data, 4000 bytes)
2026-11-26 09:30:00 data_app_crash (compressed text, 2100 bytes)
2026-11-26 09:45:00 SYSTEM_RESTART (text, 300 bytes)
@@@DETAIL@@@
========================================
2026-11-25 10:11:12 data_app_crash (compressed text, 2034 bytes)
Process: com.example.chat
java.lang.IllegalStateException: Fragment not attached to a context.
========================================
2026-11-26 09:30:00 data_app_crash (compressed text, 2100 bytes)
Process: com.example.chat
java.lang.NullPointerException: Attempt to invoke virtual method
========================================
2026-11-25 12:00:00 data_app_anr (compressed text, 90000 bytes)
Process: com.example.video
Subject: Input dispatching timed out (Waiting to send key event)
@@@BOOT@@@
reboot,userrequested
@@@END@@@
";

    #[test]
    fn incidents_and_summary() {
        let s = parse(OUT);
        assert!(s.readable);
        assert_eq!(s.since.as_deref(), Some("2026-11-24 08:00:01"));
        assert_eq!(s.system_restarts, 1);
        // Three reports, the tombstone and the restart; newest first.
        assert_eq!(s.incidents.len(), 5);
        assert_eq!(s.incidents[0].tag, "SYSTEM_RESTART");
        assert_eq!(s.incidents[1].detail, "java.lang.NullPointerException: Attempt to invoke virtual method");
        let anr = s.incidents.iter().find(|i| i.tag == "data_app_anr").unwrap();
        assert_eq!(anr.detail, "Input dispatching timed out (Waiting to send key event)");
        assert_eq!(anr.kind(), Some(Kind::Freeze));
        let apps = s.by_app();
        assert_eq!(apps[0].process, "com.example.chat");
        assert_eq!((apps[0].crashes, apps[0].freezes), (2, 0));
        assert_eq!(apps[0].last, "2026-11-26 09:30:00");
        assert_eq!(apps[1].freezes, 1);
        assert_eq!(s.last_boot_reason, "reboot,userrequested");
    }

    #[test]
    fn tombstones() {
        // As a REDMAGIC NX809J (Android 16) prints them.
        let out = "@@@LIST@@@
Drop box contents: 3 entries

2026-11-23 18:03:35 SYSTEM_TOMBSTONE_PROTO_WITH_HEADERS (compressed data, 5313 bytes)
2026-11-23 18:03:35 SYSTEM_TOMBSTONE (compressed text, 7527 bytes)
2026-11-24 09:00:00 SYSTEM_TOMBSTONE (compressed text, 9000 bytes)
@@@DETAIL@@@
========================================
2026-11-23 18:03:35 SYSTEM_TOMBSTONE (compressed text, 7527 bytes)
pid: 18030, tid: 18030, name: sh  >>> /system/bin/sh <<<
signal 35 (<debuggerd signal>), code -1 (SI_QUEUE), fault addr --------
Abort message: 'attempted to close file descriptor 3, expected to be unowned, actually owned by DIR* 0x78a6c09220'
========================================
2026-11-24 09:00:00 SYSTEM_TOMBSTONE (compressed text, 9000 bytes)
pid: 4321, tid: 4400, name: RenderThread  >>> com.example.game <<<
signal 11 (SIGSEGV), code 1 (SEGV_MAPERR), fault addr 0x0000000000000010
@@@BOOT@@@
shutdown,unplug
@@@END@@@
";
        let s = parse(out);
        // The warning dump is not a crash.
        assert_eq!(s.incidents.len(), 1);
        assert_eq!(s.incidents[0].process, "com.example.game");
        assert!(s.incidents[0].detail.starts_with("signal 11 (SIGSEGV)"));
        assert_eq!(s.incidents[0].kind(), Some(Kind::NativeCrash));
        assert_eq!(process_of("pid: 1, tid: 1, name: sh  >>> /system/bin/sh <<<").as_deref(), Some("sh"));
    }

    #[test]
    fn unreadable_and_reasons() {
        let s = parse("@@@LIST@@@\nPermission Denial: can't dump dropbox\n@@@DETAIL@@@\n@@@BOOT@@@\n\n@@@END@@@\n");
        assert!(!s.readable);
        assert!(s.incidents.is_empty());
        assert_eq!(boot_reason_text("reboot,userrequested"), "Restarted by the user");
        assert!(boot_reason_text("kernel_panic").starts_with("After an error"));
        assert!(boot_reason_text("shutdown,thermal").starts_with("For safety"));
        assert!(script().contains("data_app_anr"));
    }
}
