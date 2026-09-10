#!/usr/bin/env python3
"""Read AYANEO's vendor EC RAM (the 0xD1xx space behind ITE SuperIO 0x4E/0x4F).

READ-ONLY. Uses exactly the sequence ayaneo-platform uses for its RGB support.

  ayaec dump <label>     read all 256 bytes, save, print
  ayaec find <hexbyte>   read and report offsets holding that value
"""
import os, sys, time

ADDR, DATA, HIGH = 0x4E, 0x4F, 0xD1
PORT = "/dev/port"
OUT = os.path.expanduser("~/ec-dumps")


class Ports:
    def __init__(self):
        self.fd = os.open(PORT, os.O_RDWR)

    def outb(self, val, port):
        os.pwrite(self.fd, bytes([val]), port)

    def inb(self, port):
        return os.pread(self.fd, 1, port)[0]

    def close(self):
        os.close(self.fd)


def read_ram(p, index):
    p.outb(0x2E, ADDR); p.outb(0x11, DATA)
    p.outb(0x2F, ADDR); p.outb(HIGH, DATA)
    p.outb(0x2E, ADDR); p.outb(0x10, DATA)
    p.outb(0x2F, ADDR); p.outb(index, DATA)
    p.outb(0x2E, ADDR); p.outb(0x12, DATA)
    p.outb(0x2F, ADDR)                  # select the data register BEFORE reading
    return p.inb(DATA)


def dump():
    p = Ports()
    try:
        return bytes(read_ram(p, i) for i in range(256))
    finally:
        p.close()


def main():
    if os.geteuid() != 0:
        print("needs root"); return 1
    if len(sys.argv) < 2:
        print(__doc__); return 2
    cmd = sys.argv[1]

    if cmd == "dump":
        label = sys.argv[2] if len(sys.argv) > 2 else "vendor"
        # sample twice to spot volatile bytes
        a = dump(); time.sleep(2); b = dump()
        vol = {i for i in range(256) if a[i] != b[i]}
        os.makedirs(OUT, exist_ok=True)
        with open(os.path.join(OUT, f"vec-{label}.bin"), "wb") as fh:
            fh.write(b)
        with open(os.path.join(OUT, f"vec-{label}.volatile"), "w") as fh:
            fh.write(",".join(map(str, sorted(vol))))
        print(f"saved ~/ec-dumps/vec-{label}.bin")
        print(f"volatile: {[hex(i) for i in sorted(vol)] or 'none'}\n")
        for off in range(0, 256, 16):
            row = " ".join("--" if off+k in vol else f"{b[off+k]:02x}" for k in range(16))
            print(f"  d1{off:02x}: {row}")
        return 0

    if cmd == "diff":
        if len(sys.argv) < 4:
            print("usage: ayaec diff <labelA> <labelB>"); return 2
        def load(lbl):
            with open(os.path.join(OUT, f"vec-{lbl}.bin"), "rb") as fh:
                d = fh.read()
            vp = os.path.join(OUT, f"vec-{lbl}.volatile")
            v = set()
            if os.path.exists(vp):
                t = open(vp).read().strip()
                if t:
                    v = {int(x) for x in t.split(",")}
            return d, v
        a, va = load(sys.argv[2]); b, vb = load(sys.argv[3])
        vol = va | vb
        ch = [i for i in range(256) if a[i] != b[i] and i not in vol]
        print(f"{sys.argv[2]} -> {sys.argv[3]}")
        print(f"  {len(ch)} stable byte(s) changed "
              f"({len([i for i in range(256) if a[i]!=b[i] and i in vol])} volatile ignored)\n")
        for i in ch:
            print(f"  0xd1{i:02x} : 0x{a[i]:02x} ({a[i]:3})  ->  0x{b[i]:02x} ({b[i]:3})")
        if not ch:
            print("  no change in the 0xD1 vendor space either")
        return 0

    if cmd == "find":
        want = int(sys.argv[2], 16)
        d = dump()
        hits = [i for i in range(256) if d[i] == want]
        print(f"value 0x{want:02x} ({want}) at: " +
              (" ".join(f"0x{i:02x}" for i in hits) if hits else "nowhere"))
        return 0

    print(__doc__); return 2


if __name__ == "__main__":
    sys.exit(main())
