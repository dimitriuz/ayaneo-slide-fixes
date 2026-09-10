//! The window. Four tabs, sized for a handheld screen.
//!
//! Changes apply immediately, but writes are debounced: dragging a colour
//! slider would otherwise put hundreds of HID reports or UART frames on the
//! wire per second.

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crate::{gamepad, helper, hw, kbdlight, power, rings, state, telemetry, tray::TrayMsg};

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
    /// Pending writes, per subsystem, with the time the change was made.
    dirty_pad: Option<Instant>,
    dirty_kbd: Option<Instant>,
    dirty_rings: Option<Instant>,
    status: String,
    telemetry: telemetry::Telemetry,
    last_telemetry: Instant,
    profiles: Vec<String>,
    profile: Option<String>,
    helper_up: bool,
}

impl App {
    pub fn new(rx: Receiver<TrayMsg>, start_visible: bool) -> Self {
        let (settings, trusted) = state::load();
        let devices = hw::Devices::probe(&settings.record(), trusted);
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
            last_telemetry: Instant::now(),
            profiles: power::available(),
            profile: power::current(),
            helper_up: helper::available(),
        }
    }

    fn flush(&mut self) {
        let now = Instant::now();
        let mut saved = false;
        if let Some(t) = self.dirty_pad {
            if now.duration_since(t) > DEBOUNCE {
                self.dirty_pad = None;
                if let Some(p) = self.devices.gamepad.clone() {
                    match gamepad::send(&p, &self.settings.record()) {
                        Ok(_) => self.status = "controller applied".into(),
                        Err(e) => self.status = format!("controller: {e}"),
                    }
                }
                // once we have written, the saved record describes the hardware
                self.trusted = true;
                saved = true;
            }
        }
        if let Some(t) = self.dirty_kbd {
            if now.duration_since(t) > DEBOUNCE {
                self.dirty_kbd = None;
                if let Some(p) = self.devices.kbd.clone() {
                    match kbdlight::apply(&p, &self.settings.kbdlight) {
                        Ok(()) => self.status = "keyboard applied".into(),
                        Err(e) => self.status = format!("keyboard: {e}"),
                    }
                }
                saved = true;
            }
        }
        if let Some(t) = self.dirty_rings {
            if now.duration_since(t) > DEBOUNCE {
                self.dirty_rings = None;
                if let Some(p) = self.devices.rings.clone() {
                    match rings::apply(&p, &self.settings.rings) {
                        Ok(()) => self.status = "rings applied".into(),
                        Err(e) => self.status = format!("rings: {e}"),
                    }
                }
                saved = true;
            }
        }
        if saved {
            if let Err(e) = state::save(&self.settings) {
                self.status = format!("could not save settings: {e}");
            }
        }
    }

    fn unavailable(ui: &mut egui::Ui, what: &str, err: &Option<String>) {
        ui.colored_label(
            egui::Color32::from_rgb(220, 140, 60),
            format!("{what} unavailable"),
        );
        if let Some(e) = err {
            ui.label(egui::RichText::new(e).small());
        }
        ui.label(
            egui::RichText::new(
                "Install 70-ayaneo-tray.rules and re-plug, or run \
                 `sudo udevadm control --reload && sudo udevadm trigger`.",
            )
            .small()
            .weak(),
        );
    }

    fn level_picker(ui: &mut egui::Ui, label: &str, cur: u8, table: &[(u8, &str)]) -> Option<u8> {
        let mut chosen = None;
        ui.horizontal(|ui| {
            ui.label(format!("{label}:"));
            for (v, name) in table {
                let mut sel = cur == *v;
                if ui.selectable_label(sel, *name).clicked() {
                    sel = true;
                    chosen = Some(*v);
                }
                let _ = sel;
            }
        });
        chosen
    }

    fn controller_tab(&mut self, ui: &mut egui::Ui) {
        if self.devices.gamepad.is_none() {
            Self::unavailable(ui, "Gamepad", &self.devices.gamepad_err);
            return;
        }
        if !self.trusted {
            ui.colored_label(
                egui::Color32::from_rgb(220, 140, 60),
                "No saved settings yet — the values below are factory defaults, \
                 not what the controller currently holds. Changing anything writes \
                 the whole record.",
            );
            ui.separator();
        }
        let mut rec = self.settings.record();
        let mut changed = false;

        let mut dz = rec.deadzone();
        if ui.checkbox(&mut dz, "Stick deadzone").changed() {
            rec.set_deadzone(dz);
            changed = true;
        }
        ui.label(
            egui::RichText::new(
                "Off gives full resolution near centre. Leave on if a stick does \
                 not return to the same place twice.",
            )
            .small()
            .weak(),
        );
        ui.add_space(6.0);

        for (left, name) in [(true, "Left stick"), (false, "Right stick")] {
            ui.horizontal(|ui| {
                ui.label(format!("{name} sensitivity:"));
                for (lvl, pct) in gamepad::SENS {
                    if ui.selectable_label(rec.sens(left) == lvl, format!("{pct}")).clicked() {
                        rec.set_sens(left, lvl);
                        changed = true;
                    }
                }
            });
        }
        ui.add_space(6.0);
        if let Some(v) = Self::level_picker(ui, "Rumble", rec.rumble(), &gamepad::LEVELS) {
            rec.set_rumble(v);
            changed = true;
        }
        if let Some(v) = Self::level_picker(ui, "Trigger L2", rec.trigger(true), &gamepad::LEVELS) {
            rec.set_trigger(true, v);
            changed = true;
        }
        if let Some(v) = Self::level_picker(ui, "Trigger R2", rec.trigger(false), &gamepad::LEVELS) {
            rec.set_trigger(false, v);
            changed = true;
        }
        if let Some(v) = Self::level_picker(ui, "Gyro L1", rec.gyro(true), &gamepad::LEVELS) {
            rec.set_gyro(true, v);
            changed = true;
        }
        if let Some(v) = Self::level_picker(ui, "Gyro L2", rec.gyro(false), &gamepad::LEVELS) {
            rec.set_gyro(false, v);
            changed = true;
        }
        ui.add_space(6.0);
        ui.label("Turbo:");
        for (i, name) in ["A", "B", "X", "Y", "R1", "R2"].iter().enumerate() {
            if let Some(v) = Self::level_picker(ui, name, rec.turbo(i), &gamepad::TURBO) {
                rec.set_turbo(i, v);
                changed = true;
            }
        }
        ui.add_space(6.0);
        let mut swap = rec.swap_abxy();
        if ui.checkbox(&mut swap, "Swap ABXY").changed() {
            rec.set_swap_abxy(swap);
            changed = true;
        }

        ui.add_space(10.0);
        if ui.button("Restore factory defaults").clicked() {
            rec = gamepad::Record::default();
            changed = true;
        }

        if changed {
            self.settings.set_record(&rec);
            self.dirty_pad = Some(Instant::now());
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new(format!("record  {}", state::hex(&rec.0))).small().weak());
    }

    fn colour_row(
        ui: &mut egui::Ui,
        colour: &mut u32,
        presets: &[u32],
    ) -> bool {
        let mut changed = false;
        let mut rgb = [
            ((*colour >> 16) & 0xFF) as u8,
            ((*colour >> 8) & 0xFF) as u8,
            (*colour & 0xFF) as u8,
        ];
        ui.horizontal(|ui| {
            if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
                *colour = ((rgb[0] as u32) << 16) | ((rgb[1] as u32) << 8) | rgb[2] as u32;
                changed = true;
            }
            for p in presets {
                let c = egui::Color32::from_rgb(
                    ((p >> 16) & 0xFF) as u8,
                    ((p >> 8) & 0xFF) as u8,
                    (p & 0xFF) as u8,
                );
                if ui.add(egui::Button::new("    ").fill(c)).clicked() {
                    *colour = *p;
                    changed = true;
                }
            }
            ui.label(format!("#{:06X}", *colour));
        });
        changed
    }

    fn lighting_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Keyboard");
        if self.devices.kbd.is_none() {
            Self::unavailable(ui, "Keyboard backlight", &self.devices.kbd_err);
        } else {
            let mut k = self.settings.kbdlight;
            let mut ch = false;
            if ui.checkbox(&mut k.enable, "Backlight on").changed() {
                ch = true;
            }
            if Self::colour_row(ui, &mut k.color, &kbdlight::PRESETS) {
                ch = true;
            }
            ui.horizontal(|ui| {
                ui.label("Effect:");
                for (v, name) in kbdlight::MODES {
                    if ui.selectable_label(k.mode == v, name).clicked() {
                        k.mode = v;
                        ch = true;
                    }
                }
            });
            if ui
                .add(egui::Slider::new(&mut k.brightness, 0..=100).text("Brightness %"))
                .changed()
            {
                ch = true;
            }
            ui.label(
                egui::RichText::new(
                    "Brightness is applied by scaling RGB: the firmware has no \
                     brightness field, and AYASpace's is dead code.",
                )
                .small()
                .weak(),
            );
            if ui.checkbox(&mut k.fn_ison, "Fn indicator light").changed() {
                ch = true;
            }
            if ch {
                self.settings.kbdlight = k;
                self.dirty_kbd = Some(Instant::now());
            }
        }

        ui.add_space(14.0);
        ui.heading("Joystick rings");
        if self.devices.rings.is_none() {
            Self::unavailable(ui, "Ring LEDs", &self.devices.rings_err);
        } else {
            let mut r = self.settings.rings;
            let mut ch = false;
            if Self::colour_row(ui, &mut r.color, &rings::PRESETS) {
                ch = true;
            }
            if ui
                .add(egui::Slider::new(&mut r.brightness, 0..=255).text("Brightness"))
                .changed()
            {
                ch = true;
            }
            ui.label(
                egui::RichText::new("Brightness 0 is off. Ring state is restored at login.")
                    .small()
                    .weak(),
            );
            if ch {
                self.settings.rings = r;
                self.dirty_rings = Some(Instant::now());
            }
        }
    }

    fn power_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Power profile");
        if self.profiles.is_empty() {
            ui.label("No ACPI platform_profile on this machine.");
        } else {
            // Writable directly on some distros; otherwise the helper does it.
            let direct = power::writable_directly();
            let can = direct || self.helper_up;
            ui.horizontal(|ui| {
                for p in self.profiles.clone() {
                    let sel = self.profile.as_deref() == Some(p.as_str());
                    if ui.add_enabled(can, egui::SelectableLabel::new(sel, &p)).clicked() {
                        let r = if direct {
                            std::fs::write(power::PATH, &p).map_err(|e| e.to_string())
                        } else {
                            helper::request(&format!("profile {p}")).map(|_| ()).map_err(|e| e.to_string())
                        };
                        match r {
                            Ok(()) => {
                                self.profile = Some(p.clone());
                                self.settings.power_profile = Some(p.clone());
                                let _ = state::save(&self.settings);
                                self.status = format!("profile: {p}");
                            }
                            Err(e) => self.status = format!("profile: {e}"),
                        }
                    }
                }
            });
        }

        ui.add_space(10.0);
        ui.heading("Sustained power (TDP)");
        if !self.helper_up {
            ui.colored_label(
                egui::Color32::from_rgb(220, 140, 60),
                "Helper not running — TDP needs SMU access.",
            );
            ui.label(
                egui::RichText::new(
                    "sudo systemctl enable --now ayaneo-tray-helper   (and install ryzenadj)",
                )
                .small()
                .weak(),
            );
            if ui.button("Re-check").clicked() {
                self.helper_up = helper::available();
            }
        } else {
            let cur = self.settings.tdp_watts;
            ui.horizontal(|ui| {
                for (w, label) in power::TDP_PRESETS {
                    if ui.selectable_label(cur == Some(w), label).clicked() {
                        // one value drives all three limits: the simple knob
                        // people actually want, rather than three sliders
                        match helper::request(&format!("tdp {w} {w} {w}")) {
                            Ok(_) => {
                                self.settings.tdp_watts = Some(w);
                                let _ = state::save(&self.settings);
                                self.status = format!("TDP {w} W");
                            }
                            Err(e) => self.status = format!("TDP: {e}"),
                        }
                    }
                }
            });
            ui.label(
                egui::RichText::new(
                    "Sets STAPM and the fast/slow limits together via ryzenadj. \
                     SMU limits are volatile, so they are re-applied at login. \
                     The selection shows what was last set, not what the SMU \
                     reports: reading limits back needs the ryzen_smu kernel \
                     module, without which /dev/mem access is refused.",
                )
                .small()
                .weak(),
            );
        }

        ui.add_space(10.0);
        ui.heading("Sensors");
        ui.horizontal(|ui| {
            for (n, v) in &self.telemetry.temps {
                ui.label(format!("{n} {v:.0}°C"));
                ui.separator();
            }
        });
        if let Some(p) = self.telemetry.battery_pct {
            let st = self.telemetry.battery_status.clone().unwrap_or_default();
            let w = self
                .telemetry
                .power_now_w
                .map(|w| format!("  {w:.1} W"))
                .unwrap_or_default();
            ui.label(format!("Battery {p}%  {st}{w}"));
        }

        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(
                "Fan control is absent on purpose: this machine exposes no kernel fan \
                 interface at all, so controlling it means writing EC registers that \
                 have not been identified yet. Guessing at those is a thermal risk, so \
                 it waits for the same evidence the rest of this tool was built on.",
            )
            .small()
            .weak(),
        );
    }

    fn about_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("ayaneo-tray");
        ui.label(format!("version {}", env!("CARGO_PKG_VERSION")));
        ui.add_space(8.0);
        ui.label("Replaces the parts of AYASpace that matter, with no driver and no daemon.");
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Transports").strong());
        ui.label("Gamepad — GuLiKit MCU over an on-board 16550 UART at I/O 0x3E8, 115200 8N1");
        ui.label("Keyboard backlight — HID feature report 0x41");
        ui.label("Ring LEDs — sysfs multicolor LED, via ayaneo-platform");
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Devices in use").strong());
        for (n, p) in [
            ("gamepad", &self.devices.gamepad),
            ("keyboard", &self.devices.kbd),
            ("rings", &self.devices.rings),
        ] {
            ui.label(format!(
                "{n}: {}",
                p.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "—".into())
            ));
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new(format!("settings: {}", state::path().display())).small());
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
        // Closing the window hides to tray instead of exiting.
        if ctx.input(|i| i.viewport().close_requested()) && self.visible {
            self.visible = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        if self.last_telemetry.elapsed() > Duration::from_secs(2) {
            self.telemetry = telemetry::read();
            self.profile = power::current();
            self.last_telemetry = Instant::now();
        }
        self.flush();

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (t, name) in [
                    (Tab::Controller, "Controller"),
                    (Tab::Lighting, "Lighting"),
                    (Tab::Power, "Power"),
                    (Tab::About, "About"),
                ] {
                    if ui.selectable_label(self.tab == t, name).clicked() {
                        self.tab = t;
                    }
                }
            });
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.label(egui::RichText::new(&self.status).small());
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                Tab::Controller => self.controller_tab(ui),
                Tab::Lighting => self.lighting_tab(ui),
                Tab::Power => self.power_tab(ui),
                Tab::About => self.about_tab(ui),
            });
        });

        // keep debounced writes and telemetry ticking without busy-spinning
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}
