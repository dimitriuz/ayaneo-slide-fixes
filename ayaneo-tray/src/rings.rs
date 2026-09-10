//! Joystick ring LEDs. Already driven by ayaneo-platform as a standard
//! multicolor LED, so this is plain sysfs - no protocol work involved.
//! `brightness` scales `multi_intensity`, so brightness 0 is off.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const PRESETS: [u32; 5] = [0xFFFFFF, 0xFFD000, 0x0091FF, 0x08FF00, 0xFF0000];

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Rings {
    pub color: u32,
    pub brightness: u8,
}

impl Default for Rings {
    fn default() -> Self {
        Self { color: 0xFFFFFF, brightness: 128 }
    }
}

pub fn find_device() -> Option<PathBuf> {
    let rd = std::fs::read_dir("/sys/class/leds").ok()?;
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if n.contains("joystick_rings") && e.path().join("multi_intensity").exists() {
            return Some(e.path());
        }
    }
    None
}

pub fn apply(dir: &Path, st: &Rings) -> Result<()> {
    let (r, g, b) = ((st.color >> 16) & 0xFF, (st.color >> 8) & 0xFF, st.color & 0xFF);
    std::fs::write(dir.join("multi_intensity"), format!("{r} {g} {b}"))
        .with_context(|| format!("write multi_intensity in {}", dir.display()))?;
    std::fs::write(dir.join("brightness"), st.brightness.to_string())
        .with_context(|| format!("write brightness in {}", dir.display()))?;
    Ok(())
}

pub fn max_brightness(dir: &Path) -> u32 {
    std::fs::read_to_string(dir.join("max_brightness"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(255)
}
