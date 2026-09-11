# Charge control on the AYANEO SLIDE

## What the hardware offers

**Bypass, yes. A threshold, no.**

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

## Helper commands

```
charge status                    behaviour=auto limit=80 capacity=56 status=Discharging ec_bypass=off
charge behaviour inhibit-charge  auto | inhibit-charge
charge limit 80                  20-99
charge limit off
```
