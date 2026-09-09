# Root cause analysis

Hardware: **AYANEO SLIDE**, AMD Ryzen 7 7840U (Phoenix, DCN 3.1.4), eDP panel
`AYANEOHD` (EDID vendor `AYA`), BIOS `E.AS01_A_AYA_V.D32.M3..006`.
Kernel: `linux-cachyos-deckify` 7.2.3 (mainline 7.2.3 + CachyOS patches).

---

## 1. Starting picture

One backlight device, PWM type:

```
/sys/class/backlight/amdgpu_bl1
  type              = raw
  scale             = linear
  brightness        = 1
  actual_brightness = 6797      <- hardware sits at ~12% of scale
  max_brightness    = 56026
```

`56026 = 218 × 0x101`, which is amdgpu's 8→16-bit scaling of a firmware-reported
`max_input_signal = 218/255`.

Sweeping the full range revealed **dead zones at both ends**:

| requested | hardware does |
|---|---|
| 0 – 100 (0–0.2%) | 6797 — flat |
| 500 – 39218 | S-curve ramp |
| 44820 – 56026 (80–100%) | 56026 — flat |

## 2. Where the numbers come from

Booting with `drm.debug=0x6` makes the driver log the values everything depends on:

```
[drm] Backlight caps: min: 9766, max: 56026, ac 80, dc 50
```

Decoded against `get_brightness_range()`:

```c
} else {
    // Firmware limits are 8-bit, PWM control is 16-bit.
    *max = 0x101 * caps->max_input_signal;
    *min = 0x101 * caps->min_input_signal;
}
```

* `min = 9766` → `min_input_signal = 38` → PWM floor **14.9% duty**
* `max = 56026` → `max_input_signal = 218` → PWM ceiling **85.5% duty**

These originate from ACPI ATIF function
`QUERY_BRIGHTNESS_TRANSFER_CHARACTERISTICS`, parsed in
`amdgpu_atif_query_backlight_caps()`. The kernel sanity-checks them
(`spread >= AMDGPU_DM_MIN_SPREAD`, i.e. ≥121); spread is 180, so they are accepted
rather than falling back to the `12/255` defaults.

## 3. The regression

`amdgpu_dm_backlight_set_level()`, PWM branch:

```c
} else {
    /* power module uses millipercent */
    get_brightness_range(caps, &min, &max);
    brightness = DIV_ROUND_CLOSEST(brightness * 100, (max - min)) * 1000;
    rc = mod_power_set_backlight_percent(dm->power_module, stream,
                                         brightness, 0, false);
}
```

`brightness` arrives from `convert_brightness_from_user()` already mapped into
`[min, max]`. But the consumer wants it *relative* to that window:

```c
/* modules/power/power_abm.c */
pwm = core_power->bl_prop[inst].min_backlight_pwm +
      div_u64((u64)millipercent * core_power->bl_prop[inst].backlight_range, 100000);
```

So with `min = 9766`, `max = 56026`:

* slider `0` → `9766 × 100 / 46260 = 21` → `21000` millipercent, **not 0**
* slider `max` → `56026 × 100 / 46260 = 121` → `121000`, **over range → clamped**
* whole-percent quotient → **100 steps**, not 56026

Saturation therefore begins at `brightness = 46030`, i.e. slider
`≈ 43915 / 56026 (78.4%)`.

### Before the regression

Kernels ≤ 6.18 wrote the PWM value straight through:

```c
backlight_level_params.backlight_pwm_u16_16 = brightness;
rc = dc_link_set_backlight_level(link, &backlight_level_params);
```

Duty was simply `brightness / 65535` — correct endpoints, full resolution, no dead
zones.

Introduced by [`3c108046e1d6`](https://git.kernel.org/linus/3c108046e1d6)
*("drm/amd/display: Add power module on Linux")*, which removed the
`backlight_pwm_u16_16` write and added the millipercent conversion.

## 4. Empirical confirmation

### 4a. The rounding signature

The buggy expression predicts step boundaries wherever
`brightness × 100 / (max − min)` crosses `k + 0.5`. Measured boundaries at
slider 20420, 20960, 21560, 22100, 22640 evaluate to:

```
57.56   58.53   59.60   60.56   61.52
```

Every one sits just past a `.5` crossing — the exact fingerprint of
`DIV_ROUND_CLOSEST`. Tread width ≈ 560 ≈ `56026 / 100`.

### 4b. Predicted saturation onset

Predicted `43915`; measured onset within `(43400, 43950]`.

### 4c. The 6.18 cross-check (decisive)

Booting `linux-cachyos-lts` 6.18.48, which retains the pre-regression code, on the
**same hardware and same firmware**:

```
[drm] Backlight caps: min: 9766, max: 56026, ac 80, dc 50     <- IDENTICAL
max_brightness    = 46260        (= max - min)
actual_brightness = 0            at brightness 0   (7.2 gave 6797)
```

Transfer curve, 6.18:

```
set        0    462    925   2313   4626   9252  18504  27756  37008  46260
actual     0    461    925   2313   4626   9252  18505  27756  37008  46260
```

A perfect identity, and every 40-unit step produced a change (vs 3 of 16 on 7.2).
Identical firmware caps with completely different behaviour eliminates any hardware
or firmware explanation.

> 6.18 has a *different* bug: `props.max_brightness = max - min`, so the slider tops
> out at 73.2% duty instead of 85.5%. Both kernels are wrong, in opposite directions.

## 5. The second, independent problem

With the regression fixed the floor returns to the firmware's intended 14.9% duty —
roughly 75 nits on this panel, **still too bright in a dark room**. Confirmed
subjectively on 6.18, which already behaves that way.

So the ATIF floor itself needs overriding. Upstream's mechanism for this is
`drm_panel_backlight_quirks.c`, which already carries `.min_brightness = 1` entries
for Valve *Jupiter*/*Galileo* and several Framework panels. Adding a SLIDE entry sets
`caps->min_input_signal = min_brightness - 1 = 0`.

## 6. Alternatives ruled out

| hypothesis | evidence against |
|---|---|
| ABM / adaptive backlight | `panel_power_savings = 0` |
| userspace daemon fighting writes | values persisted across 3s re-reads; only `upowerd` running |
| custom brightness curve | `0x40000` verified correct for 7.2; `Using custom brightness curve` absent from log |
| AUX/DPCD backlight would help | DPCD `0x700..0x72f` reads **all zero** while `0x000` returns valid data (DP 1.2, 4 lanes) — panel implements no AUX brightness. `DP_EDP_BACKLIGHT_ADJUSTMENT_CAP` = 0 |
| `acpi_backlight=video` | firmware `_BCL` = `[80, 50, 10, 11 … 80]`; floor of 10 is barely under 14.9% |
| ACPI table override of ATIF values | byte pattern `50 32 26 da` (ac/dc/min/max) absent from all tables; `0xDA` never appears as an AML constant. Validated the search with a positive control (`0a 4e 0a 4f 0a 50` from `_BCL`, found) |
| newer kernel already fixes it | identical buggy line in `torvalds/master` and `amd-staging-drm-next` as of 2026-09-09 |

### On perception

A 5.7× luminance change (14.9% → 85.5% duty) *sounds* dramatic but reads as only
~1.8× perceived brightness under Stevens' law. Early in the investigation this looked
like a contradiction between the measurements and what the screen appeared to do; it
was not. Brightness judgements across a 30-second gap are also unreliable because the
eye adapts — rapid A/B alternation is far more trustworthy.

## 7. Residual limitation

~100 discrete levels remain, from the power module's LUT:

```c
if (millipercent >= (100 * 1000))
    return backlight_lut[num_backlight_levels - 1];

index = ((num_backlight_levels - 1) * millipercent) / 100000;
pwm   = backlight_lut[index];
```

A floor-indexed lookup into a ~101-entry table — matching the observed ~560-unit
treads, and why sliders 560 and 1120 both land on 657. Reached only because
`use_linear_backlight_curve` is false; that flag is internal, with no module
parameter. Unrelated to these patches and imperceptible on a 0–100% slider.
