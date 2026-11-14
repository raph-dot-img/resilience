#!/usr/bin/env bash
# Boot a Resilience ISO in a disposable QEMU VM. No root needed; nothing on
# the host is modified. The VM has NO network device unless --net is given.
#
#   scripts/boot-iso.sh result-iso/iso/*.iso            # serial console in this terminal
#   scripts/boot-iso.sh --gui result-iso/iso/*.iso      # QEMU window (VGA console)
#   scripts/boot-iso.sh --uefi --net user result-iso/iso/*.iso
#
# Options: --uefi (OVMF firmware from nixpkgs), --net user (slirp NAT),
#          --mem MiB (default 2048), --cpus N (default 2), --gui
set -euo pipefail

mem=2048 cpus=2 uefi=0 net=none gui=0
while [ $# -gt 1 ]; do
  case "$1" in
    --uefi) uefi=1 ;;
    --gui) gui=1 ;;
    --net) net="$2"; shift ;;
    --mem) mem="$2"; shift ;;
    --cpus) cpus="$2"; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done
iso="${1:?usage: boot-iso.sh [options] path/to/resilience.iso}"
[ -f "$iso" ] || { echo "no such ISO: $iso" >&2; exit 1; }

qemu=(qemu-system-x86_64 -m "$mem" -smp "$cpus" -cdrom "$iso" -boot d)
if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then qemu+=(-enable-kvm -cpu host); fi
case "$net" in
  none) qemu+=(-nic none) ;;
  user) qemu+=(-nic user,model=virtio-net-pci) ;;
  *) echo "--net must be none or user" >&2; exit 2 ;;
esac
if [ "$uefi" = 1 ]; then
  repo="$(cd "$(dirname "$0")/.." && pwd)"
  # Pinned nixpkgs from this repository's flake.lock, not the user registry.
  ovmf=$(nix build --no-link --print-out-paths --inputs-from "$repo" nixpkgs#OVMF.fd 2>/dev/null || true)
  [ -n "$ovmf" ] || { echo "could not obtain OVMF" >&2; exit 1; }
  qemu+=(-bios "$ovmf/FV/OVMF.fd")
fi
if [ "$gui" = 1 ]; then
  qemu+=(-vga std)
else
  echo "Serial console: kernel messages appear here; the TUI runs on the VGA console."
  echo "Quit QEMU with Ctrl-A then X."
  qemu+=(-nographic)
fi
exec "${qemu[@]}"
