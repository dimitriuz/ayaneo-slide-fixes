# InputPlumber: practical guide (and why a game may not see your controller)

## How it works

InputPlumber sits between the physical pad and everything else:

```
physical pad (/dev/input/eventN, xpad)
    |   InputPlumber takes it EXCLUSIVELY (EVIOCGRAB)
    v
composite device  ("AYANEO Slide", id 0)
    |   translated through the active profile
    v
target devices    <- this is ALL that games and Steam can see
```

The consequence that surprises people: **while the physical pad is managed, games
never see the real pad.** They only see whatever targets you emulate. So "the
game doesn't detect my controller" is usually a question of *which target* is
active, not whether the pad works.

Check the current state:

```bash
inputplumber devices list                 # composite devices (usually just id 0)
inputplumber device 0 targets list        # what games can actually see
inputplumber device 0 profile name        # active profile
```

On this device the default is:

```
mouse      InputPlumber Mouse
keyboard   InputPlumber Keyboard
deck-uhid  Valve Steam Deck Controller     <- the gamepad games see
```

## Why a game may not recognise the controller

### 1. The target is a Steam Deck controller

`deck-uhid` presents a **Valve Steam Deck Controller over UHID/hidraw**. Steam
understands it natively — that is the point of it, and it is why Steam Input and
the Deck UI work nicely. But a game launched **outside** Steam, using SDL or
plain evdev, may not map it, and some older titles ignore it entirely.

**Fix — emulate a plain Xbox pad instead:**

```bash
inputplumber device 0 targets set xb360 mouse keyboard
# then re-apply your profile, since changing targets can reset it
inputplumber device 0 profile load /etc/inputplumber/profiles/rightstick-mouse.yaml
```

Valid target ids (from the config schema):

```
mouse  keyboard  gamepad  xb360  xbox-elite  xbox-series
deck  deck-uhid  ds5  ds5-edge  hori-steam  touchpad  touchscreen
```

Useful choices:

| target | presents as | use when |
|---|---|---|
| `xb360` | Xbox 360 pad (XInput) | **most compatible** — the safe default for non-Steam games |
| `xbox-elite` | Xbox Elite pad | you want paddles/extra buttons |
| `xbox-series` | Xbox Series pad | modern titles |
| `deck-uhid` | Valve Steam Deck Controller | Steam / Game Mode, gyro + trackpad-style features |
| `ds5` | DualSense | games with better PlayStation support |
| `gamepad` | generic | fallback |

Always keep `mouse` and `keyboard` in the list if you use the stick-as-mouse
mapping — setting targets **replaces** the list, it does not add to it.

### 2. Steam is intercepting it

If Steam is running, Steam Input may claim the pad and present its own virtual
device — so a game started outside Steam sees nothing usable.

* In Steam: *Settings → Controller* → turn **Steam Input** off for the specific
  game (or globally) and relaunch.
* For non-Steam games, try launching with Steam closed.
* SDL sometimes needs a nudge to ignore the Steam layer:

```bash
SDL_JOYSTICK_HIDAPI_STEAM=0 %command%      # as a Steam launch option
SDL_JOYSTICK_HIDAPI=0 ./game               # or outside Steam
```

### 3. InputPlumber is not managing the device at all

Then the physical pad is exposed directly and most games work — but your
profile (stick-as-mouse, dial bindings) is inactive. Check:

```bash
inputplumber devices list      # "Found 0 composite device(s)" = not managing
```

Re-attach it:

```bash
sudo systemctl restart inputplumber
```

The profile is reloaded automatically afterwards by
`inputplumber-profile.service`.

### 4. Two daemons fighting over the pad

CachyOS's handheld image ships **both** InputPlumber and Handheld Daemon, and
both try to manage the gamepad. Symptom: HHD logs `[Errno 16] Device or resource
busy` every 3 seconds forever. See
[INPUT-CONTROLLER.md](INPUT-CONTROLLER.md#1-handheld-daemon-hhd-crash-loop).

## Diagnosing what the game can see

```bash
ls /dev/input/js*                     # joystick nodes; games often use these
sudo evtest                           # pick a device, watch events live
sudo ./scripts/stickcheck.py          # shows gamepad AND mouse output together
```

If `stickcheck` shows gamepad axis events, the emulated pad is working and the
problem is on the game/Steam side, not InputPlumber's.

Also worth knowing which node is which — the *physical* pad and the *emulated*
one have confusingly similar names:

```bash
grep -l 'X-Box 360 pad$' /sys/class/input/event*/device/name    # physical
grep -l 'X-Box 360 pad 0$' /sys/class/input/event*/device/name  # emulated
```

## Everyday commands

```bash
# state
inputplumber devices list
inputplumber device 0 info
inputplumber device 0 capabilities
inputplumber device 0 targets list
inputplumber device 0 profile name
inputplumber device 0 profile path
inputplumber device 0 profile dump          # full YAML of the active profile

# change what games see
inputplumber device 0 targets set xb360 mouse keyboard

# profiles
inputplumber device 0 profile load /path/to/profile.yaml

# hand the pad back to the kernel (e.g. to read it raw, or for USB passthrough)
inputplumber device 0 stop
sudo systemctl restart inputplumber          # take it back

# manage every supported device, or none
inputplumber devices manage-all enable
inputplumber devices manage-all disable
```

## Making a change permanent

`targets set` and `profile load` are **runtime only** — both are lost on
restart. For persistence:

* **Targets:** edit `target_devices` in the device config. Do not edit
  `/usr/share/inputplumber/devices/50-ayaneo_slide.yaml` (package-owned); copy it
  to `/etc/inputplumber/devices.d/` and edit that.
* **Profiles:** use the loader unit in this repo
  (`scripts/ip-load-profile.sh` + `systemd/inputplumber-profile.service`), which
  waits for the composite device to appear and then loads your profile.

## Gotchas found the hard way

* Changing targets **replaces** the whole list. Forget `mouse` and your
  stick-as-mouse mapping silently stops working.
* A loaded profile lives in memory only; `systemctl restart inputplumber` drops it.
* `inputplumber device 0 stop` releases the **entire composite device, including
  the IMU** — after which `iio-sensor-proxy` and KDE auto-rotate can flip a
  natively-portrait panel. Save and restore rotation around it.
* Device detection is asynchronous: a script that runs right after
  `systemctl start inputplumber` will race and find no device. Retry in a loop.
* Event node numbers change when devices re-enumerate. Match by **name** or via
  `/dev/input/by-path/`, never a hardcoded `eventN`.

---

# Axis handling: what InputPlumber can and cannot do

Verified against InputPlumber **0.79.4** source, and measured on an AYANEO
SLIDE whose left stick has a mechanical centring fault. Useful if you are
trying to tame a stick that does not return to zero.

## `deadzone` in a profile only makes buttons

`deadzone` on an axis or trigger appears in exactly two places in the source,
`translate_axis_to_button()` and `translate_trigger_to_button()`:

```rust
// src/input/event/value.rs:943
let threshold = axis.deadzone.unwrap_or(0.3);
```

Both are axis→**button** translations. Nothing in any axis→axis path reads it.
The schema description is literally accurate — *"When this deadzone threshold
is crossed, this input is considered 'pressed'"* — but it is easy to read as a
general stick deadzone. **It is not.** Putting `deadzone` on a stick that maps
to another stick does nothing at all.

## `quadratic_scaling` does work, and is undocumented

`AxisCapability` carries three fields the JSON schema does not mention:

| field | effect |
|---|---|
| `quadratic_scaling` | signed square of the normalised value |
| `invert` | negates both components |
| `deadzone` | axis→button threshold only (above) |

`device_profile_v1.json` lists only `name`, `direction` and `deadzone` with
`additionalProperties: false`, so a schema-aware editor will flag the other
two as invalid. They deserialise fine — profile YAML maps onto the same
`AxisCapability` struct as capability maps.

It must go on the **target** axis, not the source:

```yaml
- name: Left Stick
  source_event:
    gamepad:
      axis:
        name: LeftStick
  target_events:
  - gamepad:
      axis:
        name: LeftStick
        quadratic_scaling: true      # <- target side
```

The maths is `v * v.abs()` on the normalised value, so full deflection is
preserved and small deflections are squared:

| stick at | application sees |
|---|---|
| 100% | 100% |
| 50% | 25% |
| 14% | 2.0% |
| 5% | 0.25% |

That makes it a good fit for a stick with a **centring fault**: it collapses a
resting offset toward zero without a hard cutoff and without losing range. On
the SLIDE measured here it took a 15.7% direction-dependent hysteresis band
down to ~2.0% as seen by applications. It also slows genuine fine movement, so
it is a trade, not a free win.

## The axis→mouse deadzone is hardcoded at 20% — patched here

> **Fixed locally.** `patches/inputplumber/` makes this configurable, and
> `scripts/build-inputplumber.sh` builds and installs it. Jump to
> [Patching it](#patching-it) for the how.

If you drive a `mouse` target from a stick, there is a fixed threshold you
cannot configure:

```rust
// src/input/event/value.rs:713
// Check to see if the value is below a given threshold to prevent
// mouse movements for axes that don't recenter to 0.
if value.abs() < 0.20 { x = Some(0.0); }
```

`MouseMotionCapability` exposes only `direction` and `speed_pps`. So the stick
must travel 20% before the pointer moves at all, and then motion begins at
`0.20 * speed_pps` — 160 px/s at the default 800. That is why a stick-as-mouse
can feel simultaneously unresponsive and too fast, and no amount of profile
work or hardware deadzone tuning changes it. `quadratic_scaling` does not help
either: `Axis -> Mouse` routes through `translate_axis_to_mouse_motion()`,
which never reads it.

## Measuring any of this: the target may have no evdev node

The default gamepad target on a Steam-oriented image is `deck-uhid`, a **UHID**
device — `Generic Steam Controller`, `28DE:12F0`. It has **no
`/dev/input/event*` node**: it speaks HID reports on `/dev/hidraw*`. Watching
evdev for it shows nothing however far the sticks move.

Two traps that cost real time here:

* An evdev node named like the target (`Microsoft X-Box 360 pad 0`) may be
  registered by InputPlumber as a **source**, not an output. Check
  `busctl --system tree org.shadowblip.InputPlumber` — sources appear under
  `devices/source/`, outputs under `devices/target/`.
* `EVIOCGABS` on a uinput device is not a reliable window onto what has been
  written to it. Read the event stream instead.

Confirm what the target actually is:

```bash
journalctl -u inputplumber | grep 'Setting target devices' | tail -1
busctl --system get-property org.shadowblip.InputPlumber \
    /org/shadowblip/InputPlumber/CompositeDevice0 \
    org.shadowblip.Input.CompositeDevice TargetDevices
```

[`scripts/stickverify.py`](../scripts/stickverify.py) handles all of this: it
polls the physical pad with `EVIOCGABS` (which works despite InputPlumber's
exclusive grab) and decodes the emulated pad's HID reports, then prints the
physical→emulated ratio bucketed by deflection band.

```
sudo stickverify        # move both sticks fully, Ctrl+C
```

* ratio **~1.00 in every band** — linear passthrough
* ratio **tracking the deflection** (1.0 at full, 0.5 at half) —
  `quadratic_scaling` is in effect
* ratio **0.00 below ~20%** on a stick driving a mouse — the hardcoded
  threshold above

Since the emulated pad here has no evdev node, this also explains why non-Steam
games may not see the controller at all: there is no `js*` or `event*` device
for them to find. Switching the target to `xb360` gives them a standard evdev
pad — see [The target is a Steam Deck controller](#1-the-target-is-a-steam-deck-controller) above.

## Patching it

Two patches against upstream `v0.79.4`, in `patches/inputplumber/`:

* **0001** — `device_profile_v1.json` is hand-maintained: `src/generate.rs`
  only emits `capability_map_v2.json`, and the `AxisEvent` / `MouseMotionEvent`
  titles in it match no Rust type. It had drifted, documenting `deadzone` but
  not `quadratic_scaling` or `invert`, while declaring
  `additionalProperties: false` — so the one option that helps a stick which
  does not recentre looks invalid to any schema-aware editor. This documents
  both.
* **0002** — adds `deadzone` to `MouseMotionCapability` and uses it in place of
  the constant, defaulting to `0.20` so existing profiles are unaffected.

```bash
sudo pacman -S --needed rust clang libiio pkgconf
./scripts/build-inputplumber.sh
```

The build takes about 4-5 minutes on a 7840U. It installs to
`/usr/local/bin/inputplumber` and adds a systemd drop-in pointing the unit
there, so the distro package is never touched. To go back:

```bash
sudo rm /etc/systemd/system/inputplumber.service.d/99-patched-binary.conf
sudo systemctl daemon-reload && sudo systemctl restart inputplumber
```

**A package update does not refresh the override.** `pacman` upgrades
`/usr/bin/inputplumber`, while the unit keeps running the older patched build
from `/usr/local/bin`. Re-run the script after an update, and bump `TAG` if
upstream has moved — `git am` will refuse rather than misapply if the patches
need rebasing.

### Choosing a deadzone

Set it a little **above** the stick's resting offset, or the pointer drifts:
`MouseDevice::poll()` integrates a stored velocity, and `update_state()` only
changes that velocity when a translated event arrives — so a stick left resting
above the deadzone keeps the pointer moving until something else moves it.

```bash
sudo ./scripts/mouseverify.py   # move the right stick slowly out from centre
```

It prints where motion first appeared and the largest deflection that produced
none; the effective deadzone lies between them. On the SLIDE measured here the
right stick rests at up to 4.7% and wanders by a couple of percent, so `0.06`
is about the floor — still 3.3× finer than the stock `0.20`, with motion
starting near 48 px/s instead of 160.

The same event-driven behaviour is why a stationary stick produces no pointer
motion at all, however far it is deflected. Do not conclude a deadzone is too
high from a stick you are not currently moving.
