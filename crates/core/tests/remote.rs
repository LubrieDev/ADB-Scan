//! Remote: validation of what the user types and reading of the state.

use adbscan_core::remote::*;

#[test]
fn numbers() {
    assert_eq!(clean_number("+34 600 12-34-56").as_deref(), Some("+34600123456"));
    assert_eq!(clean_number("(91) 555.12.34").as_deref(), Some("915551234"));
    assert_eq!(clean_number("*#06#").as_deref(), Some("*#06#"));
    for bad in ["", "abc", "12+3", "600; reboot", "+", "6001234567890123456789012345678901"] {
        assert_eq!(clean_number(bad), None, "{bad}");
    }
}

#[test]
fn dial_commands_are_quoted_and_encoded() {
    assert_eq!(dial_command("600 123 456", true).unwrap(), "am start -a android.intent.action.CALL -d tel:600123456 2>&1");
    // `#` in a `tel:` URI starts a fragment: it is encoded.
    assert_eq!(dial_command("*#06#", false).unwrap(), "am start -a android.intent.action.DIAL -d 'tel:*%2306%23' 2>&1");
    assert!(dial_command("x", true).is_none());
}

#[test]
fn urls() {
    assert_eq!(clean_url("https://example.com/a?b=c").as_deref(), Some("https://example.com/a?b=c"));
    assert_eq!(clean_url("  example.com/path ").as_deref(), Some("https://example.com/path"));
    assert_eq!(clean_url("HTTP://Example.com").as_deref(), Some("HTTP://Example.com"));
    for bad in ["", "hola", "file:///sdcard/x", "javascript:alert(1)", "https://", "https://a b.com", "intent://x#Intent;end", ".com"] {
        assert_eq!(clean_url(bad), None, "{bad}");
    }
    // Quotes in the link cannot break the command.
    assert_eq!(open_url_command("https://e.com/?q='x'").unwrap(), "am start -a android.intent.action.VIEW -d 'https://e.com/?q='\"'\"'x'\"'\"'' 2>&1");
}

#[test]
fn sms_text_is_quoted() {
    let cmd = sms_command("600123456", "Hello; reboot $(id)").unwrap();
    assert_eq!(cmd, "am start -a android.intent.action.SENDTO -d smsto:600123456 --es sms_body 'Hello; reboot $(id)' 2>&1");
    assert!(sms_command("nope", "x").is_none());
}

#[test]
fn call_state() {
    assert_eq!(parse_call_state("  mCallState=0\n  mCallState=0\n"), CallState::Idle);
    assert_eq!(parse_call_state("  mCallState=0\n  mCallState=1\n"), CallState::Ringing);
    assert_eq!(parse_call_state("  mCallState=2\n"), CallState::InCall);
    assert_eq!(parse_call_state(""), CallState::Unknown);
}

#[test]
fn am_errors() {
    assert_eq!(am_error("Starting: Intent { act=android.intent.action.VIEW }"), None);
    assert!(am_error("Starting: Intent {}\nError: Activity not started, unable to resolve Intent").is_some());
    assert!(am_error("java.lang.SecurityException: Permission Denial: starting Intent").is_some());
}

const MEDIA: &str = "MEDIA SESSION SERVICE (dumpsys media_session)

Sessions Stack - have 2 sessions:
    com.example.podcast/session (userId=0)
      ownerPid=100, ownerUid=10200, userId=0
      package=com.example.podcast
      active=false
      state=PlaybackState {state=2, position=1000, buffered position=0, speed=0.0}
      metadata: size=3, description=Episodio 4, Autora, null
    com.example.music/media (userId=0)
      ownerPid=200, ownerUid=10201, userId=0
      package=com.example.music
      active=true
      state=PlaybackState {state=3, position=5000, buffered position=0, speed=1.0}
      metadata: size=7, description=Hello, Goodbye, The Beatles, Magical Mystery Tour
";

#[test]
fn now_playing_prefers_the_one_playing() {
    let n = parse_now_playing(MEDIA).unwrap();
    assert_eq!(n.package, "com.example.music");
    assert!(n.playing);
    // The title contains a comma: it is split from the right.
    assert_eq!(n.title, "Hello, Goodbye");
    assert_eq!(n.artist, "The Beatles");
    assert_eq!(n.album, "Magical Mystery Tour");
}

#[test]
fn now_playing_paused_or_nothing() {
    let paused = MEDIA.replace("state=3", "state=2");
    let n = parse_now_playing(&paused).unwrap();
    assert_eq!(n.package, "com.example.music"); // the active one
    assert!(!n.playing);
    assert_eq!(parse_now_playing("Sessions Stack - have 0 sessions:\n"), None);
}

#[test]
fn now_playing_android_16_state_names() {
    // Android 16 writes the state with its name: `state=PLAYING(3)`.
    let a16 = MEDIA.replace("state=3,", "state=PLAYING(3),").replace("state=2,", "state=PAUSED(2),");
    let n = parse_now_playing(&a16).unwrap();
    assert_eq!(n.package, "com.example.music");
    assert!(n.playing);
}
