//! Opening the window from a handheld button.
//!
//! The SLIDE's LC and RC buttons reach InputPlumber as the gamepad capabilities
//! `LeftTop` and `RightTop` - its `aya5` capability map turns the
//! Ctrl+Meta+F15/F16 chords the keyboard MCU sends into those. A profile can
//! then route one of them to `dbus: ui_quick`, which InputPlumber emits as an
//! `InputEvent` signal on its DBus *target* device. That is the intended hook
//! for a handheld UI, and it is what this listens for.
//!
//! Two conditions are easy to get wrong and produce a button that does nothing:
//!
//! * The DBus target has to be among the composite device's target devices.
//!   `SetTargetDevices` replaces the whole set, so switching the emulated
//!   controller drops it unless it is passed every time - see
//!   `inputplumber::set_target`.
//! * The profile has to route something to it. Out of the box LC and RC map to
//!   Elite paddles, which an Xbox 360 target cannot express, so they are
//!   translated and then dropped.
//!
//! Uses `busctl monitor` rather than a DBus crate, for the same reason the rest
//! of the InputPlumber code shells out: no second DBus stack in a binary that
//! already carries one for the tray.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

/// The action a button has to be mapped to for this to fire.
pub const ACTION: &str = "ui_quick";

/// Watch for the action and run `on_press` each time it goes down.
///
/// Reconnects if the monitor exits, which it does whenever InputPlumber
/// restarts - a button that stops working after a service restart would be a
/// miserable thing to debug.
pub fn watch(on_press: impl Fn() + Send + 'static) {
    std::thread::spawn(move || loop {
        run_once(&on_press);
        std::thread::sleep(std::time::Duration::from_secs(3));
    });
}

fn run_once(on_press: &impl Fn()) {
    let child = Command::new("busctl")
        .args([
            "--system",
            "monitor",
            "--match",
            "type='signal',interface='org.shadowblip.Input.DBusDevice',member='InputEvent'",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return };
    let Some(out) = child.stdout.take() else { return };

    // busctl prints each message as a block; the action name and its value are
    // on separate lines, so the action is remembered until its value arrives.
    //     MESSAGE "sd" {
    //             STRING "ui_quick";
    //             DOUBLE 1;
    //     };
    let mut pending = false;
    for line in BufReader::new(out).lines().map_while(Result::ok) {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("STRING \"") {
            pending = rest.starts_with(ACTION);
        } else if let Some(rest) = t.strip_prefix("DOUBLE ") {
            // Released is value 0, and acting on both edges would open the
            // window and immediately open it again.
            let down = rest.trim_end_matches(';').trim().parse::<f64>().unwrap_or(0.0) >= 0.5;
            if pending && down {
                on_press();
            }
            pending = false;
        }
    }
    let _ = child.wait();
}
