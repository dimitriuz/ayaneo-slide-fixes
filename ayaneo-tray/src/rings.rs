//! Joystick ring LEDs. Already driven by ayaneo-platform as a standard
//! multicolor LED, so this is plain sysfs - no protocol work involved.
//! `brightness` scales `multi_intensity`, so brightness 0 is off.
//!
//! # Effects
//!
//! AYASpace's ring "modes" are not EC animations. Its LED thread switches on
//! the mode and calls a different animation function for each, computing
//! colours on the host and pushing them to the controller in a loop - the EC
//! only ever displays what it was last told. So effects are ours to implement,
//! and they are implemented the same way: a thread that writes sysfs on a
//! timer.
//!
//! What that interface can express is the limit. `multi_index` is
//! `red green blue` - one triple for both rings, with no per-quadrant
//! addressing - so whole-ring effects work and the positional ones (Radar,
//! Ripple) cannot be done this way at all. They would need the per-quadrant EC
//! registers directly, behind the driver's back.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const PRESETS: [u32; 5] = [0xFFFFFF, 0xFFD000, 0x0091FF, 0x08FF00, 0xFF0000];

/// How often the effect thread repaints. Fast enough to look continuous,
/// slow enough to stay invisible in `top`: a sysfs write measured ~17ms, so
/// this is a few percent of one core.
pub const FRAME_MS: u64 = 50;

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Effect {
    /// The chosen colour, held.
    Static,
    /// The chosen colour, fading in and out.
    Breathe,
    /// Hue sweep through the spectrum, ignoring the chosen colour.
    Rainbow,
}

impl Default for Effect {
    fn default() -> Self {
        Self::Static
    }
}

pub const EFFECTS: [(Effect, &str); 3] =
    [(Effect::Static, "Static"), (Effect::Breathe, "Breathe"), (Effect::Rainbow, "Rainbow")];

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Rings {
    pub color: u32,
    pub brightness: u8,
    #[serde(default)]
    pub effect: Effect,
}

impl Default for Rings {
    fn default() -> Self {
        Self { color: 0xFFFFFF, brightness: 128, effect: Effect::Static }
    }
}

/// Colour and brightness for one frame of an effect.
///
/// `phase` runs 0.0..1.0 over one cycle. Breathe uses a raised cosine rather
/// than a triangle so the turn at each end is smooth, and floors at a tenth so
/// the rings dim rather than blink out.
pub fn frame(st: &Rings, phase: f32) -> (u32, u8) {
    match st.effect {
        Effect::Static => (st.color, st.brightness),
        Effect::Breathe => {
            let k = 0.1 + 0.9 * (0.5 - 0.5 * (phase * std::f32::consts::TAU).cos());
            (st.color, (st.brightness as f32 * k) as u8)
        }
        Effect::Rainbow => (hue(phase), st.brightness),
    }
}

/// Fully saturated RGB for a hue in 0.0..1.0.
fn hue(h: f32) -> u32 {
    let h = (h.fract() + 1.0).fract() * 6.0;
    let x = ((1.0 - (h % 2.0 - 1.0).abs()) * 255.0) as u32;
    let (r, g, b) = match h as u32 {
        0 => (255, x, 0),
        1 => (x, 255, 0),
        2 => (0, 255, x),
        3 => (0, x, 255),
        4 => (x, 0, 255),
        _ => (255, 0, x),
    };
    (r << 16) | (g << 8) | b
}

/// Seconds for one full cycle of each effect.
pub fn period(effect: Effect) -> f32 {
    match effect {
        Effect::Static => 1.0,
        Effect::Breathe => 4.0,
        Effect::Rainbow => 12.0,
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

/// Drive the ring effect for as long as this process lives.
///
/// Runs in the tray rather than the window, because an effect that stops when
/// you close the settings window is not an effect. It re-reads the saved
/// settings once a second instead of being told about changes: the window is a
/// separate process, and a second IPC channel to carry one struct is not worth
/// it when the file is already the thing both sides agree on.
///
/// A static colour is written only when it changes. Without that this would
/// rewrite the same two sysfs files twenty times a second forever, for nothing.
pub fn run_effects() {
    std::thread::spawn(|| {
        let Some(dir) = find_device() else { return };
        let mut last_load = std::time::Instant::now();
        let (mut settings, _) = crate::state::load();
        let start = std::time::Instant::now();
        let mut last_static: Option<(u32, u8)> = None;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(FRAME_MS));
            if last_load.elapsed() >= std::time::Duration::from_secs(1) {
                last_load = std::time::Instant::now();
                settings = crate::state::load().0;
            }
            let st = settings.rings;
            if st.effect == Effect::Static {
                let want = (st.color, st.brightness);
                if last_static != Some(want) {
                    last_static = Some(want);
                    let _ = apply(&dir, &st);
                }
                continue;
            }
            last_static = None;
            let phase = (start.elapsed().as_secs_f32() / period(st.effect)).fract();
            let (color, brightness) = frame(&st, phase);
            let _ = apply(&dir, &Rings { color, brightness, effect: st.effect });
        }
    });
}
