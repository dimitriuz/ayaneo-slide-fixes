#!/bin/bash
# Windows 11 VM for running AYASpace, with the AYANEO controller passed through.
#
# Nothing on the host disk is repartitioned; the guest lives in a qcow2 file.
# Run this from a terminal ON the handheld so the QEMU window opens on its screen.
set -euo pipefail
cd "$(dirname "$(realpath "$0")")"

ISO=win11.iso
DISK=win11.qcow2
VARS=OVMF_VARS.4m.fd
OVMF_CODE=/usr/share/edk2/x64/OVMF_CODE.secboot.4m.fd
OVMF_VARS_SRC=/usr/share/edk2/x64/OVMF_VARS.4m.fd
PAD_VID=045e
PAD_PID=028e
# Extra USB devices to hand to the guest, as space-separated vid:pid pairs.
# Defaults to a Logitech Unifying Receiver (K400 etc.) when one is present, so
# the VM gets a real keyboard and touchpad. Override with e.g.
#   EXTRA_USB="046d:c52b 1234:5678" ~/winvm/winvm.sh
# or disable with EXTRA_USB="" to keep it on the host.
: "${EXTRA_USB=046d:c52b}"
RAM=8G
CORES=4

[ -f "$ISO" ] || { echo "missing $ISO"; exit 1; }
[ -f "$DISK" ] || { echo "creating $DISK (80G sparse)"; qemu-img create -f qcow2 "$DISK" 80G >/dev/null; }
[ -f "$VARS" ] || { echo "seeding UEFI vars"; cp "$OVMF_VARS_SRC" "$VARS"; }

# --- TPM 2.0 (Windows 11 requires it) ---
mkdir -p tpm
if ! pgrep -f "swtpm.*$PWD/tpm" >/dev/null; then
    echo "starting swtpm..."
    swtpm socket --tpm2 --tpmstate dir="$PWD/tpm" \
        --ctrl type=unixio,path="$PWD/tpm/swtpm-sock" \
        --terminate --daemon
    sleep 1
fi

# --- release the controller from InputPlumber so QEMU can claim it ---
echo "releasing controller from InputPlumber..."
inputplumber device 0 stop >/dev/null 2>&1 || true
sleep 1

restore() {
    echo
    echo "restoring InputPlumber..."
    sudo systemctl restart inputplumber >/dev/null 2>&1 || true
    sleep 4
    inputplumber device 0 profile load /etc/inputplumber/profiles/rightstick-mouse.yaml >/dev/null 2>&1 || true
    echo "done."
}
trap restore EXIT

NIC_ARG="-nic user,model=e1000e"
if [ -n "${NONET:-}" ]; then
    NIC_ARG="-nic none"
    echo "networking DISABLED (NONET=1) - OOBE should offer an offline account"
fi

EXTRA_ARGS=()
n=0
for vp in $EXTRA_USB; do
    v=${vp%%:*}; p=${vp##*:}
    if lsusb -d "$vp" >/dev/null 2>&1; then
        n=$((n+1))
        EXTRA_ARGS+=(-device "usb-host,vendorid=0x$v,productid=0x$p,id=extra$n")
        echo "passing through $vp ($(lsusb -d "$vp" | sed "s/^.*$vp //"))"
        echo "  NOTE: the host loses this device while the VM runs."
    else
        echo "skipping $vp - not present"
    fi
done

BOOT_ARG=""
# boot from the ISO until Windows is installed (detect via a marker file)
[ -f .installed ] || BOOT_ARG="-boot order=d,menu=on"

echo "launching VM (close the window to quit)"
qemu-system-x86_64 \
  -machine q35,accel=kvm,smm=on \
  -cpu host \
  -m "$RAM" -smp "$CORES" \
  -global driver=cfi.pflash01,property=secure,value=on \
  -drive if=pflash,format=raw,unit=0,file="$OVMF_CODE",readonly=on \
  -drive if=pflash,format=raw,unit=1,file="$VARS" \
  -chardev socket,id=chrtpm,path="$PWD/tpm/swtpm-sock" \
  -tpmdev emulator,id=tpm0,chardev=chrtpm \
  -device tpm-tis,tpmdev=tpm0 \
  -drive file="$DISK",if=none,id=hd0,format=qcow2,cache=writeback \
  -device ich9-ahci,id=ahci \
  -device ide-hd,drive=hd0,bus=ahci.0 \
  -drive file="$ISO",media=cdrom,if=none,id=cd0 \
  -device ide-cd,drive=cd0,bus=ahci.1 \
  $NIC_ARG \
  -device qemu-xhci,id=xhci \
  -device usb-host,vendorid=0x0$PAD_VID,productid=0x0$PAD_PID,id=pad \
  -device usb-tablet \
  -device usb-kbd \
  "${EXTRA_ARGS[@]}" \
  -smbios type=1,manufacturer=AYANEO,product=SLIDE,version=1.0,serial=AYANEO \
  -smbios type=2,manufacturer=AYANEO,product=AS01 \
  -rtc base=localtime \
  -display gtk,show-cursor=on \
  -vga virtio \
  $BOOT_ARG
