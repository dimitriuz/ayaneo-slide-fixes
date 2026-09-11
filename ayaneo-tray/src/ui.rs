//! The window.
//!
//! Sized for a 7" handheld panel used with a thumb: one idea per row, every
//! option visible as a large target rather than hidden in a dropdown, and the
//! dense groups (turbo, triggers) collapsed until asked for. All the sizing
//! comes from `widgets`, so it stays consistent rather than drifting per tab.
//!
//! Writes are debounced: dragging a colour or duty slider would otherwise put
//! hundreds of HID reports or UART frames on the wire per second.

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crate::widgets::{
    colour_editor, fan_curve, hint, row, segmented, slider, toggle, unavailable, wide_button,
};

const SENS_LABELS: [&str; 3] = ["50", "100", "150"];
use crate::worker::{Job, Msg, Worker};
use crate::{gamepad, hw, kbdlight, power, rings, state, telemetry, tray::TrayMsg};

const DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Controller,
    Lighting,
    Power,
    Fan,
    Sensors,
    Input,
    About,
}

impl Tab {
    const ALL: [Tab; 7] = [
        Tab::Controller,
        Tab::Lighting,
        Tab::Power,
        Tab::Fan,
        Tab::Sensors,
        Tab::Input,
        Tab::About,
    ];

    fn index(self) -> usize {
        match self {
            Tab::Controller => 0,
            Tab::Lighting => 1,
            Tab::Power => 2,
            Tab::Fan => 3,
            Tab::Sensors => 4,
            Tab::Input => 5,
            Tab::About => 6,
        }
    }
    fn label(self) -> &'static str {
        ["Controller", "Lighting", "Power", "Fan", "Sensors", "Input", "About"][self.index()]
    }
    /// Splitting each section into its own page is what keeps any one screen
    /// down to a few large controls, which is the whole point on a handheld.
    fn subtabs(self) -> &'static [&'static str] {
        match self {
            Tab::Controller => &["Sticks", "Feel", "Triggers", "Gyro", "Turbo"],
            Tab::Lighting => &["Keyboard", "Rings"],
            Tab::Power => &["Profile", "TDP"],
            // A single page needs no second row of navigation.
            Tab::Fan | Tab::Sensors | Tab::Input => &[],
            Tab::About => &["Info", "Display", "Devices"],
        }
    }
}

pub struct App {
    tab: Tab,
    /// Remembered per primary tab, so switching back returns where you were.
    sub: [usize; 7],
    settings: state::Settings,
    trusted: bool,
    devices: hw::Devices,
    rx: Receiver<TrayMsg>,
    dirty_pad: Option<Instant>,
    dirty_kbd: Option<Instant>,
    dirty_rings: Option<Instant>,
    status: String,
    telemetry: telemetry::Telemetry,
    last_poll: Instant,
    profiles: Vec<String>,
    ip: crate::inputplumber::Status,
    ip_profiles: Vec<(String, std::path::PathBuf)>,
    last_ip_poll: Instant,
    profile: Option<String>,
    helper_up: bool,
    /// 0 auto, 1 manual, 2 curve
    fan_mode: u8,
    fan_note: String,
    curve_dirty: Option<Instant>,
    kbd_custom: bool,
    rings_custom: bool,
    style_applied: bool,
    worker: Option<Worker>,
    last_reprobe: Instant,
    /// Debug hook: self-close after N seconds, so close-to-tray can be tested
    /// without a human clicking the titlebar.
    selftest_close_at: Option<Instant>,
}

impl App {
    pub fn new(rx: Receiver<TrayMsg>, devices: hw::Devices) -> Self {
        let (settings, trusted) = state::load();
        Self {
            // Debug hook, same spirit as the close selftest: open straight onto a
            // page so it can be checked without someone clicking through.
            tab: match std::env::var("AYANEO_TRAY_TAB").unwrap_or_default().as_str() {
                s if s.starts_with("lighting") => Tab::Lighting,
                s if s.starts_with("power") => Tab::Power,
                s if s.starts_with("fan") => Tab::Fan,
                s if s.starts_with("sensors") => Tab::Sensors,
                s if s.starts_with("input") => Tab::Input,
                s if s.starts_with("about") => Tab::About,
                _ => Tab::Controller,
            },
            sub: {
                let mut v = [0usize; 7];
                if let Some((_, n)) = std::env::var("AYANEO_TRAY_TAB")
                    .unwrap_or_default()
                    .split_once(':')
                {
                    let n: usize = n.parse().unwrap_or(0);
                    v = [n; 7];
                }
                v
            },
            settings,
            trusted,
            devices,
            rx,
            dirty_pad: None,
            dirty_kbd: None,
            dirty_rings: None,
            status: String::new(),
            telemetry: telemetry::read(),
            last_poll: Instant::now(),
            profiles: power::available(),
            ip: Default::default(),
            ip_profiles: crate::inputplumber::profiles(),
            last_ip_poll: Instant::now() - Duration::from_secs(60),
            profile: power::current(),
            helper_up: false,
            fan_mode: 0,
            fan_note: String::new(),
            curve_dirty: None,
            // same debug spirit as AYANEO_TRAY_TAB: lets the expanded editor be
            // opened and checked without clicking into it
            kbd_custom: std::env::var("AYANEO_TRAY_CUSTOM").is_ok(),
            rings_custom: std::env::var("AYANEO_TRAY_CUSTOM").is_ok(),
            style_applied: false,
            worker: None,
            last_reprobe: Instant::now(),
            selftest_close_at: std::env::var("AYANEO_TRAY_SELFTEST_CLOSE")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map(|secs| Instant::now() + Duration::from_secs(secs)),
        }
    }

    pub fn attach_worker(&mut self, w: Worker) {
        self.worker = Some(w);
    }

    fn curve_spec(points: &[(u8, u8)]) -> String {
        points.iter().map(|(t, s)| format!("{t}:{s}")).collect::<Vec<_>>().join(",")
    }

    /// Queue device work. Never blocks the render thread.
    fn submit(&self, job: Job) {
        if let Some(w) = &self.worker {
            w.submit(job);
        }
    }

    /// Bigger text and taller hit targets than egui's desktop defaults.
    fn apply_style(&mut self, ctx: &egui::Context) {
        use egui::{FontFamily::Proportional, FontId, TextStyle::*};
        let mut style = (*ctx.style()).clone();
        style.text_styles = [
            (Heading, FontId::new(21.0, Proportional)),
            (Body, FontId::new(15.5, Proportional)),
            (Button, FontId::new(16.0, Proportional)),
            (Small, FontId::new(12.5, Proportional)),
            (Monospace, FontId::new(13.0, egui::FontFamily::Monospace)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(10.0, 9.0);
        style.spacing.button_padding = egui::vec2(14.0, 10.0);
        style.spacing.slider_width = 180.0;
        style.spacing.interact_size.y = crate::widgets::TOUCH_H;
        style.visuals.widgets.inactive.rounding = 8.0.into();
        style.visuals.widgets.hovered.rounding = 8.0.into();
        style.visuals.widgets.active.rounding = 8.0.into();
        ctx.set_style(style);
        ctx.set_zoom_factor(self.settings.ui_scale);
        self.style_applied = true;
    }

    fn flush(&mut self) {
        let now = Instant::now();
        let mut touched = false;
        if let Some(t) = self.dirty_pad {
            if now.duration_since(t) > DEBOUNCE {
                self.dirty_pad = None;
                self.submit(Job::ApplyPad(self.settings.record()));
                self.trusted = true;
                touched = true;
            }
        }
        if let Some(t) = self.dirty_kbd {
            if now.duration_since(t) > DEBOUNCE {
                self.dirty_kbd = None;
                self.submit(Job::ApplyKbd(self.settings.kbdlight));
                touched = true;
            }
        }
        if let Some(t) = self.dirty_rings {
            if now.duration_since(t) > DEBOUNCE {
                self.dirty_rings = None;
                self.submit(Job::ApplyRings(self.settings.rings));
                touched = true;
            }
        }
        // Dragging a curve point fires every frame; only send once it settles.
        if let Some(t) = self.curve_dirty {
            if now.duration_since(t) > DEBOUNCE {
                self.curve_dirty = None;
                if self.fan_mode == 2 {
                    self.submit(Job::Helper(format!(
                        "fan curve {}",
                        Self::curve_spec(&self.settings.fan_curve)
                    )));
                }
                touched = true;
            }
        }
        if touched {
            if let Err(e) = state::save(&self.settings) {
                self.status = format!("Could not save settings: {e}");
            }
        }
    }

    // ---------------------------------------------------------------- tabs

    fn controller_tab(&mut self, ui: &mut egui::Ui, sub: usize) {
        if self.devices.gamepad.is_none() {
            unavailable(ui, "Gamepad", &self.devices.gamepad_err);
            return;
        }
        if !self.trusted {
            ui.label(
                egui::RichText::new(
                    "No saved settings yet — these are factory defaults, not what the \
                     controller holds. Changing anything writes the whole record.",
                )
                .color(egui::Color32::from_rgb(226, 150, 70)),
            );
        }
        let mut rec = self.settings.record();
        let mut changed = false;

        match sub {
            0 => {
                row(ui, "Deadzone", |ui| {
                    if let Some(v) = toggle(ui, rec.deadzone()) {
                        rec.set_deadzone(v);
                        changed = true;
                    }
                });
                hint(
                    ui,
                    "Off gives full resolution near centre. Leave on if a stick does not \
                     return to the same place twice.",
                );
                let sens: Vec<(u8, &str)> =
                    gamepad::SENS.iter().map(|(lvl, pct)| (*lvl, SENS_LABELS[(*pct / 50 - 1) as usize])).collect();
                for (left, label) in [(true, "Left"), (false, "Right")] {
                    row(ui, label, |ui| {
                        if let Some(v) = segmented(ui, rec.sens(left), &sens) {
                            rec.set_sens(left, v);
                            changed = true;
                        }
                    });
                }
                hint(ui, "Stick sensitivity, as AYASpace presents it.");
            }
            1 => {
                row(ui, "Rumble", |ui| {
                    if let Some(v) = segmented(ui, rec.rumble(), &gamepad::LEVELS) {
                        rec.set_rumble(v);
                        changed = true;
                    }
                });
                row(ui, "Swap ABXY", |ui| {
                    if let Some(v) = toggle(ui, rec.swap_abxy()) {
                        rec.set_swap_abxy(v);
                        changed = true;
                    }
                });
                ui.add_space(16.0);
                if wide_button(ui, "Restore factory defaults").clicked() {
                    rec = gamepad::Record::default();
                    changed = true;
                }
                hint(ui, "Sends AYANEO's own default record to the controller.");
            }
            2 => {
                for (l2, label) in [(true, "L2"), (false, "R2")] {
                    row(ui, label, |ui| {
                        if let Some(v) = segmented(ui, rec.trigger(l2), &gamepad::LEVELS) {
                            rec.set_trigger(l2, v);
                            changed = true;
                        }
                    });
                }
                hint(ui, "Trigger sensitivity.");
            }
            3 => {
                for (l1, label) in [(true, "L1"), (false, "L2")] {
                    row(ui, label, |ui| {
                        if let Some(v) = segmented(ui, rec.gyro(l1), &gamepad::LEVELS) {
                            rec.set_gyro(l1, v);
                            changed = true;
                        }
                    });
                }
                hint(ui, "Gyro levels. The SLIDE reports these but exposes no gyro UI in AYASpace.");
            }
            _ => {
                for (i, name) in ["A", "B", "X", "Y", "R1", "R2"].iter().enumerate() {
                    row(ui, name, |ui| {
                        if let Some(v) = segmented(ui, rec.turbo(i), &gamepad::TURBO) {
                            rec.set_turbo(i, v);
                            changed = true;
                        }
                    });
                }
                hint(ui, "Burst repeats while held; auto repeats continuously.");
            }
        }

        if changed {
            self.settings.set_record(&rec);
            self.dirty_pad = Some(Instant::now());
        }
    }

    fn lighting_tab(&mut self, ui: &mut egui::Ui, sub: usize) {
        if sub == 0 {
            if self.devices.kbd.is_none() {
                unavailable(ui, "Keyboard backlight", &self.devices.kbd_err);
                return;
            }
            let mut k = self.settings.kbdlight;
            let mut ch = false;
            row(ui, "Backlight", |ui| {
                if let Some(v) = toggle(ui, k.enable) {
                    k.enable = v;
                    ch = true;
                }
            });
            let mut open = self.kbd_custom;
            ui.label(egui::RichText::new("Colour").strong());
            if colour_editor(ui, &mut k.color, &kbdlight::PRESETS, &mut open) {
                ch = true;
            }
            self.kbd_custom = open;
            row(ui, "Effect", |ui| {
                if let Some(v) = segmented(ui, k.mode, &kbdlight::MODES) {
                    k.mode = v;
                    ch = true;
                }
            });
            row(ui, "Brightness", |ui| {
                if slider(ui, &mut k.brightness, 0..=100, " %").changed() {
                    ch = true;
                }
            });
            hint(
                ui,
                "Brightness scales the colour: the firmware has no brightness field, and \
                 AYASpace's is dead code.",
            );
            row(ui, "Fn light", |ui| {
                if let Some(v) = toggle(ui, k.fn_ison) {
                    k.fn_ison = v;
                    ch = true;
                }
            });
            if ch {
                self.settings.kbdlight = k;
                self.dirty_kbd = Some(Instant::now());
            }
        } else {
            if self.devices.rings.is_none() {
                unavailable(ui, "Ring LEDs", &self.devices.rings_err);
                return;
            }
            let mut r = self.settings.rings;
            let mut ch = false;
            let mut open = self.rings_custom;
            ui.label(egui::RichText::new("Colour").strong());
            if colour_editor(ui, &mut r.color, &rings::PRESETS, &mut open) {
                ch = true;
            }
            self.rings_custom = open;
            row(ui, "Brightness", |ui| {
                if slider(ui, &mut r.brightness, 0..=255, "").changed() {
                    ch = true;
                }
            });
            hint(ui, "Zero is off. Ring colour is restored at login.");
            if ch {
                self.settings.rings = r;
                self.dirty_rings = Some(Instant::now());
            }
        }
    }

    fn power_tab(&mut self, ui: &mut egui::Ui, sub: usize) {
        if sub == 1 && !self.helper_up {
            unavailable(ui, "Helper not running", &None);
            hint(ui, "sudo systemctl enable --now ayaneo-tray-helper");
            if wide_button(ui, "Re-check").clicked() {
                self.submit(Job::PollHelper);
            }
            return;
        }
        match sub {
            0 => {
                if self.profiles.is_empty() {
                    hint(ui, "No ACPI platform_profile on this machine.");
                    return;
                }
                let direct = power::writable_directly();
                let can = direct || self.helper_up;
                let self_profiles = self.profiles.clone();
                let opts: Vec<(usize, &str)> =
                    self_profiles.iter().enumerate().map(|(i, p)| (i, p.as_str())).collect();
                let cur = self_profiles
                    .iter()
                    .position(|p| Some(p.as_str()) == self.profile.as_deref())
                    .unwrap_or(usize::MAX);
                let mut picked: Option<String> = None;
                row(ui, "Platform", |ui| {
                    ui.add_enabled_ui(can, |ui| {
                        if let Some(i) = segmented(ui, cur, &opts) {
                            picked = Some(self_profiles[i].clone());
                        }
                    });
                });
                if let Some(p) = picked {
                    if direct {
                        if let Err(e) = std::fs::write(power::PATH, &p) {
                            self.status = format!("Profile: {e}");
                        }
                    } else {
                        self.submit(Job::Helper(format!("profile {p}")));
                    }
                    self.profile = Some(p.clone());
                    self.settings.power_profile = Some(p.clone());
                    let _ = state::save(&self.settings);
                    self.status = format!("Profile: {p}");
                }
                hint(ui, "The ACPI platform profile. Coarse, but the kernel's own interface.");
            }
            _ => {
                let labels: Vec<String> =
                    power::TDP_PRESETS.iter().map(|(w, _)| format!("{w} W")).collect();
                let opts: Vec<(u32, &str)> = power::TDP_PRESETS
                    .iter()
                    .enumerate()
                    .map(|(i, (w, _))| (*w, labels[i].as_str()))
                    .collect();
                let cur_tdp = self.settings.tdp_watts.unwrap_or(0);
                let picked = row(ui, "Sustained", |ui| segmented(ui, cur_tdp, &opts));
                if let Some(w) = picked {
                    self.submit(Job::Helper(format!("tdp {w} {w} {w}")));
                    self.settings.tdp_watts = Some(w);
                    let _ = state::save(&self.settings);
                    self.status = format!("TDP {w} W…");
                }
                hint(
                    ui,
                    "Sets STAPM and the fast/slow limits together via ryzenadj. SMU limits \
                     are volatile and re-applied at login. This shows what was last set — \
                     reading them back needs the ryzen_smu module.",
                );
            }
        }
    }

    fn fan_tab(&mut self, ui: &mut egui::Ui) {
        if !self.helper_up {
            unavailable(ui, "Helper not running", &None);
            hint(ui, "sudo systemctl enable --now ayaneo-tray-helper");
            if wide_button(ui, "Re-check").clicked() {
                self.submit(Job::PollHelper);
            }
            return;
        }
            let cur = self.fan_mode;
            let picked = row(ui, "Control", |ui| {
                segmented(
                    ui,
                    cur,
                    &[(0u8, "Auto"), (1, "Manual"), (2, "Curve")],
                )
            });
            if let Some(m) = picked {
                self.fan_mode = m;
                let req = match m {
                    1 => format!("fan manual {}", self.settings.fan_pct),
                    2 => format!("fan curve {}", Self::curve_spec(&self.settings.fan_curve)),
                    _ => "fan auto".to_string(),
                };
                self.submit(Job::Helper(req));
                self.settings.fan_mode =
                    Some(["auto", "manual", "curve"][m as usize].to_string());
                let _ = state::save(&self.settings);
                self.status = format!("Fan: {}…", ["automatic", "manual", "curve"][m as usize]);
            }

            if self.fan_mode == 1 {
                let mut pct = self.settings.fan_pct;
                let commit = row(ui, "Speed", |ui| {
                    let s = ui.add_enabled_ui(true, |ui| slider(ui, &mut pct, 20..=100, " %")).inner;
                    s.drag_stopped() || s.lost_focus()
                });
                self.settings.fan_pct = pct;
                if commit {
                    self.submit(Job::Helper(format!("fan manual {pct}")));
                    let _ = state::save(&self.settings);
                    self.status = format!("Fan: manual {pct}%…");
                }
                hint(
                    ui,
                    "Speed is the PWM duty cycle — the share of time the fan is driven, \
                     which is what the hardware actually takes. It is commanded, not \
                     measured: this machine has no tachometer.",
                );
            } else if self.fan_mode == 2 {
                let live = self.telemetry.temps.iter().find(|(n, _)| n == "CPU").map(|(_, v)| *v);
                let mut pts = self.settings.fan_curve.clone();
                let moved = fan_curve(ui, &mut pts, live, (40.0, 95.0), (20.0, 100.0));
                if moved {
                    self.settings.fan_curve = pts;
                    self.curve_dirty = Some(Instant::now());
                }
                if let Some(t) = live {
                    let target = crate::fan::curve_speed(&self.settings.fan_curve, t);
                    row(ui, "Now", |ui| {
                        ui.label(
                            egui::RichText::new(format!("{t:.0} °C  ->  {target} %")).size(17.0),
                        );
                    });
                }
                if wide_button(ui, "Reset curve").clicked() {
                    self.settings.fan_curve = crate::fan::default_curve();
                    self.curve_dirty = Some(Instant::now());
                }
                hint(
                    ui,
                    "Drag a point to reshape the curve. The orange line is the current \
                     CPU temperature. Speeds below 20% are refused, and the helper \
                     applies the curve itself every two seconds — the EC has no curve \
                     of its own.",
                );
            }

            if !self.fan_note.is_empty() {
                ui.label(
                    egui::RichText::new(&self.fan_note)
                        .color(egui::Color32::from_rgb(226, 150, 70)),
                );
            }
            hint(
                ui,
                "Above 90 °C the helper hands the fan back to the EC and latches until \
                 you press Auto; in curve mode it first forces full speed at 80 °C \
                 rather than taking control away mid-game.",
            );
    }

    fn sensors_tab(&mut self, ui: &mut egui::Ui) {
            for (n, v) in &self.telemetry.temps {
                row(ui, n, |ui| {
                    ui.label(egui::RichText::new(format!("{v:.0} °C")).size(19.0));
                });
            }
            if let Some(p) = self.telemetry.battery_pct {
                let st = self.telemetry.battery_status.clone().unwrap_or_default();
                row(ui, "Battery", |ui| {
                    ui.label(egui::RichText::new(format!("{p} %  {st}")).size(19.0));
                });
            }
            if let Some(w) = self.telemetry.power_now_w {
                row(ui, "Draw", |ui| {
                    ui.label(egui::RichText::new(format!("{w:.1} W")).size(19.0));
                });
            }
            hint(ui, "Read-only, straight from hwmon and power_supply.");
    }

    fn input_tab(&mut self, ui: &mut egui::Ui) {
        if !self.ip.running {
            unavailable(ui, "InputPlumber", &Some("not responding on the system bus".into()));
            if self.helper_up && wide_button(ui, "Start InputPlumber").clicked() {
                self.submit(Job::Helper("service inputplumber start".into()));
                self.status = "Starting InputPlumber…".into();
            }
            if wide_button(ui, "Re-check").clicked() {
                self.submit(Job::PollIp);
            }
            return;
        }

        row(ui, "Device", |ui| {
            ui.label(egui::RichText::new(&self.ip.device).size(17.0));
        });
        row(ui, "Profile", |ui| {
            ui.label(egui::RichText::new(&self.ip.profile).size(17.0));
        });

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Switch profile").strong());
        hint(
            ui,
            "The profile decides what each control does. \"Default\" has no \
             stick-to-mouse mapping, so switching to it is how you turn the \
             right-stick mouse off.",
        );
        let profiles = self.ip_profiles.clone();
        let mut chosen: Option<std::path::PathBuf> = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (label, path) in &profiles {
                let selected = self.ip.profile == *label;
                let mut b = egui::Button::new(egui::RichText::new(label).size(15.0))
                    .min_size(egui::vec2(0.0, crate::widgets::TOUCH_H));
                if selected {
                    b = b.fill(ui.visuals().selection.bg_fill);
                }
                if ui.add(b).clicked() {
                    chosen = Some(path.clone());
                }
            }
        });
        if let Some(p) = chosen {
            self.submit(Job::IpProfile(p));
            self.status = "Loading profile…".into();
        }

        ui.add_space(10.0);
        ui.label(egui::RichText::new("Emulated controller").strong());
        let cur = self.ip.target.clone().unwrap_or_default();
        let opts: Vec<(&str, &str)> =
            crate::inputplumber::TARGETS.iter().map(|(id, l)| (*id, *l)).collect();
        let picked = segmented(ui, cur.as_str(), &opts);
        if let Some(id) = picked {
            self.submit(Job::IpTarget(id.to_string()));
            self.status = format!("Switching to {id}…");
        }
        hint(
            ui,
            "What games actually see. The Steam Deck target speaks HID and has no \
             /dev/input node, so titles outside Steam often cannot find it — switch to \
             an Xbox target if a game does not detect the controller.",
        );

        ui.add_space(10.0);
        let manage = self.ip.manage_all;
        if let Some(v) = row(ui, "Manage all", |ui| toggle(ui, manage)) {
            self.submit(Job::IpManageAll(v));
            self.status = "Applying…".into();
        }
        hint(
            ui,
            "Whether InputPlumber picks up devices it has no configuration for — \
             external controllers, mostly. It does not change anything for this \
             handheld: its own config sets auto_manage, and auto-managed devices are \
             skipped when this is switched off, so the built-in controller stays \
             managed either way.",
        );

        if self.helper_up {
            ui.add_space(12.0);
            let half = ((ui.available_width() - 10.0) / 2.0).max(110.0);
            let mut restart = false;
            let mut stop = false;
            ui.horizontal(|ui| {
                restart = ui
                    .add_sized(
                        egui::vec2(half, crate::widgets::TOUCH_H),
                        egui::Button::new("Restart service"),
                    )
                    .clicked();
                stop = ui
                    .add_sized(
                        egui::vec2(half, crate::widgets::TOUCH_H),
                        egui::Button::new("Stop service"),
                    )
                    .clicked();
            });
            if restart {
                self.submit(Job::Helper("service inputplumber restart".into()));
                self.status = "Restarting InputPlumber…".into();
            }
            if stop {
                self.submit(Job::Helper("service inputplumber stop".into()));
                self.status = "Stopping InputPlumber…".into();
            }
            hint(ui, &format!("InputPlumber {}", self.ip.version));
        }
    }

    fn about_tab(&mut self, ui: &mut egui::Ui, sub: usize) {
        match sub {
            0 => {
                ui.label(egui::RichText::new("ayaneo-tray").size(22.0).strong());
                hint(ui, &format!("version {}", env!("CARGO_PKG_VERSION")));
                ui.add_space(10.0);
                ui.label("The parts of AYASpace that matter, without a driver or a daemon.");
                ui.add_space(12.0);
                ui.label(egui::RichText::new("Transports").strong());
                hint(ui, "Gamepad — GuLiKit MCU over an on-board UART at I/O 0x3E8, 115200 8N1");
                hint(ui, "Keyboard backlight — HID feature report 0x41");
                hint(ui, "Ring LEDs — sysfs multicolor LED via ayaneo-platform");
                hint(ui, "Fan — EC registers 0xD1C8 (mode) and 0x1804 (duty)");
                hint(ui, "Power — ACPI platform_profile, and ryzenadj through the helper");
            }
            1 => {
                row(ui, "UI scale", |ui| {
                    let s = slider(ui, &mut self.settings.ui_scale, 1.0..=2.0, "×");
                    if s.changed() {
                        ui.ctx().set_zoom_factor(self.settings.ui_scale);
                    }
                    if s.drag_stopped() || s.lost_focus() {
                        let _ = state::save(&self.settings);
                    }
                });
                hint(ui, "Raise this if targets are too small for a thumb.");
            }
            _ => {
                for (n, p) in [
                    ("Gamepad", &self.devices.gamepad),
                    ("Keyboard", &self.devices.kbd),
                    ("Rings", &self.devices.rings),
                ] {
                    row(ui, n, |ui| {
                        ui.label(
                            p.as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "not found".into()),
                        );
                    });
                }
                row(ui, "Helper", |ui| {
                    ui.label(if self.helper_up { "running" } else { "not running" });
                });
                ui.add_space(8.0);
                hint(ui, &format!("settings: {}", state::path().display()));
                hint(ui, "Discovery re-runs every 10 s while anything is missing.");
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.style_applied {
            self.apply_style(ctx);
        }
        // Requests from the tray process, over the IPC socket.
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                TrayMsg::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                TrayMsg::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }

        if let Some(at) = self.selftest_close_at {
            if Instant::now() >= at {
                self.selftest_close_at = None;
                eprintln!("selftest: requesting close");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        // Telemetry is cheap sysfs reads; anything slower goes to the worker.
        if self.last_poll.elapsed() > Duration::from_secs(2) {
            self.telemetry = telemetry::read();
            self.profile = power::current();
            self.last_poll = Instant::now();
            self.submit(Job::PollHelper);
        }
        // InputPlumber state changes rarely and each read is several busctl
        // calls, so poll it far less often than the sensors.
        if self.tab == Tab::Input && self.last_ip_poll.elapsed() > Duration::from_secs(5) {
            self.last_ip_poll = Instant::now();
            self.submit(Job::PollIp);
        }
        // Rediscover anything missing. Cheap when everything is present (the
        // probe is skipped entirely), and it is the only way a device that
        // enumerates after login ever shows up.
        let missing = self.devices.gamepad.is_none()
            || self.devices.kbd.is_none()
            || self.devices.rings.is_none();
        if missing && self.last_reprobe.elapsed() > Duration::from_secs(10) {
            self.last_reprobe = Instant::now();
            self.submit(Job::Reprobe(self.settings.record(), self.trusted));
        }
        while let Ok(m) = self.worker.as_ref().map_or(Err(std::sync::mpsc::TryRecvError::Empty), |w| w.rx.try_recv()) {
            match m {
                Msg::Status(s) => self.status = s,
                Msg::HelperReply(Ok(s)) => self.status = s,
                Msg::HelperReply(Err(e)) => self.status = e,
                Msg::Devices(d) => {
                    let gained = d.gamepad.is_some() && self.devices.gamepad.is_none();
                    self.devices = d;
                    if gained {
                        self.status = "Controller found".into();
                    }
                }
                Msg::IpState(st) => self.ip = st,
                Msg::HelperState(up, s) => {
                    self.helper_up = up;
                    if up {
                        self.fan_mode = if s.contains("mode=curve") {
                            2
                        } else if s.contains("mode=manual") {
                            1
                        } else {
                            0
                        };
                        self.fan_note = if s.contains("tripped=true") {
                            "Thermal failsafe tripped — press Auto to clear.".into()
                        } else {
                            String::new()
                        };
                    }
                }
            }
        }
        self.flush();

        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(148.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                for t in Tab::ALL {
                    let selected = self.tab == t;
                    let mut b = egui::Button::new(egui::RichText::new(t.label()).size(16.0))
                        .min_size(egui::vec2(ui.available_width(), crate::widgets::TOUCH_H + 6.0));
                    if selected {
                        b = b.fill(ui.visuals().selection.bg_fill);
                    }
                    if ui.add(b).clicked() {
                        self.tab = t;
                    }
                    ui.add_space(6.0);
                }
            });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(3.0);
            ui.label(egui::RichText::new(&self.status).size(13.0));
            ui.add_space(3.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let tab = self.tab;
            let subs = tab.subtabs();
            let idx = tab.index();
            if self.sub[idx] >= subs.len() {
                self.sub[idx] = 0;
            }
            ui.add_space(6.0);
            if !subs.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (i, name) in subs.iter().enumerate() {
                    let selected = self.sub[idx] == i;
                    let mut b = egui::Button::new(egui::RichText::new(*name).size(15.0))
                        .min_size(egui::vec2(0.0, crate::widgets::TOUCH_H - 4.0));
                    if selected {
                        b = b.fill(ui.visuals().selection.bg_fill);
                    }
                    if ui.add(b).clicked() {
                        self.sub[idx] = i;
                    }
                }
            });
            ui.add_space(4.0);
            ui.separator();
            }
            ui.add_space(6.0);
            let sub = self.sub[idx];
            // Always reserve the scrollbar. Letting it appear and disappear
            // changes the content width between frames, so wrapped text is
            // measured at one width and drawn at another - egui then
            // underestimates the content height and the last lines cannot be
            // scrolled to at all.
            let mut area = egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .scroll_bar_visibility(
                    egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                );
            // Debug hook: jump to the end, so "is the bottom reachable" can be
            // answered with a screenshot instead of an assumption.
            if std::env::var("AYANEO_TRAY_SCROLL_BOTTOM").is_ok() {
                area = area.stick_to_bottom(true);
            }
            let dbg = std::env::var("AYANEO_TRAY_SCROLL_BOTTOM").is_ok();
            let avail_h = ui.available_height();
            let out = area.show(ui, |ui| {
                // Cap the content at the viewport width so one over-wide child
                // cannot widen the panel and stop everything wrapping.
                //
                // Only max_width: set_width would also force a *minimum*, and
                // the width available before the vertical scrollbar appears is
                // wider than after it does - so forcing it re-created the
                // overflow every frame and left the content height wrong.
                ui.set_max_width(ui.available_width());
                match tab {
                Tab::Controller => self.controller_tab(ui, sub),
                Tab::Lighting => self.lighting_tab(ui, sub),
                Tab::Power => self.power_tab(ui, sub),
                Tab::Fan => self.fan_tab(ui),
                Tab::Sensors => self.sensors_tab(ui),
                Tab::Input => self.input_tab(ui),
                Tab::About => self.about_tab(ui, sub),
                }
                // Trailing space. egui measures wrapped text slightly short, so
                // without a margin the final line stays under the viewport edge
                // even when the scroll offset is genuinely at its maximum.
                ui.add_space(56.0);
            });
            if dbg && self.last_poll.elapsed() > Duration::from_millis(900) {
                eprintln!(
                    "scroll: viewport_h={:.0} content_h={:.0} inner_h={:.0} offset={:.0} avail_h={:.0}",
                    out.inner_rect.height(),
                    out.content_size.y,
                    out.inner_rect.height(),
                    out.state.offset.y,
                    avail_h
                );
            }
        });

        ctx.request_repaint_after(Duration::from_millis(250));
    }
}
