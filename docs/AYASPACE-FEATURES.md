# What AYASpace exposes, and how much of it Linux can reach

A complete inventory of the AYASpace 3.2.0.4 hardware API, the transport behind
each feature, and whether Linux can already do it. Companion to
[GAMEPAD-PROTOCOL.md](GAMEPAD-PROTOCOL.md), which covers the gamepad UART in
detail.

## Three sources, in order of usefulness

Reverse-engineering this got much cheaper once it became clear where to look.

**1. `web/frontend/*.js` — the authoritative source for enums.** AYASpace is a
CEF app; its whole UI is unminified-enough webpack bundles sitting in
`C:\Program Files (x86)\AYASpace\web\frontend`. Every option list, every level
name, and every `AYASpaceApi("...")` call site is in there in plain text. This
is where the meaning of numeric fields lives, and no amount of decompiling the
native side recovers it.

**2. `%APPDATA%\AYASpace\database.db` — a SQLite file with the live config.**
`Config` is a key/value table holding the current state of nearly everything,
which doubles as a **known-good restore value** before poking at hardware.

```sql
sqlite3 database.db 'select key, val from Config;'
```

```
RGBIson              0
RGBBLeftrightness    100          -- their typo, not ours
RGBRightBrightness   100
RGBMode              0
LsColor              -1
RsColor              -1
KeyBoardLightConfig  {"brightness":10,"color":12287,"enable":1,"fnIson":0,"mode":1}
TouchpadCfg          { ...full trackpad key mapping... }
FanAutoByAya         1
```

**3. `AYASpaceCef.exe` — the transports.** The JS calls `AYASpaceApi(name, args)`;
each `name` maps to a native handler registered by the idiom described in
GAMEPAD-PROTOCOL.md. Decompile only what you need the *wire format* of.

## The API surface

`strings AYASpaceCef.exe | grep -E '^[a-z_]+\.[a-z_0-9]+$'` gives the whole
handler namespace. The hardware-relevant parts:

| namespace | what it covers |
|---|---|
| `master.*` | the GuLiKit gamepad MCU — 12 handlers, all one UART record |
| `rgb.*` | keyboard backlight, stick ring LEDs, AYA logo light |
| `key.*` | hardware button remap, on-screen keyboard, quick menu |
| `controller.*` | detachable-controller and mini-PC features |
| `gamepad.*`, `gamepad_transfer.*` | per-model variants (GEEK etc.), ViGEm plumbing |
| `touchpad.*` | trackpad key mapping (models that have trackpads) |
| `fancontrol.*`, `system.*` | fan curves, TDP, charge policy |
| `super_joy.*` | the external Super Joy accessory — **not** the built-in pad |

`super_joy.*` is a trap: it has an inviting `set_stick_deadzone` /
`set_stick_sensitivity` pair that has nothing to do with the built-in
controller. `master.*` is the built-in pad.

## Status per feature

| feature | transport | Linux |
|---|---|---|
| Stick deadzone | GuLiKit UART, record byte 4 hi | **`gulikit-ctl set --deadzone`** |
| Per-stick sensitivity | UART, byte 3 nibbles | **`gulikit-ctl set --left/--right`** |
| Trigger L2/R2 levels | UART, byte 1 nibbles | **`gulikit-ctl set --trigger-l2/-r2`** |
| Gyro L1/L2 levels | UART, byte 2 nibbles | **`gulikit-ctl set --gyro-l1/-l2`** |
| Rumble level | UART, byte 4 lo | **`gulikit-ctl set --rumble`** |
| Turbo per button | UART, bytes 5-7 nibbles | **`gulikit-ctl set --turbo-a`** … |
| Swap ABXY | UART, byte 8 bit 0x10 | **`gulikit-ctl set --swap-abxy`** |
| Restore factory defaults | UART, factory record | **`gulikit-ctl factory-reset`** |
| **Keyboard backlight** | HID feature report `0x41` | **`ayaneo-kbdlight`** |
| Stick ring LEDs | EC | already works — [see below](#the-stick-ring-leds-need-no-reverse-engineering) |
| TDP / power / fan | EC, ACPI | already works — HHD, `platform_profile` |
| Hardware button remap | `key.*`, not investigated | InputPlumber already remaps these better |
| Back-key remap | UART bytes 9-12 — **not sent on the SLIDE** | InputPlumber (profile paddles) |
| Trackpad mapping | `TouchpadCfg` | N/A — the SLIDE has no trackpads |

## Feature parity with AYASpace

Checked against AYASpace running on the same unit.

| AYASpace feature | status |
|---|---|
| Vibration low/medium/high/off | covered — Controller → Feel → Rumble |
| Gyro enable/disable | covered — Controller → Gyro (levels, with off); AYASpace has no gyro UI here at all |
| Keyboard gradient | covered — its colour is ignored by the firmware, so the picker is disabled in that mode |
| Desktop layout — map physical buttons | **covered** — Input → Buttons, see below |
| Joystick ring effects | **partly** — Breathe and Rainbow; Radar and Ripple are not reachable, see below |
| Charge policy, regular/bypass charge | **implemented, dead in hardware** — does nothing in AYASpace on this unit either. [CHARGING.md](CHARGING.md) |
| Motion EX | **not portable** — see below |
| VRAM size | **not safely reachable** — see below |

### The extra buttons were never dead

LC and RC appear to do nothing on Linux. They are in fact fully delivered: the
`aya5` capability map converts the chords the keyboard MCU sends —

```yaml
  - name: LC   source: Ctrl+Meta+F15  →  gamepad button LeftTop
  - name: RC   source: Ctrl+Meta+F16  →  gamepad button RightTop
```

— and the stock profile then maps those to **Elite paddles**. With an Xbox 360
target selected, the emulated pad has no paddles, so the events are translated
and then dropped. Nothing logs this.

Input → Buttons rebinds them, or from a script:

```
$ ayaneo-tray --map
  buttons: LeftTop (LC), RightTop (RC), QuickAccess (Custom)
  actions: none, esc, app, osk, guide, paddle
$ ayaneo-tray --map RightTop esc
```

Three traps worth knowing. Bindings are edits to the *running* profile, so
loading a profile discards them — they are saved and re-applied, like the
pointer settings. `SetTargetDevices` replaces the entire target set, so the
`dbus` target has to be passed every time or the UI actions (`app`, `osk`) have
nowhere to land. And an open settings window owns the settings file — it holds
the whole struct in memory and rewrites it on any change — so `--map` hands the
binding to a running window over the IPC socket instead of writing underneath
it, and only writes directly when nothing is open.

`app` maps a button to InputPlumber's `ui_quick` DBus action, which the tray
listens for — that is how the window opens with no pointer attached.

`paddle` is an Xbox Elite back paddle: the extra buttons that pad has and a
standard controller does not, which games can bind separately from the face
buttons. Which paddle depends on the button — LC becomes `LeftPaddle1`, RC
`RightPaddle1` — and **it only exists on the Xbox Elite target**. On any other
target the press is dropped, which is the same trap that made these buttons look
dead in the first place.

### Ring effects are host-side animations, not EC ones

`rgb.set_mode` stores a mode and wakes a worker thread, which switches on it:

```c
switch(*(undefined4 *)(*param_1 + 4)) {
  case 0: FUN_14034afa0(...); FUN_14034aee0(uVar5, 10000);
  case 1: FUN_14034a6d0(...);
  case 2: FUN_14034a980(uVar5, uVar4);
  ...
}
```

A different animation function per mode, computing colours on the host and
pushing them to the EC in a loop. The EC only displays what it was last told,
which is why the driver "holds" the LEDs (`0xd187 = 0xa5`, `AYANEO_LED_MC_MODE_HOLD`).

So effects are ours to write, and Breathe and Rainbow are. Both the tray and the
window run the animator, since either can be the only one alive — the window
opens without the tray, and the tray outlives the window. Exactly one may drive
the LEDs or they fight and the rings stutter, so ownership is an `flock` the
kernel releases when its holder exits, and the survivor takes over by itself.
(The first version animated only in the tray, which meant quitting the tray
silently froze the effect.) The limit is the driver's interface:

```
$ cat /sys/class/leds/ayaneo:rgb:joystick_rings/multi_index
red green blue
```

One triple for both rings, with no way to address a quadrant. Radar, Ripple and
Google Breathe are positional, so they cannot be done through this interface at
all — only by driving the per-quadrant EC registers (`0xb3`–`0xbe` and the
mirrored bank) behind the driver's back.

### Motion EX does not exist to port

Its handler answers `driver_not_installed` / `service_not_running`: it needs
AYASpace's own kernel driver and service to inject motion into the emulated
controller. It is not a hardware setting, so there is nothing to reimplement —
the Linux equivalent is mapping the gyro in InputPlumber, which already has it
as a source (`iio_device0`).

### VRAM size is a Windows driver's WMI class

AYASpace writes a PowerShell script and runs it:

```
( Get-WmiObject UMAInterface -Namespace "root/wmi" ).SetUMASize($HexAdd)
```

That class is not exposed by this firmware. There is no `_WDG` in the DSDT or in
any of the twenty SSDTs, and `/sys/bus/wmi/devices/` is empty — so it is provided
by a Windows driver, not by ACPI-WMI, and there is nothing for Linux to call.

The setting itself lives in a UEFI setup blob — `AMD_PBS_SETUP` and
`AmdSetupPHX` are both present in `efivars` — so it could in principle be
changed by writing undocumented offsets in a firmware variable. That is a
bricking risk for no gain over the BIOS menu, which needs the same reboot.
**Change it in BIOS setup.**

Two things the SLIDE cannot use even though the API exists: record bytes 9-12
are only transmitted in the AYANEO KUN's 15-byte frame, and `master.set_back_key`
writes into them. And `touchpad.*` targets hardware this model does not have.

---

# The keyboard backlight

**Transport: a HID feature report, id `0x41`, 8 bytes**, to the keyboard MCU
(SiGma Micro, `1C4F:007C`). Not the EC, and not the gamepad UART.

Pick the device by capability rather than by name — the right interface is the
one whose HID report descriptor declares report id `0x41`:

```bash
# the descriptor byte pair 85 41 is "Report ID 0x41"
$ for h in /sys/class/hidraw/hidraw*; do
      grep -qa $'\x85\x41' $h/device/report_descriptor 2>/dev/null \
        && echo "/dev/$(basename $h)"
  done
/dev/hidraw1
```

`ayaneo-kbdlight` does this itself, so `--device` is only needed to override it.

The same physical keyboard exposes a second interface with no feature reports
at all, so matching on VID/PID alone picks the wrong one half the time.

## Report layout

```
    byte 0   0x41            report id
    byte 1   R               `color` >> 16
    byte 2   G               `color` >> 8
    byte 3   B               `color` & 0xff
    byte 4   mode            effect, 0-6
    byte 5   enable          0 = off, 1 = on
    byte 6   0x40 | fnIson   high nibble is a hardcoded 4
    byte 7   0x5A            terminator
```

Built at `0x140194a70`; `color` is split into R/G/B by `0x140194520`, and byte 6
is assembled by `0x1401945c0` (the `fnIson` bit) and `0x140194600` (the constant
`4`, which has exactly one caller and is never anything else).

## Effect modes — and the two lists that are easy to confuse

AYASpace has **two** unrelated effect lists in the same bundle, and picking the
wrong one gets you wrong labels that still look plausible. The keyboard's list
is built by `KeyboardLightMList`; the LED rings' by `rgbModeList`.

| value | keyboard (`KeyboardLight.mode[n]`) | stick rings (`rgbModeList`) |
|---|---|---|
| 0 | — | Default |
| 1 | **Monochrome** | Monochrome Breathe |
| 2 | **Gradient** | RGB Breathe |
| 3 | **Breathe** | Google Breathe |
| 4 | — | Radar |
| 5 | — | Ripple |
| 6 | — | Monochromatic Always On |

English labels are from `web/language/en_US.json`; every locale agrees.

Two traps in there:

* `KeyboardLightMList` returns only keys **1 and 2**, but `en_US.json` defines
  `KeyboardLight.mode[2]` as well, and **mode 3 works on the hardware**. The
  picker under-reports what the firmware does.
* `rgbModeList` filters itself by `ProductClass` — `show:!(B||O||T)` where
  `B = "KUN"`, `O = "AIRPlus"`, `T = "Slide"` — so on a SLIDE the ring picker
  shows six modes and hides `always`. This is the list a user sees as "about
  five effects", and it is *not* the keyboard's.

Sending a ring-only value to the keyboard is accepted and does nothing visible:
**Radar and Ripple are spatial effects**, sweeping across an array of LEDs.
A single-zone keyboard backlight has nothing to sweep.

## Colour presets

From `KeyboardLightCList` (the non-`FLIP_KB` branch, which is what a SLIDE uses):

```
002FFF   0FE6FB   27F95B   0800FF   FFEA00   FF0000
```

`002FFF` is the factory default. The ring presets are separate, from
`rgbDefColorList`'s `ProductClass==="Slide"` branch:

```
FFFFFF   FFD000   0091FF   08FF00   FF0000
```

## `brightness` is a no-op

`KeyBoardLightConfig` contains `"brightness":10`, the JS sends it, and the
native setter at `0x1401945b0` is **three instructions** — spill both
arguments, return:

```
1401945b0  MOV  byte ptr [RSP + 0x10], DL
1401945b4  MOV  qword ptr [RSP + 0x8], RCX
1401945b9  RET
```

There is no brightness field in the 8-byte report, so the value has never meant
anything. `ayaneo-kbdlight --brightness` therefore scales R/G/B client-side
instead, which is the only way to dim this backlight.

## No read-back

The MCU implements SET_REPORT and **STALLs GET_REPORT** on `0x41` — every
length returns `EPIPE`. AYASpace issues the read anyway (`0x140194e10`),
checks the return value, and carries on when it fails. So the current state
cannot be queried from the hardware, and `ayaneo-kbdlight` caches it in
`/var/lib/ayaneo/kbdlight.json`, mirroring `KeyBoardLightConfig`.

## Usage

```bash
sudo install -m755 scripts/ayaneo-kbdlight.py /usr/local/bin/ayaneo-kbdlight

sudo ayaneo-kbdlight                        # print cached state, write nothing
sudo ayaneo-kbdlight --color 00ff88
sudo ayaneo-kbdlight --color red --mode breathe
sudo ayaneo-kbdlight --mode gradient
sudo ayaneo-kbdlight --brightness 40        # scales RGB locally
sudo ayaneo-kbdlight --enable off
sudo ayaneo-kbdlight --fn on                # Fn indicator light
sudo ayaneo-kbdlight --reset                # AYASpace defaults
sudo ayaneo-kbdlight --raw '41 00 2f ff 01 01 40 5a'
```

To carry your Windows settings over, read them out of the SQLite config and
convert: `color` is a plain integer, so `12287` is `#002FFF`.

---

# The stick ring LEDs need no reverse-engineering

`ayaneo-platform` already drives them, and exposes a standard multicolor LED:

```bash
D=/sys/class/leds/ayaneo:rgb:joystick_rings

cat $D/multi_index                                # red green blue
echo "255 255 255" | sudo tee $D/multi_intensity  # colour
echo 128           | sudo tee $D/brightness       # overall level, max 255
echo 0             | sudo tee $D/brightness       # off
```

`brightness` scales whatever `multi_intensity` holds, so a colour with
`brightness 0` is off, not black-on. AYASpace's `RGBIson 0` in `database.db`
simply means the rings were left switched off — nothing is broken if they start
dark.

This does not survive a reboot. A udev rule or a small systemd unit writing the
two sysfs files at boot is enough:

```ini
# /etc/systemd/system/ayaneo-rings.service   (then: systemctl enable --now ayaneo-rings)
[Unit]
Description=Restore AYANEO joystick ring LEDs
After=multi-user.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/bin/sh -c 'echo "255 208 0" > /sys/class/leds/ayaneo:rgb:joystick_rings/multi_intensity'
ExecStart=/bin/sh -c 'echo 128 > /sys/class/leds/ayaneo:rgb:joystick_rings/brightness'

[Install]
WantedBy=multi-user.target
```

The effect *modes* in the table above are AYASpace's own animations, driven from
software; the kernel LED interface gives a static colour. Animating the rings on
Linux means writing the sysfs files on a timer, not setting a mode byte.
