//! Preinstalled apps that can be removed safely (“debloat”).
//!
//! Removing means `pm uninstall -k --user 0`: the app disappears for the
//! phone's user, but its APK stays in the system partition, so it can be
//! brought back at any time with `cmd package install-existing` (from the
//! Removed list) and a factory reset restores it too. Nothing here needs
//! root, and nothing touches the system partition.
//!
//! Only packages listed here are offered, each with what it does and a risk
//! level. Packages the phone needs (`is_protected`) are never offered, not
//! even if a future list included them by mistake.

use crate::i18n::tr;

/// How safe it is to remove an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Risk {
    /// Ads, telemetry, duplicates and promotional apps: nothing stops working.
    Recommended,
    /// A feature some people use (an app store, a browser, the weather…).
    Optional,
    /// Part of a system feature: remove only if you know you do not use it.
    Expert,
}

impl Risk {
    pub const ALL: [Risk; 3] = [Risk::Recommended, Risk::Optional, Risk::Expert];

    pub fn label(self) -> &'static str {
        match self {
            Risk::Recommended => tr("Recommended"),
            Risk::Optional => tr("Optional"),
            Risk::Expert => tr("Expert"),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Risk::Recommended => tr("Ads, telemetry, duplicates and promotional apps. Nothing you use stops working."),
            Risk::Optional => tr("Apps with a feature some people use. Remove them if you do not."),
            Risk::Expert => tr("Part of a system feature. Remove only if you know you do not need it."),
        }
    }
}

/// A preinstalled app that can be removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bloat {
    pub package: &'static str,
    /// Product name (not translated).
    pub name: &'static str,
    /// What it does and what is lost without it (translated).
    pub what: &'static str,
    pub risk: Risk,
    /// Who preinstalls it (`Google`, `Facebook`, `Xiaomi`…).
    pub vendor: &'static str,
}

fn b(vendor: &'static str, package: &'static str, name: &'static str, what: &'static str, risk: Risk) -> Bloat {
    Bloat { package, name, what, risk, vendor }
}

/// The curated list. Built at runtime so every description goes through
/// `tr` with a literal (and the catalog test checks its translation).
pub fn list() -> Vec<Bloat> {
    use Risk::*;
    vec![
        // Facebook services preinstalled by the manufacturer (not the app
        // itself): they update and report on Facebook apps in the background.
        b("Facebook", "com.facebook.appmanager", "Facebook App Manager", tr("Installs and updates Facebook apps in the background. The Facebook app, if you use it, updates from Play Store."), Recommended),
        b("Facebook", "com.facebook.services", "Facebook Services", tr("Background services for Facebook apps preinstalled by the manufacturer."), Recommended),
        b("Facebook", "com.facebook.system", "Facebook App Installer", tr("Installs Facebook apps without Play Store. Not needed to use Facebook."), Recommended),
        b("Facebook", "com.facebook.katana", "Facebook", tr("The Facebook app preinstalled by the manufacturer. It can be installed again from Play Store."), Optional),
        // Other preinstalled third-party apps.
        b("Netflix", "com.netflix.partner.activation", "Netflix Partner Activation", tr("Tells Netflix the phone came with the app preinstalled. Not needed to use Netflix."), Recommended),
        b("Netflix", "com.netflix.mediaclient", "Netflix", tr("Preinstalled Netflix app. It can be installed again from Play Store."), Optional),
        b("Microsoft", "com.microsoft.skydrive", "OneDrive", tr("Preinstalled Microsoft cloud storage."), Optional),
        b("Microsoft", "com.linkedin.android", "LinkedIn", tr("Preinstalled LinkedIn app."), Recommended),
        b("Microsoft", "com.microsoft.office.officehubrow", "Microsoft 365 (Office)", tr("Preinstalled Microsoft Office app."), Optional),
        b("Spotify", "com.spotify.music", "Spotify", tr("Preinstalled Spotify app. It can be installed again from Play Store."), Optional),
        b("Amazon", "com.amazon.mShop.android.shopping", "Amazon Shopping", tr("Preinstalled Amazon shop."), Recommended),
        // Google: apps that duplicate others or that not everyone uses. Play
        // services, Play Store, the keyboard, the dialer and contacts are
        // never offered.
        b("Google", "com.google.android.feedback", "Market Feedback Agent", tr("Sends feedback reports about apps installed from Play Store."), Recommended),
        b("Google", "com.google.android.printservice.recommendation", "Print Service Recommendation", tr("Suggests printing plugins. Printing still works with the plugins you install."), Recommended),
        b("Google", "com.google.android.apps.subscriptions.red", "Google One", tr("Google One subscription app. Your Google storage is not affected."), Recommended),
        b("Google", "com.google.android.apps.magazines", "Google News", tr("News reader."), Recommended),
        b("Google", "com.google.android.apps.podcasts", "Google Podcasts", tr("Podcast player."), Recommended),
        b("Google", "com.google.android.apps.books", "Google Play Books", tr("E-book reader and shop."), Recommended),
        b("Google", "com.google.android.videos", "Google TV", tr("Film and series shop and player."), Recommended),
        b("Google", "com.google.android.apps.youtube.music", "YouTube Music", tr("Music player and streaming. YouTube keeps working."), Optional),
        b("Google", "com.google.android.apps.tachyon", "Google Meet", tr("Video calls (formerly Duo). Without it, video calls from the dialer may not be offered."), Optional),
        b("Google", "com.google.android.apps.bard", "Gemini", tr("Google's AI assistant app."), Optional),
        b("Google", "com.google.android.apps.photos", "Google Photos", tr("Gallery and cloud backup of photos. Without it, use the manufacturer's gallery."), Optional),
        b("Google", "com.google.android.apps.docs", "Google Drive", tr("Google cloud storage."), Optional),
        b("Google", "com.google.android.youtube", "YouTube", tr("The YouTube app."), Optional),
        b("Google", "com.google.android.apps.maps", "Google Maps", tr("Maps and navigation. Apps that open a map will ask for another one."), Optional),
        b("Google", "com.google.android.gm", "Gmail", tr("Google mail app."), Optional),
        b("Google", "com.google.android.keep", "Google Keep", tr("Notes."), Optional),
        b("Google", "com.google.android.calendar", "Google Calendar", tr("Calendar app. Your events stay in your Google account."), Optional),
        b("Google", "com.google.android.apps.translate", "Google Translate", tr("Translator."), Optional),
        b("Google", "com.google.android.apps.wellbeing", "Digital Wellbeing", tr("Screen time, app timers and bedtime mode."), Optional),
        b("Google", "com.google.android.projection.gearhead", "Android Auto", tr("Phone on the car's screen. Needed only if you use Android Auto."), Optional),
        b("Google", "com.google.ar.core", "Google Play Services for AR", tr("Augmented reality. Some games and shops with AR need it."), Optional),
        b("Google", "com.google.android.apps.googleassistant", "Google Assistant", tr("Shortcut to the assistant. The Google app still provides it."), Optional),
        b("Google", "com.google.android.apps.restore", "Data Restore Tool", tr("Copies apps and data from another phone during setup. Not used afterwards."), Expert),
        b("Google", "com.google.android.googlequicksearchbox", "Google", tr("Google search, Discover and the assistant. The search widget and “Hey Google” stop working."), Expert),
        // Xiaomi (MIUI / HyperOS).
        b("Xiaomi", "com.miui.analytics", "MIUI Analytics", tr("Collects usage data for Xiaomi."), Recommended),
        b("Xiaomi", "com.miui.msa.global", "MIUI System Ads", tr("Shows ads in Xiaomi's system apps."), Recommended),
        b("Xiaomi", "com.xiaomi.mipicks", "GetApps", tr("Xiaomi's app store, which also shows suggestions and ads."), Recommended),
        b("Xiaomi", "com.miui.bugreport", "Feedback", tr("Sends bug reports to Xiaomi."), Recommended),
        b("Xiaomi", "com.miui.yellowpage", "Yellow Pages", tr("Business directory and caller information."), Recommended),
        b("Xiaomi", "com.xiaomi.glgm", "Games", tr("Xiaomi's game shop."), Recommended),
        b("Xiaomi", "com.mi.globalbrowser", "Mi Browser", tr("Xiaomi's browser, with news and ads. Install another browser first."), Optional),
        b("Xiaomi", "com.miui.videoplayer", "Mi Video", tr("Xiaomi's video player."), Optional),
        b("Xiaomi", "com.miui.player", "Mi Music", tr("Xiaomi's music player."), Optional),
        b("Xiaomi", "com.miui.compass", "Compass", tr("Compass."), Optional),
        b("Xiaomi", "com.miui.weather2", "Weather", tr("Xiaomi's weather app and its widget."), Optional),
        b("Xiaomi", "com.miui.notes", "Notes", tr("Xiaomi's notes. Back up your notes first."), Optional),
        // Samsung (One UI).
        b("Samsung", "com.samsung.android.app.spage", "Samsung Free", tr("News and promotions panel to the left of the home screen."), Recommended),
        b("Samsung", "com.samsung.android.app.tips", "Tips", tr("Samsung's tips."), Recommended),
        b("Samsung", "com.samsung.android.mateagent", "Galaxy Friends", tr("Promotions for Galaxy accessories."), Recommended),
        b("Samsung", "com.samsung.android.kidsinstaller", "Samsung Kids", tr("Installer of the children's mode."), Recommended),
        b("Samsung", "com.samsung.android.arzone", "AR Zone", tr("Augmented reality effects for the camera."), Recommended),
        b("Samsung", "com.samsung.android.visionintelligence", "Bixby Vision", tr("Recognises objects and text with the camera."), Optional),
        b("Samsung", "com.samsung.android.bixby.agent", "Bixby Voice", tr("Samsung's voice assistant. The side key can open something else instead."), Optional),
        b("Samsung", "com.samsung.android.aremoji", "AR Emoji", tr("Animated avatars for the camera."), Optional),
        b("Samsung", "com.sec.android.app.sbrowser", "Samsung Internet", tr("Samsung's browser. Install another browser first."), Optional),
        b("Samsung", "com.samsung.android.game.gamehome", "Game Launcher", tr("Game hub and in-game tools (Game Booster)."), Optional),
        b("Samsung", "com.sec.android.app.samsungapps", "Galaxy Store", tr("Samsung's store. Without it, some Samsung apps stop updating."), Expert),
        // nubia / REDMAGIC / ZTE.
        b("nubia", "com.zte.recommend", "App recommendations", tr("Suggests apps to install."), Optional),
        b("nubia", "com.nubia.apphub", "App Hub", tr("nubia's app store."), Optional),
        b("nubia", "com.zte.nubrowser", "Browser", tr("nubia's browser. Install another browser first, or links will not open."), Optional),
        b("nubia", "com.zte.weather", "Weather", tr("nubia's weather app and its widget."), Optional),
        b("nubia", "cn.zte.recorder", "Recorder", tr("Voice recorder."), Optional),
        b("nubia", "com.zte.livewallpaper", "Live wallpapers", tr("nubia's animated wallpapers. Choose a still wallpaper first."), Optional),
        b("nubia", "com.zte.nebulatranslation", "Translation", tr("nubia's on-screen translation."), Optional),
        b("nubia", "com.nubia.gamecenter", "Game Center", tr("REDMAGIC's game shop and hub."), Expert),
        b("nubia", "cn.nubia.gamelauncher", "Game Space", tr("REDMAGIC's game mode: shoulder triggers, fan and performance profiles depend on it."), Expert),
    ]
}

/// Packages that are never offered for removal: without them the phone does
/// not start, cannot call, cannot install or loses the ADB connection.
pub fn is_protected(package: &str) -> bool {
    const EXACT: &[&str] = &[
        "android",
        "com.android.systemui",
        "com.android.settings",
        "com.android.phone",
        "com.android.shell",
        "com.android.vending",
        "com.android.packageinstaller",
        "com.google.android.packageinstaller",
        "com.android.permissioncontroller",
        "com.google.android.permissioncontroller",
        "com.google.android.gms",
        "com.google.android.gsf",
        "com.google.android.webview",
        "com.android.webview",
        "com.android.networkstack",
        "com.google.android.networkstack",
        "com.android.bluetooth",
        "com.android.nfc",
        "com.android.inputmethod.latin",
        "com.google.android.inputmethod.latin",
    ];
    const PREFIXES: &[&str] = &["com.android.providers.", "com.google.android.providers.", "com.android.server.", "com.qualcomm.", "com.qti.", "vendor."];
    EXACT.contains(&package) || PREFIXES.iter().any(|p| package.starts_with(p))
}

/// The list's entry for a package, if any.
pub fn lookup(package: &str) -> Option<Bloat> {
    list().into_iter().find(|b| b.package == package)
}

/// What the phone has: listed apps still installed for the user, and every
/// app removed for the user (listed or not), which can be restored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DebloatState {
    pub present: Vec<Bloat>,
    pub removed: Vec<String>,
}

/// Package names of `pm list packages` output.
pub fn parse_package_list(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.trim().strip_prefix("package:")).map(|p| p.trim().to_string()).filter(|p| crate::adb::is_valid_package_name(p)).collect()
}

/// `installed`: `pm list packages` (installed for the user).
/// `all`: `pm list packages -u` (also those removed for the user).
pub fn parse_state(installed: &str, all: &str) -> DebloatState {
    let installed: std::collections::HashSet<String> = parse_package_list(installed).into_iter().collect();
    let mut removed: Vec<String> = parse_package_list(all).into_iter().filter(|p| !installed.contains(p)).collect();
    removed.sort();
    removed.dedup();
    let mut present: Vec<Bloat> = list().into_iter().filter(|b| installed.contains(b.package) && !is_protected(b.package)).collect();
    present.sort_by(|a, b| a.risk.cmp(&b.risk).then(a.vendor.cmp(b.vendor)).then(a.name.cmp(b.name)));
    DebloatState { present, removed }
}

/// Read-only script for [`parse_state`]: both lists, split by a marker.
pub const STATE_SCRIPT: &str = "pm list packages 2>/dev/null; echo @@@ALL@@@; pm list packages -u 2>/dev/null";

/// Splits the output of [`STATE_SCRIPT`].
pub fn parse_state_output(out: &str) -> DebloatState {
    let (installed, all) = out.split_once("@@@ALL@@@").unwrap_or((out, ""));
    parse_state(installed, all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_has_valid_unique_unprotected_packages() {
        let list = list();
        let mut seen = std::collections::HashSet::new();
        for b in &list {
            assert!(crate::adb::is_valid_package_name(b.package), "{}", b.package);
            assert!(seen.insert(b.package), "duplicate {}", b.package);
            assert!(!is_protected(b.package), "protected {}", b.package);
            assert!(!b.what.is_empty() && !b.name.is_empty());
        }
    }

    #[test]
    fn state_splits_present_and_removed() {
        let out = "package:com.android.systemui\npackage:com.facebook.services\npackage:com.zte.weather\n@@@ALL@@@\npackage:com.android.systemui\npackage:com.facebook.services\npackage:com.zte.weather\npackage:com.facebook.system\npackage:com.example.gone\n";
        let s = parse_state_output(out);
        let present: Vec<_> = s.present.iter().map(|b| b.package).collect();
        // Recommended first.
        assert_eq!(present, ["com.facebook.services", "com.zte.weather"]);
        assert_eq!(s.removed, ["com.example.gone", "com.facebook.system"]);
    }

    #[test]
    fn protected_packages() {
        assert!(is_protected("com.google.android.gms"));
        assert!(is_protected("com.android.providers.contacts"));
        assert!(!is_protected("com.facebook.system"));
    }
}
