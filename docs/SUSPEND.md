# Suspend on the AYANEO SLIDE

Three separate faults, found while chasing one symptom: *"I press sleep and the
screen stays on."* Two are fixed; the third is not ours.

## 1. Suspend took 15–30 seconds — `ayaneo-platform`

### Symptom

Press sleep and the machine appears to do nothing: the screen stays lit, showing
the last frame, and is unresponsive. Press the power button again and it comes
back. The journal shows the suspend being entered and then abandoned:

```
PM: suspend entry (s2idle)
PM: Some devices failed to suspend, or early wake event detected
PM: suspend exit
PM: suspend entry (s2idle)      <- retries
```

That second message is a red herring. It appears because the *second* button
press lands in the middle of a suspend that is still in progress.

### Cause

With `pm_print_times` on, one device accounts for the whole delay:

```
ayaneo-platform: PM: platform_pm_suspend returned 0 after 18487017 usecs
usb 1-3:         PM: usb_dev_resume     returned 0 after   285019 usecs
                                          ...everything else under 0.3 s
```

The driver's bypass-charge writer thread sleeps between iterations:

```c
#define AYANEO_BYPASS_WRITER_DELAY_MS 30000
while (!kthread_should_stop()) { ...; msleep(30000); }
```

and `ayaneo_platform_suspend()` stops it with `kthread_stop()`. `msleep()` does
not return early for `kthread_stop`, so **every suspend waits out whatever is
left of that 30-second sleep** — 0 to 30 s, ~15 s on average, with the panel lit
and userspace already frozen.

### Confirming it rather than assuming it

Three samples — 17.7 s, 18.5 s, 15.2 s — look like a uniform draw from 0–30. If
the sleep is the blocker, then suspending 25 s after a resume (the thread is
restarted on resume) should cost only the remaining ~5 s:

```
$ sleep 25 && rtcwake -m mem -s 12
ayaneo-platform: platform_pm_suspend returned 0 after 5932032 usecs
```

5.93 s. Predicted, not observed after the fact.

### Fix

[`patches/ayaneo-platform-interruptible-bypass-sleep.patch`](../patches/ayaneo-platform-interruptible-bypass-sleep.patch)
— sleep in 100 ms steps and check `kthread_should_stop()`:

```
ayaneo-platform: platform_pm_suspend returned 0 after 166987 usecs   (was 18.5 s)
PM: suspend of devices complete after 235.391 msecs                  (was 18,562 ms)
```

A 110× improvement. It is a DKMS module, so this rebuilds in seconds:

```bash
sudo patch -d /usr/src/ayaneo-platform-*/ -p0 < patches/ayaneo-platform-interruptible-bypass-sleep.patch
sudo dkms build  ayaneo-platform/<version> -k "$(uname -r)"
sudo dkms install ayaneo-platform/<version> -k "$(uname -r)"
sudo reboot
```

### Do not unload the module to apply it — reboot

`modprobe -r ayaneo_platform` hits a **second** bug in the same driver:

```
WARNING: kernel/kthread.c:84 at kthread_stop+0x146
  ayaneo_platform_remove+0x15/0x440 [ayaneo_platform]
```

Its suspend, shutdown and remove paths all call `kthread_stop()` on the same
task pointers without clearing them, so removal double-stops an already-stopped
thread. The kernel warns, kills `modprobe` mid-unload, and leaves the module
wedged (`ayaneo_platform 32768 -1`) — `charge_behaviour` disappears and it cannot
be reloaded. Only a reboot clears it.

Both bugs are worth reporting upstream, and the DKMS patch is lost whenever the
package updates.

## 2. The rings stayed lit while asleep — fixed

The driver's LED `suspend_mode` defaults to `oem`, which hands the LEDs back to
the EC on suspend — and the EC lights them, so a sleeping handheld glows.

```
$ cat /sys/class/leds/ayaneo:rgb:joystick_rings/suspend_mode
[oem] keep off
```

`off` makes the driver dark them instead. It resets at every boot, so ayaHelper
sets it from a udev rule.

## 3. The panel stays lit — a KDE fault, not a handheld one

Still open, and out of scope for anything on this machine to fix properly.

KWin is supposed to blank the display before the system sleeps. It does not:

```
kwin_wayland: Failed to delay sleep: The operation inhibition has been requested
              for is already running
```

It tries to take a logind *delay* inhibitor after `PrepareForSleep` has already
arrived, which is too late by design — and it is absent from `systemd-inhibit
--list` while NetworkManager and UPower are both there.

What does **not** work, tested:

| Approach | Result |
|---|---|
| `brightness = 0` on `amdgpu_bl1` | Write sticks, panel only dims — this unit has a firmware-reported brightness floor, so no value blanks it |
| `bl_power = 4` (`FB_BLANK_POWERDOWN`) | Attribute accepts the write, panel unaffected |
| `kscreen-doctor --dpms off` | Exits 0, does nothing: connector stays `dpms=On`, backlight unchanged |
| KDE's own `Turn Off Screen` shortcut via `kglobalaccel` | Blanks for **under a second**, then restores — with *no* input events on any of the 17 evdev devices and nothing logged by KWin or powerdevil |

That last row is the interesting one: KDE blanks the panel successfully on its
own 15-minute idle timeout, so the hardware can do it. Invoked directly it
undoes itself immediately for no reason either the compositor or the logs will
admit to.

### Firmware context

Worth knowing, though it is about how deeply the SoC sleeps rather than whether
the panel can blank:

```
$ cat /sys/power/mem_sleep
[s2idle]
$ dmesg | grep amdgpu
amdgpu: Power consumption will be higher as BIOS has not been configured for
        suspend-to-idle.
```

No `deep`/S3 is offered *and* low-power S0 idle is not declared. This firmware
also needs `acpi=strict` on the kernel command line or the machine reboots at
random — so it is not a well-behaved implementation, and there is no
firmware-driven display power-down to fall back on.

## Unrelated but adjacent: the pad disappears across suspend

See [INPUT-CONTROLLER.md](INPUT-CONTROLLER.md) — the gamepad MCU powers down and
has no remote-wakeup bit, so it does not come back until you press a button on
it. Nothing to do with the above.
