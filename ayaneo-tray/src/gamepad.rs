//! GuLiKit gamepad MCU, reached over an on-board legacy 16550 UART.
//!
//! Protocol reverse-engineered from AYASpace 3.2.0.4 (CGuLiKitUtils). See
//! docs/GAMEPAD-PROTOCOL.md. 115200 8N1, an 11-byte frame carrying an absolute
//! 15-byte settings record, and a fixed 5-byte ACK. There is no read-back
//! command, so the record is cached locally exactly as AYASpace caches
//! proto.gulikit.

use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const RECORD_LEN: usize = 15;
const HDR: u8 = 0xE7;
const TERM: u8 = 0xED;
/// AYANEO factory record (0x140339960).
pub const FACTORY: [u8; RECORD_LEN] = [
    0xE7, 0x00, 0x00, 0x22, 0x02, 0x00, 0x00, 0x00, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x00, 0xED,
];

/// Stick sensitivity, as the AYASpace UI presents it.
pub const SENS: [(u8, u16); 3] = [(1, 50), (2, 100), (3, 150)];
/// Low / Medium / High, used for triggers, gyro and rumble.
pub const LEVELS: [(u8, &str); 4] = [(0, "off"), (1, "low"), (2, "medium"), (3, "high")];
/// Per-button turbo.
pub const TURBO: [(u8, &str); 3] = [(0, "off"), (1, "burst"), (2, "auto")];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Record(pub [u8; RECORD_LEN]);

impl Default for Record {
    fn default() -> Self {
        Record(FACTORY)
    }
}

fn nib(b: u8, high: bool) -> u8 {
    if high {
        (b >> 4) & 0xF
    } else {
        b & 0xF
    }
}

fn set_nib(b: u8, high: bool, v: u8) -> u8 {
    let v = v & 0xF;
    if high {
        (b & 0x0F) | (v << 4)
    } else {
        (b & 0xF0) | v
    }
}

impl Record {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != RECORD_LEN || bytes[0] != HDR || bytes[14] != TERM {
            bail!("not a valid {RECORD_LEN}-byte proto.gulikit record");
        }
        let mut r = [0u8; RECORD_LEN];
        r.copy_from_slice(bytes);
        Ok(Record(r))
    }

    /// The high nibble of byte 4 is an inverted flag: 0 means the deadzone is
    /// active, 1 means it is switched off.
    pub fn deadzone(&self) -> bool {
        self.0[4] & 0xF0 == 0
    }
    pub fn set_deadzone(&mut self, on: bool) {
        self.0[4] = (self.0[4] & 0x0F) | if on { 0x00 } else { 0x10 };
    }

    /// Byte 3: left stick in the high nibble, right in the low.
    pub fn sens(&self, left: bool) -> u8 {
        nib(self.0[3], left)
    }
    pub fn set_sens(&mut self, left: bool, level: u8) {
        self.0[3] = set_nib(self.0[3], left, level);
    }

    pub fn rumble(&self) -> u8 {
        self.0[4] & 0xF
    }
    pub fn set_rumble(&mut self, v: u8) {
        self.0[4] = (self.0[4] & 0xF0) | (v & 0xF);
    }

    /// Byte 1: L2 high, R2 low.
    pub fn trigger(&self, l2: bool) -> u8 {
        nib(self.0[1], l2)
    }
    pub fn set_trigger(&mut self, l2: bool, v: u8) {
        self.0[1] = set_nib(self.0[1], l2, v);
    }

    /// Byte 2: GyroL1 high, GyroL2 low.
    pub fn gyro(&self, l1: bool) -> u8 {
        nib(self.0[2], l1)
    }
    pub fn set_gyro(&mut self, l1: bool, v: u8) {
        self.0[2] = set_nib(self.0[2], l1, v);
    }

    /// Bytes 5-7: A/B, X/Y, R1/R2, high nibble first.
    pub fn turbo(&self, idx: usize) -> u8 {
        let (byte, high) = (5 + idx / 2, idx % 2 == 0);
        nib(self.0[byte], high)
    }
    pub fn set_turbo(&mut self, idx: usize, v: u8) {
        let (byte, high) = (5 + idx / 2, idx % 2 == 0);
        self.0[byte] = set_nib(self.0[byte], high, v);
    }

    pub fn swap_abxy(&self) -> bool {
        self.0[8] & 0x10 != 0
    }
    pub fn set_swap_abxy(&mut self, on: bool) {
        self.0[8] = (self.0[8] & 0xEF) | if on { 0x10 } else { 0x00 };
    }

    /// 11-byte frame: header, 8 payload bytes, checksum, terminator.
    fn frame(&self) -> [u8; 11] {
        let p = &self.0[0..9];
        let sum: u32 = p[1..9].iter().map(|&b| b as u32).sum();
        let mut f = [0u8; 11];
        f[..9].copy_from_slice(p);
        f[9] = (sum & 0xFF) as u8;
        f[10] = TERM;
        f
    }
}

/// Configure a tty for 115200 8N1 raw, and return it.
fn open_port(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let fd = f.as_raw_fd();
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut t) != 0 {
            bail!("tcgetattr {}", path.display());
        }
        t.c_iflag = 0;
        t.c_oflag = 0;
        t.c_cflag = libc::CS8 | libc::CREAD | libc::CLOCAL;
        t.c_lflag = 0;
        t.c_cc[libc::VMIN] = 0;
        t.c_cc[libc::VTIME] = 0;
        libc::cfsetispeed(&mut t, libc::B115200);
        libc::cfsetospeed(&mut t, libc::B115200);
        if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
            bail!("tcsetattr {}", path.display());
        }
        libc::tcflush(fd, libc::TCIOFLUSH);
    }
    Ok(f)
}

fn transact_once(path: &Path, rec: &Record, timeout: Duration) -> Result<[u8; 5]> {
    let mut f = open_port(path)?;
    let frame = rec.frame();
    f.write_all(&frame)?;
    f.flush()?;
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 5];
    let mut got = 0;
    while got < 5 && Instant::now() < deadline {
        match f.read(&mut buf[got..]) {
            Ok(0) => std::thread::sleep(Duration::from_millis(2)),
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2))
            }
            Err(e) => return Err(e.into()),
        }
    }
    if got != 5 || buf[0] != HDR || buf[4] != TERM {
        bail!("no valid ACK (got {got} bytes)");
    }
    Ok(buf)
}

/// The MCU drops roughly one reply in seven; the frame is absolute state, so
/// re-sending it is idempotent. AYASpace retries five times for the same reason.
pub fn send(path: &Path, rec: &Record) -> Result<[u8; 5]> {
    let mut last = None;
    for _ in 0..5 {
        match transact_once(path, rec, Duration::from_millis(300)) {
            Ok(ack) => return Ok(ack),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no reply")))
}

/// I/O port of a ttyS device, or None. 0x3E8 is COM3, the gamepad MCU.
fn uart_io_base(name: &str) -> Option<u32> {
    let io_type = std::fs::read_to_string(format!("/sys/class/tty/{name}/io_type")).ok()?;
    if io_type.trim() != "0" {
        return None; // not UPIO_PORT
    }
    let port = std::fs::read_to_string(format!("/sys/class/tty/{name}/port")).ok()?;
    let v = u32::from_str_radix(port.trim().trim_start_matches("0x"), 16).ok()?;
    (v != 0).then_some(v)
}

/// Candidate serial ports, COM3 (0x3E8) first: AYASpace's hint for AS01/SLIDE.
pub fn candidate_ports() -> Vec<PathBuf> {
    let mut v: Vec<(u8, PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/dev") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("ttyS") {
                continue;
            }
            if let Some(base) = uart_io_base(&name) {
                v.push((if base == 0x3E8 { 0 } else { 1 }, e.path()));
            }
        }
    }
    v.sort();
    v.into_iter().map(|(_, p)| p).collect()
}

/// Identify the MCU's port **without writing anything**.
///
/// The only way to probe this protocol is to send a frame, and a frame is an
/// absolute settings record - so probing with a record that does not match the
/// hardware silently changes settings. On this family the MCU is the UART at
/// I/O 0x3E8 (COM3, AYASpace's own hint for AS01), so identify it by address
/// and let the first real write confirm it.
pub fn find_port_readonly() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("GULIKIT_PORT") {
        return Some(PathBuf::from(p));
    }
    candidate_ports().into_iter().next()
}

/// Confirm by sending `rec`. Only safe when `rec` is known to match the
/// hardware's current state, i.e. it came from the saved settings.
///
/// Retries per port for the same reason `send` does: the MCU drops roughly one
/// reply in seven. A single-shot probe therefore reports a perfectly healthy
/// controller as missing about 15% of the time.
pub fn confirm_port(rec: &Record) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("GULIKIT_PORT") {
        return Some(PathBuf::from(p));
    }
    candidate_ports().into_iter().find(|p| {
        (0..5).any(|_| transact_once(p, rec, Duration::from_millis(300)).is_ok())
    })
}

/// Whether a port can be opened at all, as opposed to answering.
pub fn port_openable(p: &Path) -> bool {
    open_port(p).is_ok()
}
