//! Per-app tuning without root:
//!
//! - **Game mode** (Android 13+, `cmd game`): for apps that declare
//!   themselves games (`android:appCategory="game"`). The modes the game or
//!   the phone offer (standard, performance, battery) and the custom one:
//!   lower resolution and a frame rate cap, to save battery and heat. **It
//!   changes the phone**; standard mode and `cmd game reset` undo it.
//! - **The app's language** (Android 13+, `cmd locale`): the same as Settings
//!   → Apps → Language. **It changes the phone**; “system language” (an empty
//!   list) undoes it.
//! - **How smoothly it draws** (`dumpsys gfxinfo`, read-only): slow frames
//!   and frame times of an app's interface since its process started. Games
//!   that draw with their own engine are not measured here.
//!
//! Formats from Android 16 (`GameManagerShellCommand`,
//! `LocaleManagerShellCommand`, hwui's `JankTracker`), checked on a REDMAGIC
//! NX809J.

use crate::i18n::tr;
use crate::probe::{self, Probe};

/// Read-only script for one app. `package` must be valid
/// ([`crate::adb::is_valid_package_name`]).
pub fn script(package: &str) -> String {
    format!(
        "p={package}
echo @@@SDK@@@; getprop ro.build.version.sdk
echo @@@GAME@@@; cmd game list-modes \"$p\" 2>&1
echo @@@GAMECFG@@@; cmd game list-configs \"$p\" 2>&1
echo @@@LOCALE@@@; cmd locale get-app-locales \"$p\" --user 0 2>&1
echo @@@GFX@@@; dumpsys gfxinfo \"$p\" 2>&1 | grep -m8 -E '^(Total frames rendered|Janky frames|50th percentile|90th percentile|95th percentile|99th percentile): '
echo @@@END@@@
"
    )
}

/// Game mode of a game.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GameMode {
    /// `standard`, `performance`, `battery`, `custom`.
    pub current: String,
    pub available: Vec<String>,
    /// Custom mode: resolution scale (1.0: full) and FPS cap (0: none).
    pub scale: Option<f64>,
    pub fps: Option<u32>,
}

/// Frames of an app's interface (`dumpsys gfxinfo`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Smoothness {
    pub frames: u64,
    pub janky: u64,
    pub janky_pct: f64,
    /// Frame time percentiles, ms: 50th, 90th, 95th, 99th.
    pub p50: u32,
    pub p90: u32,
    pub p95: u32,
    pub p99: u32,
}

impl Smoothness {
    /// Readable verdict.
    pub fn verdict(&self) -> &'static str {
        match self.janky_pct {
            p if p < 1.0 => tr("Very smooth"),
            p if p < 5.0 => tr("Smooth"),
            p if p < 15.0 => tr("Some stutter"),
            _ => tr("Stutters a lot"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppTuning {
    pub sdk: Option<i64>,
    /// `Missing`: not a game (or the phone has no game manager).
    pub game: Probe<GameMode>,
    /// The app's own languages (empty: the system's).
    pub locales: Probe<Vec<String>>,
    /// `Missing`: the app is not running or draws nothing measurable.
    pub smoothness: Probe<Smoothness>,
}

/// `pkg current mode: standard, available game modes: [standard,custom]`
fn parse_modes(text: &str) -> Option<GameMode> {
    let line = text.lines().find(|l| l.contains("current mode: "))?;
    let (_, rest) = line.split_once("current mode: ")?;
    let (current, rest) = rest.split_once(',')?;
    let list = rest.split_once('[')?.1.split(']').next()?;
    let available = list.split(',').map(|m| m.trim().to_lowercase()).filter(|m| !m.is_empty()).collect();
    Some(GameMode { current: current.trim().to_lowercase(), available, scale: None, fps: None })
}

/// `{4=[Game Mode:4,Scaling:0.7,Use Angle:false,Fps:60,…]}`: the custom mode
/// (4) values.
fn parse_custom_config(text: &str) -> (Option<f64>, Option<u32>) {
    let Some(start) = text.find("Game Mode:4,") else { return (None, None) };
    let block = text[start..].split(']').next().unwrap_or("");
    let value = |key: &str| block.split(',').find_map(|kv| kv.trim().strip_prefix(key)).map(str::trim);
    let scale = value("Scaling:").and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0 && *v <= 1.0);
    let fps = value("Fps:").and_then(|v| v.parse::<u32>().ok());
    (scale, fps)
}

/// `Locales for pkg for user 0 are [es-MX,en-US]`
fn parse_locales(text: &str) -> Option<Vec<String>> {
    let line = text.lines().find(|l| l.starts_with("Locales for "))?;
    let list = line.rsplit_once('[')?.1.split(']').next()?;
    Some(list.split(',').map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
}

fn parse_smoothness(text: &str) -> Option<Smoothness> {
    let mut s = Smoothness::default();
    let mut seen = false;
    let ms = |v: &str| v.trim().trim_end_matches("ms").parse::<u32>().ok();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(": ") else { continue };
        match key {
            // Several processes: the first one (the app's main one).
            "Total frames rendered" if seen => break,
            "Total frames rendered" => {
                s.frames = value.trim().parse().ok()?;
                seen = true;
            }
            // `15 (0.65%)`
            "Janky frames" => {
                let (n, pct) = value.split_once(" (")?;
                s.janky = n.trim().parse().ok()?;
                s.janky_pct = pct.trim_end_matches(['%', ')']).parse().ok()?;
            }
            "50th percentile" => s.p50 = ms(value)?,
            "90th percentile" => s.p90 = ms(value)?,
            "95th percentile" => s.p95 = ms(value)?,
            "99th percentile" => s.p99 = ms(value)?,
            _ => {}
        }
    }
    (seen && s.frames > 0).then_some(s)
}

pub fn parse(text: &str) -> AppTuning {
    let s = crate::parsers::split_sections(text);
    let sdk = s.get("SDK").and_then(|v| v.trim().parse().ok());
    let mut game = probe::from_section(s.get("GAME"), parse_modes);
    if let (Probe::Ok(g), Some(cfg)) = (&mut game, s.get("GAMECFG")) {
        (g.scale, g.fps) = parse_custom_config(cfg);
    }
    AppTuning {
        sdk,
        game,
        locales: probe::from_section(s.get("LOCALE"), parse_locales),
        smoothness: match s.get("GFX") {
            Some(t) => parse_smoothness(t).map(Probe::Ok).unwrap_or(Probe::Missing),
            None => Probe::Missing,
        },
    }
}

/// Mode name in words.
pub fn mode_label(mode: &str) -> String {
    match mode {
        "standard" => tr("Standard").into(),
        "performance" => tr("Performance").into(),
        "battery" => tr("Battery saving").into(),
        "custom" => tr("Custom").into(),
        m => m.to_string(),
    }
}

/// Resolution scales offered for the custom mode (the values Android
/// accepts, from 0.3 to 0.9, and full resolution).
pub const SCALES: [f64; 5] = [1.0, 0.9, 0.8, 0.7, 0.5];
/// Frame rate caps (0: none).
pub const FPS_CAPS: [u32; 6] = [0, 30, 40, 45, 60, 90];

/// Commands (for `adb shell`) that put a game in a mode. Custom: `scale`
/// (1.0: full) and `fps` (0: no cap); at least one of them must limit.
pub fn game_commands(package: &str, mode: &str, scale: f64, fps: u32) -> Option<Vec<Vec<String>>> {
    let v = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<String>>();
    Some(match mode {
        "custom" => {
            let mut c = vec!["cmd".to_string(), "game".into(), "set".into()];
            if scale < 0.99 {
                if !SCALES.iter().any(|s| (s - scale).abs() < 0.001) {
                    return None;
                }
                c.extend(["--downscale".into(), format!("{scale:.1}")]);
            } else {
                c.extend(["--downscale".into(), "disable".into()]);
            }
            if !FPS_CAPS.contains(&fps) {
                return None;
            }
            c.extend(["--fps".into(), fps.to_string(), package.to_string()]);
            vec![c]
        }
        // Back to what the phone decides: the custom values are also removed.
        "standard" => vec![v(&["cmd", "game", "mode", "standard", package]), v(&["cmd", "game", "reset", package])],
        "performance" | "battery" => vec![v(&["cmd", "game", "mode", mode, package])],
        _ => return None,
    })
}

/// A language tag `settings` and `cmd locale` accept (`es`, `es-MX`,
/// `zh-Hant-TW`).
pub fn is_valid_locale(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= 20 && tag.split('-').all(|p| !p.is_empty() && p.len() <= 8 && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Command that sets the app's language (`None`: the system's).
pub fn locale_command(package: &str, tag: Option<&str>) -> Option<Vec<String>> {
    let mut c: Vec<String> = ["cmd", "locale", "set-app-locales", package, "--user", "0"].iter().map(|s| s.to_string()).collect();
    if let Some(t) = tag {
        if !is_valid_locale(t) {
            return None;
        }
        c.extend(["--locales".into(), t.to_string()]);
    }
    Some(c)
}

/// Languages offered (tag, name in that language).
pub const LANGUAGES: [(&str, &str); 16] = [
    ("es-MX", "Español (México)"),
    ("es-ES", "Español (España)"),
    ("es-US", "Español (Estados Unidos)"),
    ("en-US", "English (United States)"),
    ("en-GB", "English (United Kingdom)"),
    ("pt-BR", "Português (Brasil)"),
    ("pt-PT", "Português (Portugal)"),
    ("fr-FR", "Français"),
    ("de-DE", "Deutsch"),
    ("it-IT", "Italiano"),
    ("ja-JP", "日本語"),
    ("ko-KR", "한국어"),
    ("zh-CN", "中文 (简体)"),
    ("zh-TW", "中文 (繁體)"),
    ("ru-RU", "Русский"),
    ("ar", "العربية"),
];

#[cfg(test)]
mod tests {
    use super::*;

    // Real outputs (REDMAGIC NX809J, Android 16) with sample package names.
    const GAME: &str = "@@@SDK@@@\n36\n@@@GAME@@@\ncom.example.game current mode: custom, available game modes: [standard,custom]\n@@@GAMECFG@@@\ncom.example.game interventions: \n[Name:com.example.game Modes: {4=[Game Mode:4,Scaling:0.7,Use Angle:false,Fps:60,Loading Boost Duration:-1]}]\n@@@LOCALE@@@\nLocales for com.example.game for user 0 are [en-US]\n@@@GFX@@@\n@@@END@@@\n";
    const APP: &str = "@@@SDK@@@\n36\n@@@GAME@@@\nPackage com.example.app is not of game type, to use the game mode commands, it must specify game category in the manifest as android:appCategory=\"game\"\n@@@GAMECFG@@@\n@@@LOCALE@@@\nLocales for com.example.app for user 0 are []\n@@@GFX@@@\nTotal frames rendered: 2304\nJanky frames: 15 (0.65%)\n50th percentile: 5ms\n90th percentile: 8ms\n95th percentile: 17ms\n99th percentile: 20ms\nTotal frames rendered: 10\n@@@END@@@\n";

    #[test]
    fn game() {
        let t = parse(GAME);
        let g = t.game.ok().unwrap();
        assert_eq!(g.current, "custom");
        assert_eq!(g.available, ["standard", "custom"]);
        assert_eq!((g.scale, g.fps), (Some(0.7), Some(60)));
        assert_eq!(t.locales, Probe::Ok(vec!["en-US".to_string()]));
        assert_eq!(t.smoothness, Probe::Missing);
        assert_eq!(parse_custom_config(" No intervention found for package x"), (None, None));
    }

    #[test]
    fn app() {
        let t = parse(APP);
        assert!(t.game.ok().is_none());
        assert_eq!(t.locales, Probe::Ok(vec![]));
        let s = t.smoothness.ok().unwrap();
        assert_eq!((s.frames, s.janky, s.p50, s.p90, s.p95, s.p99), (2304, 15, 5, 8, 17, 20));
        assert!((s.janky_pct - 0.65).abs() < 1e-9);
        assert_eq!(s.verdict(), "Very smooth");
    }

    #[test]
    fn commands() {
        let c = game_commands("com.example.game", "custom", 0.7, 60).unwrap();
        assert_eq!(c[0].join(" "), "cmd game set --downscale 0.7 --fps 60 com.example.game");
        let c = game_commands("com.example.game", "custom", 1.0, 30).unwrap();
        assert_eq!(c[0].join(" "), "cmd game set --downscale disable --fps 30 com.example.game");
        assert!(game_commands("com.example.game", "custom", 0.33, 60).is_none());
        assert!(game_commands("com.example.game", "custom", 0.7, 77).is_none());
        let c = game_commands("com.example.game", "standard", 1.0, 0).unwrap();
        assert_eq!(c[1].join(" "), "cmd game reset com.example.game");
        assert!(game_commands("com.example.game", "turbo", 1.0, 0).is_none());
        assert_eq!(locale_command("com.example.app", Some("es-MX")).unwrap().join(" "), "cmd locale set-app-locales com.example.app --user 0 --locales es-MX");
        assert_eq!(locale_command("com.example.app", None).unwrap().join(" "), "cmd locale set-app-locales com.example.app --user 0");
        assert!(locale_command("com.example.app", Some("es;rm")).is_none());
        assert!(LANGUAGES.iter().all(|(t, _)| is_valid_locale(t)));
    }
}
