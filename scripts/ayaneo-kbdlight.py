#!/usr/bin/env python3
"""ayaneo-kbdlight - control the AYANEO keyboard backlight from Linux.

Protocol reverse-engineered from AYASpace 3.2.0.4 (AYASpaceCef.exe), the
`rgb.set_keyboard_light` CEF handler at 0x14023a170 and its transport at
0x140194a70.

Transport
---------
A HID **feature report**, id `0x41`, 8 bytes total, sent to the keyboard MCU
(SiGma Micro, VID 1C4F PID 007C). The device declares the report in its HID
descriptor -- 7 bytes plus the id byte -- on the interface whose report
descriptor contains `85 41` (Report ID 0x41). On the SLIDE that is
`/dev/hidraw1`.

Report layout
-------------
    byte 0   0x41                      report id
    byte 1   R                         from `color` >> 16
    byte 2   G                         from `color` >> 8
    byte 3   B                         from `color` & 0xff
    byte 4   mode                      1 Monochrome, 2 Gradient, 3 Breathe
    byte 5   enable                    0 = off, 1 = on
    byte 6   0x40 | fnIson             high nibble is a constant 4
    byte 7   0x5A                      terminator

AYASpace also accepts a `brightness` field, and it does nothing: the setter
at 0x1401945b0 is three instructions -- spill both arguments, return. There is
no brightness field in the report. To dim the backlight, scale R/G/B yourself;
`--brightness` here does exactly that, client side.

No read-back
------------
The MCU implements SET_REPORT but STALLs GET_REPORT on 0x41 (EPIPE), which
AYASpace tolerates -- it checks the read's return value and carries on. So the
current state cannot be queried, and this tool keeps its own copy, the same
way AYASpace stores KeyBoardLightConfig in its SQLite database.
"""

import argparse
import fcntl
import glob
import json
import os
import sys

REPORT_ID = 0x41
TERMINATOR = 0x5A
REPORT_LEN = 8
STATE_FILE = "/var/lib/ayaneo/kbdlight.json"
# Effect modes. AYASpace builds the keyboard's picker from KeyboardLightMList,
# which offers only keys 1 and 2 -- but en_US.json defines three labels, and all
# three work on the hardware:
#     KeyboardLight.mode[0] "Monochrome"  -> 1
#     KeyboardLight.mode[1] "Gradient"    -> 2
#     KeyboardLight.mode[2] "Breathe"     -> 3
# Do not confuse these with rgbModeList, which is the *stick ring* effect list
# (0 Default, 1 Monochrome Breathe, 2 RGB Breathe, 3 Google Breathe, 4 Radar,
# 5 Ripple, 6 Monochromatic Always On). Radar and Ripple are spatial effects
# for the rings; sending 4 or 5 here is accepted and does nothing visible.
MODES = {"monochrome": 1, "gradient": 2, "breathe": 3}
MODE_NAMES = {v: k for k, v in MODES.items()}

# The keyboard's own colour presets, from KeyboardLightCList (non-FLIP_KB):
PRESETS = ["002FFF", "0FE6FB", "27F95B", "0800FF", "FFEA00", "FF0000"]

# AYASpace's own default for this machine, from KeyBoardLightConfig
DEFAULT = {"color": 0x002FFF, "mode": 1, "enable": 1, "fnIson": 0, "brightness": 100}


def hidiocsfeature(length):
    return 0xC0000000 | (length << 16) | (ord('H') << 8) | 0x06


def find_device():
    """The HID interface that declares feature report 0x41."""
    for h in sorted(glob.glob('/sys/class/hidraw/hidraw*')):
        try:
            desc = open(f"{h}/device/report_descriptor", 'rb').read()
        except OSError:
            continue
        # 0x85 is the Report ID item tag; 0x41 its value
        if b'\x85\x41' not in desc:
            continue
        return f"/dev/{os.path.basename(h)}"
    return None


def build_report(st):
    color = st["color"] & 0xFFFFFF
    scale = max(0, min(100, st.get("brightness", 100))) / 100.0
    r = int(((color >> 16) & 0xFF) * scale)
    g = int(((color >> 8) & 0xFF) * scale)
    b = int((color & 0xFF) * scale)
    return bytes([REPORT_ID, r, g, b,
                  st["mode"] & 0xFF,
                  1 if st["enable"] else 0,
                  0x40 | (1 if st["fnIson"] else 0),
                  TERMINATOR])


def send(node, report):
    buf = bytearray(report)
    fd = os.open(node, os.O_RDWR)
    try:
        fcntl.ioctl(fd, hidiocsfeature(len(buf)), buf, True)
    finally:
        os.close(fd)


def load_state(path):
    try:
        with open(path) as f:
            st = json.load(f)
    except (OSError, ValueError):
        return dict(DEFAULT)
    out = dict(DEFAULT)
    out.update({k: v for k, v in st.items() if k in DEFAULT})
    return out


def save_state(path, st):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, 'w') as f:
        json.dump(st, f, indent=1, sort_keys=True)
    os.replace(tmp, path)


def parse_color(s):
    s = s.strip().lstrip('#')
    named = {"red": "ff0000", "green": "00ff00", "blue": "0000ff",
             "white": "ffffff", "yellow": "ffff00", "cyan": "00ffff",
             "magenta": "ff00ff", "orange": "ff7f00", "off": "000000"}
    s = named.get(s.lower(), s)
    if len(s) == 3:
        s = "".join(c * 2 for c in s)
    if len(s) != 6:
        raise argparse.ArgumentTypeError(
            "colour must be RRGGBB, RGB, or a name like 'red'")
    return int(s, 16)


def describe(st, report):
    c = st["color"]
    return "\n".join([
        f"colour      #{c:06X}   (R {(c>>16)&0xFF}, G {(c>>8)&0xFF}, B {c&0xFF})",
        f"brightness  {st['brightness']}%  (applied client-side by scaling RGB)",
        f"mode        {st['mode']} ({MODE_NAMES.get(st['mode'], 'not a keyboard mode')})",
        f"enable      {'on' if st['enable'] else 'off'}",
        f"Fn light    {'on' if st['fnIson'] else 'off'}",
        f"report      {report.hex(' ')}",
    ])


def main():
    ap = argparse.ArgumentParser(
        prog="ayaneo-kbdlight",
        description="Control the AYANEO keyboard backlight.",
        epilog="With no options, prints the cached state without writing.")
    ap.add_argument("--device", help="hidraw node (default: autodetect by report id)")
    ap.add_argument("--state-file", default=STATE_FILE)
    ap.add_argument("--color", type=parse_color, metavar="RRGGBB",
                    help="colour, hex or a name (red, blue, white, off...)")
    ap.add_argument("--brightness", type=int, metavar="0-100",
                    help="scales RGB locally; the hardware has no brightness field")
    ap.add_argument("--mode", help="effect: " + ", ".join(sorted(MODES))
                    + ", or a raw number 0-255")
    ap.add_argument("--enable", choices=["on", "off"], help="backlight on/off")
    ap.add_argument("--fn", choices=["on", "off"], help="Fn key indicator light")
    ap.add_argument("--raw", metavar="HEX",
                    help="send 8 raw bytes verbatim, e.g. '41 00 2f ff 01 01 40 5a'")
    ap.add_argument("--reset", action="store_true", help="restore AYASpace defaults")
    a = ap.parse_args()

    node = a.device or find_device()
    if not node:
        sys.exit("no HID device declaring feature report 0x41 found "
                 "(is this an AYANEO handheld?)")

    if a.raw:
        report = bytes.fromhex(a.raw.replace(' ', ''))
        if len(report) != REPORT_LEN or report[0] != REPORT_ID or report[-1] != TERMINATOR:
            sys.exit(f"--raw needs {REPORT_LEN} bytes starting {REPORT_ID:#02x} "
                     f"and ending {TERMINATOR:#02x}")
        send(node, report)
        print(f"{node}: sent {report.hex(' ')}")
        return

    st = dict(DEFAULT) if a.reset else load_state(a.state_file)
    changed = a.reset
    if a.color is not None:
        st["color"] = a.color; changed = True
    if a.brightness is not None:
        if not 0 <= a.brightness <= 100:
            sys.exit("--brightness must be 0-100")
        st["brightness"] = a.brightness; changed = True
    if a.mode:
        if a.mode in MODES:
            st["mode"] = MODES[a.mode]
        else:
            try:
                st["mode"] = int(a.mode, 0) & 0xFF
            except ValueError:
                sys.exit(f"--mode must be one of {sorted(MODES)} or a number")
        changed = True
    if a.enable:
        st["enable"] = 1 if a.enable == "on" else 0; changed = True
    if a.fn:
        st["fnIson"] = 1 if a.fn == "on" else 0; changed = True

    report = build_report(st)
    if not changed:
        print(describe(st, report))
        print("\n(cached state; the MCU has no read-back for report 0x41)")
        return
    send(node, report)
    save_state(a.state_file, st)
    print(f"{node}: sent {report.hex(' ')}")
    print(describe(st, report))


if __name__ == '__main__':
    main()
