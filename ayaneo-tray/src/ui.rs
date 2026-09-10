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

use crate::widgets::{hint, row, section, segmented, swatch, toggle, unavailable, wide_button};
use crate::worker::{Job, Msg, Worker};
use crate::{gamepad, hw, kbdlight, power, rings, state, telemetry, tray::TrayMsg};

const DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Controller,
    Lighting,
    Power,
    About,
}

pub struct App {
    tab: Tab,
    settings: state::Settings,
    trusted: bool,
    devices: hw::Devices,
    rx: Receiver<TrayMsg>,
    visible: bool,
    dirty_pad: Option<Instant>,
    dirty_kbd: Option<Instant>,
    dirty_rings: Option<Instant>,
    status: String,
    telemetry: telemetry::Telemetry,
    last_poll: Instant,
    profiles: Vec<String>,
    profile: Option<String>,
    helper_up: bool,
    fan_manual: bool,
    fan_pct: u8,
    fan_note: String,
    style_applied: bool,
    worker: Option<Worker>,
}

impl App {
    pub fn new(rx: Receiver<TrayMsg>, start_visible: bool, devices: hw::Devices) -> Self {
        let (settings, trusted) = state::load();
        Self {
            tab: Tab::Controller,
            settings,
            trusted,
            devices,
            rx,
            visible: start_visible,
            dirty_pad: None,
            dirty_kbd: None,
            dirty_rings: None,
            status: String::new(),
            telemetry: telemetry::read(),
            last_poll: Instant::now(),
            profiles: power::available(),
            profile: power::current(),
            helper_up: false,
            fan_manual: false,
            fan_pct: 45,
            fan_note: String::new(),
            style_applied: false,
            worker: None,
        }
    }

    pub fn attach_worker(&mut self, w: Worker) {
        self.worker = Some(w);
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
        style.spacing.slider_width = 240.0;
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
        if touched {
            if let Err(e) = state::save(&self.settings) {
                self.status = format!("Could not save settings: {e}");
            }
        }
    }

    // ---------------------------------------------------------------- tabs

    fn controller_tab(&mut self, ui: &mut egui::Ui) {
        if self.devices.gamepad.is_none() {
            unavailable(ui, "Gamepad", &self.devices.gamepad_err);
            return;
        }
        if !self.trusted {
            ui.label(
                egui::RichText::new(
                    "No saved settings yet. The values below are factory defaults, not \
                     what the controller currently holds — changing anything writes the \
                     whole record.",
                )
                .color(egui::Color32::from_rgb(226, 150, 70)),
            );
        }
        let mut rec = self.settings.record();
        let mut changed = false;

        section(ui, "Sticks");
        row(ui, "Deadzone", |ui| {
            if let Some(v) = toggle(ui, rec.deadzone()) {
                rec.set_deadzone(v);
                changed = true;
            }
        });
        hint(
            ui,
            "Off gives full resolution near centre. Leave on if a stick does not return \
             to the same place twice.",
        );
        let sens: Vec<(u8, &str)> = gamepad::SENS
            .iter()
            .map(|(lvl, pct)| (*lvl, if *pct == 50 { "50" } else if *pct == 100 { "100" } else { "150" }))
            .collect();
        for (left, label) in [(true, "Left sensitivity"), (false, "Right sensitivity")] {
            row(ui, label, |ui| {
                if let Some(v) = segmented(ui, rec.sens(left), &sens) {
                    rec.set_sens(left, v);
                    changed = true;
                }
            });
        }

        section(ui, "Feel");
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

        egui::CollapsingHeader::new(egui::RichText::new("Triggers and gyro").size(17.0))
            .default_open(false)
            .show(ui, |ui| {
                for (l2, label) in [(true, "Trigger L2"), (false, "Trigger R2")] {
                    row(ui, label, |ui| {
                        if let Some(v) = segmented(ui, rec.trigger(l2), &gamepad::LEVELS) {
                            rec.set_trigger(l2, v);
                            changed = true;
                        }
                    });
                }
                for (l1, label) in [(true, "Gyro L1"), (false, "Gyro L2")] {
                    row(ui, label, |ui| {
                        if let Some(v) = segmented(ui, rec.gyro(l1), &gamepad::LEVELS) {
                            rec.set_gyro(l1, v);
                            changed = true;
                        }
                    });
                }
            });

        egui::CollapsingHeader::new(egui::RichText::new("Turbo").size(17.0))
            .default_open(false)
            .show(ui, |ui| {
                for (i, name) in ["A", "B", "X", "Y", "R1", "R2"].iter().enumerate() {
                    row(ui, name, |ui| {
                        if let Some(v) = segmented(ui, rec.turbo(i), &gamepad::TURBO) {
                            rec.set_turbo(i, v);
                            changed = true;
                        }
                    });
                }
            });

        ui.add_space(14.0);
        if wide_button(ui, "Restore factory defaults").clicked() {
            rec = gamepad::Record::default();
            changed = true;
        }

        if changed {
            self.settings.set_record(&rec);
            self.dirty_pad = Some(Instant::now());
        }
    }

    /// Preset swatches plus a picker, on one touch-sized row.
    fn colour_row(ui: &mut egui::Ui, colour: &mut u32, presets: &[u32]) -> bool {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            for p in presets {
                if swatch(ui, *p, *colour == *p).clicked() {
                    *colour = *p;
                    changed = true;
                }
            }
            let mut rgb = [
                ((*colour >> 16) & 0xFF) as u8,
                ((*colour >> 8) & 0xFF) as u8,
                (*colour & 0xFF) as u8,
            ];
            if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
                *colour = ((rgb[0] as u32) << 16) | ((rgb[1] as u32) << 8) | rgb[2] as u32;
                changed = true;
            }
        });
        changed
    }

    fn lighting_tab(&mut self, ui: &mut egui::Ui) {
        section(ui, "Keyboard");
        if self.devices.kbd.is_none() {
            unavailable(ui, "Keyboard backlight", &self.devices.kbd_err);
        } else {
            let mut k = self.settings.kbdlight;
            let mut ch = false;
            row(ui, "Backlight", |ui| {
                if let Some(v) = toggle(ui, k.enable) {
                    k.enable = v;
                    ch = true;
                }
            });
            row(ui, "Colour", |ui| {
                if Self::colour_row(ui, &mut k.color, &kbdlight::PRESETS) {
                    ch = true;
                }
            });
            row(ui, "Effect", |ui| {
                if let Some(v) = segmented(ui, k.mode, &kbdlight::MODES) {
                    k.mode = v;
                    ch = true;
                }
            });
            row(ui, "Brightness", |ui| {
                if ui
                    .add(egui::Slider::new(&mut k.brightness, 0..=100).suffix(" %"))
                    .changed()
                {
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
        }

        section(ui, "Joystick rings");
        if self.devices.rings.is_none() {
            unavailable(ui, "Ring LEDs", &self.devices.rings_err);
        } else {
            let mut r = self.settings.rings;
            let mut ch = false;
            row(ui, "Colour", |ui| {
                if Self::colour_row(ui, &mut r.color, &rings::PRESETS) {
                    ch = true;
                }
            });
            row(ui, "Brightness", |ui| {
                if ui.add(egui::Slider::new(&mut r.brightness, 0..=255)).changed() {
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

    fn power_tab(&mut self, ui: &mut egui::Ui) {
        section(ui, "Profile");
        if self.profiles.is_empty() {
            hint(ui, "No ACPI platform_profile on this machine.");
        } else {
            let direct = power::writable_directly();
            let can = direct || self.helper_up;
            let self_profiles = self.profiles.clone();
            let opts: Vec<(usize, &str)> =
                self_profiles.iter().enumerate().map(|(i, p)| (i, p.as_str())).collect();
            let cur = self
                .profiles
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
        }

        section(ui, "Sustained power");
        if !self.helper_up {
            unavailable(ui, "TDP control", &Some("helper not running".into()));
            hint(ui, "sudo systemctl enable --now ayaneo-tray-helper  (and install ryzenadj)");
            if wide_button(ui, "Re-check helper").clicked() {
                self.submit(Job::PollHelper);
            }
            return;
        }
        let tdp: Vec<(u32, &str)> =
            power::TDP_PRESETS.iter().map(|(w, _)| (*w, "")).collect::<Vec<_>>();
        let labels: Vec<String> = power::TDP_PRESETS.iter().map(|(w, _)| format!("{w} W")).collect();
        let opts: Vec<(u32, &str)> = tdp
            .iter()
            .enumerate()
            .map(|(i, (w, _))| (*w, labels[i].as_str()))
            .collect();
        let cur_tdp = self.settings.tdp_watts.unwrap_or(0);
        let picked_tdp = row(ui, "TDP", |ui| segmented(ui, cur_tdp, &opts));
        if let Some(w) = picked_tdp {
            self.submit(Job::Helper(format!("tdp {w} {w} {w}")));
            self.settings.tdp_watts = Some(w);
            let _ = state::save(&self.settings);
            self.status = format!("TDP {w} W…");
        }
        hint(
            ui,
            "Sets STAPM and the fast/slow limits together. SMU limits are volatile and \
             re-applied at login. The selection shows what was last set — reading limits \
             back needs the ryzen_smu module.",
        );

        section(ui, "Fan");
        let cur_fan = self.fan_manual;
        let picked_fan =
            row(ui, "Control", |ui| segmented(ui, cur_fan, &[(false, "Auto"), (true, "Manual")]));
        if let Some(manual) = picked_fan {
            let req = if manual {
                format!("fan manual {}", self.fan_pct)
            } else {
                "fan auto".to_string()
            };
            self.submit(Job::Helper(req));
            self.fan_manual = manual;
            self.status = if manual {
                format!("Fan: manual {}%…", self.fan_pct)
            } else {
                "Fan: automatic…".into()
            };
        }
        let manual_now = self.fan_manual;
        let mut pct = self.fan_pct;
        let commit = row(ui, "Duty", |ui| {
            let s = ui.add_enabled(manual_now, egui::Slider::new(&mut pct, 20..=100).suffix(" %"));
            s.drag_stopped() || s.lost_focus()
        });
        self.fan_pct = pct;
        if commit && manual_now {
            self.submit(Job::Helper(format!("fan manual {}", self.fan_pct)));
            self.status = format!("Fan: manual {}%…", self.fan_pct);
        }
        hint(
            ui,
            "Above 85 °C the helper forces automatic control and latches until you press \
             Auto; the fan is handed back whenever the helper stops. Commanded duty — this \
             machine has no tachometer.",
        );
        if !self.fan_note.is_empty() {
            hint(ui, &self.fan_note);
        }

        section(ui, "Sensors");
        ui.horizontal_wrapped(|ui| {
            for (n, v) in &self.telemetry.temps {
                ui.label(egui::RichText::new(format!("{n}  {v:.0} °C")).size(17.0));
                ui.add_space(14.0);
            }
        });
        if let Some(p) = self.telemetry.battery_pct {
            let st = self.telemetry.battery_status.clone().unwrap_or_default();
            let w = self.telemetry.power_now_w.map(|w| format!("   {w:.1} W")).unwrap_or_default();
            ui.label(egui::RichText::new(format!("Battery  {p} %   {st}{w}")).size(17.0));
        }
    }

    fn about_tab(&mut self, ui: &mut egui::Ui) {
        section(ui, "ayaneo-tray");
        ui.label(format!("version {}", env!("CARGO_PKG_VERSION")));
        hint(ui, "The parts of AYASpace that matter, without a driver or a daemon.");

        section(ui, "Display");
        row(ui, "UI scale", |ui| {
            let s = ui.add(egui::Slider::new(&mut self.settings.ui_scale, 1.0..=2.0).step_by(0.05));
            if s.changed() {
                ui.ctx().set_zoom_factor(self.settings.ui_scale);
            }
            if s.drag_stopped() || s.lost_focus() {
                let _ = state::save(&self.settings);
            }
        });
        hint(ui, "Raise this if targets are too small for a thumb.");

        section(ui, "Devices");
        for (n, p) in [
            ("Gamepad", &self.devices.gamepad),
            ("Keyboard", &self.devices.kbd),
            ("Rings", &self.devices.rings),
        ] {
            row(ui, n, |ui| {
                ui.label(
                    p.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "—".into()),
                );
            });
        }
        row(ui, "Helper", |ui| {
            ui.label(if self.helper_up { "running" } else { "not running" });
        });

        section(ui, "Transports");
        hint(ui, "Gamepad — GuLiKit MCU over an on-board UART at I/O 0x3E8, 115200 8N1");
        hint(ui, "Keyboard backlight — HID feature report 0x41");
        hint(ui, "Ring LEDs — sysfs multicolor LED via ayaneo-platform");
        hint(ui, "Fan — EC registers 0xD1C8 (mode) and 0x1804 (duty)");
        ui.add_space(8.0);
        hint(ui, &format!("settings: {}", state::path().display()));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.style_applied {
            self.apply_style(ctx);
        }
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                TrayMsg::Toggle => self.visible = !self.visible,
                TrayMsg::Show => self.visible = true,
                TrayMsg::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(self.visible));
            if self.visible {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.visible {
            self.visible = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        // Telemetry is cheap sysfs reads; anything slower goes to the worker.
        if self.last_poll.elapsed() > Duration::from_secs(2) {
            self.telemetry = telemetry::read();
            self.profile = power::current();
            self.last_poll = Instant::now();
            self.submit(Job::PollHelper);
        }
        while let Ok(m) = self.worker.as_ref().map_or(Err(std::sync::mpsc::TryRecvError::Empty), |w| w.rx.try_recv()) {
            match m {
                Msg::Status(s) => self.status = s,
                Msg::HelperReply(Ok(s)) => self.status = s,
                Msg::HelperReply(Err(e)) => self.status = e,
                Msg::HelperState(up, s) => {
                    self.helper_up = up;
                    if up {
                        self.fan_manual = s.contains("manual=true");
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

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let w = (ui.available_width() - 24.0) / 4.0;
                for (t, name) in [
                    (Tab::Controller, "Controller"),
                    (Tab::Lighting, "Lighting"),
                    (Tab::Power, "Power"),
                    (Tab::About, "About"),
                ] {
                    let sel = self.tab == t;
                    let mut b = egui::Button::new(egui::RichText::new(name).size(16.0))
                        .min_size(egui::vec2(w, crate::widgets::TOUCH_H + 4.0));
                    if sel {
                        b = b.fill(ui.visuals().selection.bg_fill);
                    }
                    if ui.add(b).clicked() {
                        self.tab = t;
                    }
                }
            });
            ui.add_space(6.0);
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(3.0);
            ui.label(egui::RichText::new(&self.status).size(13.0));
            ui.add_space(3.0);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| match self.tab {
                Tab::Controller => self.controller_tab(ui),
                Tab::Lighting => self.lighting_tab(ui),
                Tab::Power => self.power_tab(ui),
                Tab::About => self.about_tab(ui),
            });
        });

        ctx.request_repaint_after(Duration::from_millis(250));
    }
}
