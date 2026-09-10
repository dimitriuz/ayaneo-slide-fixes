#!/usr/bin/env bash
# Rebuild InputPlumber with the local patches from patches/inputplumber/ and
# install it to /usr/local/bin, leaving the distro package alone.
#
# Needed after an InputPlumber package update, since the override in
# /usr/local/bin does not get refreshed by pacman -- it just keeps running the
# older patched build until you re-run this.
#
# Deps (Arch/CachyOS):  rust clang libiio libudev pkgconf
set -euo pipefail

PATCH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../patches/inputplumber" && pwd)"
BUILD_DIR="${BUILD_DIR:-$HOME/ip-build}"
REPO="https://github.com/ShadowBlip/InputPlumber"
# The version the patches were written against. A different tag may need them
# rebased -- git am will say so rather than applying something wrong.
TAG="${TAG:-v0.79.4}"

echo "==> patches:  $PATCH_DIR"
echo "==> building: $TAG in $BUILD_DIR"

for c in cargo clang pkg-config git; do
    command -v "$c" >/dev/null || { echo "missing: $c" >&2; exit 1; }
done
pkg-config --exists libiio  || { echo "missing: libiio (pacman -S libiio)" >&2; exit 1; }
pkg-config --exists libudev || { echo "missing: libudev" >&2; exit 1; }

if [[ ! -d "$BUILD_DIR/.git" ]]; then
    rm -rf "$BUILD_DIR"
    git clone --depth 1 --branch "$TAG" "$REPO" "$BUILD_DIR"
fi

cd "$BUILD_DIR"
git checkout -q .
git am --abort 2>/dev/null || true
git am "$PATCH_DIR"/*.patch

cargo build --release

INSTALLED_VER="$(pacman -Q inputplumber 2>/dev/null | awk '{print $2}' || echo '?')"
echo
echo "==> distro package: $INSTALLED_VER   patched build: $(./target/release/inputplumber --version)"
echo "==> installing to /usr/local/bin/inputplumber"
sudo install -m755 target/release/inputplumber /usr/local/bin/inputplumber

sudo mkdir -p /etc/systemd/system/inputplumber.service.d
sudo tee /etc/systemd/system/inputplumber.service.d/99-patched-binary.conf >/dev/null <<'CONF'
# Run the locally patched InputPlumber (configurable axis->mouse deadzone).
# Remove this file and `systemctl daemon-reload` to go back to the package.
[Service]
ExecStart=
ExecStart=/usr/local/bin/inputplumber
CONF

sudo systemctl daemon-reload
sudo systemctl restart inputplumber
sleep 3
[[ -x /usr/local/bin/ip-load-profile.sh ]] && sudo /usr/local/bin/ip-load-profile.sh || true

echo
echo "==> running: $(systemctl show -p ExecStart --value inputplumber \
                     | grep -o '/[^ ;]*inputplumber' | head -1)"
echo "==> $(systemctl is-active inputplumber)"
