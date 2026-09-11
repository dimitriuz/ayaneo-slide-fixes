//! Device discovery and apply, with human-readable reasons when something is
//! not reachable. Every failure here is a permissions problem or a missing
//! device, and the UI says which.

use std::path::PathBuf;

use crate::{gamepad, kbdlight, rings, state::Settings};

#[derive(Default, Clone)]
pub struct Devices {
    pub gamepad: Option<PathBuf>,
    pub gamepad_err: Option<String>,
    pub kbd: Option<PathBuf>,
    pub kbd_err: Option<String>,
    pub rings: Option<PathBuf>,
    pub rings_err: Option<String>,
}

fn writable(p: &std::path::Path) -> bool {
    std::fs::OpenOptions::new().read(true).write(true).open(p).is_ok()
}

impl Devices {
    /// `trusted` says whether `rec` is known to match the hardware - true only
    /// when it was loaded from saved settings. When false, the UART is
    /// identified by I/O address and nothing is written, because a probe frame
    /// would overwrite live settings with whatever `rec` happens to hold.
    pub fn probe(rec: &gamepad::Record, trusted: bool) -> Self {
        let mut d = Devices::default();

        let found =
            if trusted { gamepad::confirm_port(rec) } else { gamepad::find_port_readonly() };
        match found {
            Some(p) => d.gamepad = Some(p),
            None => {
                let cands = gamepad::candidate_ports();
                // Distinguish "cannot open" from "opened fine, nothing answered".
                // The old message blamed udev whenever *any* candidate was
                // unopenable - but the unpopulated 8250 slots never open, so it
                // always blamed udev, including when the controller was simply
                // absent.
                let openable: Vec<_> =
                    cands.iter().filter(|p| gamepad::port_openable(p)).collect();
                d.gamepad_err = Some(if cands.is_empty() {
                    "no legacy UART found (expected /dev/ttyS* at I/O 0x3E8)".into()
                } else if openable.is_empty() {
                    format!(
                        "cannot open {} - install 70-ayaneo-tray.rules, then \
                         `sudo udevadm control --reload && sudo udevadm trigger`",
                        cands
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                } else {
                    format!(
                        "{} opens but the controller does not answer. It is usually \
                         absent rather than misconfigured: check `lsusb` for 045e:028e, \
                         and if it is missing, shut down fully (not reboot) and power on \
                         again - a warm reboot does not reset the controller's power rail.",
                        openable
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                });
            }
        }

        match kbdlight::find_device() {
            Some(p) if writable(&p) => d.kbd = Some(p),
            Some(p) => {
                d.kbd_err = Some(format!("cannot open {} - needs the udev rule", p.display()))
            }
            None => d.kbd_err = Some("no HID device declares feature report 0x41".into()),
        }

        match rings::find_device() {
            Some(p) => {
                if std::fs::OpenOptions::new().write(true).open(p.join("brightness")).is_ok() {
                    d.rings = Some(p);
                } else {
                    d.rings_err =
                        Some(format!("{}/brightness is not writable", p.display()));
                }
            }
            None => {
                d.rings_err = Some("no joystick_rings LED (is ayaneo-platform loaded?)".into())
            }
        }
        d
    }

    /// Push everything in `s` to the hardware. Returns per-device outcomes so
    /// the caller can report partial success rather than a single bool.
    pub fn apply_all(&self, s: &Settings) -> Vec<(&'static str, Result<(), String>)> {
        let mut out = Vec::new();
        if let Some(p) = &self.gamepad {
            out.push((
                "gamepad",
                gamepad::send(p, &s.record()).map(|_| ()).map_err(|e| e.to_string()),
            ));
        }
        if let Some(p) = &self.kbd {
            out.push(("keyboard", kbdlight::apply(p, &s.kbdlight).map_err(|e| e.to_string())));
        }
        if let Some(p) = &self.rings {
            out.push(("rings", rings::apply(p, &s.rings).map_err(|e| e.to_string())));
        }
        out
    }
}
