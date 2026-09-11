//! Touch-sized building blocks.
//!
//! The first version of this UI put twenty-odd controls on one page as small
//! selectable labels. That reads fine on a desktop and is unusable with a thumb
//! on a 7" panel, so everything here is built around a few large, consistent
//! primitives rather than raw egui widgets scattered through the layout.

use egui::{vec2, Align, Button, Color32, Layout, Response, RichText, Ui, Vec2};

/// Minimum height of anything you are expected to hit with a finger.
pub const TOUCH_H: f32 = 46.0;
/// Width reserved for a row's label, so rows line up down the page.
pub const LABEL_W: f32 = 104.0;

/// A labelled row: name on the left, controls on the right.
///
/// The control area is explicitly bounded to what is left over. Without that,
/// a too-wide child (a fixed-width slider, a third segmented option) silently
/// widens the whole panel, and everything below it is then clipped at the
/// window edge rather than wrapping.
pub fn row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let mut out = None;
    let avail = ui.available_width();
    ui.horizontal(|ui| {
        ui.set_min_height(TOUCH_H);
        ui.add_sized(
            vec2(LABEL_W, TOUCH_H),
            egui::Label::new(RichText::new(label).strong()).halign(Align::LEFT),
        );
        let w = (avail - LABEL_W - 12.0).max(140.0);
        ui.allocate_ui_with_layout(vec2(w, TOUCH_H), Layout::left_to_right(Align::Center), |ui| {
            out = Some(add(ui));
        });
    });
    out.unwrap()
}

/// A slider that fills the row rather than assuming a fixed width.
pub fn slider<T: egui::emath::Numeric>(
    ui: &mut Ui,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    suffix: &str,
) -> Response {
    // Leave room for the value readout egui draws beside the track. It is sized
    // for the widest text it can hold - "2000 px/s" needs visibly more than
    // "40 %", and guessing one width for both clips the longer one.
    let digits = format!("{:.0}", range.end().to_f64()).len();
    let reserve = (34.0 + (digits + suffix.len()) as f32 * 9.0).max(76.0);
    ui.spacing_mut().slider_width = (ui.available_width() - reserve).max(90.0);
    ui.add(egui::Slider::new(value, range).suffix(suffix))
}

/// A segmented control. Returns the newly chosen value, if any.
///
/// Used instead of radio buttons and dropdowns throughout: every option stays
/// visible and every option is a large target, which matters more on a handheld
/// than the space a dropdown would save.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    current: T,
    options: &[(T, &str)],
) -> Option<T> {
    let mut chosen = None;
    // wrapped, so a long set of options folds onto a second line instead of
    // running off the edge
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (value, text) in options {
            let selected = *value == current;
            let w = (text.len() as f32 * 8.0 + 26.0).max(58.0);
            let mut b = Button::new(RichText::new(*text).size(16.0)).min_size(vec2(w, TOUCH_H));
            if selected {
                b = b.fill(ui.visuals().selection.bg_fill);
            }
            if ui.add(b).clicked() {
                chosen = Some(*value);
            }
        }
    });
    chosen
}

/// An on/off pair. Clearer than a checkbox at arm's length, and a bigger target.
pub fn toggle(ui: &mut Ui, on: bool) -> Option<bool> {
    segmented(ui, on, &[(true, "On"), (false, "Off")])
}

/// A full-width primary action.
pub fn wide_button(ui: &mut Ui, text: &str) -> Response {
    ui.add_sized(
        vec2(ui.available_width(), TOUCH_H + 4.0),
        Button::new(RichText::new(text).size(16.0)),
    )
}

/// A large colour swatch, for preset rows.
pub fn swatch(ui: &mut Ui, rgb: u32, selected: bool) -> Response {
    let c = Color32::from_rgb(
        ((rgb >> 16) & 0xFF) as u8,
        ((rgb >> 8) & 0xFF) as u8,
        (rgb & 0xFF) as u8,
    );
    let size = Vec2::splat(TOUCH_H);
    let mut b = Button::new("").fill(c).min_size(size);
    if selected {
        b = b.stroke(egui::Stroke::new(3.0, ui.visuals().selection.stroke.color));
    }
    ui.add_sized(size, b)
}

/// Section heading with breathing room above it.
pub fn section(ui: &mut Ui, title: &str) {
    ui.add_space(14.0);
    ui.label(RichText::new(title).size(19.0).strong());
    ui.add_space(2.0);
}

/// Small explanatory text under a control.
pub fn hint(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.5).weak()).wrap());
    ui.add_space(2.0);
}

/// Something else on the system is fighting this control.
///
/// Louder than a hint and quieter than `unavailable`: the control still works,
/// it just may not be the last word on the setting.
pub fn warn(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.add(
        egui::Label::new(
            RichText::new(text).size(13.5).color(Color32::from_rgb(226, 150, 70)),
        )
        .wrap(),
    );
    ui.add_space(2.0);
}

/// A warning that a subsystem is not reachable.
pub fn unavailable(ui: &mut Ui, what: &str, err: &Option<String>) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(format!("{what} unavailable"))
            .size(16.0)
            .color(Color32::from_rgb(226, 150, 70)),
    );
    if let Some(e) = err {
        hint(ui, e);
    }
    hint(
        ui,
        "Install 70-ayaneo-tray.rules, then: sudo udevadm control --reload && sudo udevadm trigger",
    );
}

/// Preset swatches plus an inline RGB editor.
///
/// egui's `color_edit_button_srgb` opens a popup with a saturation/value square
/// and a hue strip, which is taller than a handheld window: on this panel the
/// square was clipped by the window edge and the hue strip was off-screen
/// entirely, with no way to scroll a popup. So the custom editor is inline,
/// built from the same rows and sliders as the rest of the interface, and it
/// cannot overflow because it is part of the scrolling page.
pub fn colour_editor(ui: &mut Ui, colour: &mut u32, presets: &[u32], open: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for p in presets {
            if swatch(ui, *p, *colour == *p).clicked() {
                *colour = *p;
                changed = true;
            }
        }
        let is_preset = presets.contains(colour);
        let mut b = Button::new(RichText::new("Custom").size(15.0))
            .min_size(vec2(96.0, TOUCH_H));
        if *open || !is_preset {
            b = b.fill(ui.visuals().selection.bg_fill);
        }
        if ui.add(b).clicked() {
            *open = !*open;
        }
    });

    if *open {
        let (mut r, mut g, mut b) = (
            ((*colour >> 16) & 0xFF) as u8,
            ((*colour >> 8) & 0xFF) as u8,
            (*colour & 0xFF) as u8,
        );
        let mut touched = false;
        for (name, v) in [("R", &mut r), ("G", &mut g), ("B", &mut b)] {
            row(ui, name, |ui| {
                if slider(ui, v, 0..=255, "").changed() {
                    touched = true;
                }
            });
        }
        if touched {
            *colour = ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
            changed = true;
        }
        row(ui, "Preview", |ui| {
            let c = Color32::from_rgb(r, g, b);
            ui.add_sized(vec2(TOUCH_H * 2.0, TOUCH_H), Button::new("").fill(c));
            ui.label(RichText::new(format!("#{:06X}", *colour)).size(16.0));
        });
    }
    changed
}

/// A draggable fan curve.
///
/// Drawn by hand rather than with a plotting crate so the handles can be
/// finger-sized: a plot library's default hit areas are built for a mouse, and
/// on this panel they are close to unusable. Temperature runs left to right,
/// speed bottom to top, and the live temperature is marked so the curve can be
/// read against what the machine is actually doing.
///
/// Returns true when a point moved.
pub fn fan_curve(
    ui: &mut Ui,
    points: &mut [(u8, u8)],
    live_temp: Option<f32>,
    t_range: (f32, f32),
    s_range: (f32, f32),
) -> bool {
    // Allocate room for the axis labels as well as the plot: painter_at clips
    // to the rect it is given, so anything drawn below it simply vanishes.
    const AXIS_H: f32 = 18.0;
    let h = 210.0;
    let w = ui.available_width().min(430.0);
    let (outer, _) = ui.allocate_exact_size(vec2(w, h + AXIS_H), egui::Sense::hover());
    let rect = egui::Rect::from_min_max(
        outer.min,
        egui::pos2(outer.max.x, outer.max.y - AXIS_H),
    );
    let p = ui.painter_at(outer);
    let vis = ui.visuals();

    p.rect_filled(rect, 6.0, vis.extreme_bg_color);

    let to_screen = |t: f32, s: f32| -> egui::Pos2 {
        let x = rect.left() + (t - t_range.0) / (t_range.1 - t_range.0) * rect.width();
        let y = rect.bottom() - (s - s_range.0) / (s_range.1 - s_range.0) * rect.height();
        egui::pos2(x, y)
    };

    // grid: every 10 C and every 25 %
    let grid = vis.weak_text_color().gamma_multiply(0.35);
    let mut t = (t_range.0 / 10.0).ceil() * 10.0;
    while t <= t_range.1 {
        let x = to_screen(t, s_range.0).x;
        p.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, grid),
        );
        // A label centred on the very edge loses half its digits to the clip
        // rect, so pin the outermost ones inward.
        let align = if x - rect.left() < 12.0 {
            egui::Align2::LEFT_TOP
        } else if rect.right() - x < 24.0 {
            egui::Align2::RIGHT_TOP
        } else {
            egui::Align2::CENTER_TOP
        };
        p.text(
            egui::pos2(x, rect.bottom() + 3.0),
            align,
            format!("{t:.0}"),
            egui::FontId::proportional(11.0),
            vis.weak_text_color(),
        );
        t += 10.0;
    }
    for s in [25.0f32, 50.0, 75.0, 100.0] {
        let y = to_screen(t_range.0, s).y;
        p.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.0, grid),
        );
        p.text(
            egui::pos2(rect.left() + 3.0, y),
            egui::Align2::LEFT_BOTTOM,
            format!("{s:.0}%"),
            egui::FontId::proportional(11.0),
            vis.weak_text_color(),
        );
    }

    // live temperature marker
    if let Some(lt) = live_temp {
        if lt >= t_range.0 && lt <= t_range.1 {
            let x = to_screen(lt, 0.0).x;
            p.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(2.0, Color32::from_rgb(226, 150, 70)),
            );
        }
    }

    // the curve, flat beyond the end points
    let line = vis.selection.bg_fill;
    let mut path: Vec<egui::Pos2> = Vec::new();
    if let Some(first) = points.first() {
        path.push(to_screen(t_range.0, first.1 as f32));
    }
    for (t, s) in points.iter() {
        path.push(to_screen(*t as f32, *s as f32));
    }
    if let Some(last) = points.last() {
        path.push(to_screen(t_range.1, last.1 as f32));
    }
    for w in path.windows(2) {
        p.line_segment([w[0], w[1]], egui::Stroke::new(2.5, line));
    }

    // draggable handles, deliberately larger than they look
    let mut changed = false;
    let n = points.len();
    for i in 0..n {
        let (t, s) = points[i];
        let pos = to_screen(t as f32, s as f32);
        let hit = egui::Rect::from_center_size(pos, Vec2::splat(TOUCH_H));
        let id = ui.id().with(("fan_curve_pt", i));
        let resp = ui.interact(hit, id, egui::Sense::drag());
        let r = if resp.dragged() { 13.0 } else { 10.0 };
        p.circle_filled(pos, r, line);
        p.circle_stroke(pos, r, egui::Stroke::new(2.0, vis.extreme_bg_color));

        if resp.dragged() {
            if let Some(m) = resp.interact_pointer_pos() {
                let nt = t_range.0
                    + ((m.x - rect.left()) / rect.width()).clamp(0.0, 1.0)
                        * (t_range.1 - t_range.0);
                let ns = s_range.0
                    + ((rect.bottom() - m.y) / rect.height()).clamp(0.0, 1.0)
                        * (s_range.1 - s_range.0);
                // keep points ordered and inside their neighbours, so the curve
                // stays a function of temperature however it is dragged
                let lo = if i == 0 { t_range.0 } else { points[i - 1].0 as f32 + 1.0 };
                let hi = if i + 1 == n { t_range.1 } else { points[i + 1].0 as f32 - 1.0 };
                points[i].0 = nt.clamp(lo, hi).round() as u8;
                points[i].1 = ns.clamp(s_range.0, s_range.1).round() as u8;
                changed = true;
            }
        }
    }
    p.text(
        egui::pos2(rect.right(), rect.bottom() + 3.0),
        egui::Align2::RIGHT_TOP,
        "°C",
        egui::FontId::proportional(11.0),
        vis.weak_text_color(),
    );
    ui.add_space(10.0);
    changed
}
