#!/usr/bin/env python3
"""mouseverify - live view of right-stick deflection against the pointer motion
InputPlumber actually emits, for tuning the axis-to-mouse deadzone.

Run it, move the right stick slowly outward from centre, watch where motion
starts. Ctrl+C to stop.

Why it reads what it reads
--------------------------
The physical pad is polled with EVIOCGABS, which reflects the kernel's current
value despite InputPlumber's exclusive grab. Pointer motion is read from the
REL_X/REL_Y stream of the "InputPlumber Mouse" evdev device, which is the real
output -- unlike the emulated *gamepad*, which is a UHID device with no evdev
node at all.

Note that InputPlumber is event-driven: MouseDevice::poll() integrates a stored
velocity over elapsed time, and that velocity only changes when a translated
event arrives. A perfectly stationary stick emits nothing, so it produces no
motion however far it is deflected -- and conversely, a stick left resting
above the deadzone keeps the pointer moving until the next event. Both are
expected; move the stick to see anything at all.
"""
import argparse
import array
import fcntl
import glob
import os
import select
import struct
import sys
import time

EVFMT = 'llHHi'
EVSZ = struct.calcsize(EVFMT)
FS = 32768.0
ABS_RX, ABS_RY = 3, 4


def name_of(dev):
    try:
        return open(f"/sys/class/input/{os.path.basename(dev)}/device/name").read().strip()
    except OSError:
        return "?"


def find():
    mouse = phys = None
    for d in sorted(glob.glob('/dev/input/event*'),
                    key=lambda p: int(p.rsplit('event', 1)[1])):
        n = name_of(d)
        if n == 'InputPlumber Mouse' and mouse is None:
            mouse = d
        elif n == 'Microsoft X-Box 360 pad' and phys is None:
            try:
                ph = open(f"/sys/class/input/{os.path.basename(d)}/device/phys").read()
            except OSError:
                ph = ""
            if ph.startswith('usb-'):
                phys = d
    return phys, mouse


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--physical')
    ap.add_argument('--mouse')
    a = ap.parse_args()
    phys, mouse = find()
    phys = a.physical or phys
    mouse = a.mouse or mouse
    if not phys or not mouse:
        sys.exit(f"could not find both devices (phys={phys} mouse={mouse})")
    print(f'physical : {phys}  "{name_of(phys)}"')
    print(f'mouse    : {mouse}  "{name_of(mouse)}"')
    print("\nMove the RIGHT stick slowly outward from centre. Ctrl+C to stop.\n")

    fp = open(phys, 'rb', buffering=0)
    fm = open(mouse, 'rb', buffering=0)
    os.set_blocking(fm.fileno(), False)

    def rd(code):
        b = array.array('i', [0] * 6)
        fcntl.ioctl(fp, 0x80184540 + code, b, True)
        return b[0]

    # smallest deflection that coincided with pointer motion, and the largest
    # that produced none -- the deadzone sits between them
    moved_min = None
    still_max = 0.0
    px = 0
    t0 = time.time()
    try:
        while True:
            rx, ry = rd(ABS_RX), rd(ABS_RY)
            defl = max(abs(rx), abs(ry)) / FS
            moved = 0
            while select.select([fm.fileno()], [], [], 0)[0]:
                try:
                    data = fm.read(EVSZ * 64)
                except (BlockingIOError, OSError):
                    break
                if not data:
                    break
                for i in range(0, len(data) - EVSZ + 1, EVSZ):
                    _, _, typ, code, val = struct.unpack(EVFMT, data[i:i + EVSZ])
                    if typ == 2 and code in (0, 1):
                        moved += abs(val)
            px += moved
            if moved:
                moved_min = defl if moved_min is None else min(moved_min, defl)
            elif defl > still_max and moved_min is None:
                still_max = defl
            bar = '#' * int(defl * 40)
            print(f"\r{time.time()-t0:6.1f}  RX={rx:7d} RY={ry:7d}  "
                  f"deflection {defl*100:5.1f}%  motion {moved:3d}px  {bar:<40}",
                  end="", flush=True)
            time.sleep(0.03)
    except KeyboardInterrupt:
        pass
    print("\n")
    print(f"total pointer travel        : {px} px")
    if moved_min is None:
        print("no pointer motion seen — did the right stick move?")
    else:
        print(f"motion first seen at        : {moved_min*100:.1f}% deflection")
        print(f"largest still deflection    : {still_max*100:.1f}%")
        print(f"\n  The effective deadzone lies between those two. Set it in the"
              f"\n  profile's mouse motion target, a little ABOVE the stick's"
              f"\n  resting offset so it does not drift:"
              f"\n\n      - mouse:"
              f"\n          motion:"
              f"\n            speed_pps: 800"
              f"\n            deadzone: 0.06")


if __name__ == '__main__':
    main()
