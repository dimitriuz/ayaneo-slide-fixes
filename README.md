# AYANEO SLIDE — Linux backlight fixes

Fixes two independent backlight problems on the **AYANEO SLIDE** (AMD Ryzen 7 7840U,
Phoenix / DCN 3.1.4, eDP panel `AYANEOHD`) running a mainline‑based kernel **7.x**:

1. **Minimum brightness far too bright** — even at 0% the panel stays at ~21% PWM duty.
2. **Whole slider range feels compressed** — only ~100 usable steps and the top ~20%
   of the slider does nothing.

Problem 1 has two causes stacked on top of each other: a **kernel 7.x regression**
that affects *every* AMD device on the PWM backlight path, and a **firmware‑reported
floor** specific to this machine.

> Verified on CachyOS, `linux-cachyos-deckify` 7.2.3. The regression fix applies to
> any distro on kernel ≥ 7.0.

---

## TL;DR

```bash
git clone https://github.com/dimitriuz/ayaneo-slide-fixes
cd ayaneo-slide-fixes
# apply patches/ to a 7.x kernel tree, rebuild, install
# then set ONE kernel parameter:
#   amdgpu.dcdebugmask=0x40000
```

Full walkthrough: **[docs/BUILD.md](docs/BUILD.md)** ·
Why it broke: **[docs/ROOT-CAUSE.md](docs/ROOT-CAUSE.md)** ·
Parameters: **[docs/KERNEL-PARAMS.md](docs/KERNEL-PARAMS.md)**

Also here, unrelated to the backlight: the **gamepad settings protocol**
(stick deadzone and sensitivity) reverse-engineered and implemented for Linux —
**[docs/GAMEPAD-PROTOCOL.md](docs/GAMEPAD-PROTOCOL.md)**.

---

## Symptoms

| | before | after |
|---|---|---|
| `max_brightness` | 56026 | 56026 |
| `actual_brightness` at brightness `0` | **6797** (~21% duty) | **0** |
| usable slider steps | ~100, top ~20% dead | ~100, full range |
| peak brightness | 100% duty | 85.5% duty (firmware max) |
| dimmest setting | too bright for a dark room | panel fully off at 0 |

---

## Root cause 1 — kernel 7.x regression (affects all AMD PWM backlights)

Commit [`3c108046e1d6`](https://git.kernel.org/linus/3c108046e1d6) *("drm/amd/display:
Add power module on Linux")* replaced a direct 16‑bit PWM write with a millipercent
conversion:

```c
/* power module uses millipercent */
get_brightness_range(caps, &min, &max);
brightness = DIV_ROUND_CLOSEST(brightness * 100, (max - min)) * 1000;
```

`brightness` has already been mapped into the absolute range `[min, max]`, but the
expression never subtracts `min` and divides by the wrong span. The consumer,
`backlight_millipercent_to_pwm()`, expects a value *relative* to that range:

```c
pwm = min_backlight_pwm + millipercent * backlight_range / 100000;
```

Three defects result:

* **`min` not subtracted** → slider 0 requests `21000` millipercent instead of `0`.
* **whole‑percent rounding** → resolution collapses to 100 steps regardless of `max_brightness`.
* **can exceed `100000`** → the top of the slider clamps and becomes a dead zone.

Fixed by [`patches/0001-*`](patches/).

## Root cause 2 — firmware floor (AYANEO SLIDE specific)

ACPI ATIF `QUERY_BRIGHTNESS_TRANSFER_CHARACTERISTICS` reports:

```
min_input_signal = 38    max_input_signal = 218    ac_level = 80    dc_level = 50
```

amdgpu scales these by `0x101`, so the PWM floor is `38 × 257 = 9766` of 65535 —
**14.9% duty**, still too bright in a dark room even with the regression fixed.

The panel advertises **no AUX/DPCD backlight control** (DPCD `0x700..0x72f` reads all
zero), so PWM is the only control path and this floor cannot be side‑stepped by
switching backlight control type.

Fixed by [`patches/0002-*`](patches/), a `.min_brightness = 1` quirk — the same
treatment upstream already gives the Steam Deck and Framework panels.

---

## How this was confirmed

Booting **6.18 LTS**, which still has the pre‑regression code, was the decisive test:
the firmware caps were **byte‑identical**, but behaviour differed completely — which
rules out any hardware or firmware explanation.

| measurement | 7.2 (buggy) | 6.18 LTS |
|---|---|---|
| caps line | `min: 9766, max: 56026, ac 80, dc 50` | **identical** |
| `max_brightness` | 56026 | 46260 (`max - min`) |
| `actual` at brightness 0 | 6797 | **0** |
| transfer curve | S‑curve, ~100 steps | perfect identity, `set == actual` |
| 40‑unit steps | no change (~560 treads) | every step changes |

Independent confirmation of the whole‑percent rounding: computing
`brightness × 100 / (max − min)` at each observed step boundary gives
`57.56, 58.53, 59.60, 60.56, 61.52` — every boundary lands just past a `k + 0.5`
crossing, the exact signature of `DIV_ROUND_CLOSEST`. Predicted saturation onset
43915 of 56026; measured in `(43400, 43950]`.

Ruled out along the way, each with evidence rather than assumption: ABM
(`panel_power_savings = 0`), a userspace daemon overwriting writes (values persisted),
AUX/DPCD backlight (register block all zero), the ACPI video path (firmware `_BCL`
floor is 10, barely below 14.9%), and an ACPI table override (`0xDA` appears nowhere
as an AML constant — checked with a positive control).

---

## Known limitation

~100 discrete levels remain. This is **not** from these patches — the DC power module
floor‑indexes a ~101‑entry lookup table:

```c
index = ((num_backlight_levels - 1) * millipercent) / 100000;
pwm   = backlight_lut[index];
```

Matches the measured `56026 / 100 ≈ 560` unit treads. It is upstream behaviour on this
code path (`use_linear_backlight_curve` is false) and is not perceptible on a
0–100% slider.

## Heads-up

With `min_brightness = 1` the floor is 0% duty, so **brightness `0` turns the panel
off**. If you would rather have a dim-but-never-off floor, use `.min_brightness = 6`
in `patches/0002-*` (≈2% duty).

---

## Upstream status

As of **2026‑09‑09** the regression is present in `torvalds/master` **and** in AMD's
own `amd-staging-drm-next`. No newer, test, dev or beta kernel fixes it.

`patches/` are formatted for `git am` with `Fixes:` tags. They carry **no
`Signed-off-by`** — that line is a DCO certification only the sender can make, so use
`git am -s`. See [docs/BUILD.md](docs/BUILD.md#sending-upstream).

## Repo layout

```
patches/   the two kernel patches (git am format)
docs/      root cause, build guide, kernel parameters, controller/input
           findings, and the reverse-engineered gamepad protocol
config/    InputPlumber stick-to-mouse mapping
systemd/   unit that reloads the InputPlumber profile at boot
ghidra/    Dockerfile for the Ghidra + ghidra-cli container used for the RE
scripts/   gulikit-ctl.py (gamepad settings over the MCU's UART);
           ayaneo-kbdlight.py (keyboard backlight over HID report 0x41);
           stickverify.py (physical vs emulated stick comparison);
           measure-backlight.sh, measure-stick.py, sticklive.py,
           stickcheck.py (diagnostics); ayaneo-ctl.py (vendor HID channel);
           rebuild.sh, ip-load-profile.sh, winvm.sh
```

### Using InputPlumber

If a game does not detect your controller — with or without InputPlumber — see
**[docs/INPUTPLUMBER-GUIDE.md](docs/INPUTPLUMBER-GUIDE.md)**. The usual cause is
that InputPlumber grabs the physical pad exclusively and games only ever see the
*emulated* target, which by default here is a Valve Steam Deck Controller that
non-Steam titles may not map. Switching the target to `xb360` fixes most cases.

### AYASpace features from Linux

A full inventory of what AYASpace can do, the transport behind each feature, and
how much of it works on Linux: **[docs/AYASPACE-FEATURES.md](docs/AYASPACE-FEATURES.md)**.
Two independent transports carry nearly all of it, and both are now implemented:

```bash
# gamepad MCU, over an on-board UART
sudo gulikit-ctl set --deadzone off --right 50
sudo gulikit-ctl set --trigger-l2 high --turbo-a burst --rumble medium

# keyboard backlight, over HID feature report 0x41
sudo ayaneo-kbdlight --color 00ff88 --mode breath
```

### Gamepad settings from Linux

**The gamepad settings protocol is solved** — see
**[docs/GAMEPAD-PROTOCOL.md](docs/GAMEPAD-PROTOCOL.md)**. AYASpace does not use
USB or the EC for these; it talks to a **GuLiKit gamepad MCU over an on-board
legacy 16550 UART** — I/O `0x3E8` (COM3 on Windows, `/dev/ttyS2` on Linux) at
115200 8N1. `scripts/gulikit-ctl.py` implements it:

```bash
sudo install -m755 scripts/gulikit-ctl.py /usr/local/bin/gulikit-ctl
sudo gulikit-ctl init --factory
sudo gulikit-ctl probe
sudo gulikit-ctl set --deadzone off --right 50
```

It covers the stick deadzone, per-stick sensitivity (50/100/150), rumble level,
trigger and gyro levels, per-button turbo, and ABXY swap.

The **keyboard backlight** is solved too, on a different transport: a HID
feature report `0x41` to the keyboard MCU — `scripts/ayaneo-kbdlight.py`. Of the
rest, the stick ring RGB (`ayaneo:rgb:joystick_rings`) and TDP/power (HHD,
`platform_profile`) already work on Linux and need nothing.

## Also in this repo: controller / right-stick fixes

Separate from the backlight, three input issues on the same device — see
**[docs/INPUT-CONTROLLER.md](docs/INPUT-CONTROLLER.md)**:

1. **Handheld Daemon crash-looping every 3 s** (fixable) — HHD and InputPlumber both
   try to manage the gamepad; InputPlumber wins the exclusive grab and HHD retries
   forever with `EBUSY`.
2. **Right-stick pointer far too fast, and KDE's slider does nothing** (fixable) —
   the motion comes from Steam's Desktop Layout injected via XTEST, which bypasses
   libinput acceleration. Fixed by driving InputPlumber's own `mouse` target instead,
   which exposes a `speed_pps` knob.
3. **Stick deadzone far too wide** (**fixed, natively**) — both sticks, 15-25%
   (left) and 30-50% (right) of full scale before anything registers. It is a
   *stored setting* in the gamepad MCU, not hardware. It can now be turned off
   from Linux with `gulikit-ctl set --deadzone off`; no Windows, no VM, no
   firmware flash. Protocol: **[docs/GAMEPAD-PROTOCOL.md](docs/GAMEPAD-PROTOCOL.md)**.

## License

Patches are kernel code: **GPL-2.0**. Documentation and scripts: GPL-2.0 as well, for
simplicity.

## Disclaimer

Custom kernels and backlight registers. Verified on one AYANEO SLIDE. A bad backlight
floor can leave you with a dark screen — know how to boot a previous kernel entry
before you start.
