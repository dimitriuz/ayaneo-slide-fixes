//! Device I/O on a background thread.
//!
//! Everything this program talks to is slow by UI standards. A gamepad write
//! retries up to five times at 300 ms; a helper request is a blocking socket
//! round-trip that can wait on the EC mutex; probing walks every serial port.
//! Run any of that on the render thread and the compositor marks the window
//! "Not Responding" - which is exactly what the first version did.
//!
//! So the UI only ever sends jobs and drains results. Jobs are coalesced by
//! kind: while a gamepad write is in flight, further edits replace the pending
//! one rather than queueing behind it, so dragging a slider cannot build a
//! backlog of stale writes.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

use crate::{gamepad, helper, kbdlight, rings};

pub enum Job {
    /// Re-run device discovery. Needed because hardware can appear late: on a
    /// warm reboot this controller has been seen enumerating eight minutes in,
    /// and a probe done once at startup would call it missing forever.
    Reprobe(gamepad::Record, bool),
    ApplyPad(gamepad::Record),
    ApplyKbd(kbdlight::KbdLight),
    ApplyRings(rings::Rings),
    Helper(String),
    PollHelper,
    /// Read the SMU's actual power limits back.
    PollTdp,
    /// Read charge behaviour, limit and battery state.
    PollCharge,
    /// "auto" or "inhibit-charge".
    ChargeBehaviour(String),
    /// 0 turns supervision off.
    ChargeLimit(u32),
    /// InputPlumber: poll state, or act on it.
    PollIp,
    /// Load a profile, then re-apply the saved pointer speed it resets.
    IpProfile(std::path::PathBuf, Option<u32>),
    IpMouseSpeed(u32),
    IpMouseDeadzone(u32),
    /// Bind one handheld button to one action.
    IpButton(String, String),
    IpTarget(String),
    IpManageAll(bool),
}

pub enum Msg {
    /// Human-readable outcome for the status line.
    Status(String),
    /// Reply to an explicit Helper job.
    HelperReply(Result<String, String>),
    /// Result of PollHelper: (helper_up, fan status line).
    HelperState(bool, String),
    /// Fresh device discovery.
    Devices(crate::hw::Devices),
    /// Fresh InputPlumber state.
    IpState(crate::inputplumber::Status),
    /// Result of PollTdp: the limits in watts, or why they could not be read.
    TdpLimits(Result<(u32, u32, u32), String>),
    /// Fresh charge state.
    ChargeState(crate::charge::Status),
}

/// A helper reply about charging, or an empty state if the helper is not there
/// - which the page renders as "unavailable" rather than as an error banner.
fn charge_msg(r: anyhow::Result<String>) -> Msg {
    match r {
        Ok(line) => Msg::ChargeState(crate::charge::parse(&line)),
        Err(_) => Msg::ChargeState(crate::charge::Status::default()),
    }
}

fn key(job: &Job) -> &'static str {
    match job {
        Job::Reprobe(..) => "probe",
        Job::ApplyPad(_) => "pad",
        Job::ApplyKbd(_) => "kbd",
        Job::ApplyRings(_) => "rings",
        Job::Helper(_) => "helper",
        Job::PollHelper => "poll",
        Job::PollTdp => "polltdp",
        Job::PollCharge => "pollcharge",
        Job::ChargeBehaviour(_) => "chgbehave",
        Job::ChargeLimit(_) => "chglimit",
        Job::PollIp => "pollip",
        Job::IpProfile(..) => "ipprofile",
        Job::IpMouseSpeed(_) => "ipspeed",
        Job::IpMouseDeadzone(_) => "ipdz",
        Job::IpButton(..) => "ipbutton",
        Job::IpTarget(_) => "iptarget",
        Job::IpManageAll(_) => "ipmanage",
    }
}

#[derive(Default)]
struct Queue {
    /// At most one pending job per kind; a newer one supersedes an older.
    pending: HashMap<&'static str, Job>,
    order: Vec<&'static str>,
}

pub struct Worker {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    pub rx: Receiver<Msg>,
}

impl Worker {
    pub fn spawn(devices: crate::hw::Devices, repaint: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = channel();
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let q = queue.clone();
        std::thread::spawn(move || run(q, tx, devices, repaint));
        Worker { queue, rx }
    }

    pub fn submit(&self, job: Job) {
        let (lock, cv) = &*self.queue;
        let mut q = lock.lock().unwrap();
        let k = key(&job);
        if q.pending.insert(k, job).is_none() {
            q.order.push(k);
        }
        cv.notify_one();
    }
}

fn run(
    queue: Arc<(Mutex<Queue>, Condvar)>,
    tx: Sender<Msg>,
    mut devices: crate::hw::Devices,
    repaint: impl Fn(),
) {
    loop {
        let job = {
            let (lock, cv) = &*queue;
            let mut q = lock.lock().unwrap();
            while q.order.is_empty() {
                q = cv.wait(q).unwrap();
            }
            let k = q.order.remove(0);
            q.pending.remove(k)
        };
        let Some(job) = job else { continue };

        let msg = match job {
            Job::Reprobe(rec, trusted) => {
                devices = crate::hw::Devices::probe(&rec, trusted);
                Msg::Devices(devices.clone())
            }
            Job::ApplyPad(rec) => match &devices.gamepad {
                Some(p) => match gamepad::send(p, &rec) {
                    Ok(_) => Msg::Status("Controller updated".into()),
                    Err(e) => Msg::Status(format!("Controller: {e}")),
                },
                None => Msg::Status("Controller unavailable".into()),
            },
            Job::ApplyKbd(k) => match &devices.kbd {
                Some(p) => match kbdlight::apply(p, &k) {
                    Ok(()) => Msg::Status("Keyboard light updated".into()),
                    Err(e) => Msg::Status(format!("Keyboard: {e}")),
                },
                None => Msg::Status("Keyboard unavailable".into()),
            },
            Job::ApplyRings(r) => match &devices.rings {
                Some(p) => match rings::apply(p, &r) {
                    Ok(()) => Msg::Status("Ring lights updated".into()),
                    Err(e) => Msg::Status(format!("Rings: {e}")),
                },
                None => Msg::Status("Rings unavailable".into()),
            },
            Job::Helper(cmd) => {
                Msg::HelperReply(helper::request(&cmd).map_err(|e| e.to_string()))
            }
            Job::PollHelper => match helper::request("fan status") {
                Ok(s) => Msg::HelperState(true, s),
                Err(_) => Msg::HelperState(false, String::new()),
            },
            Job::PollTdp => Msg::TdpLimits(match helper::request("tdp info") {
                Ok(s) => {
                    let v: Vec<u32> = s.split_whitespace().flat_map(str::parse).collect();
                    match v.as_slice() {
                        [a, b, c] => Ok((*a, *b, *c)),
                        _ => Err(format!("unexpected reply {s:?}")),
                    }
                }
                Err(e) => Err(e.to_string()),
            }),
            // All three answer with the same status line, so one path reads it.
            Job::PollCharge => charge_msg(helper::request("charge status")),
            Job::ChargeBehaviour(b) => {
                charge_msg(helper::request(&format!("charge behaviour {b}")))
            }
            Job::ChargeLimit(pct) => charge_msg(helper::request(&if pct == 0 {
                "charge limit off".to_string()
            } else {
                format!("charge limit {pct}")
            })),
            Job::IpButton(src, act) => {
                match crate::inputplumber::set_button_action(&src, &act) {
                    Ok(()) => Msg::Status(format!("{src} → {act}")),
                    Err(e) => Msg::Status(e),
                }
            }
            Job::PollIp => Msg::IpState(crate::inputplumber::status()),
            Job::IpProfile(path, speed) => match crate::inputplumber::load_profile(&path) {
                Ok(()) => {
                    // Loading a profile replaces speed_pps with whatever the
                    // file says, so put the chosen value back.
                    if let Some(pps) = speed {
                        let _ = crate::inputplumber::set_mouse_speed(pps);
                    }
                    // Same for the button bindings, which are live-only edits.
                    for (src, act) in crate::state::load().0.button_map {
                        let _ = crate::inputplumber::set_button_action(&src, &act);
                    }
                    Msg::IpState(crate::inputplumber::status())
                }
                Err(e) => Msg::Status(e),
            },
            Job::IpMouseSpeed(pps) => match crate::inputplumber::set_mouse_speed(pps) {
                Ok(()) => Msg::IpState(crate::inputplumber::status()),
                Err(e) => Msg::Status(e),
            },
            Job::IpMouseDeadzone(pct) => match crate::inputplumber::set_mouse_deadzone(pct) {
                Ok(()) => Msg::IpState(crate::inputplumber::status()),
                Err(e) => Msg::Status(e),
            },
            Job::IpTarget(id) => match crate::inputplumber::set_target(&id) {
                // The target is torn down and rebuilt, so let it settle before
                // reading back what it became.
                Ok(()) => {
                    std::thread::sleep(std::time::Duration::from_millis(1200));
                    Msg::IpState(crate::inputplumber::status())
                }
                Err(e) => Msg::Status(e),
            },
            Job::IpManageAll(on) => match crate::inputplumber::set_manage_all(on) {
                Ok(()) => Msg::IpState(crate::inputplumber::status()),
                Err(e) => Msg::Status(e),
            },
        };
        if tx.send(msg).is_err() {
            return;
        }
        repaint();
    }
}
