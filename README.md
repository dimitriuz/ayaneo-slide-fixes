# AYANEO SLIDE on Linux — problems, causes and fixes

What went wrong on one handheld, why, and what fixed it. Every entry below is
something that was actually broken on this machine, investigated until the cause
was known, and written up with the evidence rather than the guess.

**Tested on exactly one machine:** AYANEO SLIDE (board `AS01`, EC `0x001b0100`,
Ryzen 7 7840U / Phoenix, eDP panel `AYANEOHD`) running CachyOS with
`linux-cachyos-deckify` 7.2.3 and KDE Plasma on Wayland. Some findings generalise
to other AYANEO models or to any AMD laptop; most do not. Each document says
which.

> The tray application that came out of this work — controller, lighting, fan,
> power, charging and button bindings — is its own project:
> **[ayaHelper](https://github.com/dimitriuz/ayahelper)**. This repository is the
> research it stands on.

---

## Find your problem

### Display

| Symptom | Cause | Read |
|---|---|---|
| Minimum brightness far too bright — at 0% the panel still sits at ~21% duty | Two stacked causes: a kernel 7.x regression on the AMD PWM path, and a firmware-reported floor specific to this panel | [ROOT-CAUSE.md](docs/ROOT-CAUSE.md) → [BUILD.md](docs/BUILD.md) |
| Brightness slider feels compressed — ~100 usable steps, top ~20% does nothing | The same millipercent conversion bug | [ROOT-CAUSE.md](docs/ROOT-CAUSE.md) |
| Which kernel parameters this machine needs, and why | `amdgpu.dcdebugmask=0x40000`, `acpi=strict` | [KERNEL-PARAMS.md](docs/KERNEL-PARAMS.md) |

Fixes: [`patches/0001-drm-amd-display-*`](patches/) and
[`patches/0002-drm-panel-*`](patches/), applied to a 7.x tree.

### Sleep and battery

| Symptom | Cause | Read |
|---|---|---|
| Press sleep and nothing happens for 15–30 s; screen stays lit and unresponsive | `ayaneo-platform` stops a thread sitting in `msleep(30000)`, and waits out the remainder | [SUSPEND.md §1](docs/SUSPEND.md) → [`patches/ayaneo-platform-interruptible-bypass-sleep.patch`](patches/ayaneo-platform-interruptible-bypass-sleep.patch) |
| Joystick rings glow while the machine is asleep | The driver's `suspend_mode` defaults to `oem`, which hands the LEDs to the EC | [SUSPEND.md §2](docs/SUSPEND.md) |
| Screen stays lit through suspend | Deck-mode Steam holds a Wayland idle inhibitor, invisible to every inhibitor list | [SUSPEND.md §3](docs/SUSPEND.md) |
| A sleep hook that tries to close an application does nothing | systemd freezes user sessions *before* running sleep hooks | [SUSPEND.md §3b](docs/SUSPEND.md) |
| Battery flat after a night asleep | Firmware declares neither low-power S0 idle nor an AMD PMC, and offers no S3 — suspend costs ~1.9 W | [SUSPEND.md §4](docs/SUSPEND.md) |
| Controller gone after resume until you press a button on it | The gamepad MCU powers down and its descriptor has no remote-wakeup bit | [INPUT-CONTROLLER.md](docs/INPUT-CONTROLLER.md) |

### Controller and input

| Symptom | Cause | Read |
|---|---|---|
| Stick deadzone and sensitivity cannot be changed from Linux | They are a stored setting on a GuLiKit MCU, reached over an on-board UART — not the EC, not HID | [GAMEPAD-PROTOCOL.md](docs/GAMEPAD-PROTOCOL.md) |
| A stick does not return to the same place twice | Direction-dependent mechanical hysteresis, ~16% on the left stick. Not calibration — there is no calibration command | [INPUT-CONTROLLER.md](docs/INPUT-CONTROLLER.md) |
| Squeezing a trigger moves the cursor | Trigger-to-stick crosstalk, ~11.5% peak — measure it with [`scripts/crosstalk.py`](scripts/crosstalk.py) | [INPUT-CONTROLLER.md](docs/INPUT-CONTROLLER.md) |
| LC and RC buttons do nothing | They *are* delivered — as gamepad capabilities the emulated target cannot express, so they are translated and dropped | [AYASPACE-FEATURES.md](docs/AYASPACE-FEATURES.md) |
| Stick-as-mouse ignores the desktop's pointer settings and moves too fast | The Steam Deck target speaks HID; Steam claims it and drives the pointer itself | [INPUTPLUMBER-GUIDE.md](docs/INPUTPLUMBER-GUIDE.md) |
| A game does not see the controller at all | The Steam Deck target has no `/dev/input` node | [INPUTPLUMBER-GUIDE.md](docs/INPUTPLUMBER-GUIDE.md) |
| Handheld Daemon crash-looping against InputPlumber | Two daemons managing the same pad | [INPUT-CONTROLLER.md §1](docs/INPUT-CONTROLLER.md) |

### Power, charging and lighting

| Symptom | Cause | Read |
|---|---|---|
| Charge limit and bypass charging do nothing | The write reaches EC `0xd1d1` and the battery charges through it. AYASpace's own charge settings do nothing on this unit either, on Windows | [CHARGING.md](docs/CHARGING.md) |
| TDP set in a tool does not stick | Decky's PowerControl re-runs `ryzenadj` every 15 s; hhd resets it after every resume | [AYASPACE-FEATURES.md](docs/AYASPACE-FEATURES.md) |
| Keyboard backlight cannot be controlled | HID feature report `0x41` on the vendor interface | [AYASPACE-FEATURES.md](docs/AYASPACE-FEATURES.md) |
| Ring LED effects (Radar, Ripple) cannot be reproduced | They are host-side animations, and the driver exposes one colour for both rings | [AYASPACE-FEATURES.md](docs/AYASPACE-FEATURES.md) |
| VRAM size cannot be changed from Linux | AYASpace calls a WMI method provided by a *Windows driver*, not by firmware | [AYASPACE-FEATURES.md](docs/AYASPACE-FEATURES.md) |

---

## Reverse engineering, if you want to go further

| Document | What it covers |
|---|---|
| [GAMEPAD-PROTOCOL.md](docs/GAMEPAD-PROTOCOL.md) | The full gamepad settings protocol: framing, checksum, every field, and how the AYASpace decompilation got there |
| [AYASPACE-FEATURES.md](docs/AYASPACE-FEATURES.md) | What AYASpace exposes, feature by feature, and how much of it Linux can reach |
| [EC-INVESTIGATION.md](docs/EC-INVESTIGATION.md) | How the EC was ruled out as the home of the stick settings — a closed dead end, kept because it was convincing |
| [CAPTURE-PLAN.md](docs/CAPTURE-PLAN.md) · [CAPTURE-DEADZONE.md](docs/CAPTURE-DEADZONE.md) | Capturing AYASpace's traffic from Windows, when static analysis is not enough |
| [INPUTPLUMBER-GUIDE.md](docs/INPUTPLUMBER-GUIDE.md) | InputPlumber in practice: profiles, capability maps, target devices, and the traps in each |

The Ghidra container used for the AYASpace work is in [`ghidra/`](ghidra), with
the exact commands in GAMEPAD-PROTOCOL.md. `AYASpaceCef.exe` is not
redistributed here — extract it from an installer.

## What else is in here

| Path | |
|---|---|
| [`patches/`](patches) | Kernel backlight patches, the `ayaneo-platform` suspend patch, and InputPlumber patches |
| [`scripts/`](scripts) | Measurement and control tools — `gulikit-ctl.py`, `ayaneo-kbdlight.py`, `crosstalk.py`, `stickverify.py`, the EC dumpers |
| [`config/`](config) · [`systemd/`](systemd) | InputPlumber profile and unit |

Most of what these scripts do by hand, ayaHelper does with a UI.

## Scope, and a warning

This is reverse engineering of undocumented hardware, verified on one machine.
Several entries above are **negative results** — things that cannot be made to
work on this unit, recorded so nobody spends an evening rediscovering them.

Anything here that writes to the embedded controller, to SMU power limits or to
firmware settings can destabilise or damage a machine it was not written for.
The model gates in the tooling are a guard, not a guarantee. There is no warranty
of any kind; you run this at your own risk. Unaffiliated with AYANEO.
