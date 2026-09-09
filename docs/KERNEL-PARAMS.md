# Kernel parameters

**Short answer: you need exactly one.**

```
amdgpu.dcdebugmask=0x40000
```

Everything else commonly suggested for handheld backlight problems is either inert or
actively unhelpful on this machine. All of the below was **measured**, not assumed —
each configuration was booted and the transfer curve sampled.

---

## Required

### `amdgpu.dcdebugmask=0x40000`  →  `DC_DISABLE_CUSTOM_BRIGHTNESS_CURVE`

Disables AMD's custom brightness curve, which remaps brightness through the ATIF
luminance table. On the SLIDE panel that curve **costs about 20% of peak brightness**
and squashes the bottom of the range.

Measured, patched kernel, identical hardware, only this parameter differing:

| slider | curve **disabled** (`0x40000`) | curve **enabled** (no param) |
|---|---|---|
| 0% | 0 | 0 |
| 1% | 657 | **0** |
| 20% | 13158 | 9210 |
| 40% | 22204 | 17681 |
| 80% | 47390 | 35311 |
| **100%** | **56026** | **45103** |

With the curve enabled the driver also reports `scale = non-linear` and logs
`[drm] Using custom brightness curve`.

Verify the bit value for *your* kernel before copying this — `DC_DEBUG_MASK` values
shift as flags are added:

```bash
grep -n 'DC_DISABLE_CUSTOM_BRIGHTNESS_CURVE' \
  drivers/gpu/drm/amd/include/amd_shared.h
```

It is `0x40000` in 7.2. Confirm it took effect:

```bash
cat /sys/module/amdgpu/parameters/dcdebugmask   # 262144
cat /sys/class/backlight/amdgpu_bl1/scale       # linear
```

---

## Not needed

### `amdgpu.backlight=0`

A common suggestion, and **commonly misunderstood** — it does not "disable a broken
backlight". From the driver:

```c
MODULE_PARM_DESC(backlight, "Backlight control (0 = pwm, 1 = aux, -1 auto (default))")
```

It *forces the PWM path*. On this panel that is already what auto-detection picks,
because the panel reports no AUX backlight capability, so the parameter changes
nothing. Verified: `amdgpu_bl1` registers and behaves identically without it.

> **Do not set `amdgpu.backlight=1`** on this device. It forces `aux_support = true`
> unconditionally, and this panel cannot do AUX brightness — DPCD `0x700..0x72f` reads
> all zero. You would be selecting a control path the hardware does not implement.

### `acpi_backlight=native`

Not needed. `amdgpu_bl1` still registers as the only backlight device without it, and
the `min_brightness` quirk still applies.

Also worth knowing: `acpi_backlight=video` is **not** a fix here. The firmware's own
`_BCL` table is:

```
[80, 50, 10, 11, 12, ... 79, 80]
```

i.e. AC level 80, battery level 50, supported levels 10–80. Its floor of 10 is barely
below the driver's 14.9%, so it does not solve the too-bright minimum.

### `acpi=strict`

Unrelated to backlight. Left over from experimentation; drop it.

---

## Applying them

### Limine (CachyOS default)

Edit `/etc/default/limine`:

```bash
KERNEL_CMDLINE[default]+="quiet splash rw rootflags=subvol=/@ root=UUID=<YOUR-ROOT-UUID> amdgpu.dcdebugmask=0x40000"
```

Then regenerate — **`limine-mkinitcpio`, not `limine-update`**:

```bash
sudo limine-mkinitcpio
```

Careful with `sed` here: a pattern like `s/"$/ extra"/` will also match
`ESP_PATH="/boot"` and corrupt it. Scope it:

```bash
sudo sed -i '/^KERNEL_CMDLINE/{ s/"$/ amdgpu.dcdebugmask=0x40000"/ }' /etc/default/limine
```

### GRUB

```bash
# /etc/default/grub → GRUB_CMDLINE_LINUX_DEFAULT
sudo grub-mkconfig -o /boot/grub/grub.cfg
```

### systemd-boot

Append to the `options` line in `/boot/loader/entries/*.conf`.

---

## Diagnostic parameter

For investigating backlight behaviour, add temporarily:

```
drm.debug=0x6      # DRM_UT_DRIVER | DRM_UT_KMS
```

This makes the driver log the values everything else depends on:

```
[drm] Applying panel backlight quirk, min_brightness: 0
[drm] Backlight caps: min: 0, max: 56026, ac 80, dc 50
```

Read with `sudo journalctl -k -b | grep -i 'backlight caps'`. It is verbose — remove
it once you are done.
