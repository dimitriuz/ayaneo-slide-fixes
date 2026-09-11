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
FS = 32768.0


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

    def rd(code):
        b = array.array("i", [0] * 6)
        fcntl.ioctl(f, 0x80184540 + code, b, True)
        return b[0]

    base = {c: rd(c) for c in AXES}
    worst = {c: 0 for c in (3, 4)}
    print("   L2     R2      RX        RY     | shift from rest")
    try:
        while True:
            v = {c: rd(c) for c in AXES}
            # triggers rest at the negative end on this pad; normalise to 0..1
            l2 = (v[2] - base[2]) / FS
            r2 = (v[5] - base[5]) / FS
            drx = abs(v[3] - base[3]) / FS
            dry = abs(v[4] - base[4]) / FS
            if max(l2, r2) > 0.05:
                worst[3] = max(worst[3], drx)
                worst[4] = max(worst[4], dry)
            print(f"\r {l2*100:5.0f}% {r2*100:5.0f}%  {v[3]:7d} {v[4]:7d}   "
                  f"RX {drx*100:4.1f}%  RY {dry*100:4.1f}%   ", end="", flush=True)
            time.sleep(0.04)
    except KeyboardInterrupt:
        pass
    print("\n")
    print(f"largest stick shift while a trigger was pressed: "
          f"RX {worst[3]*100:.1f}%   RY {worst[4]*100:.1f}%")
    peak = max(worst.values()) * 100
    if peak < 1.0:
        print("  No measurable crosstalk. If the cursor still moves, it is not this.")
    else:
        print(f"  Set the pointer deadzone above {peak:.0f}% to stop the cursor reacting")
        print("  to trigger presses. Below that it will keep happening.")


if __name__ == "__main__":
    main()
