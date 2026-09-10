#!/usr/bin/env python3
"""gulikit-ctl - control the GuLiKit gamepad MCU in AYANEO handhelds from Linux.

Protocol reverse-engineered from AYASpace 3.2.0.4 (AYASpaceCef.exe),
class CGuLiKitUtils in utils_aya/Game/ComGamePad/GuLiKitUtils.cpp.

Transport
---------
An on-board legacy 16550 UART, *not* USB and *not* the EC.  On the AYANEO
SLIDE (AS01) it is I/O port 0x3E8 -- COM3 under Windows, /dev/ttyS2 under
Linux -- at 115200 8N1.

Frame (all models except AYANEO KUN)
------------------------------------
    byte  0      0xE7      header
    bytes 1..8   payload
    byte  9      checksum = sum(payload[1..8]) & 0xFF
    byte 10      0xED      terminator
The device replies with exactly 5 bytes: e7 55 aa <status> ed.
(AYANEO KUN uses a 15-byte frame carrying payload bytes 1..12 instead.)

State layout (the 15-byte record AYASpace caches as proto.gulikit)
------------------------------------------------------------------
    byte 0   0xE7 header
    byte 1   hi: TriggerL2 level     lo: TriggerR2 level
    byte 2   hi: GyroL1 level        lo: GyroL2 level
    byte 3   hi: left stick sens     lo: right stick sens   (1=50 2=100 3=150)
    byte 4   hi: deadzone DISABLE    lo: Motor (rumble) level
             (hi nibble 0 => deadzone active, 1 => deadzone off)
    byte 5   hi: BurstA             lo: BurstB      (turbo rates)
    byte 6   hi: BurstX             lo: BurstY
    byte 7   hi: BurstR1            lo: BurstR2
    byte 8   bit 0x10: SwapAbxy;  low nibble reserved (default 0x0F)
    bytes 9..12   AYANEO KUN only - not transmitted on other models
    byte 13  unused
    byte 14  0xED

There is no read-back command: the MCU only ever answers with a fixed ACK.
AYASpace keeps the authoritative copy in proto.gulikit and so do we.
"""

import argparse
import glob
import os
import select
import struct
import sys
import termios
import time

HDR, TERM = 0xE7, 0xED
STATE_FILE = "/var/lib/ayaneo/proto.gulikit"
FACTORY = bytes([0xE7, 0x00, 0x00, 0x22, 0x02, 0x00,
                 0x00, 0x00, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x00, 0xED])
SENS = {50: 1, 100: 2, 150: 3}
SENS_REV = {v: k for k, v in SENS.items()}

# From AYASpace's own UI option lists (web/frontend bundle 9906):
#   [{key:1,label:"Low"},{key:2,label:"Medium"},{key:3,label:"High"}]
#   [{key:0,label:"OFF"},{key:1,label:"Burst"},{key:2,label:"Auto"}]
LEVEL = {"off": 0, "low": 1, "medium": 2, "high": 3}
LEVEL_REV = {v: k for k, v in LEVEL.items()}
TURBO = {"off": 0, "burst": 1, "auto": 2}
TURBO_REV = {v: k for k, v in TURBO.items()}


def _nib(byte, high):
    return (byte >> 4) & 0xF if high else byte & 0xF


def _set_nib(byte, high, val):
    val &= 0xF
    return (byte & 0x0F) | (val << 4) if high else (byte & 0xF0) | val


# ---------------------------------------------------------------- transport

def open_port(dev):
    fd = os.open(dev, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    a = termios.tcgetattr(fd)
    a[0] = 0                                              # iflag: raw
    a[1] = 0                                              # oflag: raw
    a[2] = termios.CS8 | termios.CREAD | termios.CLOCAL   # 8N1, no flow ctl
    a[3] = 0                                              # lflag: raw
    a[4] = a[5] = termios.B115200
    a[6] = list(a[6])
    a[6][termios.VMIN] = 0
    a[6][termios.VTIME] = 0
    termios.tcsetattr(fd, termios.TCSANOW, a)
    termios.tcflush(fd, termios.TCIOFLUSH)
    return fd


def build_frame(state):
    payload = state[0:9]
    return bytes(payload) + bytes([sum(payload[1:9]) & 0xFF, TERM])


def _transact_once(dev, state, timeout):
    """One open/send/read/close cycle, mirroring what AYASpace does."""
    try:
        fd = open_port(dev)
    except (OSError, termios.error):
        # termios.error is not an OSError subclass, so it must be named.
        # All 32 8250 slots exist as /dev/ttyS* nodes; unpopulated ones
        # raise EIO here rather than simply staying silent.
        return build_frame(state), b""
    try:
        frame = build_frame(state)
        termios.tcflush(fd, termios.TCIOFLUSH)
        os.write(fd, frame)
        termios.tcdrain(fd)
        reply, deadline = b"", time.time() + timeout
        while len(reply) < 5:
            remaining = deadline - time.time()
            if remaining <= 0:
                break
            if select.select([fd], [], [], remaining)[0]:
                try:
                    chunk = os.read(fd, 5 - len(reply))
                except BlockingIOError:
                    continue
                if chunk:
                    reply += chunk
        return frame, reply
    finally:
        os.close(fd)


def transact(dev, state, timeout=0.3, attempts=5):
    """Send the state record, return (frame_sent, reply_bytes).

    The MCU answers in ~7 ms but drops roughly one reply in seven, which is
    why AYASpace probes each port up to five times.  The frame carries the
    complete absolute state rather than a delta, so re-sending it is
    idempotent and a lost reply costs nothing but another round trip.
    """
    frame = build_frame(state)
    for _ in range(attempts):
        frame, reply = _transact_once(dev, state, timeout)
        if len(reply) == 5 and reply[0] == HDR and reply[4] == TERM:
            return frame, reply
    return frame, reply


def plausible(reply):
    return len(reply) == 5 and reply[0] == HDR and reply[4] == TERM


def uart_io_base(name):
    """I/O port of a ttyS device, or None. 0x3E8 is COM3 = the gamepad MCU."""
    try:
        with open(f"/sys/class/tty/{name}/io_type") as f:
            if f.read().strip() != "0":     # UPIO_PORT
                return None
        with open(f"/sys/class/tty/{name}/port") as f:
            base = int(f.read().strip(), 16)
        return base or None          # 0x0 means the slot is unpopulated
    except OSError:
        return None


def candidate_ports():
    """ttyS devices, COM3 (0x3E8) first - AYASpace's hint for AS01/SLIDE."""
    ports = []
    for path in sorted(glob.glob("/dev/ttyS*")):
        base = uart_io_base(os.path.basename(path))
        if base is not None:
            ports.append((0 if base == 0x3E8 else 1, path, base))
    ports.sort()
    return [(p, b) for _, p, b in ports]


def find_port(state, explicit=None, verbose=False):
    """Locate the MCU. Probing re-sends the cached state, so it is a no-op."""
    if explicit:
        return explicit
    if os.environ.get("GULIKIT_PORT"):
        return os.environ["GULIKIT_PORT"]
    for path, base in candidate_ports():
        _, reply = transact(path, state)
        if verbose:
            print(f"  probe {path} (io 0x{base:x}): "
                  f"{reply.hex(' ') if reply else '<no reply>'}", file=sys.stderr)
        if plausible(reply):
            return path
    die("no GuLiKit gamepad MCU found on any ttyS port "
        "(try --port, or check that this is an AYANEO handheld)")


# -------------------------------------------------------------------- state

def die(msg):
    print(f"gulikit-ctl: {msg}", file=sys.stderr)
    sys.exit(1)


def load_state(path):
    try:
        with open(path, "rb") as f:
            data = f.read()
    except FileNotFoundError:
        die(f"no state file at {path}\n"
            f"  The MCU cannot be queried, so the current settings must be "
            f"seeded once:\n"
            f"    gulikit-ctl init --factory              # AYANEO defaults\n"
            f"    gulikit-ctl init --from FILE            # a Windows proto.gulikit\n"
            f"    gulikit-ctl init --raw <30 hex chars>   # a known record")
    if len(data) != 15 or data[0] != HDR or data[14] != TERM:
        die(f"{path} is not a valid 15-byte proto.gulikit record")
    return bytearray(data)


def save_state(path, state):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "wb") as f:
        f.write(bytes(state))
    os.replace(tmp, path)


# ------------------------------------------------------------------ display

def _lbl(table, v):
    name = table.get(v)
    return f"{v} ({name})" if name else str(v)


def describe(state):
    b = state
    dz_off = (b[4] & 0xF0) != 0
    left, right = (b[3] >> 4) & 0xF, b[3] & 0xF
    out = [
        f"raw record      {bytes(b).hex(' ')}",
        f"stick deadzone  {'OFF (disabled)' if dz_off else 'ON (active)'}",
        f"left stick sens {left}  ({SENS_REV.get(left, '?')})",
        f"right stick sens {right}  ({SENS_REV.get(right, '?')})",
        f"rumble level    {_lbl(LEVEL_REV, b[4] & 0xF)}",
        f"trigger L2/R2   {_lbl(LEVEL_REV, (b[1] >> 4) & 0xF)} / {_lbl(LEVEL_REV, b[1] & 0xF)}",
        f"gyro L1/L2      {_lbl(LEVEL_REV, (b[2] >> 4) & 0xF)} / {_lbl(LEVEL_REV, b[2] & 0xF)}",
        f"turbo A/B       {_lbl(TURBO_REV, (b[5] >> 4) & 0xF)} / {_lbl(TURBO_REV, b[5] & 0xF)}",
        f"turbo X/Y       {_lbl(TURBO_REV, (b[6] >> 4) & 0xF)} / {_lbl(TURBO_REV, b[6] & 0xF)}",
        f"turbo R1/R2     {_lbl(TURBO_REV, (b[7] >> 4) & 0xF)} / {_lbl(TURBO_REV, b[7] & 0xF)}",
        f"swap ABXY       {'yes' if b[8] & 0x10 else 'no'}",
    ]
    return "\n".join(out)


# ----------------------------------------------------------------- commands

def cmd_init(args):
    if args.factory:
        state = bytearray(FACTORY)
    elif args.raw:
        raw = bytes.fromhex(args.raw.replace(" ", ""))
        if len(raw) != 15:
            die("--raw needs exactly 15 bytes (30 hex chars)")
        state = bytearray(raw)
    else:
        with open(args.source, "rb") as f:
            state = bytearray(f.read())
        if len(state) != 15:
            die(f"{args.source} is not a 15-byte proto.gulikit record")
    save_state(args.state_file, state)
    print(f"seeded {args.state_file}")
    print(describe(state))


def cmd_show(args):
    state = load_state(args.state_file)
    print(describe(state))
    print(f"\n(cached record; the MCU has no read-back command)")


def cmd_set(args):
    state = load_state(args.state_file)
    before = bytes(state)

    if args.deadzone is not None:
        # high nibble of byte 4 is a *disable* flag
        state[4] = (state[4] & 0x0F) | (0x00 if args.deadzone == "on" else 0x10)
    if args.left is not None:
        state[3] = (state[3] & 0x0F) | (SENS[args.left] << 4)
    if args.right is not None:
        state[3] = (state[3] & 0xF0) | SENS[args.right]
    if args.rumble is not None:
        state[4] = (state[4] & 0xF0) | LEVEL[args.rumble]
    if args.swap_abxy is not None:
        state[8] = (state[8] & 0xEF) | (0x10 if args.swap_abxy == "on" else 0x00)
    # byte 1: trigger levels, byte 2: gyro levels; index 1 is the high nibble
    for opt, idx, high in (("trigger_l2", 1, True), ("trigger_r2", 1, False),
                           ("gyro_l1", 2, True), ("gyro_l2", 2, False)):
        val = getattr(args, opt, None)
        if val is not None:
            state[idx] = _set_nib(state[idx], high, LEVEL[val])
    # bytes 5-7: per-button turbo
    for opt, idx, high in (("turbo_a", 5, True), ("turbo_b", 5, False),
                           ("turbo_x", 6, True), ("turbo_y", 6, False),
                           ("turbo_r1", 7, True), ("turbo_r2", 7, False)):
        val = getattr(args, opt, None)
        if val is not None:
            state[idx] = _set_nib(state[idx], high, TURBO[val])

    if bytes(state) == before and not args.force:
        print("no change requested")
        return

    port = find_port(state, args.port, args.verbose)
    frame, reply = transact(port, state)
    if not plausible(reply):
        die(f"{port}: no valid reply to {frame.hex(' ')} "
            f"(got {reply.hex(' ') if reply else 'nothing'}) - nothing saved")
    save_state(args.state_file, state)
    print(f"{port}: sent {frame.hex(' ')}  ack {reply.hex(' ')}")
    print(describe(state))


def cmd_apply(args):
    """Re-send the cached record, e.g. from a boot unit."""
    state = load_state(args.state_file)
    port = find_port(state, args.port, args.verbose)
    frame, reply = transact(port, state)
    if not plausible(reply):
        die(f"{port}: no valid reply (got {reply.hex(' ') if reply else 'nothing'})")
    print(f"{port}: sent {frame.hex(' ')}  ack {reply.hex(' ')}")


def cmd_factory_reset(args):
    """What master.restore_factory does: send the factory record, then cache it."""
    state = bytearray(FACTORY)
    port = find_port(state, args.port, args.verbose)
    frame, reply = transact(port, state)
    if not plausible(reply):
        die(f"{port}: no valid reply (got {reply.hex(' ') if reply else 'nothing'})")
    save_state(args.state_file, state)
    print(f"{port}: sent {frame.hex(' ')}  ack {reply.hex(' ')}")
    print(describe(state))


def cmd_probe(args):
    state = load_state(args.state_file) if os.path.exists(args.state_file) \
        else bytearray(FACTORY)
    print("probing ttyS ports (re-sends the cached record, so it is a no-op):")
    found = None
    for path, base in candidate_ports():
        _, reply = transact(path, state)
        ok = plausible(reply)
        print(f"  {path:14s} io 0x{base:03x}  "
              f"{reply.hex(' ') if reply else '<no reply>':20s} {'<== MCU' if ok else ''}")
        if ok and found is None:
            found = path
    if not found:
        die("no MCU found")
    print(f"\ngamepad MCU: {found}")


def main():
    ap = argparse.ArgumentParser(
        prog="gulikit-ctl",
        description="Control the GuLiKit gamepad MCU in AYANEO handhelds.")
    ap.add_argument("--state-file", default=STATE_FILE,
                    help=f"cached 15-byte record (default {STATE_FILE})")
    ap.add_argument("--port", help="serial device (default: autodetect, COM3/0x3E8 first)")
    ap.add_argument("-v", "--verbose", action="store_true")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("init", help="seed the cached record")
    g = p.add_mutually_exclusive_group(required=True)
    g.add_argument("--factory", action="store_true", help="AYANEO defaults")
    g.add_argument("--from", dest="source", metavar="FILE",
                   help="a Windows AppData/Roaming/AYASpace/proto.gulikit")
    g.add_argument("--raw", metavar="HEX", help="15 bytes as hex")
    p.set_defaults(func=cmd_init)

    p = sub.add_parser("show", help="print the cached settings")
    p.set_defaults(func=cmd_show)

    p = sub.add_parser("probe", help="find the MCU's serial port")
    p.set_defaults(func=cmd_probe)

    p = sub.add_parser("apply", help="re-send the cached record to the MCU")
    p.set_defaults(func=cmd_apply)

    p = sub.add_parser("factory-reset",
                       help="send the AYANEO factory record (master.restore_factory)")
    p.set_defaults(func=cmd_factory_reset)

    p = sub.add_parser("set", help="change settings and write them to the MCU")
    p.add_argument("--deadzone", choices=["on", "off"],
                   help="stick deadzone (off = full analog resolution near centre)")
    p.add_argument("--left", type=int, choices=[50, 100, 150],
                   help="left stick sensitivity")
    p.add_argument("--right", type=int, choices=[50, 100, 150],
                   help="right stick sensitivity")
    p.add_argument("--rumble", choices=sorted(LEVEL),
                   help="rumble motor level (off/low/medium/high)")
    for side in ("l2", "r2"):
        p.add_argument(f"--trigger-{side}", choices=sorted(LEVEL),
                       help=f"{side.upper()} trigger sensitivity")
    p.add_argument("--gyro-l1", choices=sorted(LEVEL), help="gyro L1 level")
    p.add_argument("--gyro-l2", choices=sorted(LEVEL), help="gyro L2 level")
    for btn in ("a", "b", "x", "y", "r1", "r2"):
        p.add_argument(f"--turbo-{btn}", choices=sorted(TURBO),
                       help=f"turbo mode for {btn.upper()}")
    p.add_argument("--swap-abxy", choices=["on", "off"])
    p.add_argument("--force", action="store_true",
                   help="send even if nothing changed")
    p.set_defaults(func=cmd_set)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
