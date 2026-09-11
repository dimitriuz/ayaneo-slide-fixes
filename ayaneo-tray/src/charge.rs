//! Charge control: bypass, and a charge limit built on top of it.
//!
//! # Bypass
//!
//! The ayaneo-platform module registers a power_supply extension called
//! `ayaneo-bypass-charge`, which adds `charge_behaviour` to the battery with
//! two settings: `auto` and `inhibit-charge`. Writing `inhibit-charge` makes
//! the EC run the machine from the adapter and leave the cell alone. The module
//! gates this on the model and on the EC version - AYANEO SLIDE with EC 0x1b or
//! later qualifies - so the file's presence is the feature test, not the model.
//!
//! It reaches the EC through a writer thread that wakes every 30 seconds, so a
//! write to sysfs takes up to half a minute to become true in hardware. That is
//! why `ec_bypass()` exists: sysfs reports what was asked for, and the EC
//! register reports what is actually in force. Measured on a SLIDE:
//!
//! ```text
//! just after write   ec d1d1=0x65 CLOSE   sysfs=[inhibit-charge]
//! +30s               ec d1d1=0x01 OPEN    sysfs=[inhibit-charge]
//! ```
//!
//! # Limit
//!
//! There is no charge threshold register: nothing in AYANEO's EC takes a "stop
//! at 80%" value, and writes to the registers that looked like candidates had
//! no effect. So the limit is supervised rather than delegated - hold the
//! charge behaviour at `inhibit-charge` above the target and release it below -
//! which is why it lives in the always-running helper rather than in the GUI.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// EC RAM address of the bypass control, in the 0xD1 page. Same register the
/// kernel module writes; read here only to report what is actually in force.
pub const EC_BYPASS: (u8, u8) = (0xD1, 0xD1);
pub const EC_BYPASS_OPEN: u8 = 0x01;
pub const EC_BYPASS_CLOSE: u8 = 0x65;

/// Offered limits. A limit below half a charge is worse for the cell than it is
/// good, and 100 is what "off" already means.
pub const LIMIT_PRESETS: [u32; 4] = [60, 70, 80, 90];

/// How far capacity must fall below the limit before charging is allowed again.
///
/// Without it the supervisor would toggle at every reported percent, and each
/// toggle costs up to 30 seconds of the module's writer cycle. Three points is
/// a few minutes of use on this battery.
pub const HYSTERESIS: u32 = 3;

fn battery() -> Option<PathBuf> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    dir.flatten()
        .map(|e| e.path())
        .find(|p| p.join("charge_behaviour").exists())
}

/// Where the behaviour is written, if this kernel exposes it at all.
pub fn path() -> Option<PathBuf> {
    battery().map(|b| b.join("charge_behaviour"))
}

fn read_trim(p: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// The selected behaviour, parsed out of the kernel's `a [b] c` list format.
pub fn behaviour() -> Option<String> {
    let raw = read_trim(path()?)?;
    raw.split_whitespace()
        .find(|w| w.starts_with('[') && w.ends_with(']'))
        .map(|w| w.trim_matches(['[', ']']).to_string())
}

/// Every behaviour this battery accepts, in the order the kernel lists them.
pub fn available() -> Vec<String> {
    let Some(raw) = path().and_then(read_trim) else { return Vec::new() };
    raw.split_whitespace().map(|w| w.trim_matches(['[', ']']).to_string()).collect()
}

pub fn set_behaviour(what: &str) -> Result<()> {
    let p = path().context("no charge_behaviour on this battery")?;
    if !available().iter().any(|b| b == what) {
        bail!("{what:?} is not one of: {}", available().join(", "));
    }
    std::fs::write(&p, what).with_context(|| format!("writing {}", p.display()))
}

pub fn capacity() -> Option<u32> {
    read_trim(battery()?.join("capacity"))?.parse().ok()
}

pub fn status() -> Option<String> {
    read_trim(battery()?.join("status"))
}

/// Charge in microwatt-hours. A thousand times finer than `capacity`, which is
/// what makes "is it still taking charge?" answerable in a couple of minutes
/// rather than a couple of percent.
pub fn energy_uwh() -> Option<u64> {
    read_trim(battery()?.join("energy_now"))?.parse().ok()
}

/// What the EC is actually doing, as opposed to what sysfs was told.
///
/// `None` when the register holds neither documented value, which would mean
/// this model does not use the register the way the driver assumes - better to
/// say nothing than to report a guess.
pub fn ec_bypass() -> Option<bool> {
    match crate::ec::read(EC_BYPASS.0, EC_BYPASS.1).ok()? {
        EC_BYPASS_OPEN => Some(true),
        EC_BYPASS_CLOSE => Some(false),
        _ => None,
    }
}

// ---------------------------------------------------------------- supervisor

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// Whether inhibiting charge actually does anything on this machine.
///
/// It does not on every AYANEO: on an AS01 (SLIDE) with EC 0x001b0100 the write
/// reaches EC `0xd1d1` - confirmed by reading the register back - and the
/// battery carries on charging at full rate through both documented values,
/// written while plugged in. AYASpace's own `system.set_charge_config` takes
/// that identical path for this board, so there is no better register to use;
/// the EC simply ignores it. Rather than show a limit that silently does
/// nothing, measure it and say so.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Honoured {
    /// Not yet observed under the conditions needed to tell.
    Unknown,
    /// Charge stopped while inhibit was in force.
    Yes,
    /// Charge continued while inhibit was in force.
    No,
}

static HONOURED: AtomicU32 = AtomicU32::new(0);

pub fn honoured() -> Honoured {
    match HONOURED.load(Ordering::SeqCst) {
        1 => Honoured::Yes,
        2 => Honoured::No,
        _ => Honoured::Unknown,
    }
}

/// Long enough that a slow charger still moves the needle, short enough to have
/// an answer before the battery has gained a percent.
const VERIFY_SECS: u64 = 120;
/// Charge gained over that window that counts as "still charging". The gauge
/// steps in ~485 mWh increments on this pack, so anything above one step is
/// real movement rather than quantisation.
const VERIFY_UWH: u64 = 300_000;

/// Watch whether an in-force inhibit actually stops the charge.
///
/// Only judges while the request has reached the EC register - sysfs alone lags
/// it by up to 30 seconds, and judging during that window would blame the
/// hardware for the driver's writer thread.
fn verify(state: &mut Option<(std::time::Instant, u64)>) {
    let inhibiting = behaviour().as_deref() == Some("inhibit-charge")
        && ec_bypass() == Some(true)
        && status().as_deref() == Some("Charging");
    if !inhibiting {
        *state = None;
        return;
    }
    let Some(now) = energy_uwh() else { return };
    match state {
        None => *state = Some((std::time::Instant::now(), now)),
        Some((since, start)) => {
            if since.elapsed().as_secs() < VERIFY_SECS {
                return;
            }
            let verdict = if now.saturating_sub(*start) > VERIFY_UWH { 2 } else { 1 };
            if HONOURED.swap(verdict, Ordering::SeqCst) != verdict {
                eprintln!(
                    "charge: inhibit-charge is {} by this EC (+{} mWh over {VERIFY_SECS}s)",
                    if verdict == 2 { "ignored" } else { "honoured" },
                    (now.saturating_sub(*start)) / 1000,
                );
            }
            *state = None;
        }
    }
}

/// 0 means no limit. Lives in an atomic rather than behind the helper's socket
/// state so the supervisor thread can read it without contending for a lock it
/// would hold across a sysfs write.
static LIMIT: AtomicU32 = AtomicU32::new(0);

/// Survives a helper restart and a reboot. Deliberately not in the GUI's
/// settings file: a charge limit that only applies once somebody logs in is not
/// a charge limit.
const STATE: &str = "/var/lib/ayaneo/charge-limit";

pub fn limit() -> u32 {
    LIMIT.load(Ordering::SeqCst)
}

/// `0` disables supervision and leaves the behaviour wherever it stands.
pub fn set_limit(pct: u32) -> Result<()> {
    if pct != 0 && !(20..=99).contains(&pct) {
        bail!("limit must be 0 (off) or 20-99");
    }
    LIMIT.store(pct, Ordering::SeqCst);
    if let Some(dir) = Path::new(STATE).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(STATE, pct.to_string()).with_context(|| format!("writing {STATE}"))?;
    // Don't make the user wait a whole supervision cycle to see it act.
    apply_once();
    Ok(())
}

/// Bring the behaviour into line with the limit. Returns what it decided, if
/// anything - `None` when there is no limit, or nothing needs changing.
fn apply_once() -> Option<&'static str> {
    let limit = limit();
    if limit == 0 {
        return None;
    }
    let cap = capacity()?;
    let want = if cap >= limit {
        "inhibit-charge"
    } else if cap + HYSTERESIS <= limit {
        "auto"
    } else {
        // Inside the hysteresis band: whichever way it is set is acceptable, so
        // leave it rather than fight the band's own edges.
        return None;
    };
    if behaviour().as_deref() == Some(want) {
        return None;
    }
    match set_behaviour(want) {
        Ok(()) => Some(want),
        Err(e) => {
            eprintln!("charge: {e}");
            None
        }
    }
}

/// Start supervising, and restore a limit saved by an earlier run.
///
/// Polls rather than waiting on a uevent: capacity changes are reported on a
/// timer anyway, and the module's own writer thread means a decision cannot
/// take effect faster than 30 seconds regardless.
pub fn start_monitor() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    if path().is_none() {
        return;
    }
    if let Some(saved) = read_trim(STATE).and_then(|s| s.parse::<u32>().ok()) {
        LIMIT.store(saved, Ordering::SeqCst);
        eprintln!("charge: limit {saved}% restored");
    }
    std::thread::spawn(|| {
        let mut window = None;
        loop {
            if let Some(set) = apply_once() {
                eprintln!("charge: {set} at {:?}%", capacity());
            }
            verify(&mut window);
            std::thread::sleep(Duration::from_secs(20));
        }
    });
}

// ------------------------------------------------------------- client parsing

/// Everything the Charge page shows, as the helper reports it.
#[derive(Default, Clone, PartialEq, Eq)]
pub struct Status {
    /// Empty when this kernel has no charge_behaviour to offer.
    pub behaviour: String,
    /// 0 when no limit is being supervised.
    pub limit: u32,
    pub capacity: Option<u32>,
    pub status: String,
    /// What the EC register says, which lags sysfs by up to 30 seconds.
    pub ec_bypass: Option<bool>,
    /// Whether an in-force inhibit was observed to actually stop the charge.
    pub honoured: Option<bool>,
}

impl Status {
    pub fn available(&self) -> bool {
        !self.behaviour.is_empty() && self.behaviour != "-"
    }
    pub fn inhibiting(&self) -> bool {
        self.behaviour == "inhibit-charge"
    }
}

/// Parse the helper's `charge status` line.
pub fn parse(line: &str) -> Status {
    let mut s = Status::default();
    for kv in line.split_whitespace() {
        let Some((k, v)) = kv.split_once('=') else { continue };
        match k {
            "behaviour" => s.behaviour = v.to_string(),
            "limit" => s.limit = v.parse().unwrap_or(0),
            "capacity" => s.capacity = v.parse().ok(),
            "status" => s.status = v.to_string(),
            "ec_bypass" => {
                s.ec_bypass = match v {
                    "on" => Some(true),
                    "off" => Some(false),
                    _ => None,
                }
            }
            "honoured" => {
                s.honoured = match v {
                    "yes" => Some(true),
                    "no" => Some(false),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    s
}
