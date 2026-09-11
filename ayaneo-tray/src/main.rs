//! ayaneo-tray - a tray utility for AYANEO handhelds on Linux.
//!
//! Replaces the parts of AYASpace that matter, without a driver or a daemon:
//! the gamepad MCU over its UART, the keyboard backlight over a HID feature
//! report, and the ring LEDs over sysfs. See the docs/ directory of
//! ayaneo-slide-fixes for how each protocol was established.

mod charge;
mod conflicts;
mod ec;
mod fan;
mod gamepad;
mod helper;
mod hotkey;
mod ipc;
mod hw;
mod inputplumber;
mod kbdlight;
mod power;
mod rings;
mod state;
mod telemetry;
mod tray;
mod widgets;
mod worker;
mod ui;

use anyhow::Result;

const USAGE: &str = "\
ayaneo-tray - AYANEO handheld control

    ayaneo-tray              run the tray icon (default; the window is a
                             separate process, started when you click it)
    ayaneo-tray --window     open the settings window
    ayaneo-tray --status     print device and settings state, change nothing
    ayaneo-tray --restore    re-apply saved settings and exit (for a login unit)
    ayaneo-tray --map B A    bind handheld button B to action A (no args: list them)
    ayaneo-tray --helper     run the privileged helper (systemd service)
    ayaneo-tray --fan-auto   hand the fan back to the EC and exit (failsafe)
    ayaneo-tray --help
";

fn print_status() {
    let (s, trusted) = state::load();
    let rec = s.record();
    let d = hw::Devices::probe(&rec, trusted);

    println!("settings file : {}", state::path().display());
    if !trusted {
        println!("  (no saved settings and nothing to import - the gamepad record");
        println!("   below is the factory default, NOT what the hardware holds,");
        println!("   and nothing has been written to it)");
    }
    println!();
    let line = |name: &str, ok: &Option<std::path::PathBuf>, err: &Option<String>| {
        match (ok, err) {
            (Some(p), _) => println!("  {name:9} {}", p.display()),
            (None, Some(e)) => println!("  {name:9} unavailable - {e}"),
            (None, None) => println!("  {name:9} unavailable"),
        }
    };
    println!("devices:");
    line("gamepad", &d.gamepad, &d.gamepad_err);
    line("keyboard", &d.kbd, &d.kbd_err);
    line("rings", &d.rings, &d.rings_err);

    // Anything here can silently undo a setting made in the GUI, so name it
    // where someone reporting "it did not stick" will see it.
    let others = conflicts::scan();
    if !others.is_empty() {
        println!();
        println!("also running:");
        for c in others {
            println!("  {:14} {}", c.name, c.effect);
        }
    }

    println!();
    println!("gamepad record  {}", state::hex(&rec.0));
    println!("  deadzone      {}", if rec.deadzone() { "on" } else { "off" });
    let sens = |l: u8| gamepad::SENS.iter().find(|(k, _)| *k == l).map(|(_, v)| *v);
    println!(
        "  stick sens    left {:?}  right {:?}",
        sens(rec.sens(true)),
        sens(rec.sens(false))
    );
    let lvl = |v: u8| gamepad::LEVELS.iter().find(|(k, _)| *k == v).map(|(_, n)| *n).unwrap_or("?");
    println!("  rumble        {}", lvl(rec.rumble()));
    println!("  trigger L2/R2 {} / {}", lvl(rec.trigger(true)), lvl(rec.trigger(false)));
    println!("  gyro L1/L2    {} / {}", lvl(rec.gyro(true)), lvl(rec.gyro(false)));
    let tb = |v: u8| gamepad::TURBO.iter().find(|(k, _)| *k == v).map(|(_, n)| *n).unwrap_or("?");
    println!(
        "  turbo A/B/X/Y/R1/R2  {} {} {} {} {} {}",
        tb(rec.turbo(0)), tb(rec.turbo(1)), tb(rec.turbo(2)),
        tb(rec.turbo(3)), tb(rec.turbo(4)), tb(rec.turbo(5))
    );
    println!("  swap ABXY     {}", rec.swap_abxy());

    println!();
    let k = s.kbdlight;
    let mode = kbdlight::MODES.iter().find(|(v, _)| *v == k.mode).map(|(_, n)| *n).unwrap_or("?");
    println!("keyboard light  #{:06X}  {}  brightness {}%  {}  fn {}",
        k.color, mode, k.brightness,
        if k.enable { "on" } else { "off" },
        if k.fn_ison { "on" } else { "off" });
    println!("  report        {}", state::hex(&k.report()));
    println!("rings           #{:06X}  brightness {}", s.rings.color, s.rings.brightness);

    println!();
    println!("power profile   {:?}  of {:?}", power::current(), power::available());
    println!("  writable      {}", power::writable_directly());

    let t = telemetry::read();
    println!();
    print!("sensors        ");
    for (n, v) in &t.temps {
        print!(" {n} {v:.0}C");
    }
    if let Some(w) = t.apu_power_w {
        print!("  APU {w:.1}W");
    }
    if let Some(p) = t.battery_pct {
        print!("  battery {p}%");
    }
    if let Some(st) = &t.battery_status {
        print!(" {st}");
    }
    if let Some(w) = t.power_now_w {
        print!(" {w:.1}W");
    }
    println!();

    if let Some(b) = charge::behaviour() {
        let lim = helper::request("charge status")
            .ok()
            .map(|l| charge::parse(&l).limit)
            .filter(|l| *l > 0);
        println!();
        print!("charging        behaviour {b}");
        match lim {
            Some(l) => print!("  limit {l}%"),
            None => print!("  no limit"),
        }
        match charge::ec_bypass() {
            Some(true) => print!("  EC bypassing"),
            Some(false) => print!("  EC charging"),
            // Only root can read the EC; not being able to is normal here.
            None => {}
        }
        println!();
    }
}

fn restore() -> Result<()> {
    let (s, trusted) = state::load();
    if !trusted {
        anyhow::bail!(
            "no saved settings to restore, and nothing to import from \
             /var/lib/ayaneo - refusing to write the factory record over \
             whatever the hardware currently holds"
        );
    }
    let d = hw::Devices::probe(&s.record(), trusted);
    let mut failed = false;
    for (name, r) in d.apply_all(&s) {
        match r {
            Ok(()) => println!("{name}: applied"),
            Err(e) => {
                eprintln!("{name}: {e}");
                failed = true;
            }
        }
    }
    // The fan is handed back to the EC whenever the helper stops, so a chosen
    // curve or duty has to be re-established rather than assumed to survive.
    match s.fan_mode.as_deref() {
        Some("curve") => {
            let spec = s
                .fan_curve
                .iter()
                .map(|(t, p)| format!("{t}:{p}"))
                .collect::<Vec<_>>()
                .join(",");
            match helper::request(&format!("fan curve {spec}")) {
                Ok(_) => println!("fan: curve"),
                Err(e) => eprintln!("fan: {e}"),
            }
        }
        Some("manual") => match helper::request(&format!("fan manual {}", s.fan_pct)) {
            Ok(_) => println!("fan: manual {}%", s.fan_pct),
            Err(e) => eprintln!("fan: {e}"),
        },
        _ => {}
    }

    // SMU limits are volatile, so restore covers them too when a helper is up.
    if let Some(w) = s.tdp_watts {
        match helper::request(&format!("tdp {w} {w} {w}")) {
            Ok(_) => println!("tdp: {w} W"),
            Err(e) => eprintln!("tdp: {e}"),
        }
    }
    if let Some(p) = &s.power_profile {
        let r = if power::writable_directly() {
            std::fs::write(power::PATH, p).map_err(|e| e.to_string())
        } else {
            helper::request(&format!("profile {p}")).map(|_| ()).map_err(|e| e.to_string())
        };
        match r {
            Ok(()) => println!("profile: {p}"),
            Err(e) => eprintln!("profile: {e}"),
        }
    }

    for (name, err) in [
        ("gamepad", &d.gamepad_err),
        ("keyboard", &d.kbd_err),
        ("rings", &d.rings_err),
    ] {
        if let Some(e) = err {
            eprintln!("{name}: unavailable - {e}");
        }
    }
    if failed {
        anyhow::bail!("some settings could not be applied");
    }
    Ok(())
}

fn run_gui() -> Result<()> {
    // One window at a time: hand off to a running one rather than opening a
    // second copy that would fight it over the same hardware.
    if ipc::send_to_gui("show").is_ok() {
        return Ok(());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    // Whichever of the two processes is running drives the ring effect; the
    // lock inside decides which, so this is safe to call from both.
    rings::run_effects();

    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([820.0, 620.0])
            .with_min_inner_size([620.0, 460.0])
            .with_title("AYANEO")
            .with_visible(true),
        ..Default::default()
    };
    eframe::run_native(
        "ayaneo-tray",
        opts,
        Box::new(move |cc| {
            // Probe once here rather than per-frame: it walks every serial port.
            let (settings, trusted) = state::load();
            let devices = hw::Devices::probe(&settings.record(), trusted);
            let mut app = ui::App::new(rx, devices.clone());
            let ctx = cc.egui_ctx.clone();
            app.attach_worker(worker::Worker::spawn(devices, {
                let ctx = ctx.clone();
                move || ctx.request_repaint()
            }));
            ipc::listen(tx, move || ctx.request_repaint());
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => tray::run_daemon(false),
        Some("--window") => run_gui(),
        Some("--status") => {
            print_status();
            Ok(())
        }
        Some("--restore") => restore(),
        // Scriptable equivalent of the Buttons page, and the way to check the
        // mapping without a screen.
        Some("--map") => match (args.get(1), args.get(2)) {
            (Some(button), Some(action)) => {
                inputplumber::set_button_action(button, action).map_err(anyhow::Error::msg)?;
                let mut s = state::load().0;
                s.button_map.insert(button.clone(), action.clone());
                state::save(&s)?;
                println!("{button} -> {action}");
                Ok(())
            }
            _ => {
                println!("usage: ayaneo-tray --map <button> <action>");
                println!(
                    "  buttons: {}",
                    inputplumber::SOURCES
                        .iter()
                        .map(|(id, l)| format!("{id} ({l})"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                println!(
                    "  actions: {}",
                    inputplumber::ACTIONS
                        .iter()
                        .map(|a| a.id)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                Ok(())
            }
        },
        Some("--helper") => helper::run(),
        // Used by the helper unit's ExecStopPost, so an unclean exit still
        // leaves the fan under the EC's own control.
        Some("--fan-auto") => {
            fan::restore_on_exit();
            Ok(())
        }
        Some("--help" | "-h") => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => {
            eprintln!("unknown argument: {other}\n\n{USAGE}");
            std::process::exit(2);
        }
    }
}
