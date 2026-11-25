//! Resident icon in the tray (StatusNotifierItem, compatible with Cinnamon's
//! XApp applet through `xapp-sn-watcher`).
//!
//! Left click: quick panel. Right click: menu.

use adbscan_core::i18n::tr;
use ksni::blocking::TrayMethods;
use ksni::menu::{MenuItem, StandardItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayIcon {
    Connected,
    Waiting,
    Disconnected,
    Error,
}

impl TrayIcon {
    fn name(self) -> &'static str {
        match self {
            Self::Connected => "adb-scan-connected",
            Self::Waiting => "adb-scan-waiting",
            Self::Disconnected => "adb-scan-disconnected",
            Self::Error => "adb-scan-error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCmd {
    ShowWindow,
    TogglePanel,
    Screenshot,
    Mirror,
    Settings,
    Quit,
}

struct AdbTray {
    icon: TrayIcon,
    tooltip: String,
    theme_path: String,
    tx: async_channel::Sender<crate::ctx::Msg>,
}

impl AdbTray {
    fn send(&self, cmd: TrayCmd) {
        let _ = self.tx.send_blocking(crate::ctx::Msg::Tray(cmd));
    }
}

impl ksni::Tray for AdbTray {
    const MENU_ON_ACTIVATE: bool = false;

    fn id(&self) -> String {
        "adb-scan".into()
    }
    fn title(&self) -> String {
        "ADB Scan".into()
    }
    fn icon_name(&self) -> String {
        self.icon.name().into()
    }
    fn icon_theme_path(&self) -> String {
        self.theme_path.clone()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: "ADB Scan".into(), description: self.tooltip.clone(), ..Default::default() }
    }
    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCmd::TogglePanel);
    }
    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCmd::ShowWindow);
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let connected = self.icon == TrayIcon::Connected;
        let item = |label: &str, icon: &str, cmd: TrayCmd, enabled: bool| -> MenuItem<Self> {
            StandardItem { label: label.into(), icon_name: icon.into(), enabled, activate: Box::new(move |t: &mut Self| t.send(cmd)), ..Default::default() }.into()
        };
        vec![
            StandardItem { label: self.tooltip.clone(), enabled: false, ..Default::default() }.into(),
            MenuItem::Separator,
            item(tr("Open ADB Scan"), "adb-house-symbolic", TrayCmd::ShowWindow, true),
            item(tr("Quick panel"), "adb-layout-dashboard-symbolic", TrayCmd::TogglePanel, true),
            MenuItem::Separator,
            item(tr("Screenshot"), "adb-camera-photo-symbolic", TrayCmd::Screenshot, connected),
            item(tr("Mirror screen"), "adb-video-display-symbolic", TrayCmd::Mirror, connected),
            MenuItem::Separator,
            item(tr("Preferences"), "adb-settings-symbolic", TrayCmd::Settings, true),
            item(tr("Quit"), "adb-log-out-symbolic", TrayCmd::Quit, true),
        ]
    }
}

pub struct TrayHandle {
    handle: ksni::blocking::Handle<AdbTray>,
    /// Last state sent: avoids D-Bus calls on every snapshot.
    last: std::cell::RefCell<Option<(TrayIcon, String)>>,
}

impl TrayHandle {
    pub fn spawn(tx: async_channel::Sender<crate::ctx::Msg>) -> Option<Self> {
        let tray = AdbTray { icon: TrayIcon::Disconnected, tooltip: tr("No device").into(), theme_path: crate::theme::icons_dir().display().to_string(), tx };
        match tray.spawn() {
            Ok(handle) => Some(Self { handle, last: std::cell::RefCell::new(None) }),
            Err(e) => {
                log::warn!("Could not create the tray icon: {e}");
                None
            }
        }
    }

    pub fn set_state(&self, icon: TrayIcon, tooltip: String) {
        let state = (icon, tooltip.clone());
        if self.last.borrow().as_ref() == Some(&state) {
            return;
        }
        *self.last.borrow_mut() = Some(state);
        self.handle.update(move |t| {
            if t.icon != icon || t.tooltip != tooltip {
                t.icon = icon;
                t.tooltip = tooltip;
            }
        });
    }
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        self.handle.shutdown().wait();
    }
}
