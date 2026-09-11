//! Persisted settings.
//!
//! Neither the gamepad MCU nor the keyboard MCU can be read back, so a local
//! copy is authoritative - the same design AYASpace uses (proto.gulikit and
//! KeyBoardLightConfig). Ring LED state lives in sysfs and does not survive a
//! reboot, so it is stored here too and re-applied at login.

use anyhow::Result;
use std::path::PathBuf;

use crate::{gamepad, kbdlight, rings};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// The 15-byte gamepad record, hex-encoded for legibility.
    pub gamepad_record: String,
    pub kbdlight: kbdlight::KbdLight,
    pub rings: rings::Rings,
    /// Applied on restore only when set, so we never fight another daemon.
    pub power_profile: Option<String>,
    /// Sustained power limit in watts, applied via the helper. None leaves the
    /// SMU alone entirely.
    pub tdp_watts: Option<u32>,
    /// UI scale. Handheld panels are small and high-DPI, and what the
    /// compositor reports is rarely what a thumb wants.
    #[serde(default = "default_scale")]
    pub ui_scale: f32,
    /// Fan curve points, (temperature C, speed %).
    #[serde(default = "crate::fan::default_curve")]
    pub fan_curve: Vec<(u8, u8)>,
    /// "auto", "manual" or "curve"; re-applied by --restore.
    #[serde(default)]
    pub fan_mode: Option<String>,
    #[serde(default = "default_fan_pct")]
    pub fan_pct: u8,
    /// InputPlumber pointer speed, re-applied after a profile load.
    #[serde(default)]
    pub ip_mouse_speed: Option<u32>,
}

fn default_fan_pct() -> u8 {
    45
}

fn default_scale() -> f32 {
    1.35
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gamepad_record: hex(&gamepad::FACTORY),
            kbdlight: Default::default(),
            rings: Default::default(),
            power_profile: None,
            tdp_watts: None,
            ui_scale: default_scale(),
            fan_curve: crate::fan::default_curve(),
            fan_mode: None,
            fan_pct: default_fan_pct(),
            ip_mouse_speed: None,
        }
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join("")
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

impl Settings {
    pub fn record(&self) -> gamepad::Record {
        unhex(&self.gamepad_record)
            .and_then(|b| gamepad::Record::parse(&b).ok())
            .unwrap_or_default()
    }
    pub fn set_record(&mut self, r: &gamepad::Record) {
        self.gamepad_record = hex(&r.0);
    }
}

pub fn path() -> PathBuf {
    // Per-user, since the GUI runs unprivileged. XDG config, not /var/lib.
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ayaneo-tray/settings.json")
}

/// Returns the settings and whether they describe the real hardware state.
///
/// `false` means we are guessing (first run, nothing to import), and callers
/// must not write the gamepad record on the strength of it.
pub fn load() -> (Settings, bool) {
    if let Some(s) = std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
    {
        return (s, true);
    }
    // Carry over from the CLI tools in this repo, so the two agree rather than
    // fighting: gulikit-ctl's record and ayaneo-kbdlight's cache.
    let mut st = Settings::default();
    let mut trusted = false;
    if let Ok(b) = std::fs::read("/var/lib/ayaneo/proto.gulikit") {
        if let Ok(r) = gamepad::Record::parse(&b) {
            st.set_record(&r);
            trusted = true;
        }
    }
    if let Some(k) = std::fs::read_to_string("/var/lib/ayaneo/kbdlight.json")
        .ok()
        .and_then(|s| serde_json::from_str::<kbdlight::KbdLight>(&s).ok())
    {
        st.kbdlight = k;
    }
    (st, trusted)
}

pub fn save(s: &Settings) -> Result<()> {
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(s)?)?;
    std::fs::rename(tmp, p)?;
    Ok(())
}
