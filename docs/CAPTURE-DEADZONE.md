# Capturing the deadzone command (do this from Windows)

The AYANEO stick deadzone is a **stored setting** written by AYASpace over a
vendor HID channel. Static analysis identified the channel exactly but not the
command bytes (the call chain disappears into C++ virtual dispatch). A short USB
capture while changing the setting gives us those bytes, after which the same
change can be made natively from Linux.

## What we already know to look for

```
device : USB VID 1C4F  PID 007C   (the slide-out keyboard's MCU)
iface  : 01            (Usage Page 0xFF00, Usage 0x02 - AYASpace matches "&mi_02#")
report : FEATURE, Report ID 0x41, 7 data bytes, write-only (GET_REPORT stalls)
```

So we are looking for **SET_REPORT (Feature), Report ID 0x41** transfers to that
device. Everything else can be ignored.

---

## FIRST: check whether you even need the capture

Before capturing, just **set the deadzone in AYASpace and reboot into Linux**.

The whole reason this problem exists is that the setting *persists in the
controller*. So setting it once on real hardware may fix the device permanently,
and no Linux tool is needed at all.

Verify from Linux with:

```bash
sudo sticklive          # hold small deflections; compare 'held min' per axis
```

Baseline before any change (deflection at which motion starts):

| | horizontal | vertical |
|---|---|---|
| RIGHT | -30% ... +50% | -40% ... +30% |
| LEFT  | -15% ... +15% | -15% ... +25% |

If those numbers drop, you are done.

**Still do the capture anyway if you can** - it is what allows changing the value
later from Linux without booting Windows again.

---

## The capture

### 1. Install USBPcap

<https://desowin.org/usbpcap/> (Wireshark's installer can include it). Reboot if
its installer asks.

### 2. Find the right root hub

```
"C:\Program Files\USBPcap\USBPcapCMD.exe"
```

It lists root hubs and the devices under each. Pick the number of the hub whose
tree contains **VID_1C4F & PID_007C** (it will show as a keyboard). Note the
`\\.\USBPcapN` name.

### 3. Capture while changing the setting

```
"C:\Program Files\USBPcap\USBPcapCMD.exe" -d \\.\USBPcap1 -o C:\dz.pcap
```

Leave it running, then in AYASpace go to
`Assistant -> EVO -> Master Controller -> Joystick` and make these changes,
**pausing ~3 seconds between each** so they are easy to separate:

1. deadzone **OFF**
2. deadzone **ON**
3. deadzone to its **lowest** value
4. deadzone to its **highest** value
5. if there is a numeric field, set it to **10**, then **20**, then **50**

Then stop the capture with Ctrl+C.

### 4. WRITE DOWN WHAT YOU DID

This matters as much as the capture. Save a plain text file next to it, e.g.
`C:\dz-notes.txt`, listing the changes **in order**, with the exact values and
roughly how many seconds apart. Without the labels the bytes are far harder to
decode; with them it is usually obvious.

### 5. Leave both files where Linux can read them

Save `C:\dz.pcap` and `C:\dz-notes.txt` on the **Windows To Go drive itself**.
After rebooting into Linux, mount it and copy them off:

```bash
lsblk -o NAME,SIZE,LABEL,FSTYPE            # find the NTFS partition
sudo mount -t ntfs-3g -o ro,force /dev/sdXN /mnt/win
cp /mnt/win/dz.pcap /mnt/win/dz-notes.txt ~/
```

`force` matters - Windows fast-startup leaves NTFS dirty and a plain read-only
mount will fail with I/O errors.

---

## Also worth grabbing while you are in Windows

* **The AYASpace log** - `C:\Program Files (x86)\AYASpace\AYASpace.log` (or a
  `logs` folder). It may name the command it sends.
* **Whether the setting sticks** - reboot to Linux and re-measure. That answers
  the only question that really matters.

---

## What happens next

With the capture, the bytes for report `0x41` become readable directly:

```bash
tshark -r dz.pcap -Y 'usb.transfer_type == 2' -T fields \
       -e frame.time_relative -e usb.data_fragment
```

and the Linux implementation is then a short `HIDIOCSFEATURE` write to
`/dev/hidraw*` on the interface with Usage Page 0xFF00 - see
`scripts/ayaneo-deadzone.py` in this repo, which already has everything except
the payload.
