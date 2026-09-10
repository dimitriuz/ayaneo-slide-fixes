#!/usr/bin/env python3
"""Dump the ACPI EC RAM, and profile which bytes are volatile.

  ecdump <label>        take N samples, save them, report volatile offsets

Volatile bytes (temperatures, battery, fan) change on their own and must be
ignored when diffing a settings change, or they drown the signal.
"""
import os, sys, time
SRC="/sys/kernel/debug/ec/ec0/io"
OUT=os.path.expanduser("~/ec-dumps")

def ensure_ec():
    """ec_sys is not loaded by default and does not survive a reboot."""
    if os.path.exists(SRC):
        return
    os.system("modprobe ec_sys >/dev/null 2>&1")
    if not os.path.ismount("/sys/kernel/debug"):
        os.system("mount -t debugfs none /sys/kernel/debug >/dev/null 2>&1")
    time.sleep(0.5)


def read():
    with open(SRC,"rb") as fh:
        return fh.read()

def main():
    label = sys.argv[1] if len(sys.argv)>1 else "dump"
    n = int(sys.argv[2]) if len(sys.argv)>2 else 6
    ensure_ec()
    if not os.path.exists(SRC):
        print('EC interface unavailable: is ec_sys loaded?'); return 1
    os.makedirs(OUT, exist_ok=True)
    samples=[]
    for i in range(n):
        samples.append(read())
        if i < n-1: time.sleep(2)
    path=os.path.join(OUT, f"ec-{label}.bin")
    with open(path,"wb") as fh:
        fh.write(samples[-1])
    volatile={i for i in range(len(samples[0]))
              if len({s[i] for s in samples}) > 1}
    with open(os.path.join(OUT, f"ec-{label}.volatile"),"w") as fh:
        fh.write(",".join(str(i) for i in sorted(volatile)))
    print(f"saved {path} ({len(samples[-1])} bytes, {n} samples)")
    print(f"volatile offsets ({len(volatile)}): "
          + " ".join(f"0x{i:02x}" for i in sorted(volatile)))
    print("\nstable snapshot:")
    d=samples[-1]
    for off in range(0, len(d), 16):
        row=" ".join(("--" if off+k in volatile else f"{d[off+k]:02x}")
                     for k in range(16) if off+k < len(d))
        print(f"  {off:03x}: {row}")

if __name__=="__main__":
    sys.exit(main())
