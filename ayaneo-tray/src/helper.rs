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

/// Read the limits back out of the SMU.
///
/// Returns them in watts as "stapm fast slow". This is the only way to answer
/// "what is the TDP right now" rather than "what did this app last ask for" -
/// anything else on the system can have set it since, and several things do.
///
/// It needs ryzenadj's power metrics table, which lives in ordinary RAM and is
/// therefore unreachable through /dev/mem on a kernel built with
/// CONFIG_STRICT_DEVMEM (most of them). The ryzen_smu module is what makes it
/// readable; without it ryzenadj says so on stderr and exits 0 anyway, so the
/// absence of the values is what has to be detected, not the exit status.
fn tdp_info() -> Result<String> {
    let out = std::process::Command::new("ryzenadj")
        .arg("--info")
        .output()
        .context("running ryzenadj (is it installed?)")?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // Lines look like: "| STAPM LIMIT | 15.000 | stapm-limit |"
    let field = |name: &str| -> Option<f32> {
        text.lines()
            .find(|l| l.contains(name))
            .and_then(|l| l.split('|').nth(2))
            .and_then(|v| v.trim().parse::<f32>().ok())
    };
    match (field("STAPM LIMIT"), field("PPT LIMIT FAST"), field("PPT LIMIT SLOW")) {
        (Some(a), Some(b), Some(c)) => Ok(format!("{a:.0} {b:.0} {c:.0}")),
        _ => {
            let why = text
                .lines()
                .find(|l| l.contains("Unable to"))
                .unwrap_or("ryzenadj returned no limits")
                .trim();
            bail!("{why}")
        }
    }
}

/// One line the GUI can parse, and a person can read.
fn charge_status() -> String {
    let ec = match crate::charge::ec_bypass() {
        Some(true) => "on",
        Some(false) => "off",
        None => "?",
    };
    format!(
        "behaviour={} limit={} capacity={} status={} ec_bypass={ec}",
        crate::charge::behaviour().unwrap_or_else(|| "-".into()),
        crate::charge::limit(),
        crate::charge::capacity().map_or("-".into(), |c| c.to_string()),
        crate::charge::status().unwrap_or_else(|| "-".into()),
    )
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
            ["tdp", "info"] => tdp_info(),
            ["charge", "status"] => Ok(charge_status()),
            ["charge", "behaviour", what] => {
                crate::charge::set_behaviour(what).map(|_| charge_status())
            }
            ["charge", "limit", "off"] => {
                crate::charge::set_limit(0).map(|_| charge_status())
            }
            ["charge", "limit", pct] => match pct.parse::<u32>() {
                Ok(p) => crate::charge::set_limit(p).map(|_| charge_status()),
                Err(_) => Err(anyhow::anyhow!("charge limit needs a percentage or \"off\"")),
            },
            ["profile", name] => set_profile(name).map(|_| "ok".into()),
            ["fan", "auto"] => {
                // also clears a latched thermal trip
                crate::fan::set_auto().map(|_| "auto".into())
            }
            ["fan", "manual", pct] => match pct.parse::<u8>() {
                Ok(p) => crate::fan::set_manual(p, false).map(|_| format!("manual {p}%")),
                Err(_) => Err(anyhow::anyhow!("fan manual needs a percentage")),
            },
            ["fan", "manual", pct, "force"] => match pct.parse::<u8>() {
                Ok(p) => crate::fan::set_manual(p, true).map(|_| format!("manual {p}%")),
                Err(_) => Err(anyhow::anyhow!("fan manual needs a percentage")),
            },
            // "fan curve 45:20,55:30,..."
            ["fan", "curve", spec] => {
                let mut pts = Vec::new();
                let mut bad = false;
                for part in spec.split(',') {
                    match part.split_once(':') {
                        Some((t, s)) => match (t.parse::<u8>(), s.parse::<u8>()) {
                            (Ok(t), Ok(s)) => pts.push((t, s)),
                            _ => bad = true,
                        },
                        None => bad = true,
                    }
                }
                if bad {
                    Err(anyhow::anyhow!("curve points must be temp:speed, comma separated"))
                } else {
                    let n = pts.len();
                    crate::fan::set_curve(pts).map(|_| format!("curve, {n} points"))
                }
            }
            // Only this one unit, and only these verbs: the helper must not
            // become a general-purpose way to run systemctl as root.
            ["service", "inputplumber", op @ ("start" | "stop" | "restart")] => {
                match std::process::Command::new("systemctl")
                    .args([op, "inputplumber"])
                    .output()
                {
                    Ok(o) if o.status.success() => Ok(format!("inputplumber {op}")),
                    Ok(o) => Err(anyhow::anyhow!(
                        "systemctl {op} inputplumber: {}",
                        String::from_utf8_lossy(&o.stderr).trim().to_string()
                    )),
                    Err(e) => Err(anyhow::anyhow!("systemctl: {e}")),
                }
            }
            ["fan", "status"] => crate::fan::status().map(|st| {
                let (mode, target) = match &st.mode {
                    crate::fan::Mode::Auto => ("auto".to_string(), String::new()),
                    crate::fan::Mode::Manual(p) => ("manual".to_string(), format!(" target={p}")),
                    crate::fan::Mode::Curve(pts) => (
                        "curve".to_string(),
                        format!(
                            " curve={}",
                            pts.iter()
                                .map(|(t, s)| format!("{t}:{s}"))
                                .collect::<Vec<_>>()
                                .join(",")
                        ),
                    ),
                };
                format!(
                    "supported={} mode={} ec=0x{:02x} duty={} tripped={} temp={}{}",
                    st.supported,
                    mode,
                    st.mode_raw,
                    st.duty_raw,
                    st.tripped,
                    st.temp_c.map(|t| format!("{t:.1}")).unwrap_or_else(|| "?".into()),
                    target
                )
            }),
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

extern "C" fn handle_signal(_sig: libc::c_int) {
    crate::fan::restore_on_exit();
    std::process::exit(0);
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
    // Establish a known-good fan state before anything else: an earlier
    // instance may have died without running its teardown.
    crate::fan::reset_at_start();
    // Supervises manual fan mode; see fan.rs for why this is not optional.
    crate::fan::start_monitor();
    // Holds a saved charge limit, including before anyone has logged in.
    crate::charge::start_monitor();
    // Hand the fan back to the EC on any orderly exit. The unit repeats this
    // in ExecStopPost so an unclean kill is covered too.
    for sig in [libc::SIGINT, libc::SIGTERM] {
        unsafe { libc::signal(sig, handle_signal as *const () as libc::sighandler_t) };
    }
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
