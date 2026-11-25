//! Icons with meaningful motion, in the Lucide style (2-unit stroke on a 24
//! grid, round caps and joins).
//!
//! GTK 4 paints symbolic icons as a texture: CSS cannot reach the strokes
//! inside the SVG. That is why:
//!
//! - [`PackageIcon`] draws the box with Cairo and animates its flaps.
//! - [`TerminalIcon`] stacks two SVGs (frame with `>` and cursor `_`) and CSS
//!   only blinks the cursor layer.
//! - [`set_busy`] spins a button's icon (`-gtk-icon-transform`).
//! - [`SectionIcon`]: when a section opens, its icon switches to the active
//!   Lucide variant if there is one (folder → open folder, shield → shield with
//!   check…) and plays a gesture once that says what it is (the phone vibrates,
//!   the pulse beats, the USB plugs in…). The Wi-Fi arcs light up one after
//!   another.
//!
//! No animation moves on its own: all of them respond to a state change
//! (selection, active page, reload in progress). With “Animations” turned off
//! or the system's reduced motion (`gtk-enable-animations`), libadwaita jumps
//! to the final state and GTK does not run CSS animations.

use adw::prelude::*;
use gtk::cairo;
use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// App box (Lucide “package”) that opens when selected
// ---------------------------------------------------------------------------

type Pt = (f64, f64);

// Geometry of Lucide “package” on its 24 × 24 grid.
const TOP: Pt = (12.0, 1.70);
const RIGHT: Pt = (21.0, 6.84);
const RIGHT_LOW: Pt = (21.0, 17.16);
const BOTTOM: Pt = (12.0, 22.30);
const LEFT_LOW: Pt = (3.0, 17.16);
const LEFT: Pt = (3.0, 6.84);
const FRONT: Pt = (12.0, 12.0);
/// Seam between the two flaps (halves of the back-left and front-right edges).
const SEAM_A: Pt = (7.5, 4.27);
const SEAM_B: Pt = (16.5, 9.42);
/// Trim of the outline's rounded corners (like Lucide's arcs).
const CORNER: f64 = 1.15;
/// On-screen height of an upright flap (its projected side).
const FLAP_UP: f64 = 4.6;
/// Maximum opening of each flap. The front flap turns almost edge-on to the
/// isometric view: past vertical its projection narrows (at 115° it keeps 39 %
/// of its area), so it stays raised (100°, ~70 %). The back flap reads well
/// when opened further.
const FRONT_MAX: f64 = 100.0 * PI / 180.0;
const BACK_MAX: f64 = 120.0 * PI / 180.0;

fn sub(a: Pt, b: Pt) -> Pt {
    (a.0 - b.0, a.1 - b.1)
}
fn lerp(a: Pt, b: Pt, t: f64) -> Pt {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// Position of the free edge of a flap rotating `angle` about its hinge: lying
/// flat (`d`) to upright (`up`) and beyond.
fn rotate(hinge: Pt, d: Pt, angle: f64) -> Pt {
    let (c, s) = (angle.cos(), angle.sin());
    (hinge.0 + c * d.0, hinge.1 + c * d.1 - s * FLAP_UP)
}

/// Corners of the two flaps for a pair of angles: (front, back).
fn flaps(a: f64, b: f64) -> ([Pt; 4], [Pt; 4]) {
    // Front left: hinge LEFT→FRONT, opens forwards.
    let fa = [LEFT, FRONT, rotate(FRONT, sub(SEAM_B, FRONT), a), rotate(LEFT, sub(SEAM_A, LEFT), a)];
    // Back right: hinge TOP→RIGHT, opens backwards.
    let fb = [TOP, RIGHT, rotate(RIGHT, sub(SEAM_B, RIGHT), b), rotate(TOP, sub(SEAM_A, TOP), b)];
    (fa, fb)
}

fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Angles of each flap for a progress of 0..1: the back flap opens first.
fn angles(p: f64) -> (f64, f64) {
    let back = ease_out_cubic(p * 1.25) * BACK_MAX;
    let front = ease_out_cubic((p - 0.2) * 1.25) * FRONT_MAX;
    (front, back)
}

/// Framing (centre, side) in grid units. Closed is Lucide's; open zooms out
/// just enough for the flaps to fit.
fn view(p: f64) -> (Pt, f64) {
    static OPEN: std::sync::LazyLock<(Pt, f64)> = std::sync::LazyLock::new(|| {
        let (a, b) = angles(1.0);
        let (fa, fb) = flaps(a, b);
        let pts: Vec<Pt> = fa.iter().chain(fb.iter()).copied().chain([TOP, RIGHT, RIGHT_LOW, BOTTOM, LEFT_LOW, LEFT]).collect();
        // Half a stroke + half a pixel of margin.
        let m = 1.5;
        let (x0, x1) = pts.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.0 - m), hi.max(p.0 + m)));
        let (y0, y1) = pts.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.1 - m), hi.max(p.1 + m)));
        let side = (x1 - x0).max(y1 - y0).max(24.0);
        (((x0 + x1) / 2.0, (y0 + y1) / 2.0), side)
    });
    let t = ease_out_cubic(p);
    let (c, side) = *OPEN;
    (lerp((12.0, 12.0), c, t), 24.0 + (side - 24.0) * t)
}

/// Closed polygon with rounded corners (curve through the corner).
fn rounded_polygon(cr: &cairo::Context, pts: &[Pt], cut: f64) {
    let n = pts.len();
    let toward = |from: Pt, to: Pt| {
        let (dx, dy) = sub(to, from);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let k = (cut / len).min(0.5);
        (from.0 + dx * k, from.1 + dy * k)
    };
    for i in 0..n {
        let (prev, cur, next) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        let (a, b) = (toward(cur, prev), toward(cur, next));
        if i == 0 {
            cr.move_to(a.0, a.1);
        } else {
            cr.line_to(a.0, a.1);
        }
        // Quadratic a → b with its control point at the corner, expressed as a
        // cubic.
        let c1 = lerp(a, cur, 2.0 / 3.0);
        let c2 = lerp(b, cur, 2.0 / 3.0);
        cr.curve_to(c1.0, c1.1, c2.0, c2.1, b.0, b.1);
    }
    cr.close_path();
}

fn polygon(cr: &cairo::Context, pts: &[Pt]) {
    cr.move_to(pts[0].0, pts[0].1);
    for p in &pts[1..] {
        cr.line_to(p.0, p.1);
    }
    cr.close_path();
}

fn draw_box(cr: &cairo::Context, p: f64) {
    cr.set_line_width(2.0);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    // Body: outline, front edges of the lid and centre edge.
    rounded_polygon(cr, &[TOP, RIGHT, RIGHT_LOW, BOTTOM, LEFT_LOW, LEFT], CORNER);
    cr.move_to(3.29, 7.0);
    cr.line_to(FRONT.0, FRONT.1);
    cr.line_to(20.71, 7.0);
    cr.move_to(FRONT.0, FRONT.1);
    cr.line_to(BOTTOM.0, 22.0);
    if p <= 0.001 {
        // Closed: exactly the Lucide icon (the seam between the flaps).
        cr.move_to(SEAM_A.0, SEAM_A.1);
        cr.line_to(SEAM_B.0, SEAM_B.1);
        let _ = cr.stroke();
        return;
    }
    let _ = cr.stroke();
    let (a, b) = angles(p);
    let (fa, fb) = flaps(a, b);
    // Each flap covers what lies behind it: its inside is cleared and its edge
    // is stroked. Order: back flap, then front flap.
    for flap in [fb, fa] {
        cr.save().ok();
        cr.set_operator(cairo::Operator::Clear);
        polygon(cr, &flap);
        let _ = cr.fill();
        cr.restore().ok();
        polygon(cr, &flap);
        let _ = cr.stroke();
    }
}

struct BoxState {
    progress: Cell<f64>,
    open: Cell<bool>,
    anim: RefCell<Option<adw::TimedAnimation>>,
}

/// Box that opens when its app is selected.
#[derive(Clone)]
pub struct PackageIcon {
    pub widget: gtk::DrawingArea,
    state: Rc<BoxState>,
}

impl PackageIcon {
    pub fn new(pixel_size: i32) -> Self {
        let widget = gtk::DrawingArea::builder().content_width(pixel_size).content_height(pixel_size).valign(gtk::Align::Center).build();
        widget.add_css_class("box-icon");
        let state = Rc::new(BoxState { progress: Cell::new(0.0), open: Cell::new(false), anim: RefCell::new(None) });
        let s = state.clone();
        widget.set_draw_func(move |w, cr, width, height| {
            let p = s.progress.get();
            let (center, side) = view(p);
            let scale = f64::from(width.min(height)) / side;
            cr.translate(f64::from(width) / 2.0, f64::from(height) / 2.0);
            cr.scale(scale, scale);
            cr.translate(-center.0, -center.1);
            let c = w.color();
            cr.set_source_rgba(c.red().into(), c.green().into(), c.blue().into(), c.alpha().into());
            draw_box(cr, p);
        });
        Self { widget, state }
    }

    /// Opens or closes the box. Without `animate` (recycled list rows) it
    /// changes instantly.
    pub fn set_open(&self, open: bool, animate: bool) {
        if self.state.open.replace(open) == open && self.state.anim.borrow().is_none() {
            return;
        }
        if let Some(a) = self.state.anim.take() {
            a.pause();
        }
        let target = if open { 1.0 } else { 0.0 };
        if !animate || !self.widget.is_mapped() {
            self.state.progress.set(target);
            self.widget.queue_draw();
            return;
        }
        let s = self.state.clone();
        let w = self.widget.clone();
        let cb = adw::CallbackAnimationTarget::new(move |v| {
            s.progress.set(v);
            w.queue_draw();
        });
        let from = self.state.progress.get();
        // Opening takes a little longer than closing: the flaps travel further.
        let ms = ((if open { 320.0 } else { 220.0 }) * (target - from).abs()).max(60.0) as u32;
        let anim = adw::TimedAnimation::builder().widget(&self.widget).value_from(from).value_to(target).duration(ms).easing(adw::Easing::Linear).target(&cb).build();
        let s = self.state.clone();
        anim.connect_done(move |_| {
            s.anim.take();
        });
        anim.play();
        *self.state.anim.borrow_mut() = Some(anim);
    }
}

// ---------------------------------------------------------------------------
// Terminal with a blinking cursor
// ---------------------------------------------------------------------------

/// Two-layer terminal icon: the frame with a fixed `>` and the `_` cursor,
/// which blinks only while the Terminal section is open.
#[derive(Clone)]
pub struct TerminalIcon {
    pub widget: gtk::Overlay,
    cursor: gtk::Image,
}

impl TerminalIcon {
    pub fn new() -> Self {
        let frame = gtk::Image::from_icon_name("adb-square-terminal-frame-symbolic");
        let cursor = gtk::Image::from_icon_name("adb-square-terminal-cursor-symbolic");
        cursor.add_css_class("terminal-cursor");
        // The cursor must not receive events or take up space of its own.
        cursor.set_can_target(false);
        let widget = gtk::Overlay::builder().child(&frame).build();
        widget.add_overlay(&cursor);
        Self { widget, cursor }
    }

    /// Starts blinking. When the class is removed, GTK stops the animation and
    /// the cursor instantly returns to its normal opacity.
    pub fn set_active(&self, active: bool) {
        if active {
            self.cursor.add_css_class("blinking");
        } else {
            self.cursor.remove_css_class("blinking");
        }
    }
}

// ---------------------------------------------------------------------------
// Gesture of each sidebar section
// ---------------------------------------------------------------------------

/// Icon of the “active” Lucide variant for a section: the sidebar icon switches
/// to it while the section is open (the folder opens, the shield shows the
/// check…).
fn active_icon(section: &str) -> Option<&'static str> {
    Some(match section {
        "dashboard" => "adb-layout-grid-symbolic",
        "device" => "adb-vibrate-symbolic",
        "performance" => "adb-heart-pulse-symbolic",
        "battery" => "adb-battery-charging-symbolic",
        "files" => "adb-folder-open-symbolic",
        "mirror" => "adb-monitor-play-symbolic",
        "logcat" => "adb-scroll-text-symbolic",
        "security" => "adb-shield-check-symbolic",
        _ => return None,
    })
}

/// Icon of a sidebar section. With the section open (class `play`): if Lucide
/// has an active variant, it crossfades to it and stays that way for the visit;
/// it also plays its CSS gesture once (`anim-<section>` in style.css). Network
/// is layered: the Wi-Fi arcs light up one after another.
#[derive(Clone)]
pub struct SectionIcon {
    pub widget: gtk::Widget,
}

impl SectionIcon {
    pub fn new(section: &str, icon_name: &str) -> Self {
        let overlay = |base: &gtk::Image, layers: &[gtk::Image]| {
            let o = gtk::Overlay::builder().child(base).build();
            for l in layers {
                l.set_can_target(false);
                o.add_overlay(l);
            }
            o.upcast::<gtk::Widget>()
        };
        let widget = if section == "network" {
            let img = |name: &str, class: &str| {
                let i = gtk::Image::from_icon_name(name);
                i.add_css_class(class);
                i
            };
            let base = img("adb-wifi-dot-symbolic", "l0");
            overlay(&base, &[img("adb-wifi-arc1-symbolic", "l1"), img("adb-wifi-arc2-symbolic", "l2"), img("adb-wifi-arc3-symbolic", "l3")])
        } else if let Some(active) = active_icon(section) {
            let off = gtk::Image::from_icon_name(icon_name);
            off.add_css_class("off");
            let on = gtk::Image::from_icon_name(active);
            on.add_css_class("on");
            let w = overlay(&off, &[on]);
            w.add_css_class("swap");
            w
        } else {
            gtk::Image::from_icon_name(icon_name).upcast()
        };
        widget.add_css_class("section-icon");
        widget.add_css_class(&format!("anim-{section}"));
        Self { widget }
    }

    pub fn set_active(&self, active: bool) {
        if active {
            self.widget.add_css_class("play");
        } else {
            self.widget.remove_css_class("play");
        }
    }
}

// ---------------------------------------------------------------------------
// Refresh buttons that spin while they work
// ---------------------------------------------------------------------------

/// Spins the button's icon while a reload lasts.
pub fn set_busy(button: &impl IsA<gtk::Widget>, busy: bool) {
    let b = button.as_ref();
    if busy {
        b.add_css_class("busy");
    } else {
        b.remove_css_class("busy");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_flaps_match_lucide_top_face() {
        let (fa, fb) = flaps(0.0, 0.0);
        let close = |a: Pt, b: Pt| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9;
        // Lying flat, their free edges match Lucide's seam.
        assert!(close(fa[2], SEAM_B) && close(fa[3], SEAM_A));
        assert!(close(fb[2], SEAM_B) && close(fb[3], SEAM_A));
    }

    #[test]
    fn open_box_fits_its_view() {
        let (center, side) = view(1.0);
        let (a, b) = angles(1.0);
        let (fa, fb) = flaps(a, b);
        for p in fa.iter().chain(fb.iter()) {
            assert!((p.0 - center.0).abs() + 1.0 <= side / 2.0 && (p.1 - center.1).abs() + 1.0 <= side / 2.0, "{p:?} outside {center:?}/{side}");
        }
        // Closed does not zoom out at all: same size as the other icons.
        assert_eq!(view(0.0), ((12.0, 12.0), 24.0));
        // Zoom out just enough: less than 25 %.
        assert!(side < 30.0);
    }

    #[test]
    fn back_flap_opens_first() {
        let (front, back) = angles(0.3);
        assert!(back > front);
        assert_eq!(angles(1.0), (FRONT_MAX, BACK_MAX));
        assert_eq!(angles(0.0), (0.0, 0.0));
    }
}
