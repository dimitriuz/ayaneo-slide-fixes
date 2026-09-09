# Build & apply from scratch

Two routes:

* **[A — CachyOS / Arch](#a--cachyos--arch-pkgbuild)** — rebuild the distro kernel package (recommended if you are on CachyOS).
* **[B — any distro](#b--any-distro-vanilla-tree)** — patch a vanilla tree.

Both end with the same [verification](#verify) and the single kernel parameter from
[KERNEL-PARAMS.md](KERNEL-PARAMS.md).

> **Before you start:** make sure you can boot a *different* kernel entry (an LTS
> kernel, or a btrfs snapshot). Patch 2 sets the backlight floor to 0% duty, so a
> mistake can leave you looking at a black screen.

---

## A — CachyOS / Arch (PKGBUILD)

### 1. Prerequisites

```bash
sudo pacman -S --needed base-devel bc cpio pahole perl python rust rust-bindgen \
                        rust-src tar xxhash xz zlib zstd git
```

`bc` and `rust-bindgen` are the two most commonly missing. If `CONFIG_RUST=y` in the
config (it is, for CachyOS) and `rust-bindgen` is absent, `make olddefconfig` will
**silently disable `CONFIG_RUST`** rather than fail — you get a kernel that differs
from the official package. Install it first.

### 2. Get the PKGBUILD

```bash
git clone --depth 1 https://github.com/CachyOS/linux-cachyos
cd linux-cachyos/linux-cachyos-deckify      # or linux-cachyos for the plain variant
```

### 3. Import the signing keys

```bash
gpg --recv-keys E18447AC260021D31F3FF6C4C8A2A4774B8B63C4 \
                E8B9AA39F054E30E8290D492C3C4820857F654FE
```

If keyservers are blocked (`keyserver receive failed`), fetch over HTTPS instead:

```bash
for k in E18447AC260021D31F3FF6C4C8A2A4774B8B63C4 \
         E8B9AA39F054E30E8290D492C3C4820857F654FE; do
  curl -sfL "https://keys.openpgp.org/vks/v1/by-fingerprint/$k" | gpg --import
done
```

### 4. Add the patches

Copy both patches next to the PKGBUILD, then append to the **end** of the PKGBUILD —
after the `b2sums=(...)` assignment, so array ordering is unambiguous and the patches
apply *last* (the deckify `0001-handheld.patch` touches the same function):

```bash
cp /path/to/ayaneo-slide-fixes/patches/*.patch .

cat >> PKGBUILD <<'EOF'

source+=("0001-drm-amd-display-fix-PWM-backlight-millipercent-conve.patch")
b2sums+=('SKIP')
source+=("0002-drm-panel-add-AYANEO-SLIDE-minimum-backlight-quirk.patch")
b2sums+=('SKIP')
EOF
```

`prepare()` auto-applies every `*.patch` in `source`, so no further wiring is needed.

### 5. Target the right CPU

The default branch enables `X86_NATIVE_CPU` (`-march=native`), which tunes for the
*build* machine. If you build on a different box, set it explicitly — the SLIDE is
Zen 4:

```bash
sed -i 's|^: "${_processor_opt:=}"$|: "${_processor_opt:=zen4}"|' PKGBUILD
```

### 6. Build

```bash
MAKEFLAGS=-j$(nproc) makepkg -Cf --noconfirm
```

Expect **~2h45m on the SLIDE itself** (7840U at handheld TDP, full CachyOS module set,
~6500 modules). A desktop is not necessarily faster; the module set dominates.

> **Building over SSH on a deckify install?** It ships
> `/etc/systemd/logind.conf.d/steam-deckify.conf` with `KillUserProcesses=True`, which
> reaps your build the moment the session closes — `nohup`/`setsid` will **not** save
> you. Use a system-scope transient unit (`scripts/rebuild.sh` in this repo does
> exactly this), or `tmux`.

### 7. Install

```bash
sudo pacman -U linux-cachyos-deckify-*.pkg.tar.zst \
               linux-cachyos-deckify-headers-*.pkg.tar.zst
```

Then set the kernel parameter per [KERNEL-PARAMS.md](KERNEL-PARAMS.md) and reboot.

### 8. Kernel updates will silently revert this

Any `pacman -Syu` that bumps the kernel restores the stock build and the bug returns.
Either hold it:

```ini
# /etc/pacman.conf
IgnorePkg = linux-cachyos-deckify linux-cachyos-deckify-headers
```

…or re-run the build after each bump (`scripts/rebuild.sh`). Holding a kernel means
no kernel security updates — your call which trade-off you prefer.

---

## B — any distro (vanilla tree)

```bash
git clone --depth 1 --branch v7.2 \
  https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git
cd linux
git am /path/to/ayaneo-slide-fixes/patches/*.patch     # or: patch -Np1 < ...
```

On kernels where `amdgpu_dm.c` has been split (mainline after ~2026-04), the backlight
code lives in `drivers/gpu/drm/amd/display/amdgpu_dm/amdgpu_dm_backlight.c`. Patch 1
then needs its path adjusted — the code itself is unchanged:

```bash
grep -rn 'power module uses millipercent' drivers/gpu/drm/amd/display/
```

Then build normally:

```bash
cp /boot/config-$(uname -r) .config    # or zcat /proc/config.gz > .config
make olddefconfig
make -j$(nproc)
sudo make modules_install install
```

### Patch 2 needs your DMI strings

The quirk matches on DMI. Confirm yours:

```bash
cat /sys/class/dmi/id/sys_vendor     # AYANEO
cat /sys/class/dmi/id/product_name   # SLIDE
```

`dmi_match()` is an exact string compare, so a different `product_name` means you must
edit the patch. If your machine is **not** an AYANEO SLIDE but has the same
too-bright-minimum symptom, read your own floor first:

```bash
# boot with drm.debug=0x6
sudo journalctl -k -b | grep -i 'backlight caps'
# -> Backlight caps: min: 9766, max: 56026, ac 80, dc 50
```

A `min` well above 0 means your firmware reports the same kind of floor, and a quirk
entry with your own DMI strings will help. Apply patch 1 regardless — it is not
device-specific.

---

## Verify

```bash
# 1. both fixes active
sudo journalctl -k -b | grep -iE 'backlight caps|panel backlight quirk'
#   [drm] Applying panel backlight quirk, min_brightness: 0
#   [drm] Backlight caps: min: 0, max: 56026, ac 80, dc 50      <- min: 0

# 2. full range, true zero floor
cd /sys/class/backlight/amdgpu_bl1
cat max_brightness            # 56026
echo 0 | sudo tee brightness; cat actual_brightness   # 0   (was 6797)

# 3. sweep the curve
/path/to/ayaneo-slide-fixes/scripts/measure-backlight.sh
```

A good result: `actual` tracks `set` monotonically from 0 to `max_brightness`, reaching
`max_brightness` at 100%.

Allow ~0.5s settle before reading `actual_brightness` — the panel ramps, and sampling
sooner returns mid-ramp values that look non-monotonic.

You can also confirm the quirk compiled in without rebooting:

```bash
zstd -dqf /usr/lib/modules/$(uname -r)/kernel/drivers/gpu/drm/drm_panel_backlight_quirks.ko.zst -o /tmp/q.ko
strings /tmp/q.ko | grep -x SLIDE
```

---

## Sending upstream

Patch 1 is a genuine upstream regression fix worth submitting — it affects every AMD
device on the PWM backlight path, not just handhelds.

```bash
git am -s patches/*.patch      # -s adds YOUR Signed-off-by (required; DCO)
./scripts/checkpatch.pl --strict patches/*.patch
./scripts/get_maintainer.pl -f drivers/gpu/drm/amd/display/amdgpu_dm/amdgpu_dm.c
```

Send to `amd-gfx@lists.freedesktop.org` and `dri-devel@lists.freedesktop.org`, Cc the
author of the commit named in the `Fixes:` tag.
