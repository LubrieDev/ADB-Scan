//! Scheduled actions: when each one is due.

use adbscan_core::schedule::*;

fn at(day: i64, weekday: u8, h: u16, m: u16) -> LocalTime {
    LocalTime { day, weekday, minute: h * 60 + m }
}

fn weekdays_2300() -> Scheduled {
    Scheduled { id: 1, enabled: true, minute: 23 * 60, days: [true, true, true, true, true, false, false], action: ScheduledAction::DndOn }
}

#[test]
fn runs_once_in_its_window() {
    let s = weekdays_2300();
    assert!(!s.due(at(100, 0, 22, 59), None));
    assert!(s.due(at(100, 0, 23, 0), None));
    assert!(s.due(at(100, 0, 23, 4), None)); // 5-minute window
    assert!(!s.due(at(100, 0, 23, 5), None));
    // Already ran today: it is not repeated.
    assert!(!s.due(at(100, 0, 23, 1), Some(100)));
    // The next day, it is.
    assert!(s.due(at(101, 1, 23, 0), Some(100)));
}

#[test]
fn respects_days_and_enabled() {
    let s = weekdays_2300();
    assert!(!s.due(at(105, 5, 23, 0), None)); // Saturday
    let off = Scheduled { enabled: false, ..weekdays_2300() };
    assert!(!off.due(at(100, 0, 23, 0), None));
}

#[test]
fn texts() {
    let s = weekdays_2300();
    assert_eq!(s.time_text(), "23:00");
    assert_eq!(s.days_text(), "Monday to Friday");
    let custom = Scheduled { days: [true, false, true, false, false, false, true], minute: 125, ..weekdays_2300() };
    assert_eq!(custom.days_text(), "L X D");
    assert_eq!(custom.time_text(), "02:05");
}

#[test]
fn local_time_is_sane() {
    let t = LocalTime::now();
    assert!(t.weekday < 7 && t.minute < 1440 && t.day > 20_000);
}
