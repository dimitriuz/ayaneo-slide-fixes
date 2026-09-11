# ayaneo-tray

A tray utility for AYANEO handhelds on Linux — the parts of AYASpace that
matter, in one ~5 MB binary with no driver, no CEF, and no daemon for anything
that does not strictly need one.

```
Controller   deadzone, per-stick sensitivity, trigger and gyro levels,
             per-button turbo, rumble, ABXY swap, factory reset
Lighting     keyboard backlight (colour, 3 effects, brightness, Fn light)
             joystick ring LEDs (colour, brightness)
Power        ACPI platform profile, sustained TDP via ryzenadj
Fan          automatic, fixed speed, or a draggable curve
Sensors      temperatures, battery, draw
Input        InputPlumber: profile, emulated controller, service control
```

Protocols are documented in [`../docs/GAMEPAD-PROTOCOL.md`](../docs/GAMEPAD-PROTOCOL.md)
and [`../docs/AYASPACE-FEATURES.md`](../docs/AYASPACE-FEATURES.md); everything here
was established by reverse-engineering AYASpace 3.2.0.4 and verified on hardware.

## Interface

Built for a 7" panel used with a thumb, not a desktop with a mouse: one idea per
row, every option visible as a large target rather than folded into a dropdown,
and the dense groups — triggers, gyro, turbo — collapsed until asked for. All
sizing comes from `widgets.rs` so it stays consistent instead of drifting per
tab, and *About* has a UI scale slider because what a compositor reports for a
high-DPI handheld is rarely what a thumb wants.

**No device I/O happens on the render thread.** A gamepad write retries up to
five times at 300 ms, a helper request is a blocking socket round-trip that can
wait on the EC mutex, and probing walks every serial port — any of which stalls
the compositor into marking the window "Not Responding", which is exactly what
the first version did. `worker.rs` runs all of it on a background thread, and
coalesces jobs by kind so dragging a slider replaces the pending write instead
of queueing a backlog of stale ones.

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

The GUI keeps its cache in `~/.config/ayaneo-tray/settings.json`, seeded once
from the CLI tools' `/var/lib/ayaneo/` on first run. After that the two are
independent — the GUI runs unprivileged and cannot write a root-owned
directory — so if you use both, expect them to drift. The hardware is the thing
both are describing, and neither can read it back.

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
ayaneo-tray --window     # open the window (or raise the running instance)
ayaneo-tray --status     # print device and settings state, write nothing
ayaneo-tray --restore    # re-apply saved settings (used by the login unit)
```

Click the tray icon to open the window; close the window to dismiss it. Quit,
in the tray menu, stops the tray itself.

**The tray and the window are separate processes**, and that is deliberate.
egui's `ViewportCommand::Visible(false)` is a no-op on Wayland — KWin kept
reporting `hidden=false visible=true` after the app believed it had hidden
itself, so the close button appeared to do nothing at all. Hiding a toplevel
is not really a Wayland operation. With the tray in a process that has no
window, closing the window closes a process and the tray is untouched, because
it was never part of it.

**Starting it.** After installing the units above:

```bash
systemctl --user enable --now ayaneo-tray     # autostart at login, tray only
```

It also appears in the application menu as **AYANEO**. Launching it again does
not start a second copy: the first instance owns a socket in `$XDG_RUNTIME_DIR`
and later launches hand their request over and exit, so the menu entry raises
the existing window. That matters more than it sounds — two copies means two
tray icons, two caches, and two things writing the same hardware.

**Devices that appear late.** Discovery re-runs every ten seconds while
anything is missing. On a warm reboot this controller has been observed
enumerating on USB *eight minutes* after boot, and a probe done once at startup
would have called it missing for the rest of the session.

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

## Charging: decoded, not yet verified

`system.set_charge_config` on an AS01 resolves to a single EC register:

```
EC[0xD1,0xD1]   AYASpace writes 101 to charge normally, 1 to stop charging
                its getter treats 1..100 as "limited", >=101 or 0 as "normal"
```

The register accepts and holds arbitrary values — 80, 1 and 101 all stick — so
it is a real byte rather than a boolean that snaps back, which is consistent
with a charge threshold in percent. The JS API carries a `val` percentage
alongside `enable`, and on this model only `enable` is ever sent, which hints
the firmware supports a threshold that AYASpace does not expose here.

**None of that is observed behaviour, and it is not implemented for that
reason.** An attempt to verify at 97% charge was inconclusive: the EC had
already stopped charging on its own — normal Li-ion recharge hysteresis, which
declines to top up a nearly-full pack — so charging was not happening during
the test window and no write could have shown an effect. `EC[0xD1,0xD1]`
holding `80` proves only that the EC did not reject the byte.

A valid test needs the battery well below the resume point (~85%) with charging
actively sustained, then a threshold set below the current level. Until that
happens this stays out of the tool: every other feature here was confirmed by
watching the hardware do something, and this one has not been.

## What is deliberately missing

**TDP read-back.** `ryzenadj` sets limits fine but cannot read them here:
without the `ryzen_smu` kernel module, `/dev/mem` access is refused. The UI
shows what was last set, not what the SMU reports.
