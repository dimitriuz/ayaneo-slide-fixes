# Capture plan: everything needed for a Linux AYANEO utility

Goal: capture AYASpace's USB traffic once, thoroughly enough to implement the
missing features natively on Linux.

## Scope first — do not capture what Linux already does

Checked on an AYANEO SLIDE running CachyOS:

| Feature | Linux today | Capture? |
|---|---|---|
| Stick ring RGB | **works** — `/sys/class/leds/ayaneo:rgb:joystick_rings` (`brightness`, `multi_intensity`) via `ayaneo-platform` | no |
| TDP / power limits | **works** — Handheld Daemon, plus `/sys/firmware/acpi/platform_profile` (`low-power`/`balanced`/`performance`) | no |
| Fan control | via HHD / EC | no |
| **Keyboard backlight** | **missing** — no LED node exists | **yes** |
| **Stick deadzone on/off** | **missing** | **yes** |
| **Per-stick sensitivity** | **missing** | **yes** |
| **Hall stick / calibration** (`master.set_hall_stick`) | **missing** | **yes** |
| Button remapping / macros | missing | yes, if you use it |

EC-based features (RGB, TDP, fan) travel over I/O ports `0x4E/0x4F`, **not USB**,
so USBPcap cannot see them anyway — and they are already supported on Linux.
Everything in the "capture" column goes over USB and is therefore capturable.

## What we already know

```
vendor channel : USB VID 1C4F PID 007C   (the slide-out keyboard's MCU)
                 interface with Usage Page 0xFF00 / Usage 0x02  -> /dev/hidraw1
                 FEATURE report, ID 0x41, 7 data bytes, write-only
also seen       : a 9-byte feature report, report[0]=0x02, report[1]=0..100
                  (110 special) - a brightness-shaped command whose target
                  device is not yet confirmed
```

So expect most traffic to be **SET_REPORT (Feature)** to `VID_1C4F`. Capture the
whole hub anyway - other devices may be involved and it costs nothing.

## Setup

1. Install **USBPcap** — <https://desowin.org/usbpcap/>
2. Run `"C:\Program Files\USBPcap\USBPcapCMD.exe"` and note the `\\.\USBPcapN`
   whose tree contains **VID_1C4F & PID_007C**.
3. Start capturing to a file on the **Windows partition** (so Linux can read it
   later):

```
"C:\Program Files\USBPcap\USBPcapCMD.exe" -d \\.\USBPcap1 -o C:\aya\capture.pcap
```

## Protocol — label everything

The labels matter as much as the bytes. Keep a text file `C:\aya\notes.txt` and
write each action **as you do it**, in order. Leave **~3 seconds between
actions** so they separate cleanly in the timeline.

Work through AYASpace one setting at a time:

### A. Stick deadzone (highest value - it is boolean, so 2 samples)
```
deadzone OFF
deadzone ON
deadzone OFF        (confirm it repeats identically)
```

### B. Stick sensitivity (both sticks)
```
LEFT  sensitivity -> minimum
LEFT  sensitivity -> maximum
LEFT  sensitivity -> middle
RIGHT sensitivity -> minimum
RIGHT sensitivity -> maximum
RIGHT sensitivity -> middle
```
Note the **exact numbers** the UI shows. Min/max/middle is what reveals the
encoding and the range.

### C. Keyboard backlight
```
brightness -> 0
brightness -> 50
brightness -> 100
backlight OFF
backlight ON
```
plus any colour or effect controls, naming each colour/effect exactly.

### D. Hall stick / calibration (if present)
```
open the hall stick / joystick correction page
run one calibration
```

### E. Anything else you actually use
Remaps, macros, trigger settings, gyro. Same rule: one change, label it, pause.

Stop with Ctrl+C.

## Getting the files back

They are on the Windows partition already, so from Linux:

```bash
lsblk -o NAME,SIZE,LABEL,FSTYPE                     # find the ~146G NTFS
sudo mount -t ntfs-3g -o ro,force /dev/nvme0n1p4 /mnt/win
cp /mnt/win/aya/capture.pcap /mnt/win/aya/notes.txt ~/
sudo umount /mnt/win
```

`force` is required — Windows fast-startup leaves NTFS dirty and a plain
read-only mount fails with I/O errors.

## Also grab, while you are there

* `C:\Program Files (x86)\AYASpace\AYASpace.log` (and any `logs\` folder) — it
  may name commands directly and save decoding work.
* Note the **AYASpace version** (Help/About).

## Decoding afterwards

```bash
tshark -r capture.pcap -Y 'usb.transfer_type == 2' \
       -T fields -e frame.time_relative -e usb.device_address -e usb.data_fragment
```

Line the timestamps up against `notes.txt`, and with min/max/middle samples the
field encoding usually falls out immediately. Then each command becomes a short
`HIDIOCSFEATURE` write in `scripts/ayaneo-ctl.py`, which already finds the
channel and sends reports — only the payloads are missing.
