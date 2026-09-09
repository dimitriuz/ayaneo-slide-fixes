#!/usr/bin/env python3
"""Live check: does each stick drive BOTH the emulated gamepad and the mouse?

No timers - Ctrl+C when done. Reads the emulated pad (gamepad axes) and the
InputPlumber mouse target (relative motion) at the same time, so you can see
whether a stick feeds both paths.
"""
import glob, os, select, struct, sys, time

FMT, SZ = "llHHi", struct.calcsize("llHHi")
FS = 32767


def find(substr):
    for p in sorted(glob.glob("/dev/input/event*")):
        try:
            nm = open(f"/sys/class/input/{os.path.basename(p)}/device/name").read().strip()
        except OSError:
            continue
        if substr.lower() in nm.lower():
            return p, nm
    return None, None


def main():
    if os.geteuid() != 0:
        print("run with sudo"); return 1
    pad, padnm = find("X-Box 360 pad 0")      # InputPlumber's emulated gamepad
    mouse, mousenm = find("InputPlumber Mouse")
    if not pad or not mouse:
        print(f"missing device (pad={padnm}, mouse={mousenm})"); return 1
    print(f"gamepad : {padnm}  ({pad})")
    print(f"mouse   : {mousenm}  ({mouse})")
    print("\nMove each stick. Both columns should react.")
    print("Ctrl+C when done.\n")
    fds = {}
    for p in (pad, mouse):
        fds[os.open(p, os.O_RDONLY | os.O_NONBLOCK)] = p
    ax = {0: 0, 1: 0, 3: 0, 4: 0}
    rel = {0: 0, 1: 0}
    n_ax = n_rel = 0
    try:
        while True:
            r, _, _ = select.select(list(fds), [], [], 0.05)
            for fd in r:
                try:
                    data = os.read(fd, SZ * 128)
                except OSError:
                    continue
                for i in range(0, len(data) - SZ + 1, SZ):
                    _, _, typ, code, val = struct.unpack(FMT, data[i:i + SZ])
                    if fds[fd] == pad and typ == 3 and code in ax:
                        ax[code] = val; n_ax += 1
                    elif fds[fd] == mouse and typ == 2 and code in rel:
                        rel[code] = val; n_rel += 1
            sys.stdout.write(
                f"\rGAMEPAD  L({ax[0]:>6},{ax[1]:>6}) R({ax[3]:>6},{ax[4]:>6}) ev={n_ax:<6}"
                f" |  MOUSE  dx={rel[0]:>4} dy={rel[1]:>4} ev={n_rel:<6}")
            sys.stdout.flush()
    except KeyboardInterrupt:
        pass
    for fd in fds:
        os.close(fd)
    print("\n\n=== RESULT ===")
    print(f"  gamepad axis events : {n_ax}   -> {'OK, games still work' if n_ax else 'BROKEN - passthrough lost'}")
    print(f"  mouse motion events : {n_rel}   -> {'OK, stick drives pointer' if n_rel else 'not working'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
