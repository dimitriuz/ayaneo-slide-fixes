#!/usr/bin/env python3
"""Compare two EC RAM dumps taken by `ecdump`.

  ecdiff <labelA> <labelB>

Ignores offsets marked volatile in either dump, and annotates each changed byte
with decimal values so settings like "sensitivity 200" are obvious.
"""
import os, sys
OUT=os.path.expanduser("~/ec-dumps")

def load(label):
    with open(os.path.join(OUT,f"ec-{label}.bin"),"rb") as fh:
        data=fh.read()
    vpath=os.path.join(OUT,f"ec-{label}.volatile")
    vol=set()
    if os.path.exists(vpath):
        t=open(vpath).read().strip()
        if t: vol={int(x) for x in t.split(",")}
    return data, vol

def main():
    if len(sys.argv)<3:
        print(__doc__); return 2
    a,va = load(sys.argv[1])
    b,vb = load(sys.argv[2])
    vol = va | vb
    if len(a)!=len(b):
        print("dumps differ in size"); return 1
    changed=[i for i in range(len(a)) if a[i]!=b[i] and i not in vol]
    noisy=[i for i in range(len(a)) if a[i]!=b[i] and i in vol]
    print(f"{sys.argv[1]}  ->  {sys.argv[2]}")
    print(f"  {len(changed)} stable byte(s) changed, {len(noisy)} volatile ignored\n")
    if not changed:
        print("  no stable change - the setting is not in the 256-byte ACPI EC space.")
        print("  It may live in the vendor EC space reached via ITE SuperIO 0x4E/0x4F.")
        return 0
    for i in changed:
        print(f"  offset 0x{i:02x} ({i:3}) : "
              f"0x{a[i]:02x} ({a[i]:3})  ->  0x{b[i]:02x} ({b[i]:3})")
        if b[i] in (0,1) and a[i] in (0,1):
            print("        looks like a boolean flag")
    return 0

if __name__=="__main__":
    sys.exit(main())
