#!/usr/bin/env bash
# Rebuild a patched CachyOS kernel package after a version bump.
#
# Run from the PKGBUILD directory (patches already added to source=, per docs/BUILD.md).
# Bump _major/_minor in the PKGBUILD and refresh the tarball checksum first:
#     updpkgsums
#
# Uses a SYSTEM-scope transient unit because deckify installs
# KillUserProcesses=True, which kills builds when the SSH session closes.
set -euo pipefail
cd "$(dirname "$(realpath "$0")")"
[[ -f PKGBUILD ]] || { echo "Run this from the PKGBUILD directory." >&2; exit 1; }

JOBS=$(nproc)
rm -rf src pkg
sudo systemctl reset-failed slidekbuild.service 2>/dev/null || true
sudo systemd-run --unit=slidekbuild \
  --property=User="$USER" --property=Group="$USER" \
  --property=WorkingDirectory="$PWD" \
  --property=RemainAfterExit=yes \
  --property=TimeoutStartSec=infinity \
  --setenv=HOME="$HOME" \
  --setenv=MAKEFLAGS="-j$JOBS" \
  --setenv=PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  --property=StandardOutput=append:"$PWD/build.log" \
  --property=StandardError=append:"$PWD/build.log" \
  /usr/bin/makepkg -Cf --noconfirm

cat <<EOF

build started as slidekbuild.service (-j$JOBS)
  progress : tail -f "$PWD/build.log"
  state    : systemctl is-active slidekbuild.service
  stop     : sudo systemctl stop slidekbuild.service

Expect ~2h45m on an AYANEO SLIDE.
EOF
