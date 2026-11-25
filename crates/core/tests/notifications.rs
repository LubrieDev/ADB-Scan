//! Phone notifications. Format of `dumpsys notification --noredact` (AOSP);
//! packages and texts are made up.

use adbscan_core::notifications::*;

const DUMP: &str = "Current Notification Manager state:
  Notification List:
    NotificationRecord(0x0a1b2c3d: pkg=com.example.chat user=UserHandle{0} id=7 tag=null importance=4 key=0|com.example.chat|7|null|10200 appImportanceLocked=false: Notification(channel=msgs shortcut=null contentView=null vibrate=null sound=null defaults=0x0 flags=0x10 color=0x00000000 category=msg vis=PRIVATE))
      uid=10200 userId=0
      extras={
        android.title=String (Ana)
        android.text=String (Meet at 8?)
        android.subText=null
      }
    NotificationRecord(0x0b2c3d4e: pkg=com.example.music user=UserHandle{0} id=1 tag=null importance=2 key=0|com.example.music|1|null|10201 appImportanceLocked=false: Notification(channel=play flags=ONGOING_EVENT|NO_CLEAR|FOREGROUND_SERVICE category=transport vis=PUBLIC))
      extras={
        android.title=String (Song)
        android.text=String (Artist)
      }
    NotificationRecord(0x0c3d4e5f: pkg=com.example.chat user=UserHandle{0} id=0 tag=null importance=4 key=0|com.example.chat|0|null|10200 appImportanceLocked=false: Notification(channel=msgs flags=GROUP_SUMMARY category=msg vis=PRIVATE))
      extras={
        android.title=String (2 messages)
      }
    NotificationRecord(0x0d4e5f60: pkg=com.example.mail user=UserHandle{0} id=3 tag=null importance=3 key=0|com.example.mail|3|null|10202 appImportanceLocked=false: Notification(channel=mail flags=AUTO_CANCEL category=email vis=PRIVATE))
      extras={
        android.title=SpannableString (November invoice)
        android.text=String (Attached…)
        android.bigText=SpannableString (Attached is the November invoice. Thanks.)
      }
  Snoozed notifications:
    NotificationRecord(0x0e5f6071: pkg=com.example.hidden user=UserHandle{0} id=9 key=0|com.example.hidden|9|null|10203: Notification(flags=0x0))
";

#[test]
fn parses_the_list() {
    let n = parse(DUMP);
    assert_eq!(n.len(), 4);
    assert_eq!(n[0].package, "com.example.chat");
    assert_eq!(n[0].title, "Ana");
    assert_eq!(n[0].text, "Meet at 8?");
    assert_eq!(n[0].key, "0|com.example.chat|7|null|10200");
    assert!(n[0].worth_showing());
    assert!(n[1].ongoing && !n[1].worth_showing());
    assert!(n[2].summary && !n[2].worth_showing());
    // The long text completes the short one.
    assert_eq!(n[3].text, "Attached is the November invoice. Thanks.");
}

#[test]
fn redacted_text_is_not_shown_as_text() {
    let redacted = DUMP.replace("String (Ana)", "String [length=3]").replace("String (Meet at 8?)", "String [length=18]");
    let n = parse(&redacted);
    assert_eq!(n[0].title, "");
    assert_eq!(n[0].text, "");
    assert!(!n[0].worth_showing());
}

#[test]
fn hex_flags() {
    let hex = DUMP.replace("flags=ONGOING_EVENT|NO_CLEAR|FOREGROUND_SERVICE", "flags=0x62").replace("flags=GROUP_SUMMARY", "flags=0x200");
    let n = parse(&hex);
    assert!(!n[0].ongoing); // 0x10: AUTO_CANCEL
    assert!(n[1].ongoing);
    assert!(n[2].summary);
}
