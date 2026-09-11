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
    // leave room for the value readout egui draws beside the track
    ui.spacing_mut().slider_width = (ui.available_width() - 76.0).max(90.0);
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
