//! Remote: control the phone from the laptop (media, calls, sending links and
//! messages, finding the phone).
//!
//! **It changes the phone**, except [`STATUS_SCRIPT`] (read-only). Every
//! command comes from an explicit user action. What the user types (numbers,
//! links, messages) is validated and quoted before being sent: it never reaches
//! the phone as part of a command.

use crate::i18n::tr;
use crate::exec::shell_quote;

/// Call state and what is playing. Read-only.
pub const STATUS_SCRIPT: &str = "echo @@@CALL@@@; dumpsys telephony.registry 2>&1 | grep 'mCallState='
echo @@@MEDIA@@@; dumpsys media_session 2>&1
echo @@@END@@@
";

// ---------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------

/// Media keys: Android delivers them to the app that is playing (or the last
/// one that played), like the buttons on a pair of headphones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKey {
    PlayPause,
    Next,
    Previous,
}

impl MediaKey {
    pub fn keycode(self) -> &'static str {
        match self {
            Self::PlayPause => "KEYCODE_MEDIA_PLAY_PAUSE",
            Self::Next => "KEYCODE_MEDIA_NEXT",
            Self::Previous => "KEYCODE_MEDIA_PREVIOUS",
        }
    }
}

/// What is playing (or played last) on the phone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NowPlaying {
    pub package: String,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
}

/// A `dumpsys media_session` session while it is being read.
#[derive(Default)]
struct Session {
    package: String,
    active: bool,
    state: Option<i64>,
    meta: Option<(String, String, String)>,
}

/// `PlaybackState.STATE_PLAYING`.
const STATE_PLAYING: i64 = 3;

/// Chooses the session that is playing; if none is playing, the first active
/// one with a title (paused). `None` if there is nothing to show.
pub fn parse_now_playing(text: &str) -> Option<NowPlaying> {
    let mut sessions: Vec<Session> = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(pkg) = line.strip_prefix("package=") {
            sessions.push(Session { package: pkg.trim().to_string(), ..Default::default() });
            continue;
        }
        let Some(s) = sessions.last_mut() else {
            continue;
        };
        if let Some(v) = line.strip_prefix("active=") {
            s.active = v.trim() == "true";
        } else if let Some(rest) = line.strip_prefix("state=PlaybackState {state=") {
            // `state=3` or, on Android 16, `state=PLAYING(3)`.
            let token = rest.split(',').next().unwrap_or("");
            let digits = token.split_once('(').map_or(token, |(_, r)| r.trim_end_matches(')'));
            s.state = digits.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("metadata:") {
            // `size=7, description=Title, Artist, Album`. The title may contain
            // commas; artist and album hardly ever do: it is split from the
            // right.
            if let Some(desc) = rest.split_once("description=").map(|(_, d)| d) {
                let mut parts = desc.rsplitn(3, ", ").collect::<Vec<_>>();
                parts.reverse();
                let clean = |s: Option<&&str>| s.map(|v| v.trim()).filter(|v| *v != "null" && !v.is_empty()).unwrap_or("").to_string();
                let (t, a, b) = match parts.len() {
                    3 => (clean(parts.first()), clean(parts.get(1)), clean(parts.get(2))),
                    _ => (clean(parts.first()), String::new(), String::new()),
                };
                if !t.is_empty() {
                    s.meta = Some((t, a, b));
                }
            }
        }
    }
    let pick = sessions
        .iter()
        .find(|s| s.state == Some(STATE_PLAYING) && s.meta.is_some())
        .or_else(|| sessions.iter().find(|s| s.active && s.meta.is_some()))
        .or_else(|| sessions.iter().find(|s| s.meta.is_some()))?;
    let (title, artist, album) = pick.meta.clone().unwrap_or_default();
    Some(NowPlaying { package: pick.package.clone(), playing: pick.state == Some(STATE_PLAYING), title, artist, album })
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CallState {
    #[default]
    Unknown,
    Idle,
    Ringing,
    InCall,
}

impl CallState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => tr("Unknown"),
            Self::Idle => tr("No calls"),
            Self::Ringing => tr("Incoming call"),
            Self::InCall => tr("In a call"),
        }
    }
}

/// `mCallState=` of each SIM (0 idle, 1 ringing, 2 in a call): the most active
/// one wins.
pub fn parse_call_state(text: &str) -> CallState {
    let states: Vec<i64> = text.split("mCallState=").skip(1).filter_map(|r| r.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()).collect();
    if states.contains(&1) {
        CallState::Ringing
    } else if states.contains(&2) {
        CallState::InCall
    } else if states.is_empty() {
        CallState::Unknown
    } else {
        CallState::Idle
    }
}

/// Clean phone number (without spaces, hyphens, dots or parentheses). Only
/// digits, `*`, `#` and a leading `+`. `None` if it is not a number.
pub fn clean_number(input: &str) -> Option<String> {
    let n: String = input.chars().filter(|c| !matches!(c, ' ' | '-' | '.' | '(' | ')' | '\u{a0}')).collect();
    let ok = !n.is_empty() && n.len() <= 32 && n.chars().enumerate().all(|(i, c)| c.is_ascii_digit() || c == '*' || c == '#' || (c == '+' && i == 0)) && n.chars().any(|c| c.is_ascii_digit());
    ok.then_some(n)
}

/// `am start` to call (`call`) or to open the dialer with the number. `#` is
/// encoded: in a `tel:` URI it starts a fragment.
pub fn dial_command(number: &str, call: bool) -> Option<String> {
    let n = clean_number(number)?;
    let action = if call { "android.intent.action.CALL" } else { "android.intent.action.DIAL" };
    Some(format!("am start -a {action} -d {} 2>&1", shell_quote(&format!("tel:{}", n.replace('#', "%23")))))
}

// ---------------------------------------------------------------------------
// Send to the phone
// ---------------------------------------------------------------------------

/// Valid web link: `http(s)://…`, or a domain without a scheme (`https://` is
/// added). No spaces or control characters.
pub fn clean_url(input: &str) -> Option<String> {
    let t = input.trim();
    if t.is_empty() || t.len() > 2048 || t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    let url = if lower.starts_with("http://") || lower.starts_with("https://") {
        t.to_string()
    } else if !t.contains("://") && t.contains('.') && !t.starts_with('.') {
        format!("https://{t}")
    } else {
        return None;
    };
    let host = url.split_once("://")?.1.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then_some(url)
}

pub fn open_url_command(url: &str) -> Option<String> {
    let url = clean_url(url)?;
    Some(format!("am start -a android.intent.action.VIEW -d {} 2>&1", shell_quote(&url)))
}

/// Opens the messaging app with the number and text filled in; sending it is
/// confirmed by the user on the phone (Android does not allow sending SMS over
/// ADB without root).
pub fn sms_command(number: &str, text: &str) -> Option<String> {
    let n = clean_number(number)?;
    let text: String = text.chars().filter(|c| *c != '\0').take(2000).collect();
    Some(format!("am start -a android.intent.action.SENDTO -d {} --es sms_body {} 2>&1", shell_quote(&format!("smsto:{}", n.replace('#', "%23"))), shell_quote(&text)))
}

// ---------------------------------------------------------------------------
// Find the phone
// ---------------------------------------------------------------------------

/// 1-second timer on the phone's clock: it rings at alarm volume (which usually
/// bypasses silent mode and Do Not Disturb) until it is stopped on the phone.
/// The timer's label goes in the interface language and is quoted.
pub fn find_phone_command() -> String {
    format!(
        "am start -a android.intent.action.SET_TIMER --ei android.intent.extra.alarm.LENGTH 1 \
         --ez android.intent.extra.alarm.SKIP_UI true --es android.intent.extra.alarm.MESSAGE {} 2>&1",
        shell_quote(tr("ADB Scan: here I am"))
    )
}

/// Did Android reject the command? (`am start` prints the error and exits with
/// 0.)
pub fn am_error(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("Error") || l.contains("Exception") || l.contains("Permission Denial") || l.contains("unable to resolve Intent"))
        .map(|l| l.chars().take(160).collect())
}
