#!/usr/bin/env python3
"""Measure analog-stick and pointer behaviour on a Linux handheld.

  --abs N [dev]   read raw ABS_RX/ABS_RY for N seconds and report steady-state
                  values as a percentage of full scale, plus the smallest
                  non-zero magnitude seen (i.e. the effective hardware deadzone)
  --rel N [dev]   read REL_X/REL_Y for N seconds and report the histogram of
                  step sizes (small steps == fine pointer control)
  --info [dev]    print each axis's min/max/fuzz/flat (works on a grabbed device)

HOLD positions still rather than sweeping. Sweeps sample transients and overstate
what the hardware reports near centre.

To read the physical pad, first release InputPlumber's exclusive grab:
    inputplumber device 0 stop        # ...measure...
    sudo systemctl restart inputplumber
"""
import fcntl, glob, os, select, struct, sys, time

FMT = "llHHi"
SZ = struct.calcsize(FMT)
EV_KEY, EV_REL, EV_ABS = 1, 2, 3
ABS_RX, ABS_RY = 3, 4
REL_X, REL_Y = 0, 1
FS = 32767


def dev_name(path):
    node = os.path.basename(path)
    try:
        with open(f"/sys/class/input/{node}/device/name") as fh:
            return fh.read().strip()
    except OSError:
        return "?"


def absinfo(fd, axis):
    """EVIOCGABS - works even while another process holds an exclusive grab."""
    buf = bytearray(24)
    fcntl.ioctl(fd, (2 << 30) | (24 << 16) | (ord("E") << 8) | (0x40 + axis), buf)
    return struct.unpack("6i", bytes(buf))  # value min max fuzz flat resolution


def find(kind):
    """Pick a sensible default device: one advertising ABS_RX, or a mouse."""
    for path in sorted(glob.glob("/dev/input/event*")):
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
        except OSError:
            continue
        try:
            if kind == "abs":
                _, mn, mx, _, _, _ = absinfo(fd, ABS_RX)
                if mx > mn:
                    return path
            else:
                if "mouse" in dev_name(path).lower():
                    return path
        except OSError:
            pass
        finally:
            os.close(fd)
    return None


def mode_info(path):
    fd = os.open(path, os.O_RDONLY)
    print(f"{dev_name(path)}  ({path})")
    print(f"  {'axis':8} {'value':>8} {'min':>8} {'max':>8} {'fuzz':>6} {'flat':>6}")
    for axis, nm in ((0, "ABS_X"), (1, "ABS_Y"), (ABS_RX, "ABS_RX"), (ABS_RY, "ABS_RY")):
        try:
            v, mn, mx, fz, fl, _ = absinfo(fd, axis)
            pct = (100 * fl / mx) if mx else 0
            print(f"  {nm:8} {v:8} {mn:8} {mx:8} {fz:6} {fl:6}   flat={pct:.1f}% of range")
        except OSError as exc:
            print(f"  {nm:8} unavailable ({exc.strerror})")
    os.close(fd)


def mode_abs(path, dur):
    fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
    print(f"{dev_name(path)}  ({path})  -- HOLD positions, do not sweep\n")
    print(f"{'t':>7} {'ABS_RX':>8} {'%FS':>8} {'ABS_RY':>8} {'%FS':>8}")
    cur = {ABS_RX: 0, ABS_RY: 0}
    seen = {ABS_RX: set(), ABS_RY: set()}
    last, t0 = None, time.time()
    while time.time() - t0 < dur:
        ready, _, _ = select.select([fd], [], [], 0.2)
        if ready:
            try:
                data = os.read(fd, SZ * 128)
            except OSError:
                data = b""
            for i in range(0, len(data) - SZ + 1, SZ):
                _, _, typ, code, val = struct.unpack(FMT, data[i:i + SZ])
                if typ == EV_ABS and code in cur:
                    cur[code] = val
                    seen[code].add(val)
        key = (cur[ABS_RX] // 512, cur[ABS_RY] // 512)
        if key != last:
            print(f"{time.time()-t0:7.1f} {cur[ABS_RX]:8} {100*cur[ABS_RX]/FS:7.1f}%"
                  f" {cur[ABS_RY]:8} {100*cur[ABS_RY]/FS:7.1f}%")
            last = key
    os.close(fd)
    print("\n--- effective hardware deadzone ---")
    for code, nm in ((ABS_RX, "ABS_RX"), (ABS_RY, "ABS_RY")):
        nz = sorted(abs(v) for v in seen[code] if v)
        if not nz:
            print(f"  {nm}: no non-zero values seen")
            continue
        print(f"  {nm}: {len(seen[code])} distinct, smallest non-zero |v| = {nz[0]}"
              f"  ({100*nz[0]/FS:.1f}% FS)")
        if 100 * nz[0] / FS > 15:
            print(f"      -> hardware/firmware deadzone of roughly {100*nz[0]/FS:.0f}%;"
                  " no software fix can recover the missing range")


def mode_rel(path, dur):
    fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
    print(f"{dev_name(path)}  ({path})  -- move the stick/pointer now")
    hist, total, t0 = {}, 0, time.time()
    while time.time() - t0 < dur:
        ready, _, _ = select.select([fd], [], [], 0.3)
        if not ready:
            continue
        try:
            data = os.read(fd, SZ * 128)
        except OSError:
            continue
        for i in range(0, len(data) - SZ + 1, SZ):
            _, _, typ, code, val = struct.unpack(FMT, data[i:i + SZ])
            if typ == EV_REL and code in (REL_X, REL_Y):
                nm = "REL_X" if code == REL_X else "REL_Y"
                hist.setdefault(nm, {})
                hist[nm][abs(val)] = hist[nm].get(abs(val), 0) + 1
                total += 1
    os.close(fd)
    print(f"\ntotal REL events: {total}")
    if not total:
        print("  none - motion may be injected above evdev (XTEST / virtual pointer),"
              " or the device is exclusively grabbed")
    for nm, buckets in sorted(hist.items()):
        print(f"  {nm}: distinct step sizes={len(buckets)}"
              f"  smallest={sorted(buckets.items())[:12]}")


def main():
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    mode = args[0]
    if mode == "--info":
        path = args[1] if len(args) > 1 else find("abs")
        if not path:
            print("no suitable device found", file=sys.stderr)
            return 1
        mode_info(path)
        return 0
    if mode not in ("--abs", "--rel"):
        print(__doc__)
        return 2
    dur = float(args[1]) if len(args) > 1 else 30.0
    kind = "abs" if mode == "--abs" else "rel"
    path = args[2] if len(args) > 2 else find(kind)
    if not path:
        print(f"no suitable {kind} device found; pass one explicitly", file=sys.stderr)
        return 1
    (mode_abs if kind == "abs" else mode_rel)(path, dur)
    return 0


if __name__ == "__main__":
    sys.exit(main())
