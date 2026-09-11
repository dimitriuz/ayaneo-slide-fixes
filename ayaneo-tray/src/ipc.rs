//! Single instance, and "launching it again raises the window".
//!
//! A tray app that starts a second copy on every menu click is worse than
//! useless: two tray icons, two sets of cached settings, and two things writing
//! the same hardware. So the first instance owns a socket in the runtime dir,
//! and later launches hand their request to it and exit.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use crate::tray::TrayMsg;

fn socket_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(dir).join("ayaneo-tray.sock")
}

/// True if another instance took the request. The caller should exit.
pub fn hand_off_to_running(show: bool) -> bool {
    let p = socket_path();
    match UnixStream::connect(&p) {
        Ok(mut s) => {
            let _ = s.write_all(if show { b"show\n" } else { b"ping\n" });
            true
        }
        Err(_) => {
            // Nothing listening. A socket file left by a crashed instance would
            // make bind() fail, so clear it.
            let _ = std::fs::remove_file(&p);
            false
        }
    }
}

/// Own the socket and forward requests from later launches to the UI.
pub fn listen(tx: Sender<TrayMsg>, repaint: impl Fn() + Send + 'static) {
    let p = socket_path();
    let Ok(listener) = UnixListener::bind(&p) else { return };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_ok() && line.trim() == "show" {
                let _ = tx.send(TrayMsg::Show);
                repaint();
            }
        }
    });
}
