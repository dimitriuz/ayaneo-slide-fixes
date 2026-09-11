//! Fan control.
//!
//! Registers reverse-engineered from AYASpace's `CEcControl::FanSetManual` /
//! `FanSetAuto`, and confirmed on hardware:
//!
//!     EC[0xD1,0xC8]   mode   0x00 = EC automatic curve, 0xA5 = manual
//!     EC[0x18,0x04]   duty   0-255, duty = percent / 100 * 255
//!
//! The duty register was verified read-only first: under load it climbs
//! monotonically with CPU temperature, saturates at 0xFF, and returns to an
//! identical 0x4D (30%) idle floor. The mode register's page comes from the
//! class constructor, which initialises the address word to 0xD100.
//!
//! Model gating matters. This register pair is what AYASpace uses for the
//! AB05/AS01 family only; an AB10 uses 0x1809 and 0x2F1, and other models an
//! 0xFE8004xx block entirely. On anything unrecognised this refuses rather
//! than writing a plausible-looking address.
//!
//! # Why the guards exist
//!
//! Every other setting in this program is inert if it is wrong. A fan left at
//! a low duty under load is not: the CPU will throttle at Tjmax rather than
//! come to harm, but it is a real thermal and comfort failure, and it can be
//! caused by something as ordinary as the process dying while manual mode is
//! engaged. So manual mode is never left unsupervised:
//!
//!   * a monitor thread forces automatic control above FORCE_AUTO_C, and
//!     latches so it cannot flap back into manual on its own;
//!   * the helper restores automatic control when it exits, and its unit
//!     repeats that in ExecStopPost so a SIGKILL is still covered;
//!   * duties below MIN_MANUAL_PCT are refused unless explicitly forced.

use anyhow::{bail, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::ec;

const MODE: (u8, u8) = (0xD1, 0xC8);
const DUTY: (u8, u8) = (0x18, 0x04);
const MODE_AUTO: u8 = 0x00;
const MODE_MANUAL: u8 = 0xA5;

/// Above this the EC curve takes over regardless of what was asked for.
pub const FORCE_AUTO_C: f32 = 85.0;
/// Below this a manual duty is refused; this is the regime where a mistake
/// actually bites.
pub const MIN_MANUAL_PCT: u8 = 20;

/// Above this a curve is overridden to full speed rather than trusted.
const CURVE_PANIC_C: f32 = 80.0;
/// If it still climbs this far the fan is not coping; hand back to the EC.
const GIVE_UP_C: f32 = 90.0;

/// How the fan is being driven.
#[derive(Clone, PartialEq)]
pub enum Mode {
    /// The EC's own curve. Nothing of ours is running.
    Auto,
    /// A fixed duty.
    Manual(u8),
    /// (temperature °C, speed %) points, sorted by temperature, applied by the
    /// monitor thread. The EC has no notion of a curve - AYASpace implements
    /// its own the same way, in CFanAuto::Update.
    Curve(Vec<(u8, u8)>),
}

fn mode() -> &'static Mutex<Mode> {
    static M: OnceLock<Mutex<Mode>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(Mode::Auto))
}

static TRIPPED: AtomicBool = AtomicBool::new(false);

/// A sane starting curve for a 7840U handheld: quiet until it matters.
pub fn default_curve() -> Vec<(u8, u8)> {
    vec![(45, 20), (55, 30), (65, 45), (75, 70), (85, 100)]
}

/// Linear interpolation between points; flat outside the ends.
pub fn curve_speed(points: &[(u8, u8)], temp: f32) -> u8 {
    if points.is_empty() {
        return 50;
    }
    if temp <= points[0].0 as f32 {
        return points[0].1;
    }
    if temp >= points[points.len() - 1].0 as f32 {
        return points[points.len() - 1].1;
    }
    for w in points.windows(2) {
        let (t0, s0) = (w[0].0 as f32, w[0].1 as f32);
        let (t1, s1) = (w[1].0 as f32, w[1].1 as f32);
        if temp >= t0 && temp <= t1 {
            let f = if (t1 - t0).abs() < f32::EPSILON { 0.0 } else { (temp - t0) / (t1 - t0) };
            return (s0 + (s1 - s0) * f).round() as u8;
        }
    }
    points[points.len() - 1].1
}

fn dmi(field: &str) -> String {
    std::fs::read_to_string(format!("/sys/class/dmi/id/{field}"))
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The models whose fan lives at the register pair above. Anything else is
/// refused: the addresses are per-family and a wrong guess writes into an
/// unrelated register.
///
/// Match on `board_name`, not `product_name`. AYASpace's model codes are board
/// codes: on this machine product_name is "SLIDE" while board_name is "AS01",
/// and "AS01" is what its own ProductClass checks against.
pub fn model_supported() -> bool {
    let board = dmi("board_name");
    dmi("sys_vendor") == "AYANEO" && (board == "AS01" || board.starts_with("AB05"))
}

fn model_or_bail() -> Result<()> {
    if !model_supported() {
        bail!(
            "fan control is verified for AYANEO AS01 and AB05 boards only \
             (this is vendor {:?}, board {:?}, product {:?}); other models use \
             different EC registers and are not guessed at",
            dmi("sys_vendor"),
            dmi("board_name"),
            dmi("product_name")
        );
    }
    Ok(())
}

pub fn cpu_temp_c() -> Option<f32> {
    let rd = std::fs::read_dir("/sys/class/hwmon").ok()?;
    for e in rd.flatten() {
        let name = std::fs::read_to_string(e.path().join("name")).unwrap_or_default();
        if name.trim() == "k10temp" {
            let v = std::fs::read_to_string(e.path().join("temp1_input")).ok()?;
            return v.trim().parse::<f32>().ok().map(|m| m / 1000.0);
        }
    }
    None
}

/// Hand control back to the EC. This is also the documented way to clear a
/// latched thermal trip, so it must actually clear it - the error returned by
/// set_manual() tells the user to come here.
pub fn set_auto() -> Result<()> {
    model_or_bail()?;
    ec::write(MODE.0, MODE.1, MODE_AUTO)?;
    *mode().lock().unwrap() = Mode::Auto;
    TRIPPED.store(false, Ordering::SeqCst);
    Ok(())
}

/// Write a duty without touching the stored mode. Used by the monitor thread.
fn write_duty(pct: u8) -> Result<()> {
    let duty = ((pct.min(100) as u32 * 255) / 100) as u8;
    ec::write(MODE.0, MODE.1, MODE_MANUAL)?;
    ec::write(DUTY.0, DUTY.1, duty)
}

pub fn set_curve(points: Vec<(u8, u8)>) -> Result<()> {
    model_or_bail()?;
    if points.len() < 2 {
        bail!("a curve needs at least two points");
    }
    if points.len() > 8 {
        bail!("at most 8 points");
    }
    for (t, s) in &points {
        if *s < MIN_MANUAL_PCT || *s > 100 {
            bail!("speed {s}% out of range {MIN_MANUAL_PCT}-100");
        }
        if *t > 105 {
            bail!("temperature {t}C out of range");
        }
    }
    let mut pts = points;
    pts.sort_by_key(|(t, _)| *t);
    if pts.windows(2).any(|w| w[0].0 == w[1].0) {
        bail!("two points share a temperature");
    }
    if TRIPPED.load(Ordering::SeqCst) {
        bail!("thermal failsafe has tripped; send 'fan auto' to clear it");
    }
    // Apply immediately so the change is felt, then let the monitor maintain it.
    if let Some(t) = cpu_temp_c() {
        write_duty(curve_speed(&pts, t))?;
    }
    *mode().lock().unwrap() = Mode::Curve(pts);
    Ok(())
}

pub fn set_manual(pct: u8, force_low: bool) -> Result<()> {
    model_or_bail()?;
    if pct > 100 {
        bail!("duty {pct}% out of range 0-100");
    }
    if pct < MIN_MANUAL_PCT && !force_low {
        bail!(
            "duty {pct}% is below the {MIN_MANUAL_PCT}% floor; pass 'force' to override"
        );
    }
    if TRIPPED.load(Ordering::SeqCst) {
        bail!(
            "thermal failsafe has tripped (CPU exceeded {FORCE_AUTO_C:.0}C); \
             send 'fan auto' to clear it"
        );
    }
    // mode first, then duty: the EC ignores duty writes while in auto
    write_duty(pct)?;
    *mode().lock().unwrap() = Mode::Manual(pct);
    Ok(())
}

pub struct Status {
    pub mode_raw: u8,
    pub duty_raw: u8,
    pub mode: Mode,
    pub tripped: bool,
    pub temp_c: Option<f32>,
    pub supported: bool,
}

pub fn status() -> Result<Status> {
    Ok(Status {
        mode_raw: ec::read(MODE.0, MODE.1)?,
        duty_raw: ec::read(DUTY.0, DUTY.1)?,
        mode: mode().lock().unwrap().clone(),
        tripped: TRIPPED.load(Ordering::SeqCst),
        temp_c: cpu_temp_c(),
        supported: model_supported(),
    })
}

/// Restore the EC's own control. Called on every helper entry and exit path.
///
/// Reports rather than swallowing errors: a failsafe that fails silently is
/// worse than none, because it looks like it worked.
pub fn restore_on_exit() {
    if !model_supported() {
        return;
    }
    match ec::write(MODE.0, MODE.1, MODE_AUTO) {
        Ok(()) => {}
        Err(e) => eprintln!("fan: FAILED to restore automatic control: {e}"),
    }
}

/// Force a known-good state at helper startup.
///
/// Teardown hooks are not sufficient on their own. `systemctl kill` takes the
/// whole cgroup including ExecStopPost, and a hard power-loss obviously runs
/// nothing at all - so manual mode could survive into a session with nothing
/// supervising it. Since the helper is the only thing that ever engages manual
/// mode, and it restarts on failure, resetting here makes "fan is manual" and
/// "a live supervisor exists" the same condition.
pub fn reset_at_start() {
    if !model_supported() {
        return;
    }
    match ec::read(MODE.0, MODE.1) {
        Ok(m) if m != MODE_AUTO => {
            eprintln!("fan: found mode 0x{m:02x} at startup, restoring automatic control");
            restore_on_exit();
        }
        Ok(_) => {}
        Err(e) => eprintln!("fan: could not read mode at startup: {e}"),
    }
}

/// Supervises manual mode. Started once by the helper.
pub fn start_monitor() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(2));
        let current = mode().lock().unwrap().clone();
        if current == Mode::Auto {
            continue;
        }
        let Some(t) = cpu_temp_c() else {
            // Lost the sensor: we can no longer supervise, so hand control back
            // rather than hold a duty blind.
            *mode().lock().unwrap() = Mode::Auto;
            let _ = ec::write(MODE.0, MODE.1, MODE_AUTO);
            eprintln!("fan: CPU temperature unreadable, restored EC automatic control");
            continue;
        };

        if t >= GIVE_UP_C {
            // Full speed did not hold it. Something is wrong; the EC's own
            // curve is a better bet than ours, and this latches.
            TRIPPED.store(true, Ordering::SeqCst);
            *mode().lock().unwrap() = Mode::Auto;
            let _ = ec::write(MODE.0, MODE.1, MODE_AUTO);
            eprintln!("fan: {t:.0}C >= {GIVE_UP_C:.0}C, restored EC automatic control");
            continue;
        }

        let target = match &current {
            Mode::Auto => continue,
            // A fixed duty is never allowed to stay low while hot.
            Mode::Manual(p) => {
                if t >= FORCE_AUTO_C {
                    TRIPPED.store(true, Ordering::SeqCst);
                    *mode().lock().unwrap() = Mode::Auto;
                    let _ = ec::write(MODE.0, MODE.1, MODE_AUTO);
                    eprintln!("fan: {t:.0}C >= {FORCE_AUTO_C:.0}C, restored EC automatic control");
                    continue;
                }
                *p
            }
            // A curve is expected to raise the fan itself, so override it to
            // full rather than tearing control away - handing back mid-game
            // because a curve was a few percent conservative would be worse
            // than simply running the fan flat out.
            Mode::Curve(points) => {
                let s = curve_speed(points, t);
                if t >= CURVE_PANIC_C {
                    s.max(100)
                } else {
                    s
                }
            }
        };
        // Re-assert every tick: cheap, and it covers anything else touching
        // the registers.
        let _ = write_duty(target);
    });
}
