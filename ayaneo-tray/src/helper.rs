//! The privileged half.
//!
//! Everything the GUI does itself is reachable through udev-granted device
//! access. Two things are not: SMU power limits, which ryzenadj reaches over
//! PCI config space, and /sys/firmware/acpi/platform_profile, which is
//! root-owned. Rather than run the whole GUI as root or wrap every click in
//! pkexec, this runs as a tiny system service and accepts a handful of
//! commands on a Unix socket.
//!
//! The socket is group-owned and mode 0660, so membership of that group is
//! equivalent to being able to set power limits. That is the whole security
//! boundary, and it is deliberately small: the command set below is the entire
//! surface, values are range-checked here rather than trusted from the client,
//! and nothing takes a path or a shell string.

use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

pub const SOCKET: &str = "/run/ayaneo-tray/helper.sock";
/// Sanity bounds for a 15-54W class handheld APU. A typo should not be able to
/// ask the SMU for 400 W.
const TDP_MIN_W: u32 = 4;
const TDP_MAX_W: u32 = 54;

fn set_tdp(stapm: u32, fast: u32, slow: u32) -> Result<()> {
    for v in [stapm, fast, slow] {
        if !(TDP_MIN_W..=TDP_MAX_W).contains(&v) {
            bail!("TDP {v}W out of range {TDP_MIN_W}-{TDP_MAX_W}");
        }
    }
    // ryzenadj takes milliwatts
    let out = std::process::Command::new("ryzenadj")
        .arg(format!("--stapm-limit={}", stapm * 1000))
        .arg(format!("--fast-limit={}", fast * 1000))
        .arg(format!("--slow-limit={}", slow * 1000))
        .output()
        .context("running ryzenadj (is it installed?)")?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        bail!("ryzenadj failed: {}", text.trim());
    }
    // Exit status alone is not enough: ryzenadj reports a per-limit failure on
    // stdout and can still exit 0, so require its own confirmation for each of
    // the three limits rather than assume success.
    for want in ["stapm_limit", "fast_limit", "slow_limit"] {
        if !text.lines().any(|l| l.contains("Successfully set") && l.contains(want)) {
            bail!("ryzenadj did not confirm {want}: {}", text.trim());
        }
    }
    Ok(())
}

fn set_profile(name: &str) -> Result<()> {
    // Only ever one of the values the kernel itself advertises.
    let choices = std::fs::read_to_string(crate::power::CHOICES).unwrap_or_default();
    if !choices.split_whitespace().any(|c| c == name) {
        bail!("{name:?} is not one of: {}", choices.trim());
    }
    std::fs::write(crate::power::PATH, name).context("writing platform_profile")
}

fn handle(stream: UnixStream) {
    let peer = stream.try_clone();
    let reader = BufReader::new(stream);
    let mut out = match peer {
        Ok(s) => s,
        Err(_) => return,
    };
    for line in reader.lines().map_while(Result::ok) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let reply = match parts.as_slice() {
            ["ping"] => Ok("pong".to_string()),
            ["tdp", a, b, c] => match (a.parse(), b.parse(), c.parse()) {
                (Ok(x), Ok(y), Ok(z)) => set_tdp(x, y, z).map(|_| "ok".into()),
                _ => Err(anyhow::anyhow!("tdp needs three integers, in watts")),
            },
            ["profile", name] => set_profile(name).map(|_| "ok".into()),
            [] => continue,
            _ => Err(anyhow::anyhow!("unknown command")),
        };
        let line = match reply {
            Ok(s) => format!("ok {s}\n"),
            Err(e) => format!("err {e}\n"),
        };
        if out.write_all(line.as_bytes()).is_err() {
            return;
        }
    }
}

pub fn run() -> Result<()> {
    let path = Path::new(SOCKET);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path).with_context(|| format!("bind {SOCKET}"))?;
    // 0660: owner root, group as set by the unit's Group=. Anyone in that
    // group can set power limits, which is the intended boundary.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
    eprintln!("ayaneo-tray helper listening on {SOCKET}");
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                std::thread::spawn(move || handle(s));
            }
            Err(e) => eprintln!("accept: {e}"),
        }
    }
    Ok(())
}

/// Client side, used by the GUI. Absent helper is not an error: the UI just
/// shows the privileged controls as unavailable.
pub fn request(cmd: &str) -> Result<String> {
    let mut s = UnixStream::connect(SOCKET)
        .with_context(|| format!("{SOCKET} not available (is ayaneo-tray-helper running?)"))?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    s.write_all(format!("{cmd}\n").as_bytes())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line)?;
    let line = line.trim().to_string();
    match line.strip_prefix("ok ") {
        Some(rest) => Ok(rest.to_string()),
        None => bail!("{}", line.strip_prefix("err ").unwrap_or(&line).to_string()),
    }
}

pub fn available() -> bool {
    request("ping").is_ok()
}
