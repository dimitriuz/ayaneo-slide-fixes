# ayaneo-tray

A tray utility for AYANEO handhelds on Linux — the parts of AYASpace that
matter, in one ~5 MB binary with no driver, no CEF, and no daemon for anything
that does not strictly need one.

```
Controller   deadzone, per-stick sensitivity, trigger and gyro levels,
             per-button turbo, rumble, ABXY swap, factory reset
Lighting     keyboard backlight (colour, 3 effects, brightness, Fn light)
             joystick ring LEDs (colour, brightness)
Power        ACPI platform profile, sustained TDP via ryzenadj, sensors
```

Protocols are documented in [`../docs/GAMEPAD-PROTOCOL.md`](../docs/GAMEPAD-PROTOCOL.md)
and [`../docs/AYASPACE-FEATURES.md`](../docs/AYASPACE-FEATURES.md); everything here
was established by reverse-engineering AYASpace 3.2.0.4 and verified on hardware.

## Design

**The GUI is unprivileged.** One udev rules file grants the *active session
user* the two device nodes it drives — the gamepad UART and the keyboard's
vendor HID interface — via `uaccess`, so there is no polkit, no setuid, and no
root GUI. The ring LEDs are plain sysfs attributes.

**Only power needs a helper.** SMU limits (ryzenadj) and `platform_profile` are
root-only, so a small system service accepts three commands on a Unix socket:
`ping`, `tdp W W W`, `profile NAME`. Values are range-checked in the helper
rather than trusted from the client, and nothing takes a path or a shell
string. The socket is `root:wheel 0660`; membership of that group is the entire
security boundary.

**Nothing can be read back.** Neither MCU supports a read: the gamepad has no
read command at all, and the keyboard STALLs `GET_REPORT`. Settings are cached
in `~/.config/ayaneo-tray/settings.json`, exactly as AYASpace caches
`proto.gulikit` and `KeyBoardLightConfig`. On first run the CLI tools' caches
in `/var/lib/ayaneo/` are imported so the two agree.

Because of that, **discovery never writes.** The only way to probe the gamepad
protocol is to send a frame, and a frame is an absolute settings record — so
probing with a record that does not match the hardware silently changes
settings. With no saved state, the UART is identified by I/O address (0x3E8)
and nothing is written; `--restore` refuses rather than writing defaults over
whatever the hardware holds.

## Install

```bash
cargo build --release
sudo install -m755 target/release/ayaneo-tray /usr/bin/ayaneo-tray
sudo install -m644 rootfs/usr/lib/udev/rules.d/70-ayaneo-tray.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger

# optional, for TDP and profile switching
paru -S ryzenadj
sudo install -m644 rootfs/usr/lib/systemd/system/ayaneo-tray-helper.service /etc/systemd/system/
sudo systemctl enable --now ayaneo-tray-helper

# re-apply ring colour and TDP at login (both are volatile)
sudo install -m644 rootfs/usr/lib/systemd/user/ayaneo-tray-restore.service /etc/systemd/user/
systemctl --user enable ayaneo-tray-restore
```

Build needs `rust`; runtime needs only libc, EGL and Wayland/X11 — 7 shared
libraries in total.

## Usage

```bash
ayaneo-tray              # tray applet, window hidden until the icon is clicked
ayaneo-tray --window     # open the window immediately
ayaneo-tray --status     # print device and settings state, write nothing
ayaneo-tray --restore    # re-apply saved settings (used by the login unit)
```

Closing the window hides it to the tray; Quit is in the tray menu.

## Desktop support

The tray uses StatusNotifierItem over D-Bus, so it needs an SNI host. KDE
Plasma has one built in. GNOME needs the AppIndicator extension — without it
the app still runs, but there is no icon, so use `--window`.

## What is deliberately missing

**Fan control.** This machine exposes no kernel fan interface at all: no
`pwm*`, no `fan*_input`, and `ayaneo-platform` provides only LEDs. Controlling
the fan means writing EC registers that have not been identified. Guessing at
registers on a fan controller is a thermal risk, so it waits for the same
evidence the rest of this was built on — AYASpace's `fancontrol.set_cfg`
decompiled, registers confirmed, behaviour measured.

**TDP read-back.** `ryzenadj` sets limits fine but cannot read them here:
without the `ryzen_smu` kernel module, `/dev/mem` access is refused. The UI
shows what was last set, not what the SMU reports.
