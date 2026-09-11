//! InputPlumber control, over its system-bus DBus API.
//!
//! Everything here is reachable by an ordinary user - loading a profile,
//! switching the emulated target device, and toggling ManageAllDevices all
//! succeed without root, which is why none of it goes through the helper. Only
//! starting and stopping the unit itself needs privilege.
//!
//! Calls go through `busctl` rather than a DBus crate. It ships with systemd,
//! which InputPlumber already requires, and it keeps a second D-Bus stack out
//! of a binary that already carries one for the tray. These are user-initiated
//! actions a few times a session, so the cost of a process is irrelevant - but
//! they are still slow enough to belong on the worker thread.

use std::path::PathBuf;
use std::process::Command;

const SERVICE: &str = "org.shadowblip.InputPlumber";
const COMPOSITE: &str = "/org/shadowblip/InputPlumber/CompositeDevice0";
const COMPOSITE_IF: &str = "org.shadowblip.Input.CompositeDevice";
const MANAGER: &str = "/org/shadowblip/InputPlumber/Manager";
const MANAGER_IF: &str = "org.shadowblip.InputManager";
const TARGET_IF: &str = "org.shadowblip.Input.Target";

/// Emulated gamepads worth offering. The full list includes touchpads, a null
/// device and internal debug targets, which are not useful choices here.
pub const TARGETS: [(&str, &str); 6] = [
    ("deck-uhid", "Steam Deck"),
    ("xb360", "Xbox 360"),
    ("xbox-elite", "Xbox Elite"),
    ("xbox-series", "Xbox Series"),
    ("ds5", "DualSense"),
    ("unified-gamepad", "Unified"),
];

/// Pointer speed in pixels per second, from the loaded profile.
///
/// This is InputPlumber's own mouse, which the compositor sees as an ordinary
/// pointer - so it is the knob that matters when a stick drives the cursor
/// through InputPlumber. It is *not* what moves the cursor when the emulated
/// target is a Steam Deck: Steam claims that controller and drives the pointer
/// itself, bypassing both this and the desktop's own pointer settings.
pub const SPEED_RANGE: (u32, u32) = (100, 2000);

#[derive(Default, Clone)]
pub struct Status {
    pub running: bool,
    pub version: String,
    pub device: String,
    pub profile: String,
    /// The target id currently in use, if it is one we offer.
    pub target: Option<String>,
    pub manage_all: bool,
    /// None when the loaded profile has no stick-to-mouse mapping.
    pub mouse_speed: Option<u32>,
}

fn busctl(args: &[&str]) -> Option<String> {
    let out = Command::new("busctl").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// busctl prints properties as `TYPE value`; strip the type and quotes.
fn scalar(s: &str) -> String {
    s.split_once(' ').map(|(_, v)| v).unwrap_or(s).trim().trim_matches('"').to_string()
}

fn get(path: &str, iface: &str, prop: &str) -> Option<String> {
    busctl(&["--system", "get-property", SERVICE, path, iface, prop]).map(|s| scalar(&s))
}

pub fn status() -> Status {
    let mut st = Status::default();
    let Some(version) = get(MANAGER, MANAGER_IF, "Version") else {
        return st; // service not on the bus
    };
    st.running = true;
    st.version = version;
    st.device = get(COMPOSITE, COMPOSITE_IF, "Name").unwrap_or_default();
    st.profile = get(COMPOSITE, COMPOSITE_IF, "ProfileName").unwrap_or_default();
    st.manage_all = get(MANAGER, MANAGER_IF, "ManageAllDevices").unwrap_or_default() == "true";

    // Each target object carries its own DeviceType, which is exactly the id
    // SetTargetDevices takes - no need to infer anything from display names.
    if let Some(raw) = busctl(&[
        "--system", "get-property", SERVICE, COMPOSITE, COMPOSITE_IF, "TargetDevices",
    ]) {
        for path in raw.split('"').filter(|s| s.starts_with('/')) {
            if let Some(kind) = get(path, TARGET_IF, "DeviceType") {
                if TARGETS.iter().any(|(id, _)| *id == kind) {
                    st.target = Some(kind);
                    break;
                }
            }
        }
    }
    st.mouse_speed = profile_yaml().as_deref().and_then(parse_speed);
    st
}

/// busctl prints strings escaped and quoted; recover the original.
fn unescape(s: &str) -> String {
    let inner = match (s.find('"'), s.rfind('"')) {
        (Some(a), Some(b)) if b > a => &s[a + 1..b],
        _ => return String::new(),
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

pub fn profile_yaml() -> Option<String> {
    busctl(&["--system", "call", SERVICE, COMPOSITE, COMPOSITE_IF, "GetProfileYaml"])
        .map(|s| unescape(&s))
        .filter(|s| !s.is_empty())
}

fn parse_speed(yaml: &str) -> Option<u32> {
    let i = yaml.find("speed_pps:")?;
    yaml[i + 10..].trim_start().split_whitespace().next()?.parse().ok()
}

/// Rewrite speed_pps in the live profile and reload it.
///
/// This edits the running profile rather than the file on disk, so it needs no
/// privilege - and equally does not survive the profile being loaded again.
pub fn set_mouse_speed(pps: u32) -> Result<(), String> {
    let yaml = profile_yaml().ok_or("could not read the current profile")?;
    let i = yaml.find("speed_pps:").ok_or("this profile has no mouse mapping")?;
    let rest = &yaml[i + 10..];
    let digits: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return Err("could not parse speed_pps".into());
    }
    let lead = rest.len() - rest.trim_start().len();
    let new = format!("{}speed_pps: {}{}", &yaml[..i], pps, &rest[lead + digits.len()..]);
    busctl(&["--system", "call", SERVICE, COMPOSITE, COMPOSITE_IF, "LoadProfileFromYaml", "s", &new])
        .map(|_| ())
        .ok_or_else(|| "InputPlumber rejected the profile".to_string())
}

/// Profiles from both the packaged and the local directory, local last so a
/// customised profile of the same name wins.
pub fn profiles() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for dir in ["/usr/share/inputplumber/profiles", "/etc/inputplumber/profiles"] {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        let mut found: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
            .collect();
        found.sort();
        for p in found {
            // Prefer the profile's declared name over its filename.
            let label = std::fs::read_to_string(&p)
                .ok()
                .and_then(|t| {
                    t.lines()
                        .find(|l| l.starts_with("name:"))
                        .map(|l| l[5..].trim().trim_matches('"').to_string())
                })
                .unwrap_or_else(|| {
                    p.file_stem().unwrap_or_default().to_string_lossy().to_string()
                });
            out.retain(|(l, _)| l != &label);
            out.push((label, p));
        }
    }
    out
}

pub fn load_profile(path: &std::path::Path) -> Result<(), String> {
    busctl(&[
        "--system", "call", SERVICE, COMPOSITE, COMPOSITE_IF, "LoadProfilePath", "s",
        &path.to_string_lossy(),
    ])
    .map(|_| ())
    .ok_or_else(|| format!("could not load {}", path.display()))
}

pub fn set_target(id: &str) -> Result<(), String> {
    // Keyboard and mouse targets are kept: dropping them would take the
    // stick-as-mouse mapping with them.
    busctl(&[
        "--system", "call", SERVICE, COMPOSITE, COMPOSITE_IF, "SetTargetDevices", "as", "3", id,
        "keyboard", "mouse",
    ])
    .map(|_| ())
    .ok_or_else(|| format!("could not switch to {id}"))
}

pub fn set_manage_all(on: bool) -> Result<(), String> {
    busctl(&[
        "--system", "set-property", SERVICE, MANAGER, MANAGER_IF, "ManageAllDevices", "b",
        if on { "true" } else { "false" },
    ])
    .map(|_| ())
    .ok_or_else(|| "could not set ManageAllDevices".to_string())
}
