//! Developer tools without root: port forwarding between the PC and the
//! phone, the status bar's demo mode (clean screenshots), screen resolution
//! and density, animation speed and the developer switches (touches, pointer
//! location, layout bounds, stay awake).
//!
//! Every command here is fixed: the only values that reach the phone are
//! numbers checked by [`check_port`], [`check_size`] and [`check_density`],
//! or one of [`ANIMATION_SCALES`].

use crate::error::{Error, Result};
use crate::i18n::tr;

/// The developer switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Switch {
    ShowTouches,
    PointerLocation,
    LayoutBounds,
    StayAwake,
}

impl Switch {
    pub const ALL: [Switch; 4] = [Switch::ShowTouches, Switch::PointerLocation, Switch::LayoutBounds, Switch::StayAwake];

    pub fn title(self) -> &'static str {
        match self {
            Switch::ShowTouches => tr("Show taps"),
            Switch::PointerLocation => tr("Pointer location"),
            Switch::LayoutBounds => tr("Show layout bounds"),
            Switch::StayAwake => tr("Stay awake while charging"),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Switch::ShowTouches => tr("A circle where you touch: useful when recording the screen."),
            Switch::PointerLocation => tr("Coordinates and trail of every touch at the top of the screen."),
            Switch::LayoutBounds => tr("Outlines every view and its margins. Apps already open update when they redraw."),
            Switch::StayAwake => tr("The screen never turns off while the phone is charging."),
        }
    }

    /// Script that turns it on or off.
    pub fn script(self, on: bool) -> String {
        let v = |a: &str, b: &str| if on { a.to_string() } else { b.to_string() };
        match self {
            Switch::ShowTouches => format!("settings put system show_touches {}", v("1", "0")),
            Switch::PointerLocation => format!("settings put system pointer_location {}", v("1", "0")),
            // The system property, then the “refresh system properties” call
            // Settings makes so running apps read it again.
            Switch::LayoutBounds => format!("setprop debug.layout {}; service call activity 1599295570 >/dev/null 2>&1; true", v("true", "false")),
            // 7 = AC, USB and wireless.
            Switch::StayAwake => format!("settings put global stay_on_while_plugged_in {}", v("7", "0")),
        }
    }
}

/// Animation scales offered (0 = off, 1 = normal).
pub const ANIMATION_SCALES: [&str; 6] = ["0", "0.25", "0.5", "1", "1.5", "2"];

/// Current state, read with [`STATE_SCRIPT`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DevState {
    pub switches: Vec<(Switch, bool)>,
    /// `window_animation_scale` (the three are set together).
    pub animation_scale: Option<String>,
    /// Physical and override size (`wm size`).
    pub physical_size: Option<(u32, u32)>,
    pub override_size: Option<(u32, u32)>,
    pub physical_density: Option<u32>,
    pub override_density: Option<u32>,
    pub demo_allowed: bool,
}

impl DevState {
    pub fn switch(&self, s: Switch) -> bool {
        self.switches.iter().any(|(x, on)| *x == s && *on)
    }
}

/// Read-only.
pub const STATE_SCRIPT: &str = "echo touches=$(settings get system show_touches); echo pointer=$(settings get system pointer_location); echo layout=$(getprop debug.layout); \
echo stay=$(settings get global stay_on_while_plugged_in); echo anim=$(settings get global window_animation_scale); echo demo=$(settings get global sysui_demo_allowed); wm size; wm density";

fn parse_size(v: &str) -> Option<(u32, u32)> {
    let (w, h) = v.trim().split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

pub fn parse_state(out: &str) -> DevState {
    let mut s = DevState::default();
    let mut on = |sw: Switch, v: bool| s.switches.push((sw, v));
    for line in out.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim();
            match k {
                "touches" => on(Switch::ShowTouches, v == "1"),
                "pointer" => on(Switch::PointerLocation, v == "1"),
                "layout" => on(Switch::LayoutBounds, v == "true"),
                "stay" => on(Switch::StayAwake, v.parse::<u32>().is_ok_and(|n| n > 0)),
                _ => {}
            }
        }
    }
    for line in out.lines().map(str::trim) {
        if let Some(v) = line.strip_prefix("anim=") {
            // `null` means the default, 1.
            s.animation_scale = Some(if v.is_empty() || v == "null" { "1".into() } else { v.trim_end_matches(".0").to_string() });
        } else if let Some(v) = line.strip_prefix("demo=") {
            s.demo_allowed = v == "1";
        } else if let Some(v) = line.strip_prefix("Physical size:") {
            s.physical_size = parse_size(v);
        } else if let Some(v) = line.strip_prefix("Override size:") {
            s.override_size = parse_size(v);
        } else if let Some(v) = line.strip_prefix("Physical density:") {
            s.physical_density = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("Override density:") {
            s.override_density = v.trim().parse().ok();
        }
    }
    s
}

pub fn check_port(p: u32) -> Result<u16> {
    u16::try_from(p).ok().filter(|p| *p > 0).ok_or_else(|| Error::Invalid(tr("the port must be between 1 and 65535").into()))
}

/// Screen sizes between 240 and 8192 pixels per side.
pub fn check_size(w: u32, h: u32) -> Result<(u32, u32)> {
    if (240..=8192).contains(&w) && (240..=8192).contains(&h) {
        Ok((w, h))
    } else {
        Err(Error::Invalid(tr("each side must be between 240 and 8192 pixels").into()))
    }
}

/// Densities between 72 and 1200 dpi.
pub fn check_density(d: u32) -> Result<u32> {
    if (72..=1200).contains(&d) {
        Ok(d)
    } else {
        Err(Error::Invalid(tr("the density must be between 72 and 1200 dpi").into()))
    }
}

/// Script for the status bar's demo mode: on (full battery, full signal,
/// 12:00, no notifications) or off.
pub fn demo_script(on: bool) -> &'static str {
    if on {
        "settings put global sysui_demo_allowed 1; \
         am broadcast -a com.android.systemui.demo -e command enter >/dev/null; \
         am broadcast -a com.android.systemui.demo -e command clock -e hhmm 1200 >/dev/null; \
         am broadcast -a com.android.systemui.demo -e command battery -e level 100 -e plugged false >/dev/null; \
         am broadcast -a com.android.systemui.demo -e command network -e wifi show -e level 4 >/dev/null; \
         am broadcast -a com.android.systemui.demo -e command network -e mobile show -e datatype none -e level 4 >/dev/null; \
         am broadcast -a com.android.systemui.demo -e command notifications -e visible false >/dev/null; true"
    } else {
        "am broadcast -a com.android.systemui.demo -e command exit >/dev/null; true"
    }
}

/// A port forwarding rule: `local` on one side reaches `remote` on the other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortRule {
    /// `adb reverse` (phone → PC) or `adb forward` (PC → phone).
    pub reverse: bool,
    pub local: String,
    pub remote: String,
}

/// Rules of `adb forward --list` (only those of `serial`) or `adb reverse
/// --list`. Lines are `<serial or transport> <local> <remote>`.
pub fn parse_rules(text: &str, reverse: bool, serial: &str) -> Vec<PortRule> {
    text.lines()
        .filter_map(|l| {
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() < 3 || (!reverse && parts[0] != serial) {
                return None;
            }
            let (local, remote) = (parts[parts.len() - 2], parts[parts.len() - 1]);
            (local.contains(':') && remote.contains(':')).then(|| PortRule { reverse, local: local.into(), remote: remote.into() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state() {
        let out = "touches=1\npointer=0\nlayout=\nstay=0\nanim=0.5\ndemo=null\nPhysical size: 1216x2688\nOverride size: 1080x2388\nPhysical density: 520\n";
        let s = parse_state(out);
        assert!(s.switch(Switch::ShowTouches));
        assert!(!s.switch(Switch::LayoutBounds));
        assert_eq!(s.animation_scale.as_deref(), Some("0.5"));
        assert_eq!(s.physical_size, Some((1216, 2688)));
        assert_eq!(s.override_size, Some((1080, 2388)));
        assert_eq!((s.physical_density, s.override_density), (Some(520), None));
        assert!(!s.demo_allowed);
        assert_eq!(parse_state("anim=null\n").animation_scale.as_deref(), Some("1"));
        assert_eq!(parse_state("anim=1.0\n").animation_scale.as_deref(), Some("1"));
    }

    #[test]
    fn checks() {
        assert!(check_port(0).is_err() && check_port(70000).is_err());
        assert_eq!(check_port(3000).unwrap(), 3000);
        assert!(check_size(100, 2000).is_err());
        assert!(check_density(50).is_err() && check_density(420).is_ok());
    }

    #[test]
    fn rules() {
        let fwd = "ABC tcp:8080 tcp:80\nXYZ tcp:1 tcp:2\n";
        assert_eq!(parse_rules(fwd, false, "ABC"), vec![PortRule { reverse: false, local: "tcp:8080".into(), remote: "tcp:80".into() }]);
        let rev = "UsbFfs tcp:3000 tcp:3000\n";
        assert_eq!(parse_rules(rev, true, "ABC")[0].local, "tcp:3000");
    }
}
