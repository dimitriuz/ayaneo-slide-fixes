//! Keyboard backlight: HID feature report 0x41 on the keyboard MCU.
//!
//! See docs/AYASPACE-FEATURES.md. Eight bytes; the device implements
//! SET_REPORT but STALLs GET_REPORT, so state is cached locally. AYASpace's
//! `brightness` field is a no-op in the firmware path (its setter is a stub),
//! so brightness here scales R/G/B before sending.

use anyhow::{bail, Context, Result};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

const REPORT_ID: u8 = 0x41;
const TERM: u8 = 0x5A;
const LEN: usize = 8;

/// Keyboard effect modes, from KeyboardLight.mode[n] in en_US.json. Not to be
/// confused with the stick-ring list, which is longer and differently ordered.
pub const MODES: [(u8, &str); 3] = [(1, "Monochrome"), (2, "Gradient"), (3, "Breathe")];
/// KeyboardLightCList, the non-FLIP_KB branch.
pub const PRESETS: [u32; 6] = [0x002FFF, 0x0FE6FB, 0x27F95B, 0x0800FF, 0xFFEA00, 0xFF0000];

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct KbdLight {
    pub color: u32,
    pub mode: u8,
    pub enable: bool,
    pub fn_ison: bool,
    /// 0-100, applied locally by scaling RGB; the hardware has no such field.
    pub brightness: u8,
}

impl Default for KbdLight {
    fn default() -> Self {
        Self { color: 0x002FFF, mode: 1, enable: true, fn_ison: false, brightness: 100 }
    }
}

impl KbdLight {
    pub fn report(&self) -> [u8; LEN] {
        let s = self.brightness.min(100) as u32;
        let ch = |shift: u32| ((((self.color >> shift) & 0xFF) * s) / 100) as u8;
        [
            REPORT_ID,
            ch(16),
            ch(8),
            ch(0),
            self.mode,
            self.enable as u8,
            0x40 | self.fn_ison as u8,
            TERM,
        ]
    }
}

/// The interface declaring feature report 0x41. Matching on VID/PID alone is
/// wrong: the same keyboard exposes a second interface with no feature reports.
pub fn find_device() -> Option<PathBuf> {
    let rd = std::fs::read_dir("/sys/class/hidraw").ok()?;
    let mut nodes: Vec<_> = rd.flatten().map(|e| e.file_name()).collect();
    nodes.sort();
    for n in nodes {
        let name = n.to_string_lossy().to_string();
        let desc = std::fs::read(format!("/sys/class/hidraw/{name}/device/report_descriptor")).ok();
        if let Some(d) = desc {
            // 0x85 is the Report ID item tag, 0x41 its value
            if d.windows(2).any(|w| w == [0x85, REPORT_ID]) {
                return Some(PathBuf::from(format!("/dev/{name}")));
            }
        }
    }
    None
}

fn hidiocsfeature(len: usize) -> libc::c_ulong {
    (0xC000_0000u32 as libc::c_ulong)
        | ((len as libc::c_ulong) << 16)
        | ((b'H' as libc::c_ulong) << 8)
        | 0x06
}

pub fn apply(dev: &std::path::Path, st: &KbdLight) -> Result<()> {
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dev)
        .with_context(|| format!("open {}", dev.display()))?;
    let mut buf = st.report();
    let rc = unsafe { libc::ioctl(f.as_raw_fd(), hidiocsfeature(LEN), buf.as_mut_ptr()) };
    if rc < 0 {
        bail!("HIDIOCSFEATURE on {}: {}", dev.display(), std::io::Error::last_os_error());
    }
    Ok(())
}
