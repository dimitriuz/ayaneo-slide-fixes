//! The tray icon, and the process that owns it.
//!
//! This is deliberately a separate process from the window. egui's
//! `ViewportCommand::Visible(false)` is a no-op on Wayland - verified against
//! KWin, which kept reporting `hidden=false visible=true` after the app
//! believed it had hidden itself - and hiding a toplevel is not really a
//! Wayland operation at all. Keeping the tray in a process that has no window
//! sidesteps that entirely: closing the window closes a process, and the tray
//! is untouched because it was never part of it.

use std::process::Command;

/// What the window is asked to do, over the IPC socket.
pub enum TrayMsg {
    Show,
    Quit,
    /// Bind a handheld button, sent by `--map` when a window is running.
    Map(String, String),
}

/// Raise the running window, or start one.
fn open_window() {
    if crate::ipc::send_to_gui("show").is_ok() {
        return;
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| "ayaneo-tray".into());
    let _ = Command::new(exe).arg("--window").spawn();
}

pub struct Tray;

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "ayaneo-tray".into()
    }
    fn title(&self) -> String {
        "AYANEO".into()
    }
    fn icon_name(&self) -> String {
        "input-gaming".into()
    }
    /// Drawn rather than shipped, so the tray works with no icon theme and no
    /// asset files.
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        const N: i32 = 22;
        let mut data = Vec::with_capacity((N * N * 4) as usize);
        for y in 0..N {
            for x in 0..N {
                let (dx, dy) = ((x - N / 2) as f32, (y - N / 2) as f32);
                let body = dx.abs() < 8.0 && dy.abs() < 6.0;
                let stick = (dx * dx + dy * dy).sqrt() < 2.5;
                let (a, v) = if stick {
                    (255u8, 40u8)
                } else if body {
                    (255, 220)
                } else {
                    (0, 0)
                };
                data.extend_from_slice(&[a, v, v, v]); // ARGB32
            }
        }
        vec![ksni::Icon { width: N, height: N, data }]
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        open_window();
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "Open".into(),
                activate: Box::new(|_: &mut Tray| open_window()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_: &mut Tray| {
                    // Take the window with us, then stop.
                    let _ = crate::ipc::send_to_gui("quit");
                    std::thread::spawn(|| {
                        std::thread::sleep(std::time::Duration::from_millis(250));
                        std::process::exit(0);
                    });
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// The tray process: an icon and nothing else.
pub fn run_daemon(open_now: bool) -> anyhow::Result<()> {
    let service = ksni::TrayService::new(Tray);
    service.spawn();
    // A handheld button mapped to the DBus action opens the window too, which
    // is the only way to reach it without a pointer.
    crate::hotkey::watch(open_window);
    // Ring effects have to outlive the settings window, so they run here.
    crate::rings::run_effects();
    if open_now {
        open_window();
    }
    loop {
        std::thread::park();
    }
}
