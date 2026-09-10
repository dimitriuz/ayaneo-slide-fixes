# Where the stick settings are stored: the EC is ruled out

Ongoing investigation into where AYASpace stores the stick **deadzone** and
**sensitivity**, so they can be changed from Linux. The setting demonstrably
persists across operating systems, so *something* writes it to non-volatile
storage on the device.

## Tools in this repo

| script | purpose |
|---|---|
| `ecdump.py` / `ecdiff.py` | ACPI EC RAM (256 bytes via `ec_sys` debugfs) |
| `ayaec.py` | vendor EC page `0xD1xx` via ITE SuperIO — dump / diff / find |
| `ayaecfull.py` | the **whole 64KB** EC address space (256 pages), dump / diff |

The vendor EC access uses exactly the sequence `ayaneo-platform` uses for RGB:

```
outb(0x2e,0x4e); outb(0x11,0x4f)    # select high-byte register
outb(0x2f,0x4e); outb(HIGH,0x4f)    # address high byte
outb(0x2e,0x4e); outb(0x10,0x4f)    # select index register
outb(0x2f,0x4e); outb(INDEX,0x4f)   # address low byte
outb(0x2e,0x4e); outb(0x12,0x4f)    # select data register
outb(0x2f,0x4e); val = inb(0x4f)    # read
```

Implemented in userspace through `/dev/port`. Note it cannot take the ACPI
global lock that the kernel driver does, so there is a small theoretical race
with firmware — acceptable for reads, worth more care before any write.

A full 64KB pass takes ~3.6s. Of 65536 locations, ~6000 are volatile within a
session and must be masked out of any diff.

## Eliminated so far

| location | evidence |
|---|---|
| **USB** | Two USBPcap captures **covering the correct devices** (keyboard MCU `1c4f:007c` and controller `045e:028e`) recorded **zero** HID `SET_REPORT` and **zero** non-control OUT packets across eight setting changes. |
| **ACPI EC (256 bytes)** | Across a settings change, only `0x6b`, `0x71`-`0x73`, `0x91` moved: 48→50, 51→52, 50→52, 22→17. Temperatures and a fan/battery value. |
| **Vendor page `0xD1xx`** | Byte-identical across a settings change. This page holds the joystick ring LEDs (`0xd170`/`0xd1b0`, matching the driver's `LED_MC_ADDR_R/L`). |
| **Literal value storage** | No location in the entire 64KB holds 50, 100 or 150. With only three selectable values, sensitivity is presumably an enum. |

## CONCLUSION: not in the EC

Established with a validated instrument, not by guesswork.

**The method.** EC contents drift continuously, so a two-pass mask taken seconds
apart is worthless - it lets locations that move over minutes masquerade as
stable. `ayaecsample.py` instead takes 10 full 64KB passes ~12s apart and keeps a
location only if it held one value across the whole run, then compares only
locations stable in *both* runs.

**The control.** One reboot through Windows opening AYASpace and changing
**nothing**:

```
control-A -> control-B
  58163 locations stable in both runs
  0 changed outside known-noise pages (19 ignored in pages 0xce/0xc0)
```

Page `0xce` is a circular log that advances every boot (the same `0x0b`/`0xbf`
pattern appears at a different offset each time); `0xc0` holds a couple of
counters. With those filtered, a full reboot changes **nothing**.

**The measurement.** Same procedure, but changing all three settings - left
sensitivity 150 -> 50, right 50 -> 150, deadzone off -> on:

```
control-B -> settings
  58160 locations stable in both runs
  0 changed outside known-noise pages
```

**Zero.** An instrument with a demonstrated zero noise floor across 58,160
locations saw no change when three settings were altered. The settings are not
in the EC address space.

### What that leaves

The setting demonstrably persists across operating systems, so it is written to
non-volatile storage somewhere. With USB showing no command traffic and the EC
now excluded, the remaining explanation is the **controller MCU's own flash**,
written over a channel the captures missed. Plausibly:

* the write happens on *apply* or when AYASpace **closes**, outside the capture
  window that was recorded; or
* USBPcap was attached to one root hub and the write went via another; or
* the EC acts only as a **conduit** - AYASpace pokes a transient command that the
  EC forwards to the MCU over an internal bus, never storing it in EC RAM. A
  10-sample-over-2-minutes scheme cannot see a transient.

### If anyone picks this up

Capture USB again, but: **all** root hubs simultaneously, recording kept running
through changing the setting *and* closing AYASpace, with timestamps noted. That
is the one avenue not yet properly tried.

Everything else in this document is settled: the tooling works, the EC is
readable end to end from userspace, and three candidate locations are eliminated
with evidence.

## The methodological flaw to fix next

Diffing across a reboot conflates two things:

* the setting we changed, and
* everything the reboot itself perturbs — power state, fan curves, and
  AYASpace's own TDP/RGB writes while Windows is running.

A full-space diff across one Windows round trip yielded **115** stable changes,
scattered across many pages at repeating offsets (`0x07`, `0x87`, `0x27`) that
look like mirrored hardware registers. That is far too noisy to identify three
settings.

**The missing control:** one round trip that changes *nothing*.

```
Linux: sudo ayaecfull dump control-A
  -> boot Windows, open AYASpace, change NOTHING, shut down
Linux: sudo ayaecfull dump control-B
       sudo ayaecfull diff control-A control-B
```

Everything that differs there is reboot noise. Subtract that set from a real
settings diff and only the settings should remain.

## If the EC is ruled out too

Then the write goes to the **controller MCU's own storage** and the USB capture
simply missed the moment. Re-capture with **all** USBPcap root hubs running
simultaneously, keep capturing while changing the setting *and* while closing
AYASpace (the write may happen on apply/exit rather than on the click), and note
timestamps.
