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
pub const LABEL_W: f32 = 150.0;

/// A labelled row: name on the left, controls on the right.
pub fn row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.set_min_height(TOUCH_H);
        ui.add_sized(
            vec2(LABEL_W, TOUCH_H),
            egui::Label::new(RichText::new(label).strong()).halign(Align::LEFT),
        );
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            out = Some(add(ui));
        });
    });
    out.unwrap()
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
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (value, text) in options {
            let selected = *value == current;
            let w = (text.len() as f32 * 9.0 + 34.0).max(64.0);
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
    ui.label(RichText::new(text).size(12.5).weak());
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
