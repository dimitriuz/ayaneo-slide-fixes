//! Other daemons that drive the same hardware as this app.
//!
//! Handheld distributions ship one or two of these by default, and they do not
//! coordinate: Decky's PowerControl plugin re-runs `ryzenadj` every fifteen
//! seconds, so a TDP set here is quietly gone before you have put the device
//! down, and Handheld Daemon claims the pad that InputPlumber is also trying to
//! manage. Neither failure announces itself - the control simply stops meaning
//! anything - so the tabs that are affected say what else is running.
//!
//! Detection is by process name only. It costs a directory scan, needs no
//! privileges, and cannot be wrong about the thing it actually claims ("hhd is
//! running"); what that daemon is currently configured to do is a separate
//! question, which is why the wording says "if" rather than asserting.

use std::fs;

/// What a daemon competes with us over.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Over {
    /// Re-applies SMU power limits.
    Tdp,
    /// Manages the gamepad.
    Input,
    /// Both.
    Both,
}

pub struct Daemon {
    /// How to name it to the user.
    pub name: &'static str,
    /// What it does to us, in a clause that follows the name.
    pub effect: &'static str,
    over: Over,
}

impl Daemon {
    pub fn fights(&self, over: Over) -> bool {
        self.over == over || self.over == Over::Both
    }
}

/// Keyed by `/proc/<pid>/comm`, which is what these processes are actually
/// called: Decky rewrites its own title to "Decky Loader" in the child that
/// runs the plugins, and hhd's venv wrapper keeps `hhd` rather than `python3`.
///
/// `static` rather than `const` so there is one instance to compare addresses
/// of when de-duplicating the two Decky processes.
static KNOWN: [(&[&str], Daemon); 2] = [
    (
        &["hhd"],
        Daemon {
            name: "Handheld Daemon",
            effect: "also manages the gamepad, and resets TDP after every resume",
            over: Over::Both,
        },
    ),
    (
        &["PluginLoader", "Decky Loader"],
        Daemon {
            name: "Decky Loader",
            effect: "re-applies TDP every 15 seconds if its PowerControl plugin is enabled",
            over: Over::Tdp,
        },
    ),
];

/// Every known competing daemon that is currently running.
///
/// Reads `comm` rather than `cmdline`: it is one short read per process, and it
/// is the process's own name rather than a string that happens to appear on
/// somebody's command line - a match against cmdlines would fire on this app's
/// own shell-outs.
pub fn scan() -> Vec<&'static Daemon> {
    let me = std::process::id();
    let Ok(dir) = fs::read_dir("/proc") else { return Vec::new() };
    let mut out: Vec<&'static Daemon> = Vec::new();

    for entry in dir.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        if pid == me {
            continue;
        }
        let Ok(comm) = fs::read_to_string(entry.path().join("comm")) else { continue };
        let comm = comm.trim_end();

        for (names, daemon) in &KNOWN {
            if names.contains(&comm) && !out.iter().any(|d| std::ptr::eq(*d, daemon)) {
                out.push(daemon);
            }
        }
    }
    out
}
