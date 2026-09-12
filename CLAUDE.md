# Working on ayaneo-slide-fixes

This repository is **research and patches**, not an application. The tool that
came out of it lives at [ayaHelper](https://github.com/dimitriuz/ayahelper)
(`/home/dimitrius/repo/ayahelper`).

The README is a symptom → cause → document index. This file is what a future
session needs that the documents do not say.

## The machine everything was established on

| | |
|---|---|
| Board / EC | AYANEO SLIDE, `AS01`, EC `0x001b0100` |
| SoC / panel | Ryzen 7 7840U (Phoenix, DCN 3.1.4), eDP `AYANEOHD` |
| OS | CachyOS, `linux-cachyos-deckify` 7.2.3, KDE Plasma on Wayland |
| Reach it | `ssh dimitrius@192.168.1.31` (key set up; passwordless sudo via `/etc/sudoers.d/nopasswd` — **leave that file alone**) |

Its firmware is not well behaved, and two workarounds are load-bearing:

- **`acpi=strict`** is required or the machine reboots at random.
- **`amdgpu.dcdebugmask=0x40000`** is half the backlight fix.
- The firmware declares neither low-power S0 idle nor an AMD PMC, and offers no
  S3 — so suspend cannot reach S0i3 and costs ~1.9 W. Hence suspend-then-hibernate.

The remote shell is **fish**: `$?`, `VAR=x cmd`, `for` loops and most quoting
behave differently. Write scripts to files, `scp`, run. And `pkill -f 'pattern'`
over ssh matches the ssh session's own command line and kills it — kill by PID.

## The Ghidra setup

This is what makes AYASpace questions answerable in minutes rather than a day.
The container is at `/home/dimitrius/slide_fix/ghidra` with the project already
imported and analysed:

```bash
docker exec ghidra ghidra decompile 0x140266530 \
  --project /project/aya --program AYASpaceCef.exe
docker exec ghidra ghidra find string "set_charge_config" --project /project/aya --program AYASpaceCef.exe
docker exec ghidra ghidra x-ref to 0x1407aa428 --project /project/aya --program AYASpaceCef.exe
```

**Resolving a JS handler to its native function.** Handlers are registered as:

```asm
LEA RAX,[handler]        ; <- the function you want
MOV [RSP+X],RAX
LEA R8,[RSP+X]
LEA RDX,[name string]
CALL 0x140065280         ; register(name, handler)
```

So: find the name string, x-ref it, then disassemble backwards from the
reference and take the **last `LEA RAX` before it**. There is a scripted version
of this in the session notes; it is three `ghidra` calls.

`AYASpaceCef.exe` is not redistributed — extract it from an installer.

## Established negatives — do not re-investigate

These cost the most to establish and are the most valuable thing here:

- **No calibration command exists** in the gamepad protocol. The left stick's
  ~16% direction-dependent hysteresis is mechanical and cannot be fixed in
  software.
- **Charge limit and bypass do nothing on this unit.** The write reaches EC
  `0xd1d1`; the battery charges through it. AYASpace takes the identical path for
  board `AS01`, and its charge settings do nothing on Windows either — confirmed
  on the hardware.
- **The EC is not where stick settings live** (that is `EC-INVESTIGATION.md`, a
  closed dead end kept because it was convincing). They are on a GuLiKit MCU over
  an on-board UART.
- **Motion EX cannot be ported** — it needs AYASpace's own driver and service.
- **VRAM size is not reachable** — the WMI class is provided by a Windows driver,
  not firmware; there is no `_WDG` in the DSDT or any SSDT.
- **Radar/Ripple ring effects** need per-quadrant addressing the driver does not
  expose. Ring "modes" are host-side animations, not EC ones.

## Hazards

- **Never `modprobe -r ayaneo_platform`.** Its suspend, shutdown and remove paths
  all `kthread_stop()` the same pointers; unloading double-stops and wedges the
  module — `refcount -1`, `charge_behaviour` gone, no reload. Only a reboot
  clears it. This happened once already. Reboot to pick up a rebuilt module.
- **The gamepad MCU sleeps** across suspend and has no remote-wakeup bit, so it
  will not come back until a button is pressed on the pad. Check that before
  debugging a "missing" controller.
- **Decky's PowerControl** re-applies TDP every 15 s and **hhd** resets it after
  every resume. Both will silently undo power experiments.
- **Deck-mode Steam** (`steam-jupiter` always passes `-steamdeck`) holds a Wayland
  idle inhibitor invisible to `systemd-inhibit`, `ListInhibitions` and the
  ScreenSaver interface. It is why the screen never blanks. If a display
  experiment behaves impossibly, check whether Steam is running.

## Writing style for the docs

The documents are written to be read by someone hitting the symptom, so:

- **Lead with the symptom**, then the cause, then the fix.
- **Show the evidence** — the actual log lines, register values, measurements.
  Not "it was slow" but `platform_pm_suspend returned 0 after 18487017 usecs`.
- **Keep the dead ends**, with what each one actually did and why it looked
  convincing. Several entries are wrong theories that took hours; recording them
  is the point.
- **Prefer prediction to correlation.** The suspend delay was confirmed by
  predicting that waiting 25 s after a resume would cost ~5 s; it cost 5.93 s.
  That is worth more than three consistent observations.

## Verifying on hardware

Nothing here is confirmed until it is tried on the handheld. Useful tools:

```
scripts/gulikit-ctl.py     gamepad MCU over /dev/ttyS2
scripts/ayaneo-kbdlight.py keyboard backlight, HID report 0x41
scripts/crosstalk.py       trigger-to-stick crosstalk (sudo)
scripts/ayaec.py           read-only EC RAM dump of the 0xD1 page
scripts/stickverify.py     stick behaviour
```

A controlled suspend without needing the user: `sudo systemd-run --unit=x
--collect /usr/bin/rtcwake -m mem -s 25`. For per-device timings, enable
`/sys/power/pm_debug_messages` and `/sys/power/pm_print_times` first.

**Do not leave long polling loops running** that the user has to interrupt —
trigger the action, then stop and ask.
