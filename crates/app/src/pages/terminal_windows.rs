//! Terminal section on Windows. The embedded terminal (VTE) needs Unix PTYs:
//! here the section says so and opens the phone's `adb shell`, or a command
//! prompt with adb in its PATH, in a console window of their own.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use std::path::PathBuf;
use std::rc::Rc;

use crate::ctx::{Ctx, Event};
use crate::util;
use crate::window::Page;

pub struct TerminalPage {
    ctx: Rc<Ctx>,
    root: adw::StatusPage,
    phone_btn: gtk::Button,
}

pub fn build(ctx: &Rc<Ctx>) -> Rc<dyn Page> {
    let phone_btn = util::pill_button("adb-smartphone-symbolic", tr("Open the phone's shell"));
    phone_btn.add_css_class("suggested-action");
    let pc_btn = util::pill_button("adb-square-terminal-symbolic", tr("Open a command prompt with adb"));
    let buttons = gtk::Box::new(gtk::Orientation::Vertical, 12);
    buttons.set_halign(gtk::Align::Center);
    buttons.append(&phone_btn);
    buttons.append(&pc_btn);
    let root = adw::StatusPage::builder()
        .icon_name("adb-square-terminal-symbolic")
        .title(tr("Not available on Windows"))
        .description(tr("The built-in terminal needs Linux. On Windows, the phone's shell (adb shell) and a command prompt with adb and fastboot open in a window of their own."))
        .child(&buttons)
        .build();
    let page = Rc::new(TerminalPage { ctx: ctx.clone(), root, phone_btn: phone_btn.clone() });
    {
        let ctx = ctx.clone();
        phone_btn.connect_clicked(move |_| open_phone_shell(&ctx));
    }
    {
        let ctx = ctx.clone();
        pc_btn.connect_clicked(move |_| open_prompt(&ctx));
    }
    page.update();
    page
}

/// Folder of adb (and fastboot), to put first in the console's PATH.
fn tools_dir(ctx: &Ctx) -> Option<PathBuf> {
    ctx.adb().and_then(|a| a.path().parent().map(|p| p.to_path_buf()))
}

fn open_phone_shell(ctx: &Rc<Ctx>) {
    let (Some(adb), Some(serial)) = (ctx.adb(), ctx.ready_serial()) else {
        ctx.toast(tr("No device is ready"));
        return;
    };
    if let Err(e) = crate::win::open_console(adb.path(), &["-s", &serial, "shell"], tools_dir(ctx).as_deref()) {
        ctx.toast(&t!("Could not open the shell: {}", e));
    }
}

fn open_prompt(ctx: &Rc<Ctx>) {
    let cmd = std::env::var_os("ComSpec").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("cmd.exe"));
    if let Err(e) = crate::win::open_console(&cmd, &[], tools_dir(ctx).as_deref()) {
        ctx.toast(&t!("Could not open the shell: {}", e));
    }
}

impl TerminalPage {
    fn update(&self) {
        let ready = self.ctx.adb().is_some() && self.ctx.ready_serial().is_some();
        self.phone_btn.set_sensitive(ready);
        self.phone_btn.set_tooltip_text(if ready { None } else { Some(tr("Connect a phone with debugging authorised")) });
    }
}

impl Page for TerminalPage {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn shown(&self) {
        self.update();
    }

    fn on_event(&self, ev: &Event) {
        if matches!(ev, Event::Devices) {
            self.update();
        }
    }
}
