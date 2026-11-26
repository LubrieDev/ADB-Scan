pub mod apps;
pub mod automations;
pub mod basic;
pub mod bulk;
pub mod connection;
pub mod control;
pub mod battery;
pub mod battery_usage;
pub mod controls;
pub mod dashboard;
pub mod data;
pub mod debloat;
pub mod developer;
pub mod device;
pub mod facts;
pub mod files;
pub mod flasher;
pub mod history;
pub mod logcat;
pub mod migration;
pub mod mirror;
pub mod network;
pub mod performance;
pub mod permissions;
pub mod remote;
pub mod security;
pub mod storage;
// The embedded terminal is VTE (Unix PTYs). On Windows the section explains
// it and opens adb in a console window instead.
#[cfg(unix)]
pub mod terminal;
#[cfg(windows)]
#[path = "terminal_windows.rs"]
pub mod terminal;
pub mod tools;

/// A section that needs something this system does not have (Windows: the
/// USB bus read directly). It says so clearly instead of showing an empty or
/// failed state.
pub struct Unavailable(adw::StatusPage);

impl crate::window::Page for Unavailable {
    fn widget(&self) -> gtk::Widget {
        use gtk::prelude::Cast;
        self.0.clone().upcast()
    }
}

pub fn unavailable_on_windows(icon: &str, description: &str) -> std::rc::Rc<dyn crate::window::Page> {
    let page = adw::StatusPage::builder().icon_name(icon).title(adbscan_core::i18n::tr("Not available on Windows")).description(description).build();
    std::rc::Rc::new(Unavailable(page))
}
