//! Scheduled actions: “power off the phone at 2:00”, “Do Not Disturb Monday to
//! Friday at 23:00”…
//!
//! ADB Scan runs them while it is open (also when hidden in the tray) and the
//! phone is connected: the phone stores nothing. Each action runs once per
//! scheduled day; if the phone is not connected at that time, it is recorded as
//! not run and is not repeated later.

use crate::i18n::tr;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduledAction {
    PowerOff,
    Reboot,
    StopAll,
    DndOn,
    DndOff,
    ScreenOff,
}

impl ScheduledAction {
    pub const ALL: [ScheduledAction; 6] = [Self::PowerOff, Self::Reboot, Self::StopAll, Self::DndOn, Self::DndOff, Self::ScreenOff];

    pub fn label(self) -> &'static str {
        match self {
            Self::PowerOff => tr("Power off the phone"),
            Self::Reboot => tr("Restart the phone"),
            Self::StopAll => tr("Stop all apps"),
            Self::DndOn => tr("Turn on Do Not Disturb"),
            Self::DndOff => tr("Turn off Do Not Disturb"),
            Self::ScreenOff => tr("Turn off the screen"),
        }
    }

    /// Key for History.
    pub fn key(self) -> &'static str {
        match self {
            Self::PowerOff => "power_off",
            Self::Reboot => "reboot",
            Self::StopAll => "stop_all",
            Self::DndOn => "dnd_on",
            Self::DndOff => "dnd_off",
            Self::ScreenOff => "screen_off",
        }
    }
}

/// Days of the week, Monday = 0.
pub const DAY_LETTERS: [&str; 7] = ["L", "M", "X", "J", "V", "S", "D"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scheduled {
    /// Stable identifier (to remember what ran today).
    pub id: u64,
    pub enabled: bool,
    /// Minute of the day, 0–1439.
    pub minute: u16,
    /// Which days (Monday = 0). All false never runs.
    pub days: [bool; 7],
    pub action: ScheduledAction,
}

impl Scheduled {
    pub fn time_text(&self) -> String {
        format!("{:02}:{:02}", self.minute / 60, self.minute % 60)
    }

    pub fn days_text(&self) -> String {
        match self.days {
            [true, true, true, true, true, true, true] => tr("Every day").into(),
            [true, true, true, true, true, false, false] => tr("Monday to Friday").into(),
            [false, false, false, false, false, true, true] => tr("Weekends").into(),
            d => {
                let picked: Vec<&str> = (0..7).filter(|i| d[*i]).map(|i| DAY_LETTERS[i]).collect();
                if picked.is_empty() {
                    tr("No day").into()
                } else {
                    picked.join(" ")
                }
            }
        }
    }

    /// Is it due now? `now` is the local time; it runs during the 5 minutes
    /// after the scheduled time (in case ADB Scan was busy or the phone took a
    /// while to connect), once per day.
    pub fn due(&self, now: LocalTime, last_run_day: Option<i64>) -> bool {
        self.enabled && self.days[now.weekday as usize] && now.minute >= self.minute && now.minute < self.minute + 5 && last_run_day != Some(now.day)
    }
}

/// Local time: day number (for “once per day”), day of the week (Monday = 0)
/// and minute of the day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub day: i64,
    pub weekday: u8,
    pub minute: u16,
}

impl LocalTime {
    /// The system's local time (respects daylight saving time).
    #[cfg(unix)]
    pub fn now() -> Self {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as libc::time_t).unwrap_or(0);
        // SAFETY: localtime_r only writes to `tm`, which lives on this stack.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        unsafe {
            libc::localtime_r(&t, &mut tm);
        }
        let days_since_epoch = (t + tm.tm_gmtoff).div_euclid(86_400);
        Self {
            day: days_since_epoch,
            // tm_wday: Sunday = 0.
            weekday: ((tm.tm_wday + 6) % 7) as u8,
            minute: (tm.tm_hour * 60 + tm.tm_min) as u16,
        }
    }

    /// The system's local time (respects daylight saving time).
    #[cfg(windows)]
    pub fn now() -> Self {
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        // SAFETY: GetLocalTime only writes to `st`, which lives on this stack.
        let mut st: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
        unsafe { GetLocalTime(&mut st) };
        Self {
            day: crate::format::days_from_civil(st.wYear.into(), st.wMonth.into(), st.wDay.into()),
            // wDayOfWeek: Sunday = 0.
            weekday: ((st.wDayOfWeek + 6) % 7) as u8,
            minute: st.wHour * 60 + st.wMinute,
        }
    }
}
