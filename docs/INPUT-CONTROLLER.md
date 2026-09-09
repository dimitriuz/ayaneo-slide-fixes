# Controller & stick-as-mouse findings

Input issues on the **AYANEO SLIDE** (CachyOS handheld image), unrelated to the
[backlight fixes](../README.md).

Three separate things. **Two are fixable, one is a hardware fault with a good
workaround** — worth knowing which is which before spending time on it.

| # | Issue | Outcome |
|---|---|---|
| 1 | Handheld Daemon crash-looping every 3 s | **Fixed** — config |
| 2 | Stick pointer far too fast, not tunable | **Fixed** — InputPlumber profile |
| 3 | **Both** sticks have a large deadzone (L 15-25%, R 30-50%) | **Root cause found, not yet fixed** — a stored setting only AYASpace can write; use the **left** stick meanwhile |

---

## 1. Handheld Daemon (HHD) crash loop

### Symptom

Nothing visible, but `/etc/hhd/log/hhd.log` grows forever:

```
ERROR  <class 'OSError'>: [Errno 16] Device or resource busy
ERROR  Assuming controllers disconnected, restarting after 3s.
INFO   Launching emulated controller.
```

Every three seconds since boot.

### Cause

CachyOS's handheld image ships **both** `cachyos-handheld` (HHD) and
`inputplumber`, and both try to manage the gamepad:

```bash
sudo fuser -v /dev/input/event6        # your gamepad's event node
#   /dev/input/event6:  root  994 F.... hhd
#                       root 1003 F.... inputplumber
```

InputPlumber wins the exclusive `EVIOCGRAB`, so HHD's controller emulation can never
open the device and retries forever.

### Fix

Pause **only** HHD's controller emulation, keeping its TDP, RGB, IMU and PPD features
(those work fine). In `/etc/hhd/state.yml`:

```yaml
controllers:
  handheld:
    controller_mode:
      mode: disabled        # was: default
```

```bash
sudo systemctl restart hhd_local@$(whoami).service
```

### Verify

```bash
sudo tail -20 /etc/hhd/log/hhd.log | grep -c 'Launching emulated controller'   # 0
sudo tail -20 /etc/hhd/log/hhd.log | grep -c 'Device or resource busy'         # 0
sudo fuser -v /dev/input/event6      # inputplumber only
```

HHD stays `active` and keeps logging TDP/RGB work.

> Letting HHD own the pad and disabling InputPlumber instead is also valid, but
> InputPlumber provides the `deck-uhid` Steam Deck controller emulation that Steam
> expects on this image, so it is the less disruptive owner.

---

## 2. Stick-as-mouse pointer speed

### Symptom

A stick moves the mouse pointer, far too fast, and KDE's pointer-speed slider barely
helps even at minimum.

### Why KDE can't fix it

By default the pointer motion does **not** come from InputPlumber or HHD. The chain is:

```
physical pad (evdev, xpad)
  -> InputPlumber -> "deck-uhid" target = Valve Steam Deck Controller (hidraw)
    -> Steam      -> Desktop Layout, right stick = joystick_mouse
      -> XTEST / XWayland pointer injection
```

Confirm Steam is doing it:

```bash
sudo fuser -v /dev/hidraw*      # steam holds the Steam Controller node
pgrep -a Xwayland
```

Because Steam injects via XTEST rather than emitting evdev events, libinput's pointer
acceleration — which is what KDE's slider configures — is bypassed. The tell is that
monitoring every `/dev/input/event*` while moving the stick yields **zero**
`REL_X`/`REL_Y` events.

Steam's own Desktop Layout offers little either: its right-stick group carries no
sensitivity or deadzone values, so it runs on Steam's internal defaults.

### Fix — drive the pointer from InputPlumber

InputPlumber already creates an unused `mouse` target. Mapping a stick to it gives a
`speed_pps` knob and genuine 1-pixel proportional steps.

```bash
inputplumber devices list          # note the composite device id (usually 0)
inputplumber device 0 targets list # should list a "mouse" target
```

Start from your **current** profile so existing mappings survive (the dial-to-
brightness/volume bindings on this device live there):

```bash
sudo mkdir -p /etc/inputplumber/profiles
inputplumber device 0 profile dump | sudo tee /etc/inputplumber/profiles/stick-mouse.yaml >/dev/null
sudo sed -i 's/^name: Default$/name: Default + Stick Mouse/' \
    /etc/inputplumber/profiles/stick-mouse.yaml
```

Append the mappings from
[`config/inputplumber-stick-mouse.yaml`](../config/inputplumber-stick-mouse.yaml):

```yaml
- name: Left Stick Mouse
  source_event:
    gamepad:
      axis:
        name: LeftStick
  target_events:
  - gamepad:                 # keep normal gamepad behaviour for games
      axis:
        name: LeftStick
  - mouse:
      motion:
        speed_pps: 700
```

```bash
inputplumber device 0 profile load /etc/inputplumber/profiles/stick-mouse.yaml
inputplumber device 0 profile name
```

**Two things that are easy to get wrong:**

* **List the gamepad target too.** Without the `gamepad:` entry the mapping *replaces*
  the stick's normal function and you lose stick input in games. Listing both
  destinations sends the axis to each.
* **`deadzone` does nothing here.** The schema documents it as *"when this deadzone
  threshold is crossed, this input is considered 'pressed'"* — it is for
  axis→**button** conversion, not motion. Harmless to leave in, useless for this.

`speed_pps` defaults to 800; changes apply on reload, no reboot.

### Make it survive a reboot

A loaded profile is in-memory only, and is lost on `systemctl restart inputplumber`:

```bash
sudo install -m755 scripts/ip-load-profile.sh /usr/local/bin/ip-load-profile.sh
sudo install -m644 systemd/inputplumber-profile.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now inputplumber-profile.service
```

The script retries for up to 60 s because InputPlumber's device detection is
asynchronous — a bare `After=inputplumber.service` races and fails.

### Verify

```bash
systemctl is-active inputplumber-profile.service   # active
inputplumber device 0 profile name                 # Default + Stick Mouse
sudo ./scripts/stickcheck.py                       # live; Ctrl+C to finish
```

`stickcheck` reads the emulated gamepad and the mouse target simultaneously, so you
can confirm a stick feeds **both** — i.e. that you did not steal the sticks from games:

```
GAMEPAD  L( 12345,  -234) R(     0,     0) ev=1204  |  MOUSE  dx=   3 dy=  -1 ev=890
```

Fine-grained pointer output looks like small step sizes, which is the whole point:

```
REL_X: distinct step sizes=6  smallest=[(1,100),(2,108),(3,81),(4,75),(5,69),(6,19)]
```

### Note on Steam

Steam's Desktop Layout keeps injecting its own motion at large deflections, so the two
add up. In practice they are complementary — InputPlumber supplies the slow, precise
low end Steam never reached. To have only one, disable the desktop layout in
*Steam → Settings → Controller*.

---

## 3. The stick deadzones — stored configuration, not (only) worn hardware

### Symptom

The **right** stick must be pushed roughly halfway before the pointer moves at all,
and then it moves fast. Motion is asymmetric: right reaches full speed sooner than
left.

### Measure it properly: hold, don't sweep

Sweeping is misleading — fast transits produce transient low values that make the
hardware look better than it is. **This cost us a wrong conclusion:** sweep captures
initially made it look as though InputPlumber was discarding everything below 50%, and
only steady-state holds showed the pad itself does it.

Use the live tool, which only counts values you sustain for ≥0.3 s and ignores centre
noise below 512:

```bash
sudo ./scripts/sticklive.py     # releases InputPlumber, restores it on Ctrl+C
```

Or a one-shot capture:

```bash
inputplumber device 0 stop
sudo python3 scripts/measure-stick.py --abs 30 /dev/input/event6
sudo systemctl restart inputplumber
```

Pass the **physical** pad's node explicitly. Auto-detection will happily pick the
*emulated* one (`Microsoft X-Box 360 pad 0`), which tells you nothing about the
hardware:

```bash
grep -l 'X-Box 360 pad$' /sys/class/input/event*/device/name
```

### What we measured

Steady-state `ABS_RX`, full scale 32767:

```
held ~quarter travel   ->  25344   ( 77.3% FS)
held ~half travel      ->  28160   ( 85.9% FS)
held full              ->  32767   (100.0% FS)
on release             ->  25088 -> 0        (76.6% straight to zero)
```

Lowest non-zero magnitude at steady state: **16896 ≈ 51.6% FS**. So the pad reports
nothing below about half its electrical range, and a quarter of mechanical travel
already reads 77%. Every value is a multiple of 256 — the pad is 8-bit in practice —
and the range is asymmetric, **−28160 … +32767**, i.e. the electrical centre sits
offset toward the right. That asymmetry is exactly why right feels faster than left.

This is not a kernel hint. The evdev axis declares almost no deadzone:

```
ABS_RX   min -32768   max 32767   fuzz 16   flat 128     (flat = 0.4% of range)
```

`xpad` adds none either.

### The decisive test: compare the two sticks

Same firmware, same driver, same mapping, same `speed_pps` — the only variable is
which stick. On this device the **left stick gives fine control at small deflections
and the right cannot**. That rules out a firmware deadzone *policy* and points at the
right stick specifically.

**If you take one thing from this document, it is to test both sticks before blaming
software.** It is a two-minute comparison that invalidates whole classes of theory.

### What it is not

Ruled out with evidence, so you do not have to repeat it:

| Hypothesis | Evidence against |
|---|---|
| InputPlumber applies the deadzone | Steady-state raw evdev shows the same ~50% floor with InputPlumber stopped |
| Steam applies it | Steam's deadzone slider at minimum changes nothing; Steam is downstream |
| KDE pointer settings | `PointerAcceleration=-1.000` already minimum, and XTEST bypasses libinput |
| Kernel/`xpad` deadzone | `flat = 128` = 0.4% of range |
| A BIOS setting | See below — nothing in the firmware image |
| Firmware deadzone policy | The **left** stick on the same device is fine |

### The BIOS dive — dead end

The controller is **not** the EC and **not** in the BIOS. Its USB identity:

```
idVendor/idProduct : 045e:028e   (spoofing Microsoft Xbox360)
iManufacturer      : ZhiXu       <- the real vendor
iProduct           : Controller
bcdDevice          : 1.10        <- its own firmware version
bNumInterfaces     : 1           (vendor-specific class 255)
```

A dedicated **ZhiXu MCU** on an internal USB port with its own firmware. Searching the
32 MB AMI Aptio image (`AS01-BIOS-20240606`, raw plus the four LZMA volumes,
~9.5 MB decompressed) found:

```
"ZhiXu" anywhere:                        0 hits
045e:028e VID/PID byte pattern:          0 hits
stick / joystick / deadzone Setup strings: 0 hits
```

The EC-related Setup strings that do exist (`EC FW Version`, `EC FIRMWARE Update`,
eSPI routing, UCSI) are unrelated. The flash script uses `/p /b /n /k /x` with **no
`/E`** EC block, consistent with the controller firmware living elsewhere.

*Caveat:* only the LZMA volumes were decompressed. UEFI also uses Tiano/EFI
compression, so something could in principle hide in an unpacked section — but zero
hits for the MCU's own vendor name *and* the VID/PID it advertises makes that unlikely.

### The official calibration — registers, but did not help

AYANEO documents a hardware calibration ([official KB][kb]):

1. Sticks at rest
2. Hold the **⧉ View button (lower left)** + **all four ABXY buttons** — five at once
3. Hold until the device **vibrates twice**
4. Within ~10 s, **rotate both sticks 2–3 full circles** and **fully press/release both
   triggers ~3×**
5. It vibrates again when done

Two gotchas found the hard way:

* **There is no vibration feedback under Linux.** No device reports any force-feedback
  capability (`EVIOCGBIT(EV_FF)` is empty on every node), so the confirmation cue never
  fires. Absence of vibration does **not** mean the combo failed — judge by measurement.
* **Verify the combo registers.** All five buttons must be seen simultaneously:
  ```
  held=['A (South)', 'B (East)', 'SELECT/View', 'X (North)', 'Y (West)']
  ```
  Note that triggers are analog (`ABS_Z`/`ABS_RZ`), so they never appear in a button
  monitor at all.

On this device the combo registered correctly and the deadzone did not improve —
consistent with a degraded sensor rather than a lost calibration, since calibration can
correct a centre offset but not a dead sensor region.

### Not applicable: x20ctl

[x20ctl][x20] reverse-engineers the configuration protocol of ShenZhen ZhiXu chips —
the same vendor — and exposes **inner/outer deadzone plus response curves on a 0–100
scale**, shipping at **8/100** on the EasySMX X20. Tempting, but it does not apply:

* it is **Bluetooth LE GATT** (service `d7f010e0-…`, advertising as `Xpert2`), and this
  controller is internal USB with no BLE peripheral;
* its README explicitly warns **"Don't identify a pad by USB VID/PID"** — `045E:028E` is
  a generic clone ID, so the matching ID means nothing;
* no AYANEO devices are supported, and even EasySMX's own X05 was found incompatible.

It is still useful as corroboration: these chips store a deadzone in flash on a 0–100
scale, typically single digits. Ours behaves like ~50 — six times out of spec, which
reads as a fault, not a design choice.

### Controller firmware — the one remaining real avenue

The stick behaviour may be a **stored setting** rather than a dead sensor: AYASpace
under Windows can write a deadzone into the controller, and that would persist into
Linux because the MCU applies it autonomously. Re-flashing the controller firmware
should reset such a setting to defaults.

AYANEO ships a controller firmware for this board (`AS01_20231127_V02.bin`, 20 KB) and
the update procedure is **OS-agnostic** — no Windows needed:

```
1. Turn off for more than 1 minute.
2. Hold down the left joystick to turn on.
3. A USB disk named AYANEO appears. Copy the firmware .bin onto it and the
   controller updates automatically.
```

Holding the left stick at power-on puts the MCU into a **USB mass-storage
bootloader**, so on Linux you simply mount the device and copy the file:

```bash
# after booting with the left stick held:
lsblk -o NAME,LABEL,SIZE,MOUNTPOINT
sudo dmesg | tail -20            # look for a small removable disk labelled AYANEO
```

**Check for the bootloader disk before flashing anything** — entering that mode and
rebooting normally is harmless, and confirms the procedure works on your unit.

Two honest caveats:

* The image is **encrypted or compressed** (entropy 7.99 bits/byte, no strings, no
  `AS01` reference inside), so it cannot be inspected or edited. You are trusting the
  filename and the official source.
* **Version comparison is not possible.** The device reports `bcdDevice 1.10`, the file
  is named `V02` and dated 2023-11-27 — older than the 2024-06 BIOS. There is no way to
  tell from the outside whether this is an upgrade or a downgrade, so flashing carries
  a real risk of regressing other controller behaviour.

### EC firmware — not relevant

`SLIDE_EC_20240628` (ITE `IT557xE`, `ITE_Eii_00_AS01_A_V02001B.T6.bin`, 128 KB) flashes
from a UEFI shell off a FAT32 stick via `ifu.efi` / `Startup.nsh` — also Windows-free.
But its own readme states its purpose: *"Realize bypass power supply function"* (battery
bypass charging). Nothing to do with the sticks, so there is no reason to take that
risk for this problem.

### The workaround that actually works

**Map the left stick to the mouse instead.** It reports small deflections normally, so
you get real proportional control — slow when barely deflected, faster as you push.
That is section 2 above, and it is why the shipped config maps `LeftStick`.

### If you want it properly fixed

* **Warranty / support.** The evidence here is unusually strong for a ticket: raw kernel
  axis values showing the right stick reports nothing below ~50% at steady state while
  the left behaves normally, on a device where the documented calibration registers but
  does not help.
* **AYASpace under Windows** exposes a deadzone setting and a joystick correction
  function ([AYANEO on deadzone and Hall sticks][hall], [AYASpace manual][aya]).
* **Controller firmware** would come from AYANEO as a separate MCU updater, not a BIOS.
  Quote `ZhiXu`, `bcdDevice 1.10`.
* **Software anti-deadzone** cannot recover the missing range. Rescaling the usable
  50–100% band onto 0–100% (`out = sign(x)·(|x|−16384)/16384·32767`) makes the band that
  *does* report behave proportionally, but the dead mechanical travel remains dead.

---

### It is a stored setting, and only AYASpace can write it

Measured thresholds - the deflection at which motion *starts*, per direction:

| | horizontal | vertical |
|---|---|---|
| **RIGHT** | -30% ... +50% | -40% ... +30% |
| **LEFT**  | -15% ... +15% | -15% ... +25% |

Two conclusions follow:

* **Both** sticks are affected. A 15-25% deadzone on the left is still very large
  (ZhiXu-based pads typically ship at 8/100). So this is not one dead sensor.
* **All four axes are asymmetric**, each with a different offset. Four independently
  worn sensors would be a remarkable coincidence; bad stored calibration would not.

This corrects an earlier conclusion in this document's history that the right stick was
simply faulty silicon. It is more likely miscalibration affecting both sticks, worse on
the right.

**AYASpace confirms the setting exists.** Under
`Assistant -> EVO -> Master Controller -> Joystick` it exposes a deadzone on/off toggle
and separate left/right sensitivity, and under
`Assistant -> EVO -> Configure AYANEO -> Joystick/Button Correction` a correction
routine for a shifted centre. So the deadzone is a per-stick, writable value.

**And a controller firmware reflash does not clear it** (see above). Taken together,
the setting must live in storage that firmware updates deliberately preserve - which is
normal design, since you do not want an update to wipe per-unit factory calibration.

### Running AYASpace in a VM: works for inspection, cannot apply

Worth documenting because it is the obvious thing to try and it *half* works.

[`scripts/winvm.sh`](../scripts/winvm.sh) builds a Windows 11 guest with QEMU/KVM:
swtpm for the TPM 2.0 requirement, OVMF, the controller passed through by vid:pid, an
optional Logitech receiver for keyboard/touchpad, and **SMBIOS spoofed as
`AYANEO / SLIDE`** so AYASpace does not reject the machine. Nothing is repartitioned;
the guest is a qcow2 file.

Result: AYASpace installs, recognises the machine and shows the joystick pages - but
**every write fails with "check connection"**, and all live values (battery, TDP, fan)
are dead. AYASpace reaches the hardware through the **EC** via its own kernel driver,
and a VM has no EC. Passing the gamepad through gives it the pad, not the EC.

That also explains the write path: if the deadzone is written **EC -> controller MCU**,
it is stored somewhere a controller firmware flash does not erase. Consistent with
everything observed.

**So the remaining route is real hardware.** Windows To Go on an external SSD gives a
genuine EC without touching the internal disk - and the VM is still useful for building
that drive, since Rufus can write Windows To Go from inside the guest.

Gotchas found while doing this, in case you repeat it:

* `oobe\bypassnro` was **removed in Windows 11 24H2+**. On 24H2/25H2 use
  `start ms-cxh:localonly` from a Shift+F10 prompt, or launch the VM with no NIC at all
  (`NONET=1`) so OOBE cannot demand a Microsoft account.
* The emulated `usb-tablet` pointer is unreliable in Windows; passing a real USB
  keyboard/mouse receiver through is far less painful. Note the **host loses that device**
  while the VM runs, so keep another input method available.
* Boot the ISO only until Windows is installed. Leaving the CD ahead of the disk in the
  boot order restarts Setup instead of resuming OOBE.

## Reverse engineering AYASpace: the vendor HID channel

Static analysis of `AYASpaceCef.exe` 3.2.0.4 (extracted from a VM install with
`qemu-nbd` + `ntfs-3g -o ro,force`) identified the control channel exactly.

**The channel**

```
device : USB VID 1C4F PID 007C   (the slide-out keyboard's MCU - not the gamepad)
iface  : the one whose report descriptor opens Usage Page 0xFF00, Usage 0x02
         (this is the "02" in AYASpace's Windows path filter "&mi_02#")
report : FEATURE, Report ID 0x41, 7 data bytes, WRITE-ONLY - GET_REPORT stalls
```

Confirmed from the device's own report descriptor on `hidraw1`:

```
06 00 ff   Usage Page (Vendor 0xFF00)
09 02      Usage 0x02
85 41      Report ID 0x41
75 08 95 07  Report Size 8 x Count 7
b1 02      FEATURE (Data,Var,Abs)
```

**How the JS UI reaches native code.** CEF handlers are registered with the
idiom `lea rax,[handler]; lea rdx,[name]; call register`, so the handler is the
`lea rax` immediately preceding each name string:

| JS method | handler |
|---|---|
| `master.set_stick_deadzone` | `0x140305a20` |
| `master.set_hall_stick` | adjacent registration |
| `super_joy.set_stick_deadzone` | `0x1400d70e0` |
| `super_joy.get_stick_deadzone` | `0x1400d6e70` |

`master.*` is the built-in controller (the AYASpace "Master Controller" menu).
`super_joy.*` is AYANEO's separate *Super Joy* accessory - a false trail that
cost a couple of hours, so do not start there.

`master.set_stick_deadzone` takes `enable`, `StickDeadZone`, `data`, and the
enable path resolves to a config byte where the **high nibble is the enable bit,
inverted** (it is really a *disable* flag), low nibble preserved.

**Where static analysis stops.** The chain from the handler to the transport
runs through C++ virtual dispatch, which `objdump` cannot resolve. A BFS over
198 functions to depth 6 reached none of: the EC port helpers, libusb, ViGEm, or
the HID feature builder. Finishing this needs either a decompiler with vtable
analysis (Ghidra) or - far cheaper - a USB capture of report `0x41`. See
[CAPTURE-DEADZONE.md](CAPTURE-DEADZONE.md).

**Other things the same binary gave up**

* *Keyboard backlight*: HID feature report, `report[0]=0x02`, `report[1]=` value
  `0..100` (plus `110` as a special case), 9 bytes total. Directly implementable.
* *EC access*: ITE SuperIO on ports `0x4E/0x4F` - unlock `87 01 55 55`, select
  LDN 4 via reg `0x07`, read the EC base from regs `0x60`/`0x61`, exit with
  `0xAA`. Used for TDP/RGB/fan, **not** for the sticks.

**Why a VM cannot apply the setting.** With the gamepad *and* the keyboard MCU
both passed through (verified: both interfaces showed `driver=usbfs`, the host
lost `hidraw0/1`), AYASpace still failed with "check connection", and every live
value (battery, TDP, fan) was dead. AYASpace gates on the **EC**, which sits
behind I/O ports `0x4E/0x4F` and cannot be virtualised. Real hardware - Windows
To Go on external media - is the only route.

## Diagnostic methodology

Reusable on any handheld, and where the real conclusions came from.

**Test both sticks first.** The cheapest test with the highest information content. A
difference between them eliminates every software-policy explanation at once.

**Hold, don't sweep.** Steady-state values are the truth. Sweeps sample transients and
overstate what the hardware reports near centre — this produced a wrong conclusion here.

**Beware your own "steady" metric.** An axis resting at ±1 will register as a
"sustained non-zero value" and look like fine control. Ignore magnitudes below a noise
floor (512 works on a 16-bit axis).

**Find who actually generates pointer motion.** Monitor every `/dev/input/event*` at
once. If a stick moves the pointer but nothing emits `REL_X`/`REL_Y`, motion is injected
above evdev (XTEST, or a Wayland virtual-pointer protocol) — look at what holds
`/dev/hidraw*` instead.

**Include a control in every capture.** Press a keyboard key during the run. If the
keyboard appears and the stick does not, your reader works and the device is simply
grabbed, rather than your script being broken.

**Self-timestamp subjective events.** To capture "the value at the moment the pointer
started moving", have the tester press a gamepad button at that instant — the press
lands in the same report stream, so no clock correlation is needed.

**Give the tester a live readout, not a timed capture.** Fixed-duration background
captures make the person race a clock they cannot see, and produce empty runs. A live
tool they stop with Ctrl+C is strictly better.

**Compare the same input at two layers.** Reading raw evdev with the manager stopped
versus the emulated device's HID reports is what localises a transformation to a
component.

**Re-glob devices during long captures.** Daemons tear down and recreate virtual
devices, so a device list captured once at startup can go stale mid-run.

[kb]: https://help.ayaneo.com/doku.php?id=ayaneo:common_fault_solutions:what_should_i_do_if_the_joystick_and_ltrt_malfunction_or_drift
[hall]: https://www.ayaneo.com/article/263
[aya]: https://www.ayaneo.com/article/262
[x20]: https://github.com/AmjadAAYD/x20ctl
