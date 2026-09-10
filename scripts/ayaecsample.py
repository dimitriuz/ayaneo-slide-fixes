#!/usr/bin/env python3
"""Characterise the EC properly: many samples over minutes, keep only what is stable.

  ayaecsample take <label> [n] [interval]   default 10 samples, 12s apart
  ayaecsample cmp  <A> <B>                  compare two stable sets

The EC drifts continuously, so a two-pass mask taken seconds apart is useless -
it misses locations that move on a timescale of minutes. This keeps a location
only if it held ONE value across every sample in the run, and compares only
locations stable in BOTH runs.
"""
import os, sys, time

ADDR, DATA = 0x4E, 0x4F
OUT = os.path.expanduser("~/ec-dumps")


def full():
    fd = os.open("/dev/port", os.O_RDWR)
    w = lambda v, p: os.pwrite(fd, bytes([v]), p)
    try:
        out = bytearray(65536)
        for h in range(256):
            for i in range(256):
                w(0x2E, ADDR); w(0x11, DATA)
                w(0x2F, ADDR); w(h, DATA)
                w(0x2E, ADDR); w(0x10, DATA)
                w(0x2F, ADDR); w(i, DATA)
                w(0x2E, ADDR); w(0x12, DATA)
                w(0x2F, ADDR)
                out[(h << 8) | i] = os.pread(fd, 1, DATA)[0]
        return bytes(out)
    finally:
        os.close(fd)


def take(label, n, interval):
    samples = []
    for k in range(n):
        samples.append(full())
        print(f"  sample {k+1}/{n}", flush=True)
        if k < n - 1:
            time.sleep(interval)
    stable = bytearray(65536)
    mask = bytearray(65536)
    for i in range(65536):
        v = samples[0][i]
        if all(s[i] == v for s in samples):
            stable[i] = v
            mask[i] = 1
    os.makedirs(OUT, exist_ok=True)
    open(os.path.join(OUT, f"st-{label}.bin"), "wb").write(bytes(stable))
    open(os.path.join(OUT, f"st-{label}.mask"), "wb").write(bytes(mask))
    ns = sum(mask)
    print(f"\nsaved st-{label}: {ns} stable of 65536 "
          f"({65536-ns} drift over {(n-1)*interval}s)")


def cmp_(a, b):
    da = open(os.path.join(OUT, f"st-{a}.bin"), "rb").read()
    ma = open(os.path.join(OUT, f"st-{a}.mask"), "rb").read()
    db = open(os.path.join(OUT, f"st-{b}.bin"), "rb").read()
    mb = open(os.path.join(OUT, f"st-{b}.mask"), "rb").read()
    # Pages known to change on their own across a reboot, established by a
    # control run in which nothing was altered: 0xce is a circular log that
    # advances every boot, 0xc0 holds a couple of counters.
    NOISE_PAGES = {0xce, 0xc0}
    both = [i for i in range(65536) if ma[i] and mb[i]]
    ch = [i for i in both if da[i] != db[i] and (i >> 8) not in NOISE_PAGES]
    noisy = [i for i in both if da[i] != db[i] and (i >> 8) in NOISE_PAGES]
    print(f"{a} -> {b}")
    print(f"  {len(both)} locations stable in both runs")
    print(f"  {len(ch)} changed outside known-noise pages "
          f"({len(noisy)} ignored in pages 0xce/0xc0)\n")
    for i in ch:
        print(f"  EC 0x{i>>8:02x}{i & 0xff:02x} : "
              f"0x{da[i]:02x} ({da[i]:3})  ->  0x{db[i]:02x} ({db[i]:3})")
    if not ch:
        print("  none")


def main():
    if os.geteuid() != 0:
        print("needs root"); return 1
    if len(sys.argv) < 3:
        print(__doc__); return 2
    if sys.argv[1] == "take":
        n = int(sys.argv[3]) if len(sys.argv) > 3 else 10
        iv = int(sys.argv[4]) if len(sys.argv) > 4 else 12
        take(sys.argv[2], n, iv); return 0
    if sys.argv[1] == "cmp" and len(sys.argv) >= 4:
        cmp_(sys.argv[2], sys.argv[3]); return 0
    print(__doc__); return 2


if __name__ == "__main__":
    sys.exit(main())
