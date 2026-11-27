//! Cheap answer to "how wide would this label be at this height?".
//!
//! GTK 4.14's `GtkBox` lays out a horizontal row with a wrapping label
//! (`AdwActionRow` with a long subtitle, a group header with a button next
//! to it) by searching for the height at which its children fit: about
//! twenty steps, doubling and then halving. At every step it asks each
//! label its width for that height, and the label answers with a binary
//! search of its own, laying the text out about twelve times. Some 240
//! text layouts per label and measure: the header of a single page took
//! 400 of them, and the whole app 50 000 when visiting every section
//! (100–300 ms each time a page was opened, most of it shaping text with
//! HarfBuzz).
//!
//! The interface is height-for-width throughout: no container needs the
//! narrowest width at which a label fits a given height. Labels therefore
//! answer that question with their ordinary width (as for no height at
//! all), computed once per frame, while the height for a given width stays
//! exact: wrapping is unchanged. The search in `GtkBox` then finishes at
//! its first step.

use gtk::ffi::{GtkLabel, GtkOrientation, GtkWidget, GtkWidgetClass, GTK_ORIENTATION_HORIZONTAL};
use gtk::glib;
use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::os::raw::c_int;
use std::sync::OnceLock;

type Measure = unsafe extern "C" fn(*mut GtkWidget, GtkOrientation, c_int, *mut c_int, *mut c_int, *mut c_int, *mut c_int);

static ORIGINAL: OnceLock<Measure> = OnceLock::new();

thread_local! {
    /// Width of each label (address → text address, minimum, natural) during
    /// the current frame: the search asks the same label many times. A new
    /// text is a new string, so a label whose text changed misses. Cleared
    /// once the frame is done.
    static MEMO: RefCell<HashMap<usize, (usize, c_int, c_int)>> = RefCell::new(HashMap::new());
    static CLEAR_QUEUED: Cell<bool> = const { Cell::new(false) };
}

unsafe extern "C" fn measure(widget: *mut GtkWidget, orientation: GtkOrientation, for_size: c_int, minimum: *mut c_int, natural: *mut c_int, minimum_baseline: *mut c_int, natural_baseline: *mut c_int) {
    let Some(original) = ORIGINAL.get() else { return };
    if orientation != GTK_ORIENTATION_HORIZONTAL || for_size < 0 {
        original(widget, orientation, for_size, minimum, natural, minimum_baseline, natural_baseline);
        return;
    }
    let key = widget as usize;
    let text = gtk::ffi::gtk_label_get_label(widget as *mut GtkLabel) as usize;
    let (min, nat) = match MEMO.with(|m| m.borrow().get(&key).copied()) {
        Some((t, min, nat)) if t == text => (min, nat),
        _ => {
            // GtkLabel reads the baselines it is given: real variables.
            let (mut min, mut nat, mut min_base, mut nat_base) = (0, 0, -1, -1);
            original(widget, orientation, -1, &mut min, &mut nat, &mut min_base, &mut nat_base);
            MEMO.with(|m| m.borrow_mut().insert(key, (text, min, nat)));
            if !CLEAR_QUEUED.with(|q| q.replace(true)) {
                // After the frame (layout runs at a higher priority).
                glib::idle_add_local_full(glib::Priority::DEFAULT_IDLE, || {
                    MEMO.with(|m| m.borrow_mut().clear());
                    CLEAR_QUEUED.with(|q| q.set(false));
                    glib::ControlFlow::Break
                });
            }
            (min, nat)
        }
    };
    *minimum = min;
    *natural = nat;
    if !minimum_baseline.is_null() {
        *minimum_baseline = -1;
    }
    if !natural_baseline.is_null() {
        *natural_baseline = -1;
    }
}

/// Replaces `GtkLabel`'s measure (once, before creating the interface).
pub fn install() {
    if ORIGINAL.get().is_some() {
        return;
    }
    // SAFETY: the class of a static GTK type lives for the whole process
    // (`g_type_class_ref` without unref); the replacement keeps the
    // signature and calls the original for everything but width-for-height.
    unsafe {
        let class = gtk::glib::gobject_ffi::g_type_class_ref(gtk::Label::static_type().into_glib()) as *mut GtkWidgetClass;
        if let Some(original) = (*class).measure {
            let _ = ORIGINAL.set(original);
            (*class).measure = Some(measure);
        }
    }
}
