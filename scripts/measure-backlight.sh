#!/usr/bin/env bash
# Sample the backlight transfer curve and report resolution / dead zones.
# Read-only apart from writing `brightness`; restores the original value on exit.
#
# Usage: ./measure-backlight.sh [backlight-device-name]
set -euo pipefail

BL_DIR=/sys/class/backlight
DEV="${1:-}"
if [[ -z "$DEV" ]]; then
    DEV=$(ls "$BL_DIR" 2>/dev/null | head -1) || true
    [[ -n "$DEV" ]] || { echo "No backlight device found in $BL_DIR" >&2; exit 1; }
fi
BL="$BL_DIR/$DEV"
[[ -d "$BL" ]] || { echo "No such backlight device: $BL" >&2; exit 1; }

MAX=$(< "$BL/max_brightness")
ORIG=$(< "$BL/brightness")
SETTLE=0.6          # panel ramps; reading sooner returns mid-ramp values

if [[ -w "$BL/brightness" ]]; then
    write() { echo "$1" > "$BL/brightness"; }
else
    echo "note: $BL/brightness not writable, using sudo"
    write() { echo "$1" | sudo tee "$BL/brightness" >/dev/null; }
fi
restore() { write "$ORIG" 2>/dev/null || true; }
trap restore EXIT INT TERM

echo "device : $DEV"
echo "type   : $(< "$BL/type")   scale: $(cat "$BL/scale" 2>/dev/null || echo n/a)"
echo "max    : $MAX"
echo "orig   : $ORIG"
command -v journalctl >/dev/null && {
    echo "caps   : $(sudo journalctl -k -b 2>/dev/null \
        | grep -i 'backlight caps' | tail -1 | sed 's/^.*Backlight caps: //' || echo 'n/a (boot with drm.debug=0x6)')"
}
echo

printf '%-10s %-7s %-10s\n' set pct actual
for pct in 0 1 2 5 10 20 30 40 50 60 70 80 90 100; do
    v=$(( MAX * pct / 100 ))
    write "$v"; sleep "$SETTLE"
    printf '%-10s %-7s %-10s\n' "$v" "${pct}%" "$(< "$BL/actual_brightness")"
done

echo
echo "resolution probe: 16 consecutive steps of $(( MAX / 1400 + 1 )) near 40%"
STEP=$(( MAX / 1400 + 1 ))
base=$(( MAX * 40 / 100 )); prev=""; n=0
for i in $(seq 0 15); do
    v=$(( base + i * STEP ))
    write "$v"; sleep 0.4
    a=$(< "$BL/actual_brightness")
    if [[ "$a" != "$prev" ]]; then n=$(( n + 1 )); prev="$a"; fi
done
echo "distinct levels: $n / 16"
if (( n <= 4 )); then
    echo "  -> heavily quantized (~100 discrete levels)"
else
    echo "  -> fine-grained"
fi
