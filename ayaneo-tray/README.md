# ayaneo-tray

A tray utility for AYANEO handhelds on Linux — the parts of AYASpace that
matter, in one ~5 MB binary with no driver, no CEF, and no daemon for anything
that does not strictly need one.

```
Controller   deadzone, per-stick sensitivity, trigger and gyro levels,
             per-button turbo, rumble, ABXY swap, factory reset
Lighting     keyboard backlight (colour, 3 effects, brightness, Fn light)
             joystick ring LEDs (colour, brightness)
Power        ACPI platform profile, sustained TDP via ryzenadj, fan, sensors
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

## Fan control

There is no kernel fan interface on this machine — no `pwm*`, no `fan*_input`.
Control goes through two EC registers, taken from AYASpace's
`CEcControl::FanSetManual` / `FanSetAuto` and confirmed on hardware:

```
EC[0xD1,0xC8]   mode   0x00 = EC automatic curve, 0xA5 = manual
EC[0x18,0x04]   duty   0-255,  duty = percent / 100 * 255
```

The duty register was verified **read-only first**: under load it climbs
monotonically with CPU temperature, saturates at `0xFF`, and returns to an
identical `0x4D` (30%) idle floor. The mode register's page comes from the
class constructor, which initialises the address word to `0xD100` — not the
`0x18` the duty write uses, which is the sort of thing worth checking rather
than assuming.

Gated on `board_name`, not `product_name`: AYASpace's model codes are board
codes, and on a SLIDE `product_name` is "SLIDE" while `board_name` is "AS01".
Only `AS01` and `AB05*` are enabled; an AB10 uses `0x1809`/`0x2F1` and other
models an `0xFE8004xx` block, so anything else is refused rather than guessed.

### The guards, and why

Every other setting here is inert if it is wrong. A fan left at a low duty
under load is not — the CPU throttles at Tjmax rather than come to harm, but it
is a real thermal failure, and it can be caused by the process simply dying.
So manual mode is never left unsupervised. All three are tested:

| guard | behaviour | verified |
|---|---|---|
| duty floor | below 20% refused unless forced; >100% refused | rejects 5% and 150% |
| crash, main process | `ExecStopPost` restores automatic control | SIGKILL of the main PID → `mode=0x00` |
| crash, whole cgroup | next start resets to a known-good state | `systemctl kill` → restored, logged |
| thermal | above 85 °C forces automatic control and **latches** | fired at 85.4 °C under load at 20% duty |

The startup reset is the important one. Teardown hooks are not enough on their
own: `systemctl kill` takes `ExecStopPost` with the rest of the cgroup, and a
power cut runs nothing. Because the helper is the only thing that engages
manual mode and it restarts on failure, resetting at start makes "the fan is in
manual" and "a live supervisor exists" the same condition.

There is no tachometer on this machine, so the UI shows commanded duty, not
measured RPM.

## What is deliberately missing

**TDP read-back.** `ryzenadj` sets limits fine but cannot read them here:
without the `ryzen_smu` kernel module, `/dev/mem` access is refused. The UI
shows what was last set, not what the SMU reports.
