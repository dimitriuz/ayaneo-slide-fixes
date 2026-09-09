#!/usr/bin/env python3
"""AYANEO vendor HID channel tool (AYANEO SLIDE / AS01).

The stick deadzone is a setting stored in the controller, written by AYASpace
over a vendor HID feature report. This finds that channel on Linux and can talk
to it.

  --info            find and describe the vendor HID interface
  --raw AA:BB:...   send one raw feature report (DANGEROUS - see below)
  --deadzone N      set the stick deadzone (NOT YET IMPLEMENTED - needs the
                    payload bytes from a USB capture, see
                    docs/CAPTURE-DEADZONE.md)

Channel identified by reverse engineering AYASpaceCef.exe 3.2.0.4:

    device : USB VID 1C4F PID 007C   (the slide-out keyboard's MCU)
    iface  : the one with Usage Page 0xFF00, Usage 0x02
    report : FEATURE, Report ID 0x41, 7 data bytes, write-only

WARNING about --raw: this writes to the microcontroller that runs your keyboard,
its backlight and the stick configuration. Unknown commands may change settings
you did not intend. Only send bytes you have actually observed the vendor
software send. There is no read-back: the device stalls GET_REPORT.
"""
import argparse
import fcntl
import glob
import os
import sys

VENDOR_USAGE_PAGE = 0xFF00
REPORT_ID = 0x41
REPORT_LEN = 7          # data bytes, excluding the report ID


def _hidraw_nodes():
    for path in sorted(glob.glob("/sys/class/hidraw/hidraw*")):
        name = os.path.basename(path)
        try:
            with open(os.path.join(path, "device/uevent")) as fh:
                uevent = fh.read()
        except OSError:
            continue
        try:
            with open(os.path.join(path, "device/report_descriptor"), "rb") as fh:
                desc = fh.read()
        except OSError:
            desc = b""
        yield name, uevent, desc


def _is_vendor_page(desc):
    """True if the descriptor opens a vendor-defined usage page."""
    # 06 xx xx  = Usage Page (16-bit).  0xFF00 -> 06 00 ff
    return bytes([0x06, VENDOR_USAGE_PAGE & 0xFF, VENDOR_USAGE_PAGE >> 8]) in desc


def _report_ids(desc):
    ids, i = [], 0
    while i < len(desc) - 1:
        if desc[i] == 0x85:            # Report ID (1 byte)
            ids.append(desc[i + 1])
            i += 2
        else:
            i += 1
    return ids


def find_device(vid="1C4F", pid="007C"):
    """Return (path, report_ids, descriptor) for the vendor interface."""
    want = f"{vid.upper()}:{pid.upper()}"
    fallback = None
    for name, uevent, desc in _hidraw_nodes():
        ids = _report_ids(desc)
        hid_id = ""
        for line in uevent.split("\n"):
            if line.startswith("HID_ID="):
                hid_id = line.split("=", 1)[1].upper()
        matches_dev = want.replace(":", "") in hid_id.replace(":", "").replace("0000", "")
        if _is_vendor_page(desc) and REPORT_ID in ids:
            if matches_dev:
                return f"/dev/{name}", ids, desc
            fallback = fallback or (f"/dev/{name}", ids, desc)
    return fallback if fallback else (None, [], b"")


def cmd_info():
    path, ids, desc = find_device()
    print("scanning hidraw nodes for a vendor-defined (0xFF00) interface...\n")
    for name, uevent, d in _hidraw_nodes():
        hid_name = ""
        for line in uevent.split("\n"):
            if line.startswith("HID_NAME="):
                hid_name = line.split("=", 1)[1]
        mark = "  <-- vendor channel" if f"/dev/{name}" == path else ""
        print(f"  /dev/{name:10} {hid_name:34} "
              f"vendor_page={_is_vendor_page(d)!s:5} "
              f"report_ids={[hex(i) for i in _report_ids(d)]}{mark}")
    if not path:
        print("\nNo vendor channel found. Is this an AYANEO SLIDE, and is the "
              "keyboard MCU present (lsusb | grep 1c4f)?")
        return 1
    print(f"\nvendor channel : {path}")
    print(f"report id      : 0x{REPORT_ID:02x}, {REPORT_LEN} data bytes, write-only")
    print(f"descriptor     : {desc.hex(' ')}")
    return 0


def send_feature(path, payload):
    """payload = the data bytes, WITHOUT the report ID."""
    buf = bytearray([REPORT_ID]) + bytearray(payload)
    # HIDIOCSFEATURE(len) = _IOWR('H', 0x06, len)
    req = (3 << 30) | (len(buf) << 16) | (ord("H") << 8) | 0x06
    fd = os.open(path, os.O_RDWR)
    try:
        return fcntl.ioctl(fd, req, buf)
    finally:
        os.close(fd)


def cmd_raw(spec, assume_yes):
    path, _, _ = find_device()
    if not path:
        print("vendor channel not found", file=sys.stderr)
        return 1
    try:
        data = [int(x, 16) for x in spec.replace(",", ":").split(":") if x != ""]
    except ValueError:
        print("bytes must be hex, e.g. 01:02:03", file=sys.stderr)
        return 2
    if any(b < 0 or b > 255 for b in data):
        print("each byte must be 00..ff", file=sys.stderr)
        return 2
    if len(data) != REPORT_LEN:
        print(f"note: descriptor declares {REPORT_LEN} data bytes, you gave {len(data)}")
    print(f"device : {path}")
    print(f"sending: report 0x{REPORT_ID:02x} " + " ".join(f"{b:02x}" for b in data))
    if not assume_yes:
        print("\nThis writes to the keyboard/stick microcontroller. There is no "
              "read-back and no undo.")
        if input("type 'yes' to send: ").strip().lower() != "yes":
            print("aborted")
            return 1
    try:
        n = send_feature(path, data)
    except OSError as exc:
        print(f"failed: {exc.strerror}", file=sys.stderr)
        return 1
    print(f"sent ({n} bytes)")
    return 0


def cmd_deadzone(_value):
    print("Not implemented yet - the payload encoding is still unknown.\n")
    print("What is known:")
    print(f"  channel   : vendor HID, report ID 0x{REPORT_ID:02x}, {REPORT_LEN} data bytes")
    print("  handler   : master.set_stick_deadzone -> 0x140305a20 in AYASpaceCef.exe")
    print("  args      : enable, StickDeadZone, data")
    print("  enable bit: high nibble of a config byte, INVERTED (it is a disable flag)")
    print("\nWhat is missing: the actual 7 bytes. Capture them per "
          "docs/CAPTURE-DEADZONE.md, then this can be finished.")
    return 2


def main():
    ap = argparse.ArgumentParser(
        description="AYANEO vendor HID channel tool",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__)
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--info", action="store_true", help="find and describe the channel")
    g.add_argument("--raw", metavar="AA:BB:...", help="send one raw feature report")
    g.add_argument("--deadzone", type=int, metavar="N", help="set stick deadzone")
    ap.add_argument("-y", "--yes", action="store_true", help="skip the confirmation")
    args = ap.parse_args()

    if args.info:
        return cmd_info()
    if os.geteuid() != 0:
        print("writing needs root (hidraw is root-only); re-run with sudo", file=sys.stderr)
        return 1
    if args.raw:
        return cmd_raw(args.raw, args.yes)
    return cmd_deadzone(args.deadzone)


if __name__ == "__main__":
    sys.exit(main())
