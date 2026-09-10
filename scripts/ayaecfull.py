#!/usr/bin/env python3
"""Dump / diff the whole 64KB AYANEO EC address space (all 256 pages).

  ayaecfull dump <label>          two passes, saves data + volatility mask
  ayaecfull diff <labelA> <labelB>

Volatile locations are excluded from diffs; without that, ~9% of the space
changes on its own (sensors, counters, register windows).
"""
import os, sys, time

ADDR, DATA = 0x4E, 0x4F
OUT = os.path.expanduser("~/ec-dumps")


def _rd(fd, h, i):
    w = lambda v, p: os.pwrite(fd, bytes([v]), p)
    w(0x2E, ADDR); w(0x11, DATA)
    w(0x2F, ADDR); w(h, DATA)
    w(0x2E, ADDR); w(0x10, DATA)
    w(0x2F, ADDR); w(i, DATA)
    w(0x2E, ADDR); w(0x12, DATA)
    w(0x2F, ADDR)
    return os.pread(fd, 1, DATA)[0]


def full():
    fd = os.open("/dev/port", os.O_RDWR)
    try:
        return bytes(_rd(fd, h, i) for h in range(256) for i in range(256))
    finally:
        os.close(fd)


def load(label):
    with open(os.path.join(OUT, f"full-{label}.bin"), "rb") as fh:
        d = fh.read()
    vp = os.path.join(OUT, f"full-{label}.volatile")
    vol = set()
    if os.path.exists(vp):
        t = open(vp).read().strip()
        if t:
            vol = {int(x) for x in t.split(",")}
    return d, vol


def main():
    if os.geteuid() != 0:
        print("needs root"); return 1
    if len(sys.argv) < 3:
        print(__doc__); return 2

    if sys.argv[1] == "dump":
        label = sys.argv[2]
        a = full(); time.sleep(3); b = full()
        vol = {i for i in range(len(a)) if a[i] != b[i]}
        os.makedirs(OUT, exist_ok=True)
        open(os.path.join(OUT, f"full-{label}.bin"), "wb").write(b)
        open(os.path.join(OUT, f"full-{label}.volatile"), "w").write(
            ",".join(map(str, sorted(vol))))
        print(f"saved full-{label}.bin, {len(vol)} volatile locations")
        return 0

    if sys.argv[1] == "diff":
        if len(sys.argv) < 4:
            print("usage: ayaecfull diff <A> <B>"); return 2
        a, va = load(sys.argv[2]); b, vb = load(sys.argv[3])
        vol = va | vb
        ch = [i for i in range(len(a)) if a[i] != b[i] and i not in vol]
        ign = sum(1 for i in range(len(a)) if a[i] != b[i] and i in vol)
        print(f"{sys.argv[2]} -> {sys.argv[3]}")
        print(f"  {len(ch)} stable location(s) changed ({ign} volatile ignored)\n")
        for i in ch:
            print(f"  EC 0x{i>>8:02x}{i & 0xff:02x} : "
                  f"0x{a[i]:02x} ({a[i]:3})  ->  0x{b[i]:02x} ({b[i]:3})")
        if not ch:
            print("  nothing changed anywhere in the 64KB space")
        return 0

    print(__doc__); return 2


if __name__ == "__main__":
    sys.exit(main())
