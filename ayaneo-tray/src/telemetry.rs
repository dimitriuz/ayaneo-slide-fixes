//! Read-only sensors: temperatures from hwmon, battery from power_supply.
//! Nothing here writes, so it needs no privileges.

#[derive(Default, Clone)]
pub struct Telemetry {
    pub temps: Vec<(String, f32)>,
    pub battery_pct: Option<u32>,
    pub battery_status: Option<String>,
    pub power_now_w: Option<f32>,
}

fn read_trim(p: impl AsRef<std::path::Path>) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// A few labels are more useful than a raw hwmon dump.
fn pretty(name: &str) -> Option<&'static str> {
    match name {
        "k10temp" => Some("CPU"),
        "amdgpu" => Some("GPU"),
        "nvme" => Some("SSD"),
        "acpitz" => Some("Board"),
        _ => None,
    }
}

pub fn read() -> Telemetry {
    let mut t = Telemetry::default();
    if let Ok(rd) = std::fs::read_dir("/sys/class/hwmon") {
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for h in entries {
            let Some(name) = read_trim(h.join("name")) else { continue };
            let Some(label) = pretty(&name) else { continue };
            if let Some(v) = read_trim(h.join("temp1_input")).and_then(|s| s.parse::<f32>().ok()) {
                // hwmon reports millidegrees; 0 means the sensor is not wired up
                if v > 0.0 {
                    t.temps.push((label.to_string(), v / 1000.0));
                }
            }
        }
    }
    let bat = std::path::Path::new("/sys/class/power_supply/BAT0");
    if bat.exists() {
        t.battery_pct = read_trim(bat.join("capacity")).and_then(|s| s.parse().ok());
        t.battery_status = read_trim(bat.join("status"));
        // power_now is microwatts on most laptops; some report current instead
        if let Some(uw) = read_trim(bat.join("power_now")).and_then(|s| s.parse::<f32>().ok()) {
            t.power_now_w = Some(uw / 1_000_000.0);
        }
    }
    t
}
