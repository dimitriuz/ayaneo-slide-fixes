#!/usr/bin/env python3
"""stickverify - live side-by-side view of the physical gamepad and the
gamepad InputPlumber actually presents to applications.

Run it, move a stick, watch. Ctrl+C to stop.

Why it reads hidraw and not evdev
---------------------------------
InputPlumber's default gamepad target on this machine is `deck-uhid`, a
*UHID* device ("Generic Steam Controller", 28DE:12F0) with no evdev node at
all. Watching /dev/input/event* for the emulated pad therefore shows nothing,
no matter how much the sticks move -- and the evdev node that looks like the
target ("Microsoft X-Box 360 pad 0") is registered by InputPlumber as a
*source*, not an output.

So the physical pad is polled with EVIOCGABS (which reflects the kernel's
current value despite InputPlumber's exclusive grab), and the emulated pad is
decoded from its 64-byte HID input reports.

Report layout is from InputPlumber's own PackedInputDataReport
(src/drivers/steam_deck/hid_report.rs): l_stick_x/y at bytes 48..51,
r_stick_x/y at 52..55, i16 little-endian, full scale +/-32767. The deck report
uses an inverted Y axis, which is undone here so the two columns compare
directly.

Reading the ratio
-----------------
`ratio` is |emulated| / |physical|, both normalised to full scale, shown only
once physical deflection exceeds 3%.

  ~1.00 at all deflections     linear passthrough
  falls as deflection falls    quadratic_scaling: true is in effect
                               (0.50 deflection -> ratio 0.50,
                                0.14 deflection -> ratio 0.14)
  0.00 below ~20% on a mouse   the hardcoded axis->mouse deadzone
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

AXES = [('ABS_X', 0, 48, False), ('ABS_Y', 1, 50, True),
        ('ABS_RX', 3, 52, False), ('ABS_RY', 4, 54, True)]
PHYS_FS = 32768.0
EMU_FS = 32767.0
REPORT_LEN = 64


def sysfs_name(path):
    try:
        return open(f"/sys/class/input/{os.path.basename(path)}/device/name").read().strip()
    except OSError:
        return "?"


def find_physical():
    """A real USB gamepad: has a usb Phys and stick axes."""
    for d in sorted(glob.glob('/dev/input/event*'),
                    key=lambda p: int(p.rsplit('event', 1)[1])):
        n = sysfs_name(d)
        if 'X-Box' not in n and 'Xbox' not in n and 'pad' not in n.lower():
            continue
        try:
            phys = open(f"/sys/class/input/{os.path.basename(d)}/device/phys").read().strip()
        except OSError:
            phys = ""
        if phys.startswith('usb-'):
            return d, n
    return None, None


def find_emulated_hidraw():
    """InputPlumber's UHID gamepad target: a Valve (28DE) HID device with no phys."""
    for h in sorted(glob.glob('/sys/class/hidraw/hidraw*')):
        try:
            ue = open(f"{h}/device/uevent").read()
        except OSError:
            continue
        fields = dict(l.split('=', 1) for l in ue.strip().split('\n') if '=' in l)
        hid_id = fields.get('HID_ID', '')
        if '28DE' not in hid_id.upper():
            continue
        if fields.get('HID_PHYS', ''):      # a real, physically attached device
            continue
        return f"/dev/{os.path.basename(h)}", fields.get('HID_NAME', '?')
    return None, None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--physical', help='override physical evdev path')
    ap.add_argument('--emulated', help='override emulated hidraw path')
    a = ap.parse_args()

    phys, pname = (a.physical, sysfs_name(a.physical)) if a.physical else find_physical()
    emu, ename = (a.emulated, '?') if a.emulated else find_emulated_hidraw()
    if not phys:
        sys.exit("could not find the physical gamepad")
    print(f'physical : {phys}  "{pname}"  (EVIOCGABS poll)')
    if emu:
        print(f'emulated : {emu}  "{ename}"  (HID input reports)')
    else:
        print("emulated : no UHID gamepad target found - is inputplumber running?")
    print("\nMove a stick. Ctrl+C to stop.\n")

    fp = open(phys, 'rb', buffering=0)

    def poll(code):
        b = array.array('i', [0] * 6)
        fcntl.ioctl(fp, 0x80184540 + code, b, True)
        return b[0]

    fe = None
    if emu:
        try:
            fe = open(emu, 'rb', buffering=0)
            os.set_blocking(fe.fileno(), False)
        except OSError as e:
            print(f"cannot open {emu}: {e}")
            fe = None

    emu_val = {name: 0 for name, _, _, _ in AXES}
    reports = 0
    samples = {name: [] for name, _, _, _ in AXES}

    print(f"{'t':>6s} " + "".join(f"{n:>25s}" for n, _, _, _ in AXES))
    print(f"{'':6s} " + "".join(f"{'phys     emu   ratio':>25s}" for _ in AXES))
    t0 = time.time()
    try:
        while True:
            if fe:
                while select.select([fe.fileno()], [], [], 0)[0]:
                    try:
                        buf = fe.read(REPORT_LEN)
                    except (BlockingIOError, OSError):
                        break
                    if not buf or len(buf) < REPORT_LEN:
                        break
                    reports += 1
                    for name, _, off, inv in AXES:
                        v = struct.unpack_from('<h', buf, off)[0]
                        emu_val[name] = -v if inv else v
            row = ""
            for name, code, _, _ in AXES:
                p = poll(code)
                e = emu_val[name]
                pn, en = abs(p) / PHYS_FS, abs(e) / EMU_FS
                if pn > 0.03:
                    row += f"{p:7d} {e:7d} {en/pn:7.2f}  "
                    samples[name].append((pn, en))
                else:
                    row += f"{p:7d} {e:7d} {'-':>7s}  "
            print(f"\r{time.time()-t0:6.1f} {row}", end="", flush=True)
            time.sleep(0.05)
    except KeyboardInterrupt:
        pass

    print("\n")
    print(f"HID reports received from the emulated pad : {reports}")
    if reports == 0:
        print("\n  Nothing arrived from the emulated pad. Either it is not the active")
        print("  target, or nothing moved. Check: journalctl -u inputplumber | grep target")
        return
    print()
    for name, _, _, _ in AXES:
        s = samples[name]
        if not s:
            print(f"  {name:7s} not deflected past 3%")
            continue
        peak = max(p for p, _ in s)
        bands = [(0.03, 0.10), (0.10, 0.25), (0.25, 0.50), (0.50, 1.01)]
        parts = []
        for lo, hi in bands:
            rs = [e / p for p, e in s if lo <= p < hi]
            if rs:
                parts.append(f"{int(lo*100)}-{int(hi*100)}%: {sum(rs)/len(rs):.2f}")
        print(f"  {name:7s} peak {peak*100:3.0f}%   mean ratio by band  " + "   ".join(parts))
    print("\n  Ratio ~1.00 across all bands  -> linear passthrough.")
    print("  Ratio tracking the deflection -> quadratic_scaling is active.")


if __name__ == '__main__':
    main()
