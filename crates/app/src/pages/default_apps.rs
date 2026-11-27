//! Default apps dialog (from Apps → ⋮ and the search): browser, home screen,
//! phone, SMS, assistant (see `adbscan_core::roles`) and keyboard (see
//! `adbscan_core::keyboards`). Reading is
//! read-only; choosing another app changes the phone, is checked and recorded
//! in History, and is undone by choosing the previous one again.

use adbscan_core::{i18n::tr, t};
use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

use adbscan_core::roles::RoleState;

use crate::ctx::Ctx;
use crate::util;

pub fn show(ctx: &Rc<Ctx>, parent: &gtk::Widget) {
    let (Some(adb), Some(serial)) = (ctx.adb(), ctx.ready_serial()) else {
        ctx.toast(tr("No device is ready"));
        return;
    };
    let dialog = adw::Dialog::builder().title(tr("Default apps")).content_width(560).content_height(520).build();
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
        .title(tr("Default apps"))
        .description(tr("The app Android opens for links, the home button, calls, SMS and the assistant, and the keyboard you type with. The same as in Settings: choosing the previous app undoes it."))
        .build();
    let status = util::label(tr("Reading the phone…"), &["dim", "caption"]);
    status.set_wrap(true);
    group.add(&status);
    page.add(&group);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));

    let (ctx, group, status) = (ctx.clone(), group.clone(), status.clone());
    util::run_bg(
        move || adb.roles(&serial).map(|r| (r, adb.keyboards(&serial).ok())),
        move |r| match r {
            Ok((roles, keyboards)) => {
                status.set_visible(false);
                fill(&ctx, &group, roles);
                if let Some(k) = keyboards.filter(|k| !k.enabled.is_empty()) {
                    keyboard_row(&ctx, &group, k);
                }
            }
            Err(e) => status.set_text(&t!("Could not read the default apps: {}", e)),
        },
    );
}

/// The keyboard in use, among the enabled ones.
fn keyboard_row(ctx: &Rc<Ctx>, group: &adw::PreferencesGroup, k: adbscan_core::keyboards::Keyboards) {
    use adbscan_core::keyboards::package_of;
    let labels = crate::pages::apps::cached_labels();
    let names: Vec<String> = k
        .enabled
        .iter()
        .map(|id| {
            let pkg = package_of(id);
            let name = labels.get(pkg).cloned().unwrap_or_else(|| pkg.to_string());
            // The same app may offer several (typing and voice).
            if k.enabled.iter().filter(|o| package_of(o) == pkg).count() > 1 {
                format!("{name} ({})", id.rsplit(['.', '/']).next().unwrap_or(id))
            } else {
                name
            }
        })
        .collect();
    let row = adw::ComboRow::builder().title(tr("Keyboard")).model(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())).build();
    if k.enabled.len() < 2 {
        row.set_subtitle(tr("Only one keyboard is enabled: others are enabled on the phone, in Settings → System → Keyboard"));
    } else {
        row.set_subtitle(tr("Among the keyboards enabled on the phone"));
    }
    let current = k.current.as_ref().and_then(|c| k.enabled.iter().position(|e| e == c)).unwrap_or(0);
    row.set_selected(current as u32);
    let ids = Rc::new(k.enabled.clone());
    let names = Rc::new(names);
    let previous = Rc::new(Cell::new(current as u32));
    let painting = Rc::new(Cell::new(false));
    let ctx = ctx.clone();
    row.connect_selected_notify(move |row| {
        if painting.get() {
            return;
        }
        let i = row.selected();
        let Some(id) = ids.get(i as usize).cloned() else { return };
        let (Some(adb), Some(serial)) = (ctx.adb(), ctx.ready_serial()) else { return };
        row.set_sensitive(false);
        let msg = t!("{}: {}", tr("Keyboard"), names.get(i as usize).cloned().unwrap_or_default());
        let (ctx, row, previous, painting) = (ctx.clone(), row.clone(), previous.clone(), painting.clone());
        let id2 = id.clone();
        util::run_bg(
            move || adb.set_keyboard(&serial, &id2),
            move |r| {
                row.set_sensitive(true);
                match r {
                    Ok(_) => {
                        previous.set(row.selected());
                        ctx.audit("default_keyboard", Some(&id), true, None);
                        ctx.toast(&msg);
                    }
                    Err(e) => {
                        ctx.audit("default_keyboard", Some(&id), false, Some(&e.to_string()));
                        ctx.toast(&t!("Could not change it: {}", e));
                        painting.set(true);
                        row.set_selected(previous.get());
                        painting.set(false);
                    }
                }
            },
        );
    });
    group.add(&row);
}

fn fill(ctx: &Rc<Ctx>, group: &adw::PreferencesGroup, roles: Vec<RoleState>) {
    let labels = crate::pages::apps::cached_labels();
    let name = |p: &str| labels.get(p).cloned().unwrap_or_else(|| p.to_string());
    for state in roles {
        let row = adw::ComboRow::builder().title(state.role.label()).build();
        if state.unsupported {
            row.set_subtitle(tr("This Android version does not allow it over ADB"));
            row.set_sensitive(false);
            group.add(&row);
            continue;
        }
        // “None” first, then the candidates.
        let mut names: Vec<String> = vec![tr("None").into()];
        names.extend(state.candidates.iter().map(|p| name(p)));
        row.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
        let current = state.holders.first().and_then(|h| state.candidates.iter().position(|c| c == h)).map(|i| i + 1).unwrap_or(0);
        row.set_selected(current as u32);
        if state.candidates.len() < 2 {
            row.set_subtitle(tr("No other app on the phone can do this"));
        }
        let candidates = Rc::new(state.candidates.clone());
        let previous = Rc::new(Cell::new(current as u32));
        let painting = Rc::new(Cell::new(false));
        let (ctx, role) = (ctx.clone(), state.role);
        row.connect_selected_notify(move |row| {
            if painting.get() {
                return;
            }
            let i = row.selected();
            // “None” cannot be chosen: Android always keeps one for these.
            let Some(pkg) = (i as usize).checked_sub(1).and_then(|i| candidates.get(i)).cloned() else {
                painting.set(true);
                row.set_selected(previous.get());
                painting.set(false);
                return;
            };
            let (Some(adb), Some(serial)) = (ctx.adb(), ctx.ready_serial()) else { return };
            row.set_sensitive(false);
            let msg = t!("{}: {}", role.label(), crate::pages::apps::cached_labels().get(&pkg).cloned().unwrap_or(pkg.clone()));
            let (ctx, row, previous, painting) = (ctx.clone(), row.clone(), previous.clone(), painting.clone());
            util::run_bg(
                {
                    let pkg = pkg.clone();
                    move || adb.set_role(&serial, role, &pkg)
                },
                move |r| {
                    row.set_sensitive(true);
                    match r {
                        Ok(_) => {
                            previous.set(row.selected());
                            ctx.audit("default_app", Some(&format!("{} · {pkg}", role.name())), true, None);
                            ctx.toast(&msg);
                        }
                        Err(e) => {
                            ctx.audit("default_app", Some(&format!("{} · {pkg}", role.name())), false, Some(&e.to_string()));
                            ctx.toast(&t!("Could not change it: {}", e));
                            painting.set(true);
                            row.set_selected(previous.get());
                            painting.set(false);
                        }
                    }
                },
            );
        });
        group.add(&row);
    }
}
