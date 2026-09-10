# The AYANEO gamepad settings protocol

**Solved.** The gamepad settings that AYASpace writes on Windows — stick deadzone,
per-stick sensitivity, rumble, turbo, trigger and gyro levels — travel over an
**on-board legacy 16550 UART**, not USB and not the EC. On the AYANEO SLIDE that
UART is I/O port `0x3E8` (COM3 on Windows, **`/dev/ttyS2`** on Linux) at
**115200 8N1**.

Everything here is reproducible from Linux with
[`scripts/gulikit-ctl.py`](../scripts/gulikit-ctl.py). No Windows, no VM, no
firmware flash.

```
$ sudo gulikit-ctl probe
probing ttyS ports (re-sends the cached record, so it is a no-op):
  /dev/ttyS2     io 0x3e8  e7 55 aa 02 ed       <== MCU
  /dev/ttyS0     io 0x3f8  <no reply>
  /dev/ttyS1     io 0x2f8  <no reply>
  /dev/ttyS3     io 0x2e8  <no reply>

gamepad MCU: /dev/ttyS2

$ sudo gulikit-ctl set --deadzone off
/dev/ttyS2: sent e7 00 00 22 12 00 00 00 0f 43 ed  ack e7 55 aa 02 ed
```

---

## Why every earlier search missed it

Four exhaustive negative results, all of them correct, all looking in the wrong
place — see [EC-INVESTIGATION.md](EC-INVESTIGATION.md) and
[INPUT-CONTROLLER.md](INPUT-CONTROLLER.md):

| Searched | Result | Why it was negative |
|---|---|---|
| USB captures over eight setting changes | **zero** SET_REPORT, zero OUT packets | the setting never crosses USB |
| ACPI EC, 256 bytes | only temperatures moved | not the EC |
| Vendor EC page `0xD1xx` | byte-identical | not the EC |
| Full 64 KB EC, validated 10-sample instrument | **0 changes** | not the EC |

A UART is invisible to all four. It is not a USB endpoint, so `usbmon`/USBPcap
see nothing; it is not the EC's RAM, so EC dumps see nothing. The only reason to
look at a serial port at all is that the binary says so.

## How it was found

Ghidra headless (via [`ghidra-cli`](https://github.com/akiselev/ghidra-cli)) on
`AYASpaceCef.exe` 3.2.0.4, `ImageBase 0x140000000`, 53 370 functions.

The CEF handler for the JS call `master.set_stick_deadzone` is at
**`0x140305a20`**. Decompiled, it reads the `enable` argument and calls
`0x140339ed0(singleton, enable)`, which flips a bit and calls
`0x1403397a0`. That function still carries its **build paths and symbol names**:

```
D:\devel\windws\AYASpace\AyaHome\utils_aya\Game\ComGamePad\GuLiKitUtils.cpp
    CGuLiKitUtils::CommSend        line 0x101, 0x109
    CGuLiKitUtils::CommSendData    line 0xcd, 0xdc, 0xe7
```

with the error strings `"open serial port failed"`, `"serial port send failed"`,
`"serial port receive failed"` and `"not support in this version"`.

That is the whole answer: **GuLiKit** is the gamepad module vendor, and the
transport is a serial port. Static analysis had previously stalled here because
the chain runs through C++ virtual dispatch that `objdump` cannot follow — a BFS
over 198 functions to depth 6 reached neither the EC helpers, nor libusb, nor
ViGEm, nor the HID feature builder. It reached none of them because the answer
was none of them.

### Port discovery

`0x140338c80` enumerates `HKLM\Hardware\DeviceMap\SerialComm`, keeping only
values whose *name* begins with `\Device\Serial` and whose *data* matches
`COM%d`. That excludes USB CDC-ACM, which registers as `\Device\USBSER000` —
so AYASpace is deliberately looking for a **native UART**. Each candidate is
opened with

```c
FUN_14033b330(this + 0x20, port_name, 0x1c200, 0);   /* 0x1c200 == 115200 */
```

then probed by sending the current settings record up to **five times** and
waiting for a reply. The first port that answers is cached.

A per-model hint decides which port to try first:

| predicate | model | preferred port |
|---|---|---|
| `0x14040a5e0` | `AYANEO KUN` | COM4 |
| `0x14040a020` | `AYANEO 2`, `AYANEO 2S` | COM3 |
| `0x14040a210` | `AYANEO GEEK`, `AYANEO GEEK 1S` | COM3 |
| `0x14040a7c0` | `AYANEO AIR 1S` | COM3 |
| `0x14040a8b0` | `AYANEO AIR 1S Limited` | COM3 |
| `0x14040bcd0` | **`AS01`** — the SLIDE | **COM3** |
| `0x14040b9d0` | `AB05` | — |
| *(else)* | | COM2 |

`COM3` is I/O `0x3E8`. Linux enumerates the SLIDE's two real UARTs as

```
00:01: ttyS0 at I/O 0x3f8 (irq = 3, base_baud = 115200) is a 16550A
00:02: ttyS2 at I/O 0x3e8 (irq = 5, base_baud = 115200) is a 16450
```

so **COM3 == `/dev/ttyS2`**. Both are ACPI `PNP0500` devices — real Super I/O
UARTs, present at boot, no driver needed.

---

## Wire format

`CGuLiKitUtils::CommSendData` (`0x140339280`) frames the record two ways.
Everything except the KUN uses the **11-byte** frame:

```
  offset  0     1 2 3 4 5 6 7 8     9        10
        +-----+-----------------+--------+------+
        | E7  |     payload     |  csum  |  ED  |
        +-----+-----------------+--------+------+
          hdr      8 bytes        sum of   term
                                bytes 1..8
                                  & 0xFF
```

The AYANEO KUN instead sends a **15-byte** frame carrying payload bytes `1..12`,
with `csum = sum(bytes 1..12) & 0xFF`. Payload bytes `9..12` therefore exist in
the record but are **never transmitted on the SLIDE**.

The device replies with **exactly 5 bytes**: `e7 55 aa <status> ed`.

**One frame per port open.** `0x140339200` closes the port immediately after
each exchange, so AYASpace opens, sends, reads, and closes for every single
setting change. `gulikit-ctl` does the same.

**Roughly one reply in seven is dropped.** Measured over 32 transactions:
85 % answer rate, 7 ms latency when they do, independent of the gap between
transactions. This is why AYASpace retries five times. Retrying is safe: the
frame carries the **complete absolute state**, never a delta, so re-sending it
is idempotent.

## Record layout

The 15-byte record AYASpace caches as `proto.gulikit`:

| byte | high nibble | low nibble | default |
|---|---|---|---|
| 0 | `0xE7` header | | `E7` |
| 1 | TriggerL2 level | TriggerR2 level | `00` |
| 2 | GyroL1 level | GyroL2 level | `00` |
| 3 | **left stick sensitivity** | **right stick sensitivity** | `22` |
| 4 | **deadzone DISABLE flag** | rumble motor level | `02` |
| 5 | turbo A | turbo B | `00` |
| 6 | turbo X | turbo Y | `00` |
| 7 | turbo R1 | turbo R2 | `00` |
| 8 | bit `0x10` = swap ABXY | *reserved* | `0F` |
| 9–12 | *AYANEO KUN only — not sent on the SLIDE* | | `00` |
| 13 | unused | | `00` |
| 14 | `0xED` terminator | | `ED` |

Factory record: `e7 00 00 22 02 00 00 00 0f 00 00 00 00 00 ed` (`0x140339960`).

### Stick sensitivity — byte 3

`0x140339d80(this, index, level)` writes the high nibble for `index == 1` and the
low nibble otherwise; `0x140303c10(record, index)` reads them back into the JS
fields `HallLeft` (index 1) and `HallRight` (index 2). So

* **high nibble = left stick**, **low nibble = right stick**
* level `1` = 50, level `2` = 100 (default), level `3` = 150

"Hall" here is the Hall-effect stick module; the AYASpace UI labels these values
50/100/150 and offers no others.

### Deadzone — byte 4, high nibble

The flag is **inverted** — it disables:

```c
/* 0x140303c50 */
bool deadzone_enabled(record) { return (record[4] & 0xf0) == 0; }
```

`0x140339ed0(this, enable)` writes `(enable ? 0 : 1) << 4`, preserving the low
nibble. So high nibble `0` = deadzone **active**, `1` = deadzone **off**.

### Verified against the real file

The record from a live Windows install after setting left = 50, right = 150,
deadzone = on:

```
$ xxd /mnt/win/Users/admin/AppData/Roaming/AYASpace/proto.gulikit
00000000: e700 0013 0200 0000 0f00 0000 0000 ed
                ^^ ^^
```

Byte 3 = `0x13` → left `1`, right `3`. Byte 4 = `0x02` → high nibble `0`,
deadzone active. Every field decodes to exactly what was set in the UI, which
pins the `1/2/3 → 50/100/150` mapping empirically rather than by inference.

---

## There is no read-back

The MCU's only reply is the fixed 5-byte ACK — no command returns the current
settings. `CGuLiKitUtils` handles this by treating a **local file** as
authoritative: `0x140338850` loads `%APPDATA%\AYASpace\proto.gulikit` on every
access, falling back to the factory record when it is absent, and `0x140338ae0`
writes it back after every successful send.

`gulikit-ctl` mirrors that design with `/var/lib/ayaneo/proto.gulikit`. The
consequence is worth stating plainly: **if that file is lost, the current
settings cannot be recovered from the hardware** — you can only re-assert a
known record. `gulikit-ctl init` seeds it, optionally straight from a Windows
`proto.gulikit`.

The settings themselves live in the MCU's own non-volatile memory. They survive
a reboot, and they survive crossing between Windows and Linux — which is why
disabling the deadzone under Windows had already changed the behaviour under
Linux before any of this existed.

---

## Objective verification

The deadzone is measurable without touching the sticks. With it active the MCU
pins every axis to centre; with it off the axes report their true resting
offsets. Toggling it twice from Linux, reading `/dev/input/...-event-joystick`
at rest:

| | ABS_X | ABS_Y | ABS_RX | ABS_RY |
|---|---|---|---|---|
| `--deadzone on` | 0 | −1 | 0 | −1 |
| `--deadzone off` | −512 | **−3840** | 0 | +256 |
| `--deadzone on` (again) | 0 | −1 | 0 | −1 |
| `--deadzone off` (again) | −512 | **−3840** | 0 | +256 |

Bit-identical across rounds. The left stick's Y axis rests ~3840/32768 ≈ 12 %
off centre, which is the stored calibration offset found earlier — the deadzone
was masking it.

Sensitivity is visible the same way, since it scales the analog output: at
left = 50 / right = 150 the resting offsets read `−256, −2048, −512, +256`, and
at 100/100 they read `−512, −3840, 0, +256`.

## Usage

```bash
sudo install -m755 scripts/gulikit-ctl.py /usr/local/bin/gulikit-ctl

# seed the local record once (the MCU cannot be queried)
sudo gulikit-ctl init --factory
#   ... or carry the settings over from a Windows install:
sudo gulikit-ctl init --from /mnt/win/Users/<you>/AppData/Roaming/AYASpace/proto.gulikit

sudo gulikit-ctl probe                       # find the MCU's UART
sudo gulikit-ctl show                        # print the cached record
sudo gulikit-ctl set --deadzone off          # full resolution near centre
sudo gulikit-ctl set --right 50              # slower, finer right stick
sudo gulikit-ctl set --left 100 --right 100  # both sticks to default
sudo gulikit-ctl apply                       # re-send the cached record
```

`--deadzone off` plus `--right 50` is the combination that makes the right stick
usable as a mouse: the deadzone stops eating small deflections, and the lower
sensitivity keeps those deflections slow. Pair it with the InputPlumber profile
in [`config/inputplumber-stick-mouse.yaml`](../config/inputplumber-stick-mouse.yaml)
and see [INPUTPLUMBER-GUIDE.md](INPUTPLUMBER-GUIDE.md).

### Permissions

`/dev/ttyS2` is `root:uucp`, mode `0660`. Either run under `sudo`, or join the
group and drop the `sudo`:

```bash
sudo usermod -aG uucp "$USER"      # re-login to take effect
```

### Other models

The protocol is shared across the AYANEO line; only the preferred port differs,
and `gulikit-ctl probe` finds it regardless. On an **AYANEO KUN** the frame is
15 bytes rather than 11 — `gulikit-ctl` does **not** implement that variant, so
do not use it there without adding it.

## Function reference

`AYASpaceCef.exe` 3.2.0.4, SHA-256 of the analysed file recorded alongside the
Ghidra project. `ImageBase 0x140000000`.

| address | role |
|---|---|
| `0x140305a20` | JS `master.set_stick_deadzone` handler |
| `0x1403056c0` | JS `master.set_hall_stick` handler (`HallLeft`/`HallRight`) |
| `0x140305000` | JS trigger handler (`TriggerL2`/`TriggerR2`) |
| `0x140305360` | JS gyro handler (`GyroL1`/`GyroL2`) |
| `0x140305c80` | JS rumble handler (`Motor`) |
| `0x140305ee0` | JS turbo handler (`BurstA`…`BurstR2`) |
| `0x140306460` | JS `SwapAbxy` handler |
| `0x140339ed0` | set deadzone bit — byte 4 high nibble |
| `0x140339d80` | set stick sensitivity — byte 3 |
| `0x140339ae0` | set trigger levels — byte 1 |
| `0x140339c30` | set gyro levels — byte 2 |
| `0x140339fe0` | set rumble level — byte 4 low nibble |
| `0x14033a170` | set turbo levels — bytes 5–7 |
| `0x14033a520` | set swap ABXY — byte 8 |
| `0x14033a610/6c0/770/820` | set bytes 9/10/12/11 — KUN only |
| `0x1403397a0` | `CGuLiKitUtils::CommSend` — open, send, close |
| `0x140339280` | `CGuLiKitUtils::CommSendData` — framing and checksum |
| `0x140338c80` | enumerate and probe serial ports |
| `0x14033aff0` | read `HKLM\Hardware\DeviceMap\SerialComm` |
| `0x140338850` | load `proto.gulikit`, else factory record |
| `0x140338ae0` | save `proto.gulikit` |
| `0x140339960` | factory record |
| `0x140339200` | close port after each exchange |
| `0x140303c50` | read deadzone flag |
| `0x140303c10` | read stick sensitivity |
| `0x140303cb0` | read turbo levels |
| `0x14040ce80` | "this model has a GuLiKit gamepad" gate |

### Reproducing the analysis

[`ghidra/`](../ghidra) holds the container used here — Ghidra 12.1.3 on
`eclipse-temurin:21-jdk` plus `ghidra-cli`:

```bash
cd ghidra && docker build -t ghidra-cli .
docker run -d --name ghidra -v "$PWD/work":/work -v "$PWD/project":/project \
    --memory 12g ghidra-cli sleep infinity
docker exec ghidra ghidra import /work/AYASpaceCef.exe --project /project/aya
docker exec ghidra ghidra decompile 0x140305a20 \
    --project /project/aya --program AYASpaceCef.exe
```

Import plus full analysis takes about five minutes and produces a 47 MB program
database. `AYASpaceCef.exe` is not redistributed here; extract it from an
AYASpace installer.
