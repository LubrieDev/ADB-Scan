//! Default apps (Android 10+ roles): browser, home screen, phone, SMS and
//! assistant. Reading is read-only: `cmd role get-role-holders` for the
//! current app and `cmd package query-activities --components` (or
//! `query-services`) for the apps that can take the role.
//!
//! Changing one (`cmd role add-role-holder`) **changes the phone**; it is the
//! same as choosing it in Settings → Apps → Default apps, and is undone the
//! same way. Android only accepts apps that qualify for the role; for the
//! exclusive ones (all of these) the previous app stops holding it.
//!
//! Commands and output from Android 16's `RoleShellCommand` (holders joined
//! with `;`).

use crate::i18n::tr;

/// A role offered here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Browser,
    Home,
    Dialer,
    Sms,
    Assistant,
}

impl Role {
    pub const ALL: [Role; 5] = [Role::Browser, Role::Home, Role::Dialer, Role::Sms, Role::Assistant];

    /// `android.app.role.…`
    pub fn name(self) -> &'static str {
        match self {
            Role::Browser => "android.app.role.BROWSER",
            Role::Home => "android.app.role.HOME",
            Role::Dialer => "android.app.role.DIALER",
            Role::Sms => "android.app.role.SMS",
            Role::Assistant => "android.app.role.ASSISTANT",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Role::Browser => tr("Browser"),
            Role::Home => tr("Home screen (launcher)"),
            Role::Dialer => tr("Phone"),
            Role::Sms => tr("SMS"),
            Role::Assistant => tr("Digital assistant"),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Role::Browser => "BROWSER",
            Role::Home => "HOME",
            Role::Dialer => "DIALER",
            Role::Sms => "SMS",
            Role::Assistant => "ASSISTANT",
        }
    }

    /// Query for the apps that can take it.
    fn candidates_query(self) -> &'static str {
        match self {
            Role::Browser => "cmd package query-activities --components -a android.intent.action.VIEW -c android.intent.category.BROWSABLE -d http://example.com",
            Role::Home => "cmd package query-activities --components -a android.intent.action.MAIN -c android.intent.category.HOME",
            Role::Dialer => "cmd package query-activities --components -a android.intent.action.DIAL",
            Role::Sms => "cmd package query-activities --components -a android.intent.action.SENDTO -d smsto:",
            Role::Assistant => "cmd package query-services --components -a android.service.voice.VoiceInteractionService",
        }
    }
}

/// Read-only script (fixed commands).
pub fn script() -> String {
    let mut s = String::new();
    for r in Role::ALL {
        s.push_str(&format!("echo @@@HOLDER_{}@@@; cmd role get-role-holders {} 2>&1\n", r.key(), r.name()));
        s.push_str(&format!("echo @@@CAND_{}@@@; {} 2>&1\n", r.key(), r.candidates_query()));
    }
    // Android 11 and older have no `get-role-holders`: their holders come from
    // `dumpsys role` (only asked for there).
    s.push_str("echo @@@DUMP@@@; cmd role get-role-holders android.app.role.HOME 2>&1 | grep -q 'Unknown command' && dumpsys role 2>&1 | grep -E '^ *(user_id|name|holders)='\n");
    s.push_str("echo @@@END@@@\n");
    s
}

/// `dumpsys role` (lines `user_id=`, `name=`, `holders=`): role → holders of
/// user 0.
fn parse_dump(text: &str) -> std::collections::HashMap<String, Vec<String>> {
    let mut map = std::collections::HashMap::new();
    let mut user0 = true;
    let mut name: Option<String> = None;
    for line in text.lines().map(str::trim) {
        if let Some(u) = line.strip_prefix("user_id=") {
            user0 = u.trim() == "0";
        } else if let Some(n) = line.strip_prefix("name=") {
            name = user0.then(|| n.trim().to_string());
        } else if let (Some(h), Some(n)) = (line.strip_prefix("holders="), name.take()) {
            let list: Vec<String> = h.split([',', ';', '[', ']', ' ']).filter(|p| crate::adb::is_valid_package_name(p)).map(String::from).collect();
            map.entry(n).or_insert(list);
        }
    }
    map
}

/// One role on the phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleState {
    pub role: Role,
    /// Current app(s); empty: none chosen.
    pub holders: Vec<String>,
    /// Apps that can take it (sorted, without repeats).
    pub candidates: Vec<String>,
    /// The phone has no way to read roles (Android 9 or older).
    pub unsupported: bool,
}

fn packages_of_components(text: &str) -> Vec<String> {
    let mut v: Vec<String> = text
        .lines()
        .filter_map(|l| {
            let (pkg, _) = l.trim().split_once('/')?;
            crate::adb::is_valid_package_name(pkg).then(|| pkg.to_string())
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

pub fn parse(text: &str) -> Vec<RoleState> {
    let s = crate::parsers::split_sections(text);
    let empty = String::new();
    let dump = s.get("DUMP").filter(|t| t.contains("name=")).map(|t| parse_dump(t));
    Role::ALL
        .iter()
        .map(|&role| {
            let holders_text = s.get(&format!("HOLDER_{}", role.key())).unwrap_or(&empty);
            let no_command = crate::probe::classify_failure(holders_text).is_some() || holders_text.contains("Unknown command") || holders_text.contains("Can't find service");
            let unsupported = no_command && dump.is_none();
            let holders = if no_command {
                dump.as_ref().and_then(|d| d.get(role.name()).cloned()).unwrap_or_default()
            } else { holders_text.trim().split(';').map(str::trim).filter(|p| crate::adb::is_valid_package_name(p)).map(String::from).collect() };
            let mut candidates = packages_of_components(s.get(&format!("CAND_{}", role.key())).unwrap_or(&empty));
            // The current one is always among the choices.
            for h in &holders {
                if !candidates.contains(h) {
                    candidates.push(h.clone());
                }
            }
            candidates.sort();
            RoleState { role, holders, candidates, unsupported }
        })
        .collect()
}

/// Arguments to make `package` the default app for `role`.
pub fn set_command(role: Role, package: &str) -> Vec<String> {
    vec!["cmd".into(), "role".into(), "add-role-holder".into(), "--user".into(), "0".into(), role.name().into(), package.into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_11_dump() {
        let out = "@@@HOLDER_BROWSER@@@\nUnknown command: get-role-holders\n@@@CAND_BROWSER@@@\ncom.android.chrome/.Main\n\
@@@HOLDER_HOME@@@\nUnknown command: get-role-holders\n@@@CAND_HOME@@@\n\
@@@HOLDER_DIALER@@@\nUnknown command: get-role-holders\n@@@CAND_DIALER@@@\n\
@@@HOLDER_SMS@@@\nUnknown command: get-role-holders\n@@@CAND_SMS@@@\n\
@@@HOLDER_ASSISTANT@@@\nUnknown command: get-role-holders\n@@@CAND_ASSISTANT@@@\n\
@@@DUMP@@@\n    user_id=0\n        name=android.app.role.CALL_SCREENING\n        name=android.app.role.SMS\n        holders=com.google.android.apps.messaging\n\
        name=android.app.role.HOME\n        holders=com.android.launcher3\n        name=android.app.role.BROWSER\n        holders=com.android.chrome\n\
    user_id=10\n        name=android.app.role.DIALER\n        holders=com.other.dialer\n@@@END@@@\n";
        let r = parse(out);
        assert!(r.iter().all(|x| !x.unsupported));
        assert_eq!(r[0].holders, ["com.android.chrome"]);
        assert_eq!(r[1].holders, ["com.android.launcher3"]);
        assert_eq!(r[1].candidates, ["com.android.launcher3"]);
        // The dialer of another user does not count; SMS keeps its own holder.
        assert!(r[2].holders.is_empty());
        assert_eq!(r[3].holders, ["com.google.android.apps.messaging"]);
        // Without the dump either, it is unsupported (Android 9).
        let old = out.split("@@@DUMP@@@").next().unwrap().to_string() + "@@@DUMP@@@\n@@@END@@@\n";
        assert!(parse(&old).iter().all(|x| x.unsupported));
    }

    #[test]
    fn holders_and_candidates() {
        let out = "@@@HOLDER_BROWSER@@@\norg.mozilla.firefox\n@@@CAND_BROWSER@@@\ncom.android.chrome/com.google.android.apps.chrome.IntentDispatcher\norg.mozilla.firefox/org.mozilla.fenix.IntentReceiverActivity\ncom.android.chrome/.Other\n\
@@@HOLDER_HOME@@@\ncom.nubia.launcher\n@@@CAND_HOME@@@\ncom.nubia.launcher/.Launcher\ncom.android.settings/.FallbackHome\n\
@@@HOLDER_DIALER@@@\n\n@@@CAND_DIALER@@@\nNo activities found\n\
@@@HOLDER_SMS@@@\ncom.google.android.apps.messaging\n@@@CAND_SMS@@@\n\
@@@HOLDER_ASSISTANT@@@\n/system/bin/sh: cmd: not found\n@@@CAND_ASSISTANT@@@\n@@@END@@@\n";
        let r = parse(out);
        let browser = &r[0];
        assert_eq!(browser.holders, ["org.mozilla.firefox"]);
        assert_eq!(browser.candidates, ["com.android.chrome", "org.mozilla.firefox"]);
        assert_eq!(r[1].candidates.len(), 2);
        assert!(r[2].holders.is_empty() && r[2].candidates.is_empty());
        // The holder is offered even if the query found nothing.
        assert_eq!(r[3].candidates, ["com.google.android.apps.messaging"]);
        assert!(r[4].unsupported);
        assert_eq!(set_command(Role::Browser, "org.mozilla.firefox").join(" "), "cmd role add-role-holder --user 0 android.app.role.BROWSER org.mozilla.firefox");
        assert!(script().contains("query-services"));
    }
}
