//! Power profile. The ACPI platform_profile interface is the only sanctioned
//! TDP-ish control present on this machine: there is no ryzenadj packaged and
//! no vendor sysfs. It is root-owned, so writes go through the helper.
//!
//! Watt-level control would need SMU access (ryzenadj/libryzenadj) and fan
//! control would need EC registers that are not yet identified - neither is
//! implemented here rather than guessed at.

pub const PATH: &str = "/sys/firmware/acpi/platform_profile";
pub const CHOICES: &str = "/sys/firmware/acpi/platform_profile_choices";

pub fn available() -> Vec<String> {
    std::fs::read_to_string(CHOICES)
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

pub fn current() -> Option<String> {
    std::fs::read_to_string(PATH).ok().map(|s| s.trim().to_string())
}

/// True when the file is directly writable by us, i.e. no helper is needed.
pub fn writable_directly() -> bool {
    std::fs::OpenOptions::new().write(true).open(PATH).is_ok()
}

/// Handy presets for a 7840U-class handheld. The SMU accepts anything in the
/// helper's range; these are just the rungs worth having one click away.
pub const TDP_PRESETS: [(u32, &str); 5] =
    [(8, "8 W quiet"), (12, "12 W"), (15, "15 W default"), (22, "22 W"), (28, "28 W max")];
