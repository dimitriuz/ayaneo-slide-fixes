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
/// Deadzone as a percentage of full stick deflection. The upstream default is
/// a hardcoded 20%; the patch in patches/inputplumber/ makes it configurable.
pub const DEADZONE_RANGE: (u32, u32) = (0, 40);

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
    /// Mouse deadzone in percent, if the profile carries one.
    pub mouse_deadzone: Option<u32>,
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
    if let Some(y) = profile_yaml() {
        st.mouse_speed = parse_speed(&y);
        st.mouse_deadzone = parse_deadzone(&y);
    }
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

/// The mouse deadzone, as a percentage. Only the one under `motion:` counts -
/// a `deadzone` on an axis governs axis-to-button translation and has nothing
/// to do with the pointer.
fn parse_deadzone(yaml: &str) -> Option<u32> {
    let m = yaml.find("motion:")?;
    let rest = &yaml[m..];
    let i = rest.find("deadzone:")?;
    let v: f64 = rest[i + 9..].trim_start().split_whitespace().next()?.parse().ok()?;
    Some((v * 100.0).round() as u32)
}

/// Replace a numeric field in the live profile and reload it.
fn set_field(key: &str, value: &str, after: Option<&str>) -> Result<(), String> {
    let yaml = profile_yaml().ok_or("could not read the current profile")?;
    let base = match after {
        Some(a) => yaml.find(a).ok_or("this profile has no mouse mapping")?,
        None => 0,
    };
    let i = base
        + yaml[base..]
            .find(&format!("{key}:"))
            .ok_or_else(|| format!("this profile has no {key}"))?;
    let rest = &yaml[i + key.len() + 1..];
    let lead = rest.len() - rest.trim_start().len();
    let old: String = rest.trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if old.is_empty() {
        return Err(format!("could not parse {key}"));
    }
    let new = format!("{}{key}: {value}{}", &yaml[..i], &rest[lead + old.len()..]);
    busctl(&["--system", "call", SERVICE, COMPOSITE, COMPOSITE_IF, "LoadProfileFromYaml", "s", &new])
        .map(|_| ())
        .ok_or_else(|| "InputPlumber rejected the profile".to_string())
}

/// Pointer deadzone, as a percentage of full deflection.
pub fn set_mouse_deadzone(pct: u32) -> Result<(), String> {
    set_field("deadzone", &format!("{:.2}", pct as f64 / 100.0), Some("motion:"))
}

/// Rewrite speed_pps in the live profile and reload it.
///
/// This edits the running profile rather than the file on disk, so it needs no
/// privilege - and equally does not survive the profile being loaded again.
pub fn set_mouse_speed(pps: u32) -> Result<(), String> {
    set_field("speed_pps", &pps.to_string(), None)
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
    // stick-as-mouse mapping with them. So is dbus, which is what carries a
    // button mapped to a UI action - SetTargetDevices replaces the whole set,
    // so anything not named here is silently detached.
    busctl(&[
        "--system", "call", SERVICE, COMPOSITE, COMPOSITE_IF, "SetTargetDevices", "as", "4", id,
        "keyboard", "mouse", "dbus",
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

// ------------------------------------------------------------ button mapping

/// The handheld buttons worth offering, as the capability map names them.
///
/// These are what AYANEO's extra buttons become: the `aya5` capability map
/// turns the Ctrl+Meta+F15/F16 chords the keyboard MCU sends into `LeftTop` and
/// `RightTop`. Guide is deliberately absent - Steam expects it, and remapping it
/// breaks more than it fixes.
pub const SOURCES: [(&str, &str); 3] =
    [("LeftTop", "LC"), ("RightTop", "RC"), ("QuickAccess", "Custom")];

/// What a button can be made to do, as the YAML fragment it becomes.
pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    /// Body of `target_events:`, or empty to leave the button unmapped.
    pub yaml: &'static str,
}

pub const ACTIONS: [Action; 6] = [
    Action { id: "none", label: "Nothing", yaml: "" },
    Action { id: "esc", label: "Escape", yaml: "  - keyboard: KeyEsc" },
    Action { id: "app", label: "Open AYANEO", yaml: "  - dbus: ui_quick" },
    Action { id: "osk", label: "On-screen KB", yaml: "  - dbus: ui_osk" },
    Action {
        id: "guide",
        label: "Steam",
        yaml: "  - gamepad:\n      button: Guide",
    },
    Action {
        id: "paddle",
        label: "Paddle",
        yaml: "  - gamepad:\n      button: LeftPaddle1",
    },
];

pub fn action(id: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|a| a.id == id)
}

/// Split a profile into its top-level `- name:` mapping entries.
///
/// Returns (header, entries, trailer-less). InputPlumber serialises these at
/// column zero with a fixed two-space body, so a line starting exactly with
/// "- name:" is a reliable entry boundary - and staying line-based avoids
/// pulling in a YAML parser for one edit.
fn split_entries(yaml: &str) -> (String, Vec<String>) {
    let mut header = String::new();
    let mut entries: Vec<String> = Vec::new();
    for line in yaml.lines() {
        if line.starts_with("- name:") {
            entries.push(String::new());
        }
        match entries.last_mut() {
            Some(e) => {
                e.push_str(line);
                e.push('\n');
            }
            None => {
                header.push_str(line);
                header.push('\n');
            }
        }
    }
    (header, entries)
}

/// The source button an entry reacts to, if it is a plain gamepad button.
fn entry_source(entry: &str) -> Option<String> {
    let src = entry.find("source_event:")?;
    let end = entry.find("target_events:").unwrap_or(entry.len());
    entry[src..end]
        .lines()
        .find_map(|l| l.trim().strip_prefix("button: ").map(|b| b.trim().to_string()))
}

/// Which action each offered button currently performs, read from the live
/// profile. Buttons with no entry, or one this app cannot express, are absent.
pub fn button_actions() -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let Some(yaml) = profile_yaml() else { return out };
    let (_, entries) = split_entries(&yaml);
    for entry in entries {
        let Some(src) = entry_source(&entry) else { continue };
        if !SOURCES.iter().any(|(id, _)| *id == src) {
            continue;
        }
        let Some(i) = entry.find("target_events:") else { continue };
        let body = entry[i + "target_events:".len()..].trim_end();
        let body = body.trim_start_matches('\n');
        if let Some(a) = ACTIONS.iter().find(|a| !a.yaml.is_empty() && body.trim_end() == a.yaml) {
            out.insert(src, a.id.to_string());
        }
    }
    out
}

/// Point one button at one action, in the running profile.
///
/// Rewrites rather than patches: the whole entry for that button is dropped and
/// rebuilt, so switching between a keyboard action and a gamepad one - which
/// have different shapes - needs no special case.
///
/// Edits the live profile, like the pointer settings do, so it needs no
/// privilege and equally does not survive a profile being loaded again. The
/// caller re-applies it.
pub fn set_button_action(source: &str, action_id: &str) -> Result<(), String> {
    let act = action(action_id).ok_or_else(|| format!("unknown action {action_id}"))?;
    let yaml = profile_yaml().ok_or("could not read the current profile")?;
    let (header, entries) = split_entries(&yaml);

    let mut out = header;
    for entry in &entries {
        if entry_source(entry).as_deref() == Some(source) {
            continue; // replaced below
        }
        out.push_str(entry);
    }
    if !act.yaml.is_empty() {
        out.push_str(&format!(
            "- name: {source}\n  source_event:\n    gamepad:\n      button: {source}\n  target_events:\n{}\n",
            act.yaml
        ));
    }

    busctl(&["--system", "call", SERVICE, COMPOSITE, COMPOSITE_IF, "LoadProfileFromYaml", "s", &out])
        .map(|_| ())
        .ok_or_else(|| "InputPlumber rejected the profile".to_string())
}
