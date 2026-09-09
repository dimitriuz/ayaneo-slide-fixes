#!/bin/bash
# Load an InputPlumber device profile once the composite device has been detected.
#
# InputPlumber's device detection is asynchronous, so a plain
# After=inputplumber.service races and fails - hence the retry loop.
PROFILE="${1:-/etc/inputplumber/profiles/rightstick-mouse.yaml}"
DEVICE_NAME="${2:-AYANEO Slide}"

[ -f "$PROFILE" ] || { echo "no profile at $PROFILE, nothing to do"; exit 0; }

for _ in $(seq 1 60); do
    if inputplumber devices list 2>/dev/null | grep -q "$DEVICE_NAME"; then
        inputplumber device 0 profile load "$PROFILE" && exit 0
    fi
    sleep 1
done
echo "timed out waiting for InputPlumber composite device '$DEVICE_NAME'" >&2
exit 1
