#!/usr/bin/env python3
"""crosstalk - watch the triggers and the right stick together.

Run it, squeeze a trigger without touching the sticks, watch whether the stick
readings move with it. Ctrl+C to stop.

Why this matters: if pressing a trigger deflects the stick past the pointer
deadzone, the cursor moves. That is a hardware or firmware property, and the
only thing software can do about it is refuse to act on deflections that small
— so the measurement decides whether raising the deadzone is a fix or just a
way to hide a mechanical problem.

The physical pad is read with EVIOCGABS, which reflects the kernel's current
value even while InputPlumber holds an exclusive grab on it.
"""
import array
import fcntl
import glob
import os
import sys
import time

AXES = {0: "LX", 1: "LY", 3: "RX", 4: "RY", 2: "L2", 5: "R2"}

# Axes do NOT share a range on this pad: the sticks are -32768..32767 while the
# triggers are 0..255. Normalising everything by the stick scale made a fully
# squeezed trigger read as 1%.
#
# Nor is the span the right divisor. "Full deflection" is measured from where
# the axis rests, not across its whole travel: a stick resting at 0 reaches
# 32767, half its 65535 span, while a trigger resting at 0 reaches its full 255.
# So each axis is scaled by the larger distance from rest to either end, which
# EVIOCGABS provides alongside the value.


def find_pad():
    for d in sorted(glob.glob("/dev/input/event*"),
                    key=lambda p: int(p.rsplit("event", 1)[1])):
        base = os.path.basename(d)
        try:
            name = open(f"/sys/class/input/{base}/device/name").read().strip()
            phys = open(f"/sys/class/input/{base}/device/phys").read().strip()
        except OSError:
            continue
        if "X-Box" in name and phys.startswith("usb-"):
            return d, name
    return None, None


def main():
    dev, name = find_pad()
    if not dev:
        sys.exit("physical gamepad not found")
    print(f'{dev}  "{name}"\n')
    print("Squeeze L2, then R2, without touching the sticks. Ctrl+C to stop.\n")
    f = open(dev, "rb", buffering=0)

    def absinfo(code):
        """(value, min, max) for one axis."""
        b = array.array("i", [0] * 6)
        fcntl.ioctl(f, 0x80184540 + code, b, True)
        return b[0], b[1], b[2]

    def rd(code):
        return absinfo(code)[0]

    base = {c: rd(c) for c in AXES}
    span = {}
    for c in AXES:
        _, lo, hi = absinfo(c)
        reach = max(hi - base[c], base[c] - lo)
        span[c] = float(reach) if reach > 0 else 1.0
    print("axis ranges as the kernel reports them:")
    for c, n in AXES.items():
        _, lo, hi = absinfo(c)
        print(f"   {n:3s} {lo:7d} .. {hi:6d}")
    print()
    worst = {c: 0 for c in (3, 4)}
    print("   L2     R2      RX        RY     | shift from rest")
    try:
        while True:
            v = {c: rd(c) for c in AXES}
            # each axis against its own span, not a shared one
            l2 = abs(v[2] - base[2]) / span[2]
            r2 = abs(v[5] - base[5]) / span[5]
            drx = abs(v[3] - base[3]) / span[3]
            dry = abs(v[4] - base[4]) / span[4]
            if max(l2, r2) > 0.15:
                worst[3] = max(worst[3], drx)
                worst[4] = max(worst[4], dry)
            print(f"\r {l2*100:5.0f}% {r2*100:5.0f}%  {v[3]:7d} {v[4]:7d}   "
                  f"RX {drx*100:4.1f}%  RY {dry*100:4.1f}%   ", end="", flush=True)
            time.sleep(0.04)
    except KeyboardInterrupt:
        pass
    print("\n")
    peak = max(worst.values()) * 100
    print(f"largest stick shift while a trigger was pressed: "
          f"RX {worst[3]*100:.1f}%   RY {worst[4]*100:.1f}%")
    print()
    # Compare against the peak, not an average. A deadzone is a threshold, so
    # what matters is the largest excursion, not the typical one: a stick that
    # sits at 3% but spikes to 11% will still push the cursor through a 6%
    # deadzone every time.
    if peak < 1.0:
        print("  No measurable crosstalk. If the cursor still moves while you press a")
        print("  trigger, something other than the stick position is causing it.")
    else:
        print(f"  Set the pointer deadzone above {peak:.0f}% — say {min(40, int(peak) + 3)}% —")
        print("  to stop trigger presses reaching the cursor. Below that they will keep")
        print("  getting through, however small the average deflection looks.")
    print()
    print("  Keep your thumbs off the sticks while measuring: a brush against a stick")
    print("  registers as a far larger shift than the crosstalk being looked for.")


if __name__ == "__main__":
    main()
