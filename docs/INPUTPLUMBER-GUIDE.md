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
