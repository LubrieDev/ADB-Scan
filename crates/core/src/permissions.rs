//! Who can do what: the phone's sensitive permissions seen from the
//! permission's side (which apps can use the camera, the microphone, the
//! location…), and the special accesses (display over other apps, install
//! apps, accessibility, notification access…). Each one can be taken away
//! from an app with shell permissions only.
//!
//! Reading is a single read-only script ([`SCRIPT`]); the changes are
//! `pm revoke`, `appops set … deny`, `cmd notification disallow_listener` and
//! rewriting the list of accessibility services without one of them.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::i18n::tr;

/// How an access is granted, and therefore how it is taken away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Runtime permissions (`pm revoke`): any of them grants the access.
    Runtime(&'static [&'static str]),
    /// An App Op set to “allow” (`appops set … deny`).
    AppOp(&'static str),
    /// An enabled accessibility service.
    Accessibility,
    /// An app allowed to read notifications.
    NotificationListener,
}

/// One access, as the user thinks of it (“Camera”, “Display over other
/// apps”).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
    pub kind: Kind,
}

impl Access {
    pub fn is_special(&self) -> bool {
        !matches!(self.kind, Kind::Runtime(_))
    }
}

const P: &str = "android.permission.";

/// The App Ops this module reads and changes (also the only ones accepted
/// by [`crate::adb::Adb::set_app_op`]).
pub const APP_OPS: [&str; 5] = ["SYSTEM_ALERT_WINDOW", "REQUEST_INSTALL_PACKAGES", "MANAGE_EXTERNAL_STORAGE", "GET_USAGE_STATS", "WRITE_SETTINGS"];

/// The accesses shown, most sensitive first.
pub fn catalog() -> Vec<Access> {
    let a = |id, title, description, icon, kind| Access { id, title, description, icon, kind };
    vec![
        a("location", tr("Location"), tr("Where the phone is, precisely or approximately."), "adb-mark-location-symbolic", Kind::Runtime(&["ACCESS_FINE_LOCATION", "ACCESS_COARSE_LOCATION"])),
        a("background_location", tr("Location in the background"), tr("Location even when the app is not open."), "adb-mark-location-symbolic", Kind::Runtime(&["ACCESS_BACKGROUND_LOCATION"])),
        a("camera", tr("Camera"), tr("Take photos and record video."), "adb-camera-photo-symbolic", Kind::Runtime(&["CAMERA"])),
        a("microphone", tr("Microphone"), tr("Record audio."), "adb-audio-volume-high-symbolic", Kind::Runtime(&["RECORD_AUDIO"])),
        a("contacts", tr("Contacts"), tr("Read and change your contacts."), "adb-smartphone-symbolic", Kind::Runtime(&["READ_CONTACTS", "WRITE_CONTACTS", "GET_ACCOUNTS"])),
        a("sms", tr("SMS"), tr("Read, receive and send text messages."), "adb-send-to-symbolic", Kind::Runtime(&["READ_SMS", "RECEIVE_SMS", "SEND_SMS", "RECEIVE_MMS", "RECEIVE_WAP_PUSH"])),
        a("call_log", tr("Call log"), tr("Read and change the call history."), "adb-call-start-symbolic", Kind::Runtime(&["READ_CALL_LOG", "WRITE_CALL_LOG", "PROCESS_OUTGOING_CALLS"])),
        a("phone", tr("Phone"), tr("Make calls and read the phone's status and number."), "adb-call-start-symbolic", Kind::Runtime(&["CALL_PHONE", "READ_PHONE_STATE", "READ_PHONE_NUMBERS", "ANSWER_PHONE_CALLS"])),
        a("calendar", tr("Calendar"), tr("Read and change your events."), "adb-alarm-symbolic", Kind::Runtime(&["READ_CALENDAR", "WRITE_CALENDAR"])),
        a("media", tr("Photos, videos and files"), tr("Read the photos, videos, audio and files on the phone."), "adb-images-symbolic", Kind::Runtime(&["READ_MEDIA_IMAGES", "READ_MEDIA_VIDEO", "READ_MEDIA_AUDIO", "READ_EXTERNAL_STORAGE", "WRITE_EXTERNAL_STORAGE"])),
        a("sensors", tr("Body sensors"), tr("Heart rate and other health sensors."), "adb-heart-pulse-symbolic", Kind::Runtime(&["BODY_SENSORS", "BODY_SENSORS_BACKGROUND", "ACTIVITY_RECOGNITION"])),
        a("nearby", tr("Nearby devices"), tr("Find and connect to Bluetooth, Wi-Fi and UWB devices nearby."), "adb-bluetooth-symbolic", Kind::Runtime(&["BLUETOOTH_SCAN", "BLUETOOTH_CONNECT", "BLUETOOTH_ADVERTISE", "NEARBY_WIFI_DEVICES", "UWB_RANGING"])),
        a("accessibility", tr("Accessibility service"), tr("Sees everything on the screen and can tap for you. The most powerful access an app can get."), "adb-eye-off-symbolic", Kind::Accessibility),
        a("notifications", tr("Notification access"), tr("Reads every notification, including messages and codes."), "adb-bell-ring-symbolic", Kind::NotificationListener),
        a("overlay", tr("Display over other apps"), tr("Draws on top of other apps (bubbles, floating windows, and also fake screens)."), "adb-layout-grid-symbolic", Kind::AppOp("SYSTEM_ALERT_WINDOW")),
        a("install", tr("Install unknown apps"), tr("Installs APKs without going through the store."), "adb-system-software-install-symbolic", Kind::AppOp("REQUEST_INSTALL_PACKAGES")),
        a("all_files", tr("All files access"), tr("Reads and changes every file in the shared storage."), "adb-folder-symbolic", Kind::AppOp("MANAGE_EXTERNAL_STORAGE")),
        a("usage", tr("Usage access"), tr("Knows which apps you use and for how long."), "adb-chart-line-symbolic", Kind::AppOp("GET_USAGE_STATS")),
        a("settings", tr("Modify system settings"), tr("Changes settings such as brightness or the ringtone."), "adb-settings-symbolic", Kind::AppOp("WRITE_SETTINGS")),
    ]
}

/// An app that has an access.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Holder {
    pub package: String,
    /// What grants it: permissions (short names), the App Op or the
    /// component (`package/class`).
    pub grants: Vec<String>,
    /// Android does not let it be taken away (system or policy fixed).
    pub fixed: bool,
    /// Preinstalled app.
    pub system: bool,
}

/// The phone's state: for each access in [`catalog`] order, its holders.
#[derive(Clone, Debug, Default)]
pub struct Overview {
    pub accesses: Vec<(Access, Vec<Holder>)>,
}

/// Read-only script. Sections: the package dump, the user's apps, one per
/// App Op and the two secure settings.
pub const SCRIPT: &str = "dumpsys package packages 2>/dev/null; echo @@@USERAPPS@@@; pm list packages -3 2>/dev/null; \
for op in SYSTEM_ALERT_WINDOW REQUEST_INSTALL_PACKAGES MANAGE_EXTERNAL_STORAGE GET_USAGE_STATS WRITE_SETTINGS; do echo \"@@@OP $op@@@\"; appops query-op --user 0 $op allow 2>/dev/null; done; \
echo @@@A11Y@@@; settings get secure enabled_accessibility_services 2>/dev/null; echo @@@NLS@@@; settings get secure enabled_notification_listeners 2>/dev/null";

/// A runtime permission granted to user 0: short name and whether it is fixed.
type Granted = HashMap<String, Vec<(String, bool)>>;

/// Granted runtime permissions per package for user 0, from
/// `dumpsys package packages` (up to “Hidden system packages”).
pub fn parse_runtime(dump: &str) -> Granted {
    let mut out: Granted = HashMap::new();
    let (mut pkg, mut user0) = (None::<String>, false);
    for line in dump.lines() {
        if line.starts_with("Hidden system packages") {
            break;
        }
        if let Some(rest) = line.strip_prefix("  Package [") {
            pkg = rest.split(']').next().map(String::from);
            user0 = false;
            continue;
        }
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("User ") {
            if line.starts_with("    User ") {
                user0 = rest.split(':').next() == Some("0");
            }
            continue;
        }
        let (Some(p), true) = (&pkg, user0) else { continue };
        let Some(rest) = t.strip_prefix(P) else { continue };
        let Some((name, state)) = rest.split_once(": granted=") else { continue };
        if !state.starts_with("true") || !name.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()) {
            continue;
        }
        let fixed = state.contains("SYSTEM_FIXED") || state.contains("POLICY_FIXED");
        out.entry(p.clone()).or_default().push((name.to_string(), fixed));
    }
    out
}

/// Components of a `:`-separated secure setting (`null` is empty).
pub fn parse_components(value: &str) -> Vec<String> {
    let v = value.trim();
    if v.is_empty() || v == "null" {
        return Vec::new();
    }
    v.split(':').map(str::trim).filter(|c| is_valid_component(c)).map(String::from).collect()
}

/// `package/class` with the characters Android allows (the class may start
/// with `.` or contain `$`).
pub fn is_valid_component(c: &str) -> bool {
    let Some((pkg, class)) = c.split_once('/') else { return false };
    crate::adb::is_valid_package_name(pkg) && !class.is_empty() && class.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '$'))
}

/// The setting's new value without `component` (empty if none is left).
pub fn without_component(value: &str, component: &str) -> String {
    parse_components(value).into_iter().filter(|c| c != component).collect::<Vec<_>>().join(":")
}

fn sections(out: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut name = "DUMP".to_string();
    let mut body = String::new();
    for line in out.lines() {
        if let Some(n) = line.strip_prefix("@@@").and_then(|l| l.strip_suffix("@@@")) {
            map.insert(std::mem::replace(&mut name, n.to_string()), std::mem::take(&mut body));
            continue;
        }
        body.push_str(line);
        body.push('\n');
    }
    map.insert(name, body);
    map
}

/// Parses the output of [`SCRIPT`].
pub fn parse(out: &str) -> Overview {
    let s = sections(out);
    let get = |k: &str| s.get(k).map(String::as_str).unwrap_or("");
    let runtime = parse_runtime(get("DUMP"));
    let user: HashSet<String> = crate::debloat::parse_package_list(get("USERAPPS")).into_iter().collect();
    let holder = |package: String, grants: Vec<String>, fixed: bool| Holder { system: !user.contains(&package), package, grants, fixed };
    let by_component = |list: Vec<String>| {
        let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for c in list {
            if let Some((p, _)) = c.split_once('/') {
                m.entry(p.to_string()).or_default().push(c.clone());
            }
        }
        m.into_iter().map(|(p, cs)| holder(p, cs, false)).collect::<Vec<_>>()
    };
    let accesses = catalog()
        .into_iter()
        .map(|a| {
            let mut holders: Vec<Holder> = match a.kind {
                Kind::Runtime(perms) => runtime
                    .iter()
                    .filter_map(|(pkg, granted)| {
                        let hits: Vec<&(String, bool)> = granted.iter().filter(|(n, _)| perms.contains(&n.as_str())).collect();
                        (!hits.is_empty()).then(|| holder(pkg.clone(), hits.iter().map(|(n, _)| n.clone()).collect(), hits.iter().all(|(_, f)| *f)))
                    })
                    .collect(),
                Kind::AppOp(op) => crate::debloat::parse_package_list(&get(&format!("OP {op}")).lines().map(|l| format!("package:{l}\n")).collect::<String>()).into_iter().map(|p| holder(p, vec![op.to_string()], false)).collect(),
                Kind::Accessibility => by_component(parse_components(get("A11Y"))),
                Kind::NotificationListener => by_component(parse_components(get("NLS"))),
            };
            holders.sort_by(|x, y| x.system.cmp(&y.system).then(x.package.cmp(&y.package)));
            (a, holders)
        })
        .collect();
    Overview { accesses }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = "Packages:
  Package [com.example.cam] (1):
    install permissions:
      android.permission.INTERNET: granted=true
    User 0: installed=true
      runtime permissions:
        android.permission.CAMERA: granted=true, flags=[ USER_SET ]
        android.permission.RECORD_AUDIO: granted=false, flags=[ ]
        android.permission.ACCESS_FINE_LOCATION: granted=true, flags=[ USER_SET ]
    User 10: installed=true
      runtime permissions:
        android.permission.RECORD_AUDIO: granted=true, flags=[ ]
  Package [com.vendor.sys] (2):
    User 0: installed=true
      runtime permissions:
        android.permission.CAMERA: granted=true, flags=[ SYSTEM_FIXED|GRANTED_BY_DEFAULT ]
Hidden system packages:
  Package [com.example.cam] (3):
    User 0: installed=true
      runtime permissions:
        android.permission.READ_SMS: granted=true, flags=[ ]
";

    #[test]
    fn runtime_is_user_zero_only_and_stops_at_hidden() {
        let r = parse_runtime(DUMP);
        let cam = &r["com.example.cam"];
        assert_eq!(cam, &vec![("CAMERA".to_string(), false), ("ACCESS_FINE_LOCATION".to_string(), false)]);
        assert_eq!(r["com.vendor.sys"], vec![("CAMERA".to_string(), true)]);
    }

    #[test]
    fn overview_groups_holders() {
        let out = format!(
            "{DUMP}@@@USERAPPS@@@\npackage:com.example.cam\n@@@OP SYSTEM_ALERT_WINDOW@@@\ncom.termux.api\n@@@OP REQUEST_INSTALL_PACKAGES@@@\n@@@A11Y@@@\nnull\n@@@NLS@@@\ncom.a.b/com.a.b.Listener:com.c.d/.L$Inner\n"
        );
        let o = parse(&out);
        let get = |id: &str| o.accesses.iter().find(|(a, _)| a.id == id).unwrap().1.clone();
        let camera = get("camera");
        assert_eq!(camera.len(), 2);
        // User apps first; the fixed system one last.
        assert_eq!((camera[0].package.as_str(), camera[0].system, camera[0].fixed), ("com.example.cam", false, false));
        assert_eq!((camera[1].package.as_str(), camera[1].system, camera[1].fixed), ("com.vendor.sys", true, true));
        assert!(get("microphone").is_empty());
        assert_eq!(get("overlay")[0].package, "com.termux.api");
        assert!(get("accessibility").is_empty());
        let nls = get("notifications");
        assert_eq!(nls.iter().map(|h| h.package.as_str()).collect::<Vec<_>>(), ["com.a.b", "com.c.d"]);
        assert_eq!(nls[1].grants, ["com.c.d/.L$Inner"]);
    }

    #[test]
    fn components() {
        assert!(is_valid_component("com.a/.B$C"));
        assert!(!is_valid_component("com.a/B;rm -rf"));
        assert_eq!(without_component("com.a/.A:com.b/.B", "com.a/.A"), "com.b/.B");
        assert_eq!(without_component("null", "com.a/.A"), "");
    }
}
