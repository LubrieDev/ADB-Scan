//! Connection and notification state machine.
//!
//! Pure logic: it receives lists of ADB/USB devices and the time, and returns
//! actions (notify, open/close a session, log an event). The interface calls
//! [`Tracker::tick`] periodically to fire the deferred alerts. Rules:
//!
//! 1. Each alert waits for a settling time and is only sent if the condition
//!    still holds (avoids alerts caused by loose cables).
//! 2. A disconnection followed by a reconnection within the settling time
//!    generates no notification.
//! 3. “USB detected” and “waiting for authorisation” are sent once per device
//!    until it disconnects.
//! 4. Devices already connected at startup are not notified.

use crate::{i18n::tr, t};
use std::collections::{HashMap, HashSet};

use crate::models::{AdbDevice, DeviceState, Transport};
use crate::usb::{UsbDevice, UsbMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoteKind {
    UsbDetected,
    WaitingAuthorization,
    Connected,
    Disconnected,
    Error,
}

impl NoteKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::UsbDetected => "usb_detected",
            Self::WaitingAuthorization => "waiting_authorization",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::Error => "error",
        }
    }

    /// Notification texts. “USB detected” does not imply that debugging is
    /// needed (the basic analysis works without it).
    pub fn text(self) -> &'static str {
        match self {
            Self::UsbDetected => tr("An Android phone was detected over USB. The basic analysis is already available; USB debugging is optional."),
            Self::WaitingAuthorization => tr("Waiting for you to authorise the connection from your Android device. Check the phone's screen."),
            Self::Connected => tr("Connection established over USB. Device ready to use."),
            Self::Disconnected => tr("The Android device disconnected."),
            Self::Error => tr("The connection with the device could not be established. Check the diagnosis for more information."),
        }
    }

    /// “Connected” text according to the real transport: by cable or through
    /// wireless debugging (`text()` is the USB one).
    pub fn connected_text(transport: Transport) -> &'static str {
        match transport {
            Transport::Tcp => tr("Connection established over Wi-Fi (wireless debugging). Device ready to use."),
            Transport::Emulator => tr("Connection established with the emulator. Device ready to use."),
            Transport::Usb => Self::Connected.text(),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Connected => "adb-scan-connected",
            Self::Disconnected => "adb-scan-disconnected",
            Self::Error => "adb-scan-error",
            _ => "adb-scan-waiting",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Normal,
    Critical,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Notify {
        kind: NoteKind,
        key: String,
        urgency: Urgency,
    },
    SessionStart {
        serial: String,
        name: String,
    },
    SessionTouch {
        serial: String,
    },
    SessionEnd {
        serial: String,
    },
    Log {
        category: &'static str,
        message: String,
        level: &'static str,
        serial: Option<String>,
    },
    /// A device has just become ready (ask for the marketing name, etc.).
    DeviceReady {
        serial: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum Cond {
    State(String, DeviceState),
    Gone(String),
    UsbOnly,
    UsbAdbWithoutDevice,
}

#[derive(Debug, Clone)]
struct Pending {
    due: f64,
    cond: Cond,
    urgency: Urgency,
}

#[derive(Debug, Clone, Copy)]
pub struct SettleTimes {
    pub usb_detected: f64,
    pub waiting_authorization: f64,
    pub usb_waiting_authorization: f64,
    pub error: f64,
    pub disconnected: f64,
    pub startup_quiet: f64,
}

impl Default for SettleTimes {
    fn default() -> Self {
        Self { usb_detected: 6.0, waiting_authorization: 3.0, usb_waiting_authorization: 10.0, error: 5.0, disconnected: 3.0, startup_quiet: 6.0 }
    }
}

pub struct Tracker {
    pub settle: SettleTimes,
    devices: Vec<AdbDevice>,
    usb: Vec<UsbDevice>,
    old_states: HashMap<String, DeviceState>,
    notified: HashSet<(NoteKind, String)>,
    pending: HashMap<(NoteKind, String), Pending>,
    quiet_until: f64,
    usb_key: Option<String>,
}

impl Tracker {
    pub fn new(now: f64) -> Self {
        let settle = SettleTimes::default();
        Self { quiet_until: now + settle.startup_quiet, settle, devices: Vec::new(), usb: Vec::new(), old_states: HashMap::new(), notified: HashSet::new(), pending: HashMap::new(), usb_key: None }
    }

    pub fn devices(&self) -> &[AdbDevice] {
        &self.devices
    }

    pub fn usb_devices(&self) -> &[UsbDevice] {
        &self.usb
    }

    fn state_of(&self, serial: &str) -> Option<DeviceState> {
        self.devices.iter().find(|d| d.serial == serial).map(|d| d.state)
    }

    fn holds(&self, cond: &Cond) -> bool {
        match cond {
            Cond::State(s, st) => self.state_of(s) == Some(*st),
            Cond::Gone(s) => self.state_of(s).is_none(),
            Cond::UsbOnly => !self.usb.is_empty() && self.devices.is_empty(),
            Cond::UsbAdbWithoutDevice => self.usb.iter().any(UsbDevice::has_adb_interface) && self.devices.is_empty(),
        }
    }

    #[allow(clippy::too_many_arguments)] // each argument is a different setting of the alert
    fn schedule(&mut self, kind: NoteKind, key: &str, cond: Cond, delay: f64, urgency: Urgency, replace: bool, now: f64) {
        let k = (kind, key.to_string());
        if self.pending.contains_key(&k) && !replace {
            return;
        }
        self.pending.insert(k, Pending { due: now + delay, cond, urgency });
    }

    fn cancel(&mut self, kind: NoteKind, key: &str) -> bool {
        self.pending.remove(&(kind, key.to_string())).is_some()
    }

    fn forget(&mut self, serial: &str) {
        for kind in [NoteKind::Connected, NoteKind::WaitingAuthorization, NoteKind::UsbDetected] {
            self.notified.remove(&(kind, serial.to_string()));
        }
    }

    fn emit(&mut self, kind: NoteKind, key: &str, urgency: Urgency, out: &mut Vec<Action>) {
        let k = (kind, key.to_string());
        if matches!(kind, NoteKind::UsbDetected | NoteKind::WaitingAuthorization) {
            if self.notified.contains(&k) {
                return;
            }
            self.notified.insert(k);
        }
        out.push(Action::Notify { kind, key: key.to_string(), urgency });
    }

    /// New `adb devices` list (and fastboot's, if they are merged).
    pub fn update_devices(&mut self, devices: Vec<AdbDevice>, now: f64) -> Vec<Action> {
        let mut out = Vec::new();
        let new_states: HashMap<String, DeviceState> = devices.iter().map(|d| (d.serial.clone(), d.state)).collect();
        let old_states = std::mem::replace(&mut self.old_states, new_states.clone());
        self.devices = devices.clone();
        let s = self.settle;

        for d in &devices {
            let serial = d.serial.as_str();
            let old = old_states.get(serial).copied();
            match d.state {
                DeviceState::Device => {
                    for k in [NoteKind::UsbDetected, NoteKind::WaitingAuthorization, NoteKind::Error] {
                        self.cancel(k, serial);
                    }
                    if old != Some(DeviceState::Device) {
                        let flapped = self.cancel(NoteKind::Disconnected, serial);
                        self.notified.remove(&(NoteKind::WaitingAuthorization, serial.to_string()));
                        if !flapped && now >= self.quiet_until {
                            self.emit(NoteKind::Connected, serial, Urgency::Normal, &mut out);
                        }
                        out.push(Action::SessionStart { serial: serial.into(), name: d.display_name() });
                        out.push(Action::Log { category: "device", message: t!("Connected: {} ({})", d.display_name(), serial), level: "info", serial: Some(serial.into()) });
                        out.push(Action::DeviceReady { serial: serial.into() });
                    } else {
                        out.push(Action::SessionTouch { serial: serial.into() });
                    }
                }
                DeviceState::Unauthorized => {
                    self.cancel(NoteKind::Error, serial);
                    if old != Some(DeviceState::Unauthorized) {
                        self.cancel(NoteKind::Disconnected, serial);
                        out.push(Action::Log { category: "device", message: t!("Authorisation pending for {}", serial), level: "warning", serial: Some(serial.into()) });
                        self.schedule(NoteKind::WaitingAuthorization, serial, Cond::State(serial.into(), DeviceState::Unauthorized), s.waiting_authorization, Urgency::Normal, true, now);
                    }
                }
                DeviceState::Offline | DeviceState::NoPermissions | DeviceState::Unknown => {
                    if old != Some(d.state) {
                        self.cancel(NoteKind::Disconnected, serial);
                        out.push(Action::Log { category: "device", message: t!("State {} for {}", d.state.as_str(), serial), level: "error", serial: Some(serial.into()) });
                        self.schedule(NoteKind::Error, serial, Cond::State(serial.into(), d.state), s.error, Urgency::Critical, true, now);
                    }
                }
                _ => {
                    // recovery / sideload / bootloader / fastboot: no alerts.
                    if old != Some(d.state) {
                        out.push(Action::Log { category: "device", message: t!("{} in {} mode", serial, d.state.label()), level: "info", serial: Some(serial.into()) });
                    }
                }
            }
        }

        for (serial, old) in &old_states {
            if new_states.contains_key(serial) {
                continue;
            }
            out.push(Action::SessionEnd { serial: serial.clone() });
            self.cancel(NoteKind::WaitingAuthorization, serial);
            self.cancel(NoteKind::Error, serial);
            if matches!(old, DeviceState::Device | DeviceState::Unauthorized | DeviceState::Offline | DeviceState::NoPermissions) {
                out.push(Action::Log { category: "device", message: t!("Disconnected: {}", serial), level: "info", serial: Some(serial.clone()) });
                self.schedule(NoteKind::Disconnected, serial, Cond::Gone(serial.clone()), s.disconnected, Urgency::Normal, true, now);
            } else {
                self.forget(serial);
            }
        }
        out.extend(self.evaluate_usb(false, now));
        out
    }

    pub fn update_usb(&mut self, usb: Vec<UsbDevice>, now: f64) -> Vec<Action> {
        let sig = |v: &[UsbDevice]| v.iter().map(|d| (d.vendor_id.clone(), d.product_id.clone(), d.name.clone())).collect::<Vec<_>>();
        let changed = sig(&usb) != sig(&self.usb);
        self.usb = usb;
        self.evaluate_usb(changed, now)
    }

    fn evaluate_usb(&mut self, changed: bool, now: f64) -> Vec<Action> {
        let mut out = Vec::new();
        // Only phones in normal mode: fastboot/EDL do not wait for
        // authorisation.
        let relevant: Vec<&UsbDevice> = self.usb.iter().filter(|d| matches!(d.mode, UsbMode::Adb | UsbMode::Other)).collect();
        if relevant.is_empty() || !self.devices.is_empty() {
            let stale: Vec<(NoteKind, String)> =
                self.pending.keys().filter(|(k, key)| *k == NoteKind::UsbDetected || (*k == NoteKind::WaitingAuthorization && self.state_of(key).is_none())).cloned().collect();
            for (k, key) in stale {
                self.pending.remove(&(k, key));
            }
            if relevant.is_empty() {
                if let Some(key) = self.usb_key.take() {
                    self.forget(&key);
                }
            }
            return out;
        }
        let first = relevant[0];
        let key = if first.serial.is_empty() { first.name.clone() } else { first.serial.clone() };
        let has_adb = relevant.iter().any(|d| d.has_adb_interface());
        let product = first.display_name();
        if changed || self.usb_key.as_deref() != Some(&key) {
            self.usb_key = Some(key.clone());
            out.push(Action::Log { category: "usb", message: t!("Android USB device detected: {}", product), level: "info", serial: Some(key.clone()) });
        }
        let s = self.settle;
        if !self.notified.contains(&(NoteKind::UsbDetected, key.clone())) && now >= self.quiet_until {
            self.schedule(NoteKind::UsbDetected, &key, Cond::UsbOnly, s.usb_detected, Urgency::Normal, false, now);
        }
        if has_adb {
            self.schedule(NoteKind::WaitingAuthorization, &key, Cond::UsbAdbWithoutDevice, s.usb_waiting_authorization, Urgency::Normal, false, now);
        }
        out
    }

    /// Fires the alerts whose time has come and whose condition still holds.
    pub fn tick(&mut self, now: f64) -> Vec<Action> {
        let mut out = Vec::new();
        let due: Vec<(NoteKind, String)> = self.pending.iter().filter(|(_, p)| p.due <= now).map(|(k, _)| k.clone()).collect();
        for k in due {
            let Some(p) = self.pending.remove(&k) else {
                continue;
            };
            if self.holds(&p.cond) {
                if k.0 == NoteKind::Disconnected {
                    self.forget(&k.1);
                }
                self.emit(k.0, &k.1, p.urgency, &mut out);
            }
        }
        out
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(serial: &str, state: DeviceState) -> AdbDevice {
        AdbDevice { model: "Phone".into(), ..AdbDevice::new(serial, state) }
    }

    fn notes(actions: &[Action]) -> Vec<NoteKind> {
        actions.iter().filter_map(|a| if let Action::Notify { kind, .. } = a { Some(*kind) } else { None }).collect()
    }

    fn usb_adb() -> UsbDevice {
        UsbDevice { name: "2-3".into(), serial: "S1".into(), vendor_id: "19d2".into(), mode: UsbMode::Adb, ..Default::default() }
    }

    #[test]
    fn connected_text_follows_transport() {
        assert!(NoteKind::connected_text(Transport::Usb).contains("over USB"));
        let wifi = NoteKind::connected_text(Transport::Tcp);
        assert!(wifi.contains("Wi-Fi") && !wifi.contains("USB."));
    }

    #[test]
    fn devices_present_at_startup_are_quiet() {
        let mut t = Tracker::new(0.0);
        let a = t.update_devices(vec![dev("S1", DeviceState::Device)], 1.0);
        assert!(notes(&a).is_empty());
        assert!(a.iter().any(|x| matches!(x, Action::SessionStart { .. })));
    }

    #[test]
    fn connect_after_quiet_period_notifies_once() {
        let mut t = Tracker::new(0.0);
        let a = t.update_devices(vec![dev("S1", DeviceState::Device)], 10.0);
        assert_eq!(notes(&a), vec![NoteKind::Connected]);
        let a = t.update_devices(vec![dev("S1", DeviceState::Device)], 11.0);
        assert!(notes(&a).is_empty());
    }

    #[test]
    fn flapping_cable_produces_no_notifications() {
        let mut t = Tracker::new(0.0);
        t.update_devices(vec![dev("S1", DeviceState::Device)], 10.0);
        t.update_devices(vec![], 20.0);
        let a = t.update_devices(vec![dev("S1", DeviceState::Device)], 21.0);
        assert!(notes(&a).is_empty(), "quick reconnection without alerts");
        assert!(notes(&t.tick(30.0)).is_empty());
    }

    #[test]
    fn real_disconnect_notifies_after_settle() {
        let mut t = Tracker::new(0.0);
        t.update_devices(vec![dev("S1", DeviceState::Device)], 10.0);
        t.update_devices(vec![], 20.0);
        assert!(notes(&t.tick(22.0)).is_empty());
        assert_eq!(notes(&t.tick(23.5)), vec![NoteKind::Disconnected]);
        // After a real disconnection, the next connection alerts again.
        assert_eq!(notes(&t.update_devices(vec![dev("S1", DeviceState::Device)], 40.0)), vec![NoteKind::Connected]);
    }

    #[test]
    fn unauthorized_waits_then_notifies_once() {
        let mut t = Tracker::new(0.0);
        t.update_devices(vec![dev("S1", DeviceState::Unauthorized)], 10.0);
        assert_eq!(notes(&t.tick(13.5)), vec![NoteKind::WaitingAuthorization]);
        t.update_devices(vec![dev("S1", DeviceState::Unauthorized)], 14.0);
        assert!(notes(&t.tick(30.0)).is_empty());
    }

    #[test]
    fn authorizing_quickly_cancels_waiting_note() {
        let mut t = Tracker::new(0.0);
        t.update_devices(vec![dev("S1", DeviceState::Unauthorized)], 10.0);
        let a = t.update_devices(vec![dev("S1", DeviceState::Device)], 11.0);
        assert_eq!(notes(&a), vec![NoteKind::Connected]);
        assert!(notes(&t.tick(20.0)).is_empty());
    }

    #[test]
    fn usb_only_then_adb_interface_without_adb_device() {
        let mut t = Tracker::new(0.0);
        t.update_usb(vec![usb_adb()], 10.0);
        let fired = notes(&t.tick(16.5));
        assert_eq!(fired, vec![NoteKind::UsbDetected]);
        assert_eq!(notes(&t.tick(20.5)), vec![NoteKind::WaitingAuthorization]);
        assert!(notes(&t.tick(60.0)).is_empty());
    }

    #[test]
    fn usb_notes_cancelled_when_adb_sees_device() {
        let mut t = Tracker::new(0.0);
        t.update_usb(vec![usb_adb()], 10.0);
        t.update_devices(vec![dev("S1", DeviceState::Device)], 11.0);
        assert!(notes(&t.tick(30.0)).is_empty());
        assert!(!t.has_pending());
    }

    #[test]
    fn offline_error_is_critical_after_settle() {
        let mut t = Tracker::new(0.0);
        t.update_devices(vec![dev("S1", DeviceState::Offline)], 10.0);
        let a = t.tick(15.5);
        assert!(matches!(a.as_slice(), [Action::Notify { kind: NoteKind::Error, urgency: Urgency::Critical, .. }]));
    }
}
