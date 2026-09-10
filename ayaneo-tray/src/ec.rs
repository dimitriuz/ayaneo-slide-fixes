//! AYANEO embedded-controller RAM access over the ITE SuperIO index/data
//! ports, as used by AYASpace (`sdk/EcRw/EcControl.cpp`) and by the
//! ayaneo-platform kernel driver.
//!
//!     0x4E/0x4F   SuperIO index/data
//!     reg 0x11    address high byte (page)
//!     reg 0x10    address low byte (index)
//!     reg 0x12    data window
//!
//! Root only: this is raw port I/O through /dev/port. The sequence is stateful
//! across six port writes, so every access takes the mutex - two threads
//! interleaving here would read or write the wrong address.

use anyhow::{Context, Result};
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::sync::Mutex;
use std::sync::OnceLock;

struct Port(File);

fn port() -> Result<&'static Mutex<Port>> {
    static P: OnceLock<Mutex<Port>> = OnceLock::new();
    if let Some(p) = P.get() {
        return Ok(p);
    }
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/port")
        .context("open /dev/port (needs root and CAP_SYS_RAWIO)")?;
    let _ = P.set(Mutex::new(Port(f)));
    Ok(P.get().unwrap())
}

impl Port {
    fn outb(&self, addr: u64, val: u8) -> Result<()> {
        self.0.write_at(&[val], addr)?;
        Ok(())
    }
    fn inb(&self, addr: u64) -> Result<u8> {
        let mut b = [0u8; 1];
        self.0.read_at(&mut b, addr)?;
        Ok(b[0])
    }
    fn select(&self, hi: u8, lo: u8) -> Result<()> {
        self.outb(0x4E, 0x2E)?;
        self.outb(0x4F, 0x11)?;
        self.outb(0x4E, 0x2F)?;
        self.outb(0x4F, hi)?;
        self.outb(0x4E, 0x2E)?;
        self.outb(0x4F, 0x10)?;
        self.outb(0x4E, 0x2F)?;
        self.outb(0x4F, lo)?;
        self.outb(0x4E, 0x2E)?;
        self.outb(0x4F, 0x12)?;
        self.outb(0x4E, 0x2F)?;
        Ok(())
    }
}

pub fn read(hi: u8, lo: u8) -> Result<u8> {
    let p = port()?.lock().unwrap();
    p.select(hi, lo)?;
    p.inb(0x4F)
}

pub fn write(hi: u8, lo: u8, val: u8) -> Result<()> {
    let p = port()?.lock().unwrap();
    p.select(hi, lo)?;
    p.outb(0x4F, val)
}
