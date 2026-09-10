//! StatusNotifierItem tray icon.
//!
//! ksni talks D-Bus directly, so this works on KDE, and on GNOME with the
//! AppIndicator extension, without linking any widget toolkit.

use std::sync::mpsc::Sender;

/// What the tray asks the GUI thread to do.
pub enum TrayMsg {
    Toggle,
    Show,
    Quit,
}

pub struct Tray {
    pub tx: Sender<TrayMsg>,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "ayaneo-tray".into()
    }
    fn title(&self) -> String {
        "AYANEO".into()
    }
    /// A themed name first; icon_pixmap below is the fallback when a theme has
    /// no such icon, which keeps the tray usable with no asset files at all.
    fn icon_name(&self) -> String {
        "input-gaming".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        const N: i32 = 22;
        let mut data = Vec::with_capacity((N * N * 4) as usize);
        for y in 0..N {
            for x in 0..N {
                // a filled rounded square, so the icon reads at tray size
                let (dx, dy) = ((x - N / 2) as f32, (y - N / 2) as f32);
                let inside = dx.abs() < 8.0 && dy.abs() < 6.0;
                let stick = (dx * dx + dy * dy).sqrt() < 2.5;
                let (a, v) = if stick {
                    (255u8, 40u8)
                } else if inside {
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
        let _ = self.tx.send(TrayMsg::Toggle);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "Open".into(),
                activate: Box::new(|t: &mut Tray| {
                    let _ = t.tx.send(TrayMsg::Show);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut Tray| {
                    let _ = t.tx.send(TrayMsg::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
