# From a fresh CachyOS install to the working state

Everything that has to be done to an AYANEO SLIDE, in the order it has to be
done, and what an update will quietly undo.

This is the whole configuration as it actually stands on the machine the rest of
this repository was written on — not an idealised one. Each step links to the
document that explains *why*; this page is only the sequence.

> **Read the [scope warning](../README.md#scope-and-a-warning) first.** This
> patches a kernel, patches a DKMS module, writes to an embedded controller and
> changes boot configuration. It is calibrated to one machine.

## Assumptions

- CachyOS freshly installed on an AYANEO SLIDE (board `AS01`), btrfs root,
  Limine, KDE Plasma on Wayland.
- An AUR helper (`paru`) available.
- You are prepared to hold back kernel updates. If you are not, stop here —
  step 1 is undone by the next `pacman -Syu` and the rest is built on it.

## Order matters

1. **Kernel first.** The DKMS module builds against it, so installing the module
   before the kernel means building it twice.
2. **Boot parameters before hibernation** — `resume=` has to be on the command
   line of the kernel that is going to resume, which means a reboot in between.
3. **InputPlumber before ayaHelper**, or ayaHelper's Input tab has nothing to
   talk to.

---

## 1. The patched kernel

Two independent backlight bugs; see [ROOT-CAUSE.md](ROOT-CAUSE.md) for what each
one is.

Follow [BUILD.md](BUILD.md) — build `linux-cachyos-deckify` from its PKGBUILD
with `patches/0001-drm-amd-display-*` and `patches/0002-drm-panel-*` added.
Expect ~2h45m on this hardware; `scripts/rebuild.sh` runs it as a system-scope
transient unit so closing your SSH session does not kill the build.

**Then hold it** (see [the freeze table](#what-updates-undo-and-how-to-hold-it)),
or the next kernel bump silently restores the stock build and the bug returns.

Verify:

```bash
sudo dmesg | grep -i "panel backlight quirk"
# amdgpu … [drm] Applying panel backlight quirk, min_brightness: 0
```

## 2. Kernel parameters

`/etc/default/limine`, then `sudo limine-update`:

```
amdgpu.dcdebugmask=0x40000 acpi=strict
```

- `amdgpu.dcdebugmask=0x40000` — `DC_DISABLE_CUSTOM_BRIGHTNESS_CURVE`, the other
  half of the backlight fix. [KERNEL-PARAMS.md](KERNEL-PARAMS.md)
- `acpi=strict` — **required on this machine.** Without it, it reboots at random.
  Its firmware is not spec-compliant; this is not optional tuning.

Reboot.

## 3. ayaneo-platform, patched

Provides the ring LEDs and the battery's `charge_behaviour`.

```bash
paru -S ayaneo-platform-dkms-git
sudo patch -d /usr/src/ayaneo-platform-*/ -p0 \
  < patches/ayaneo-platform-interruptible-bypass-sleep.patch
sudo dkms build   ayaneo-platform/<version> -k "$(uname -r)"
sudo dkms install ayaneo-platform/<version> -k "$(uname -r)"
sudo reboot
```

The patch makes suspend take 0.167 s instead of 15–30 s — see
[SUSPEND.md §1](SUSPEND.md).

> **Reboot; do not `modprobe -r`.** The module's suspend, shutdown and remove
> paths all `kthread_stop()` the same pointers, so unloading double-stops and
> wedges it: `refcount -1`, `charge_behaviour` gone, no reload possible until a
> reboot. This is a second bug in the same driver.

## 4. InputPlumber, patched

The packaged build is fine except for one hardcoded value — the axis-to-mouse
deadzone is fixed at 20% upstream. The patches make it configurable and document
`quadratic_scaling`.

```bash
sudo pacman -S inputplumber
sudo systemctl enable --now inputplumber
./scripts/build-inputplumber.sh
```

That builds the patched binary to **`/usr/local/bin/inputplumber`** and drops in
`/etc/systemd/system/inputplumber.service.d/99-patched-binary.conf` to run it,
leaving the distro package untouched. Confirm which one is live:

```bash
systemctl show -p ExecStart --value inputplumber   # → /usr/local/bin/inputplumber
```

Then install the stick-as-mouse profile from [`config/`](../config) into
`/etc/inputplumber/profiles/` — files there win over `/usr/share` on a name
clash. [INPUTPLUMBER-GUIDE.md](INPUTPLUMBER-GUIDE.md)

## 5. ayaHelper

```bash
paru -S ryzenadj      # optional: TDP control
```

Then install from a [release](https://github.com/dimitriuz/ayahelper/releases)
or build from source — its README covers both. It brings the udev rules, the
three services, and the launcher.

```bash
ayahelper --status    # writes nothing; the right first check
```

If the gamepad shows as unavailable, press a button on the pad: the MCU sleeps
and has no remote-wakeup bit.

## 6. Suspend-then-hibernate

Suspend costs ~1.9 W on this machine because the firmware cannot reach S0i3, so
a night asleep flattens the battery. [SUSPEND.md §4](SUSPEND.md) has the
measurements; the setup is:

```bash
sudo btrfs subvolume create /swap                        # own subvol: snapshots skip it
sudo btrfs filesystem mkswapfile --size 26g /swap/swapfile   # sets NOCOW, no compression
sudo swapon /swap/swapfile
echo '/swap/swapfile none swap defaults 0 0' | sudo tee -a /etc/fstab
sudo btrfs inspect-internal map-swapfile -r /swap/swapfile   # → resume_offset
```

Size it at least as large as RAM (25.1 GiB here → 26 GiB). Add to the Limine
command line and re-run `limine-update`:

```
resume=UUID=<root fs uuid> resume_offset=<offset from above>
```

Only the two real boot entries get these; snapper's snapshot entries deliberately
do not — resuming an image into a different snapshot would corrupt the filesystem.
mkinitcpio's `systemd` hook handles resume natively, so no `resume` hook is needed.

Then make the desktop's "Sleep" mean suspend-then-hibernate. KDE only knows
suspend, hibernate and hybrid, so the redirect has to happen below it:

```ini
# /etc/systemd/system/systemd-suspend.service.d/10-then-hibernate.conf
[Service]
Environment=SYSTEMD_SLEEP_FREEZE_USER_SESSIONS=false
ExecStart=
ExecStart=/usr/lib/systemd/systemd-sleep suspend-then-hibernate
```

```ini
# /etc/systemd/sleep.conf.d/10-hibernate-delay.conf
[Sleep]
HibernateDelaySec=10min
```

Reboot, then test: `sudo systemctl suspend`. It should suspend, and ten minutes
later wake briefly and hibernate. Resuming keeps `uptime` counting from before
the cycle — that is how you tell a resume from a cold boot.

## 7. Optional: close Steam before sleeping

Deck-mode Steam holds a Wayland idle inhibitor, so with Steam running the screen
stays lit through suspend. If that bothers you, a `system-sleep` hook can shut it
down first — [SUSPEND.md §3b](SUSPEND.md), including why the obvious version of
that hook does nothing.

`SYSTEMD_SLEEP_FREEZE_USER_SESSIONS=false` in step 6 is what makes it work; it is
in that drop-in already.

---

## What updates undo, and how to hold it

Nothing is pinned by default. As installed, a `pacman -Syu` that bumps any of
these silently reverts the work:

| Package | Carries | An update… | Hold it / redo it |
|---|---|---|---|
| `linux-cachyos-deckify`<br>`…-headers` | Both backlight patches | restores the stock kernel; the brightness floor returns | `IgnorePkg`, or re-run `scripts/rebuild.sh` after each bump |
| `ayaneo-platform-dkms-git` | The suspend patch | rebuilds from unpatched source; suspend goes back to 15–30 s | `IgnorePkg`, or re-apply the patch and `dkms build/install`, then reboot |
| `inputplumber` | Nothing — the patched build lives in `/usr/local/bin` | leaves the override in place, so you keep running the **older** patched binary | re-run `scripts/build-inputplumber.sh` when you want the newer base; `git am` will say if the patches need rebasing |
| `ayahelper` | Not a package here | nothing | update from a release when you want to |

To hold the two that matter:

```ini
# /etc/pacman.conf
IgnorePkg = linux-cachyos-deckify linux-cachyos-deckify-headers ayaneo-platform-dkms-git
```

**Holding a kernel means no kernel security updates.** That is a real trade-off,
not a formality — the alternative is rebuilding after each bump, which is a
~2h45m job you can run in the background with `scripts/rebuild.sh`.

`linux-cachyos-lts` is installed alongside as a fallback and is *not* patched, so
booting it brings the backlight bug back. That is fine as a rescue kernel; just
do not be surprised by it.

## Verifying the whole thing

```bash
# backlight: the quirk is applied and 0 really is off
sudo dmesg | grep -i "panel backlight quirk"

# suspend is fast (should be well under a second)
sudo dmesg | grep "PM: suspend of devices complete"

# the patched module is the one loaded
cat /sys/module/ayaneo_platform/srcversion
modinfo -F srcversion ayaneo_platform     # must match

# the patched InputPlumber is the one running
systemctl show -p ExecStart --value inputplumber

# hibernation has somewhere to go
swapon --show ; grep -o 'resume=[^ ]* resume_offset=[^ ]*' /proc/cmdline

# devices ayaHelper can reach, and what else is fighting it
ayahelper --status
```

## What you do not need

Recorded because each was tried:

- `amdgpu.backlight=0` and `acpi_backlight=native` — neither helps;
  [KERNEL-PARAMS.md](KERNEL-PARAMS.md) says what they actually do.
- A charge limit. It does not work on this hardware, in ayaHelper *or* in
  AYASpace on Windows. [CHARGING.md](CHARGING.md)
- `ryzen_smu`. Only needed to *read* TDP back; everything still sets fine
  without it.
