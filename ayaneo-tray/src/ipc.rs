//! Talking to a running window.
//!
//! The tray lives in one process and the window in another - see main.rs for
//! why. This is the channel between them, and it also gives single-instance
//! behaviour: a second `--window` hands its request to the first and exits.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use crate::tray::TrayMsg;

fn socket_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(dir).join("ayaneo-tray-gui.sock")
}

/// Send a command to a running window. Err means there is no window.
pub fn send_to_gui(cmd: &str) -> std::io::Result<()> {
    let mut s = UnixStream::connect(socket_path())?;
    s.write_all(format!("{cmd}\n").as_bytes())
}

pub fn gui_running() -> bool {
    send_to_gui("ping").is_ok()
}

/// Own the socket for the lifetime of the window.
pub fn listen(tx: Sender<TrayMsg>, repaint: impl Fn() + Send + 'static) {
    let p = socket_path();
    // A socket file left by a crashed window would make bind() fail, and it is
    // safe to clear because connect() above just told us nothing is listening.
    if UnixStream::connect(&p).is_err() {
        let _ = std::fs::remove_file(&p);
    }
    let Ok(listener) = UnixListener::bind(&p) else { return };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_err() {
                continue;
            }
            let line = line.trim();
            match line.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["show"] => {
                    let _ = tx.send(TrayMsg::Show);
                    repaint();
                }
                ["quit"] => {
                    let _ = tx.send(TrayMsg::Quit);
                    repaint();
                }
                // The window owns the settings file while it is open, so a
                // binding made on the command line is handed to it rather than
                // written underneath it - otherwise the window's next save,
                // which writes its whole in-memory copy, puts the old value
                // back.
                ["map", button, action] => {
                    let _ = tx.send(TrayMsg::Map(button.to_string(), action.to_string()));
                    repaint();
                }
                _ => {}
            }
        }
    });
}
