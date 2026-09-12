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

## 3. The panel stayed lit — Steam, in Deck mode

**Not a handheld fault and not a compositor bug.** This one took the longest and
every intermediate theory was wrong, so the dead ends are kept below.

### Cause

`deckify` installs `steam-jupiter-stable`, whose wrapper always launches the
Deck build:

```
/usr/bin/steam -> /usr/bin/steam-jupiter
exec /usr/lib/steam/steam -steamdeck "$@"
```

In Deck mode Steam takes over display idle management — its own timers are in
`config.vdf`:

```
"IdleBacklightDimBatterySeconds"  "300"
"IdleBacklightDimACSeconds"       "900"
```

On a Deck that is correct. In a desktop session it holds a Wayland
`idle-inhibit` inhibitor and never replaces the behaviour, so nothing blanks the
screen — on idle *or* before suspend.

A Wayland idle inhibitor is invisible to every list you would think to check:

```
$ systemd-inhibit --list          # only NetworkManager, UPower, PowerDevil sleep locks
$ busctl --user call ... PolicyAgent ListInhibitions
aas 0
$ busctl --user call ... org.freedesktop.ScreenSaver GetActive
b false
```

Nothing listed, no DBus traffic, no log line — and yet `loginctl show-session`
reports `IdleHint=no` indefinitely.

### Proof

With Steam closed, and nothing else changed:

```
   2s  b=4483 blp=0 dpms=On  en=enabled
  26s  b=1346 blp=0 dpms=On  en=enabled     <- the 30s dim
  56s  b=1346 blp=0 dpms=Off en=disabled    <- the 60s blank
```

KDE blanks the panel perfectly, and it is plainly visible in sysfs as
`dpms=Off, enabled=disabled`. With Steam running, the same wait produces no
change at all. Suspend then blanks the screen correctly too.

### The dead ends, and why each was wrong

| Attempt | Result | Why it looked convincing |
|---|---|---|
| `brightness = 0` | Panel only dims | This unit has a firmware brightness floor, so it *is* true that no brightness value blanks it — just not why the screen was on |
| `bl_power = 4` | Accepted, no effect | KWin blanks by disabling the CRTC, not via the backlight device |
| `kscreen-doctor --dpms off` | Exits 0, nothing changes | It did work; Steam's inhibitor undid it before the next sample |
| KDE's `Turn Off Screen` shortcut | Blanked for **under a second**, then restored | Same again — and the sub-second blink was the clue that the panel *could* blank |
| `kwin: Failed to delay sleep` | Real message, wrong conclusion | KWin does take its inhibitor late, but that was never what kept the panel lit |

The observation that broke it open was the user's: *"KDE can successfully power
off the screen after 15 minutes of inactivity."* That ruled out the firmware
theory below and said the panel can blank, so something had to be undoing it.

### Firmware context — real, but not the cause

```
$ cat /sys/power/mem_sleep
[s2idle]
$ dmesg | grep amdgpu
amdgpu: Power consumption will be higher as BIOS has not been configured for
        suspend-to-idle.
```

No `deep`/S3 is offered *and* low-power S0 idle is not declared; this firmware
also needs `acpi=strict` or the machine reboots at random. That affects how
deeply the SoC sleeps. It does not stop the panel blanking, which is what the
test above proves.

### What to do about it

There is no Steam setting for this: Deck mode inhibits unconditionally. Either
quit Steam when using the desktop session, or launch the non-Deck client
(`/usr/lib/steam/steam`, bypassing the wrapper's `-steamdeck`) for desktop use
and keep Deck mode for Game Mode.

## 3b. Shutting Steam down before sleep — and why the obvious way fails

Since only Steam's exit releases the inhibitor, a `system-sleep` hook can ask it
to quit before suspending. The obvious implementation does not work:

```
09:48:34  systemd-sleep: Successfully froze unit 'user.slice'.
09:48:34  steam-sleep: asking Steam to shut down
09:49:01  steam-sleep: Steam still running after 25s, sleeping anyway
```

**systemd freezes user sessions before running `system-sleep` hooks.** Steam is
already frozen when the hook speaks to it, so it cannot answer `-shutdown`, the
hook waits out its whole timeout, and the machine suspends with the inhibitor
still held — the screen stays lit showing a frozen Steam window. Steam then
finishes exiting on resume, which makes it look as though the hook worked.

The switch for this is `SYSTEMD_SLEEP_FREEZE_USER_SESSIONS=false`, set on the
service, which restores the pre-v254 ordering:

```ini
# /etc/systemd/system/systemd-suspend.service.d/10-then-hibernate.conf
[Service]
Environment=SYSTEMD_SLEEP_FREEZE_USER_SESSIONS=false
ExecStart=
ExecStart=/usr/lib/systemd/systemd-sleep suspend-then-hibernate
```

With that, the hook works and the whole thing takes about two seconds — Steam
exits quickly when it is not frozen and fighting the request.

The hook itself uses Steam's own `-shutdown` rather than a signal, so downloads
and cloud saves flush, and it **skips the shutdown while a game is running**
(`pgrep -f "reaper SteamLaunch"`). Suspending mid-game is a normal thing to do
and taking the client down would take the game with it.

The cost: every suspend waits for Steam to exit, and user processes are no
longer frozen before sleep. If that trade is not worth it, the alternative is to
leave Steam alone and shorten `HibernateDelaySec` — the screen is only lit until
the machine hibernates, and the battery is safe either way.

## 4. Suspend costs ~1.9 W, because the SoC never sleeps

Left suspended overnight at 54%, the machine was flat by morning and had cut
power mid-suspend — the journal simply ends at `PM: suspend entry`.

### The arithmetic

```
20:09:05   56%   discharging 8.5 W   (awake)
20:16:25   suspend entry (s2idle)
~09:16     flat
```

~54% of a 45 Wh pack is 24 Wh, gone in about 13 hours: **~1.9 W while
"asleep"**. A working handheld suspend is 0.2–0.5 W. Nothing was malfunctioning
— that is simply what sleep costs here.

### Why

```
$ sudo python3 -c "d=open('/sys/firmware/acpi/tables/FACP','rb').read(); \
    f=int.from_bytes(d[112:116],'little'); print(hex(f), bool(f&(1<<21)))"
0xc5ad False                      <- LOW_POWER_S0_IDLE_CAPABLE not set

$ ls /sys/bus/acpi/devices/ | grep AMDI000
                                  <- no AMD PMC device at all

$ cat /sys/power/mem_sleep
[s2idle]                          <- and no S3 to fall back on
```

`amd_pmc` is the driver that drives an AMD SoC into S0i3. It loads fine but has
nothing to bind to, because the firmware never exposes the controller. So
s2idle freezes the CPUs and leaves the rails up. Same root as amdgpu's
*"BIOS has not been configured for suspend-to-idle"*, and of a piece with a
firmware that needs `acpi=strict` to avoid random reboots.

This is not fixable from the OS.

### The fix: suspend-then-hibernate

Hibernation is the way out, and it works once there is somewhere to put the
image. The default zram swap cannot hold one — it lives in RAM.

```bash
btrfs subvolume create /swap                      # own subvolume: snapshots skip it
btrfs filesystem mkswapfile --size 26g /swap/swapfile   # sets NOCOW + no compression
swapon /swap/swapfile
btrfs inspect-internal map-swapfile -r /swap/swapfile   # -> resume_offset
```

then `/swap/swapfile none swap defaults 0 0` in fstab, and on the kernel command
line (`/etc/default/limine`, then `limine-update`):

```
resume=UUID=<root fs uuid> resume_offset=<offset>
```

mkinitcpio's `systemd` hook handles resume natively — no `resume` hook needed.
Only the two real boot entries get the parameters; snapper's snapshot entries
deliberately do not, since resuming an image into a different snapshot would
corrupt the filesystem.

**KDE cannot ask for it.** PowerDevil offers only `suspendToRam`,
`suspendToDisk` and `suspendHybrid`, so the desktop can never request
suspend-then-hibernate. Rather than change how the machine is used, change what
suspend means — logind starts `systemd-suspend.service`, so intervene there:

```ini
# /etc/systemd/system/systemd-suspend.service.d/10-then-hibernate.conf
[Service]
ExecStart=
ExecStart=/usr/lib/systemd/systemd-sleep suspend-then-hibernate
```

with `HibernateDelaySec=30min` in `/etc/systemd/sleep.conf.d/`.

### Verified

```
09:36:39  Performing sleep operation 'suspend'...
09:38:40  System returned from sleep operation 'suspend-then-hibernate'.
09:38:40  Performing sleep operation 'hibernate'...
09:40:10  System returned from sleep operation 'suspend-then-hibernate'.
```

`uptime` still counted from before the cycle, and a `ping` left running in a
terminal carried on afterwards — it resumed rather than cold-booted.

One caveat worth knowing when testing: a cold boot logs
`PM: Image not found (code -22)` because there is no image yet. That line is
easy to mistake for a failed resume when reading the log of the boot that
*later* hibernated.

## Unrelated but adjacent: the pad disappears across suspend

See [INPUT-CONTROLLER.md](INPUT-CONTROLLER.md) — the gamepad MCU powers down and
has no remote-wakeup bit, so it does not come back until you press a button on
it. Nothing to do with the above.
