#!/usr/bin/env python3
"""Live analog-stick readout. No timers, no rush - Ctrl+C when done.

Releases InputPlumber's grab on start and restores it on exit.
Tracks, per axis, the smallest value you HELD (>=0.3s) that is above noise.
"""
import os, select, signal, struct, subprocess, sys, time

FMT, SZ = "llHHi", struct.calcsize("llHHi")
AX = {0: "LEFT  X", 1: "LEFT  Y", 3: "RIGHT X", 4: "RIGHT Y"}
FS, NOISE, HOLD = 32767, 512, 0.30
DEV = "/dev/input/event6"
PROFILE = "/etc/inputplumber/profiles/rightstick-mouse.yaml"


def sh(*a):
    return subprocess.run(a, capture_output=True, text=True).stdout.strip()


def restore():
    print("\n\nrestoring InputPlumber...", flush=True)
    sh("systemctl", "restart", "inputplumber")
    time.sleep(4)
    if os.path.exists(PROFILE):
        sh("inputplumber", "device", "0", "profile", "load", PROFILE)
    print("done.")


def main():
    if os.geteuid() != 0:
        print("run with sudo"); return 1
    print("releasing InputPlumber grab...")
    sh("inputplumber", "device", "0", "stop")
    time.sleep(1.5)
    try:
        fd = os.open(DEV, os.O_RDONLY | os.O_NONBLOCK)
    except OSError as e:
        print(f"cannot open {DEV}: {e}"); restore(); return 1

    cur = {k: 0 for k in AX}
    since = {k: time.time() for k in AX}
    best = {k: None for k in AX}          # smallest HELD |v| above noise
    peak = {k: 0 for k in AX}

    print("\nMove each stick. Hold small deflections still for ~1s.")
    print("'held min' = smallest value you sustained; that is the real deadzone.")
    print("Press Ctrl+C when finished.\n")
    signal.signal(signal.SIGINT, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt))
    try:
        while True:
            r, _, _ = select.select([fd], [], [], 0.05)
            now = time.time()
            if r:
                try:
                    data = os.read(fd, SZ * 128)
                except OSError:
                    data = b""
                for i in range(0, len(data) - SZ + 1, SZ):
                    _, _, typ, code, val = struct.unpack(FMT, data[i:i + SZ])
                    if typ == 3 and code in cur and val != cur[code]:
                        cur[code], since[code] = val, now
            for k in AX:
                v = abs(cur[k])
                peak[k] = max(peak[k], v)
                if v > NOISE and now - since[k] >= HOLD:
                    if best[k] is None or v < best[k]:
                        best[k] = v
            line = []
            for k, nm in AX.items():
                b = best[k]
                line.append(f"{nm} {cur[k]:>7} ({100*cur[k]/FS:5.1f}%) "
                            f"heldmin {b if b else '-':>6}")
            sys.stdout.write("\r" + " | ".join(line[:2]) + "\n" +
                             " | ".join(line[2:]) + "\033[1A\r")
            sys.stdout.flush()
    except KeyboardInterrupt:
        pass
    os.close(fd)
    print("\n\n=== RESULT ===")
    print(f"{'axis':10} {'held min':>9} {'%FS':>7} {'peak':>8} {'%FS':>7}   verdict")
    for k, nm in AX.items():
        b = best[k] or 0
        pc = 100 * b / FS
        verdict = ("no held value above noise" if not b else
                   "GOOD - fine control near centre" if pc < 10 else
                   f"deadzone ~{pc:.0f}% - usable" if pc < 20 else
                   f"BAD - deadzone ~{pc:.0f}% of full scale")
        print(f"{nm:10} {b:>9} {pc:6.1f}% {peak[k]:>8} {100*peak[k]/FS:6.1f}%   {verdict}")
    print("\nCompare LEFT vs RIGHT: if left is <10% and right is >40%,")
    print("the right stick is faulty rather than the firmware applying a policy.")
    restore()
    return 0


if __name__ == "__main__":
    sys.exit(main())
