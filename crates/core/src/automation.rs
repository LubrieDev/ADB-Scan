//! Automations: actions ADB Scan runs by itself when a phone connects
//! (“when my phone connects, copy the new photos and turn on Do Not
//! Disturb”).
//!
//! A phone counts as connected when its first telemetry arrives: after it is
//! plugged in, paired over Wi-Fi, or when ADB Scan starts with it already
//! connected. It is recognised by its hardware serial number (`ro.serialno`),
//! which does not change between USB and Wi-Fi. The same automation does not
//! run again for the same phone within [`COOLDOWN_SECS`], so a connection
//! that drops and comes back does not repeat it.

use serde::{Deserialize, Serialize};

use crate::i18n::tr;

/// Minimum time between two runs of the same automation on the same phone.
pub const COOLDOWN_SECS: f64 = 600.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutoAction {
    BackupPhotos,
    DndOn,
    DndOff,
    StayAwakeOn,
    StayAwakeOff,
    StopAll,
    ClearCaches,
    /// Run by the interface (it opens a window).
    Mirror,
}

impl AutoAction {
    pub const ALL: [AutoAction; 8] = [Self::BackupPhotos, Self::DndOn, Self::DndOff, Self::StayAwakeOn, Self::StayAwakeOff, Self::StopAll, Self::ClearCaches, Self::Mirror];

    pub fn label(self) -> &'static str {
        match self {
            Self::BackupPhotos => tr("Copy the new photos to the PC"),
            Self::DndOn => tr("Turn on Do Not Disturb"),
            Self::DndOff => tr("Turn off Do Not Disturb"),
            Self::StayAwakeOn => tr("Keep the screen on while charging"),
            Self::StayAwakeOff => tr("Let the screen turn off while charging"),
            Self::StopAll => tr("Stop all apps"),
            Self::ClearCaches => tr("Clear all caches"),
            Self::Mirror => tr("Mirror the screen"),
        }
    }

    /// Key for History.
    pub fn key(self) -> &'static str {
        match self {
            Self::BackupPhotos => "backup_photos",
            Self::DndOn => "dnd_on",
            Self::DndOff => "dnd_off",
            Self::StayAwakeOn => "stay_awake_on",
            Self::StayAwakeOff => "stay_awake_off",
            Self::StopAll => "stop_all",
            Self::ClearCaches => "clear_caches",
            Self::Mirror => "mirror",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Automation {
    /// Stable identifier.
    pub id: u64,
    pub enabled: bool,
    pub action: AutoAction,
    /// Hardware serial (`ro.serialno`) of the phone; empty = any phone.
    #[serde(default)]
    pub device: String,
    /// Name shown for that phone (its model when it was chosen).
    #[serde(default)]
    pub device_name: String,
}

impl Automation {
    pub fn applies_to(&self, hw_serial: &str) -> bool {
        self.enabled && (self.device.is_empty() || self.device == hw_serial)
    }
}

/// The automations to run now for a phone that just connected: enabled, for
/// it, and not run on it within the cooldown. `last_run(id)` is when that
/// automation last ran on this phone.
pub fn due<'a>(list: &'a [Automation], hw_serial: &str, now: f64, last_run: impl Fn(u64) -> Option<f64>) -> Vec<&'a Automation> {
    list.iter().filter(|a| a.applies_to(hw_serial) && !matches!(last_run(a.id), Some(t) if now - t < COOLDOWN_SECS)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(id: u64, device: &str, enabled: bool) -> Automation {
        Automation { id, enabled, action: AutoAction::DndOn, device: device.into(), device_name: String::new() }
    }

    #[test]
    fn due_filters_device_enabled_and_cooldown() {
        let list = vec![a(1, "", true), a(2, "ABC", true), a(3, "XYZ", true), a(4, "", false)];
        let ids = |v: Vec<&Automation>| v.iter().map(|a| a.id).collect::<Vec<_>>();
        assert_eq!(ids(due(&list, "ABC", 1000.0, |_| None)), [1, 2]);
        // 1 ran 100 s ago: still cooling down; 2 ran long ago.
        assert_eq!(ids(due(&list, "ABC", 1000.0, |id| Some(if id == 1 { 900.0 } else { 0.0 }))), [2]);
    }

    #[test]
    fn settings_round_trip() {
        let x = a(7, "ABC", true);
        let json = serde_json::to_string(&x).unwrap();
        assert_eq!(serde_json::from_str::<Automation>(&json).unwrap(), x);
        // Older files without the device fields.
        let old: Automation = serde_json::from_str(r#"{"id":1,"enabled":true,"action":"StopAll"}"#).unwrap();
        assert!(old.applies_to("anything"));
    }
}
