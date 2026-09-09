# Controller & right-stick-mouse findings

Second set of issues on the **AYANEO SLIDE** (CachyOS handheld image), unrelated to
the [backlight fixes](../README.md).

Three separate things were found. **Two are fixable, one is not** — and it is worth
knowing which is which before you spend time on it.

| # | Issue | Fixable? |
|---|---|---|
| 1 | Handheld Daemon crash-looping every 3 s | **Yes** — config |
| 2 | Right-stick pointer far too fast, not tunable | **Yes** — InputPlumber profile |
| 3 | Right-stick deadzone far too wide | **No** — controller firmware |

---

## 1. Handheld Daemon (HHD) crash loop

### Symptom

Nothing visible, but `/etc/hhd/log/hhd.log` fills up forever:

```
ERROR  Received the following error:
       <class 'OSError'>: [Errno 16] Device or resource busy
ERROR  Assuming controllers disconnected, restarting after 3s.
INFO   Launching emulated controller.
```

Since boot, every three seconds. Wasted CPU and log churn.

### Cause

CachyOS's handheld image ships **both** `cachyos-handheld` (HHD) and
`inputplumber`, and both try to manage the gamepad. Confirm with:

```bash
sudo fuser -v /dev/input/event6        # your gamepad's event node
#   /dev/input/event6:  root  994 F.... hhd
#                       root 1003 F.... inputplumber
```

InputPlumber wins the exclusive `EVIOCGRAB`, so HHD's controller emulation can never
open the device and retries forever.

### Fix

Pause **only** HHD's controller emulation, keeping its TDP, RGB, IMU and PPD features
(which do work). Edit `/etc/hhd/state.yml`:

```yaml
controllers:
  handheld:
    controller_mode:
      mode: disabled        # was: default
```

Then:

```bash
sudo systemctl restart hhd_local@$(whoami).service
```

### Verify

```bash
sudo tail -20 /etc/hhd/log/hhd.log | grep -c 'Launching emulated controller'   # 0
sudo tail -20 /etc/hhd/log/hhd.log | grep -c 'Device or resource busy'         # 0
sudo fuser -v /dev/input/event6      # inputplumber only, no hhd
```

HHD should still be `active`, and still logging TDP/RGB work.

> Choosing the other way round (let HHD own the pad, disable InputPlumber) is also
> valid, but InputPlumber is what provides the `deck-uhid` Steam Deck controller
> emulation that Steam expects on this image, so it is the less disruptive owner.

---

## 2. Right-stick pointer speed

### Symptom

Right stick moves the mouse, but far too fast, and KDE's pointer-speed slider barely
helps even at its minimum.

### Why KDE can't fix it

The pointer motion does **not** come from InputPlumber or HHD by default. The chain is:

```
physical pad (evdev, xpad)
  -> InputPlumber  -> "deck-uhid" target = Valve Steam Deck Controller (hidraw)
    -> Steam       -> Desktop Layout, right stick = joystick_mouse
      -> XTEST / XWayland pointer injection
```

Confirm Steam is the one doing it:

```bash
sudo fuser -v /dev/hidraw*      # steam holds the "Generic Steam Controller" node
pgrep -a Xwayland
```

Because Steam injects via XTEST rather than emitting evdev events, libinput's pointer
acceleration — which is what KDE's slider configures — is largely bypassed. You can
see this: monitoring every `/dev/input/event*` while moving the stick yields **zero**
`REL_X`/`REL_Y` events.

Steam's own Desktop Layout is also not much help: its right-stick group carries no
sensitivity or deadzone values at all, so it runs on Steam's internal defaults.

```bash
grep -c 'deadzone_inner_radius' ~/.local/share/Steam/controller_base/desktop_neptune.vdf
# the two hits belong to the LEFT stick's joystick_move groups, not the right stick
```

### Fix — drive the pointer from InputPlumber instead

InputPlumber already creates an unused `mouse` target. Mapping the right stick to it
gives a `speed_pps` knob and genuine 1-pixel proportional steps.

```bash
inputplumber devices list          # note the composite device id (usually 0)
inputplumber device 0 targets list # should list a "mouse" target
```

Start from your **current** profile so you keep existing mappings (the dial-to-
brightness/volume bindings on this device live there):

```bash
sudo mkdir -p /etc/inputplumber/profiles
inputplumber device 0 profile dump | sudo tee /etc/inputplumber/profiles/rightstick-mouse.yaml >/dev/null
sudo sed -i 's/^name: Default$/name: Default + RightStick Mouse/' \
    /etc/inputplumber/profiles/rightstick-mouse.yaml
```

Append the mapping (also in [`config/inputplumber-rightstick-mouse.yaml`](../config/inputplumber-rightstick-mouse.yaml)):

```yaml
- name: Right Stick Mouse
  source_event:
    gamepad:
      axis:
        name: RightStick
        deadzone: 0.05
  target_events:
  - mouse:
      motion:
        speed_pps: 350
```

Load it:

```bash
inputplumber device 0 profile load /etc/inputplumber/profiles/rightstick-mouse.yaml
inputplumber device 0 profile name
```

`speed_pps` defaults to 800; 350 is noticeably slower. Tune to taste — changes apply
on reload, no reboot needed.

> `deadzone` here has no effect on motion mappings. The schema documents it as
> *"When this deadzone threshold is crossed, this input is considered 'pressed'"* —
> i.e. it is for axis→**button** conversion. It is harmless to leave in.

### Make it survive a reboot

A loaded profile is in-memory only, and is lost on `systemctl restart inputplumber`.
Install the loader and unit from this repo:

```bash
sudo install -m755 scripts/ip-load-profile.sh /usr/local/bin/ip-load-profile.sh
sudo install -m644 systemd/inputplumber-profile.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now inputplumber-profile.service
```

The script retries for up to 60 s because InputPlumber's device detection is
asynchronous — a plain `After=inputplumber.service` alone races and fails.

### Verify

```bash
systemctl is-active inputplumber-profile.service     # active
inputplumber device 0 profile name                   # Default + RightStick Mouse
sudo python3 scripts/measure-stick.py --rel 20       # move the stick; expect REL steps of 1,2,3...
```

Fine-grained output looks like this — small step sizes are the point:

```
REL_X: distinct step sizes=6  smallest steps=[(1,100),(2,108),(3,81),(4,75),(5,69),(6,19)]
```

### Note on Steam

Steam's Desktop Layout keeps injecting its own motion at large deflections, so the two
add up. In practice they are complementary: InputPlumber supplies the slow, precise
low end that Steam never reached. If you would rather have only one, disable Steam's
desktop layout in *Steam → Settings → Controller*.

---

## 3. Right-stick deadzone — NOT fixable in software

### Symptom

The stick must be pushed a long way before the pointer moves at all, and motion is
asymmetric (right reaches full speed sooner than left).

### The measurement that settles it

Read the **raw kernel axis** with InputPlumber not managing the device, and hold
positions steady. Sweeping is misleading: fast transits produce transient low values
that make the hardware look better than it is. (This cost us a wrong conclusion —
initially the deadzone was blamed on InputPlumber.)

```bash
inputplumber device 0 stop            # release the exclusive grab
sudo python3 scripts/measure-stick.py --abs 30 /dev/input/event6
# ...hold the stick at a quarter, then half, then full deflection...
sudo systemctl restart inputplumber   # restore
```

Pass the **physical** pad's event node explicitly. Auto-detection will happily pick
the *emulated* one (`Microsoft X-Box 360 pad 0`), which tells you nothing about the
hardware. Identify the real one with:

```bash
grep -l 'X-Box 360 pad$' /sys/class/input/event*/device/name
ls -l /dev/input/by-path/ | grep joystick
```

Measured on this device (`ABS_RX`, full scale 32767):

```
held ~quarter travel   ->  25344   ( 77.3% FS)
held ~half travel      ->  28160   ( 85.9% FS)
held full              ->  32767   (100.0% FS)
on release             ->  25088 -> 0     (76.6% straight to zero)
```

Lowest non-zero magnitude ever observed at steady state: **16896 ≈ 51.6% of FS**.

So the pad reports **nothing** below roughly half its electrical range, and a quarter
of mechanical travel already reads 77%. The axis is also 8-bit in practice — every
value is a multiple of 256 — and asymmetric, spanning **−28160 … +32767**, i.e. the
electrical centre sits offset toward the right. That asymmetry is exactly why right
feels faster than left.

The evdev axis itself declares almost no deadzone, so this is not a kernel hint:

```
ABS_RX   min -32768   max 32767   fuzz 16   flat 128     (flat = 0.4% of range)
```

`xpad` adds no deadzone either. The device presents as `045e:028e Microsoft Corp.
Xbox360 Controller`, so the dead band and the compressed curve are in **AYANEO's
firmware**, upstream of the kernel.

### Why no software fix exists

There is no data below ~50% to rescale, expand or curve. Anti-deadzone, response
curves and `quadratic_scaling` can only redistribute values that arrive; they cannot
invent the missing half. Anything claiming otherwise is just amplifying the jump.

### What might actually help

- **BIOS** — check for stick calibration / deadzone options in setup.
- **AYASpace under Windows** — on some AYANEO models its stick calibration is written
  to firmware and therefore persists into Linux.
- **Controller mode switch** — InputPlumber's device config for the SLIDE also lists a
  `Nintendo Co., Ltd. Pro Controller` source, so the hardware can present as a Switch
  Pro Controller. A different firmware mode may use a different curve.
- **Suspect the hardware** — a ~50% dead band with an offset centre is extreme. Worth
  comparing against another unit, or testing the stick under Windows, before assuming
  it is normal for the model.

---

## Diagnostic methodology

Worth reusing on any handheld, and it is where the real conclusions came from.

**Find who actually generates pointer motion.** Monitor every `/dev/input/event*` at
once. If a stick moves the pointer but no device emits `REL_X`/`REL_Y`, motion is
being injected above evdev (XTEST, or a Wayland virtual-pointer protocol) — look at
what holds `/dev/hidraw*` instead.

**Include a control in every capture.** Press a keyboard key during the run. If the
keyboard shows up and the stick does not, your reader works and the device is simply
grabbed — rather than your script being broken. This distinction wasted a cycle here.

**Self-timestamp subjective events.** To capture "the value at the moment the pointer
started moving", have the tester press a gamepad button at that instant. The press
lands in the same report stream, so no clock correlation is needed.

**Hold, don't sweep.** Steady-state values are the truth. Sweeps sample transients and
will overstate what the hardware reports near centre.

**Compare the same input at two layers.** Reading raw evdev with the manager stopped,
versus the emulated device's HID reports, is what localises a transformation to a
specific component.

**Re-glob devices during long captures.** Daemons tear down and recreate virtual
devices, so a device list captured once at startup can go stale mid-run.
