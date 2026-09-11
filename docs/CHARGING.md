# Charge control on the AYANEO SLIDE

> **Verdict for the AYANEO SLIDE (AS01, EC `0x001b0100`): neither works.** The
> kernel exposes bypass, the write reaches the EC, and the battery charges
> straight through it. The rest of this document is how that was established and
> why there is no better register to try.

## What the hardware offers

**An interface for bypass, yes. A threshold, no. Neither has an effect here.**

The `ayaneo-platform` module registers a `power_supply` extension on the battery:

```
$ ls /sys/class/power_supply/BAT0/extensions/
ayaneo-bypass-charge -> ../../BAT0
$ cat /sys/class/power_supply/BAT0/charge_behaviour
[auto] inhibit-charge
```

`inhibit-charge` makes the EC run the machine from the adapter and leave the cell
alone. It is gated in the driver on both the model and the EC version — `slide` is
listed, needing EC `0x1b` or later — so **the presence of the file is the feature
test**, not the model name.

There is no `charge_control_end_threshold`, and no EC register takes one. Writes to
the registers that looked like candidates produced no observable effect.

## How it reaches the hardware

The driver does not write the EC when sysfs is written. A writer thread wakes every
30 seconds (`AYANEO_BYPASS_WRITER_DELAY_MS`) and pushes the current behaviour to EC
RAM `0xd1d1` — `0x01` to bypass, `0x65` for normal. Measured, reading the register
directly while toggling sysfs:

```
baseline           ec d1d1=0x65 CLOSE   sysfs=[auto] inhibit-charge
just after write   ec d1d1=0x65 CLOSE   sysfs=auto [inhibit-charge]
+5s … +25s         ec d1d1=0x65 CLOSE   sysfs=auto [inhibit-charge]
+30s               ec d1d1=0x01 OPEN    sysfs=auto [inhibit-charge]
```

So **sysfs says what was asked for and the EC register says what is in force**, and
they disagree for up to half a minute. Anything reporting "it didn't work" inside
that window is reporting the lag. `ayaneo-tray --status` shows both:

```
charging        behaviour auto  limit 80%  EC charging
```

(the `EC …` half needs root, since it is raw port I/O.)

## The charge limit

With no threshold register, a limit has to be supervised: hold `inhibit-charge`
above the target, release it below. `ayaneo-tray`'s helper does this in a thread
that polls every 20 seconds, with **3 points of hysteresis** — without it the
supervisor would toggle on every reported percent, and each toggle costs up to 30
seconds of the driver's writer cycle.

It lives in the helper, not the GUI, because a charge limit that only applies once
somebody logs in is not a charge limit. The value is kept in
`/var/lib/ayaneo/charge-limit` (via the unit's `StateDirectory=ayaneo`) and restored
at start:

```
$ journalctl -u ayaneo-tray-helper -n1
charge: limit 80% restored
```

Behaviour at the edges, with the battery at 56%:

| Limit set | Decision  | Why |
|-----------|-----------|-----|
| 50        | inhibit   | 56 ≥ 50 |
| 90        | auto      | 56 + 3 ≤ 90 |
| 58        | unchanged | inside the hysteresis band |
| off       | unchanged | supervision off; whatever was set by hand stands |

While a limit is set the GUI disables the manual Normal/Bypass control — the
supervisor would undo a click within twenty seconds, and showing why beats letting
it be fought.

## It does not actually stop the charge

Bypass engages at the register and the battery keeps charging at full rate.
Measured with the charger attached, `energy_now` rather than `capacity` because
it is a thousand times finer:

```
 start   67%   32.501Wh   12.90W  Charging  ec=0x01  auto [inhibit-charge]
  +15s   67%   32.501Wh   12.90W  Charging  ec=0x65  [auto] inhibit-charge
 +240s   69%   33.471Wh   12.90W  Charging  ec=0x65  [auto] inhibit-charge
```

Ruled out along the way:

* **The lag.** The verdict is only taken once the EC register itself has
  changed, not when sysfs was written.
* **A plug cycle.** Bypass engaged, charger unplugged and reattached: charging
  resumed. So the EC is not merely latching the setting at charge start.
* **Both values, actually written.** The driver skips a write when the register
  already holds the value, so `0x65` had only ever been *sitting* there, never
  written during a charge. Forced properly — `0x01` charges, `0x65` charges.
* **A wrong register.** See below.

## AYASpace does exactly the same thing

From `AYASpaceCef.exe`, `system.set_charge_config` (handler `0x140266530`) takes
`{enable, val}`, stores both, and passes only `enable` to `0x140348200`:

```c
cVar1 = FUN_14040b860(model);       // AB05-AMD | AB05-Intel | AB05N | AS01
if (cVar1 == '\0') {                // any other model
    local_14 = param_2 ? 0xaa : 0x55;
    FUN_1403474a0(param_1, 0xfe80041e, local_14);   // mapped physical memory
} else {                            // AS01 - the SLIDE
    FUN_14032fcd0(uVar2, param_2);
}
```

and the branch this board takes is:

```c
void FUN_14032fcd0(longlong param_1, char param_2) {
    local_14 = param_2 ? 0x65 : 1;
    uVar1 = *(ushort *)(param_1 + 4);
    FUN_1401bab40(uVar2, (int)(uint)uVar1 >> 8, 0xd1, local_14);  // page, index, value
}
```

`FUN_1401bab40` bottoms out in the familiar `0x4e/0x4f` sequence — `0x11` page,
`0x10` index, `0x12` data. The page comes from an object field rather than a
literal, but the sibling writer at `FUN_14032ebd0` takes the same field
expression with index `200` (`0xc8`) — the fan register, which is known to live
on page `0xd1`. So AYASpace writes **`0xd1d1`**, the same register, with the same
two values.

`val` — the percentage — never reaches the EC at all. AYASpace supervises the
threshold in software, exactly as this helper does.

So there is no better register to find: the vendor's own software would be just
as ineffective on this unit. Whether it is the EC firmware version
(`0x001b0100`, exactly the minimum `ayaneo-platform` requires) or the board, the
controller does not act on `0xd1d1`.

## What the app does about it

It measures, rather than assuming either way. While a request is in force at the
register *and* the battery reports Charging, the helper samples `energy_now` over
two minutes; a gain of more than 300 mWh (the gauge steps in ~485 mWh, so this is
above quantisation) means the EC ignored it:

```
charge: inhibit-charge is ignored by this EC (+485 mWh over 120s)
```

That shows up as `honoured=no` in `charge status` and as a warning on the Charge
page, so the limit control cannot quietly pretend to work. If a firmware update
ever implements the register, the same check flips to `honoured=yes` with no
change here.

## Helper commands

```
charge status                    behaviour=auto limit=80 capacity=56 status=Discharging ec_bypass=off honoured=no
charge behaviour inhibit-charge  auto | inhibit-charge
charge limit 80                  20-99
charge limit off
```
