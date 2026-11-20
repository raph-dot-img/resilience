#!/usr/bin/env bash
# Take the README screenshots again (tests/screenshots.nix: the standard
# edition in a QEMU VM, no network) and copy the ones the README uses into
# docs/images/. No root needed; one VM, 2 vCPU, 2 GiB. Look at the pictures
# before committing them.
#
#   scripts/update-screenshots.sh
set -euo pipefail

cd "$(dirname "$0")/.."
out=$(mktemp -d "${XDG_CACHE_HOME:-$HOME/.cache}/resilience-screenshots.XXXXXX")
trap 'rm -rf "$out"' EXIT

nix build -L --max-jobs 1 -o "$out/result" .#screenshots
mkdir -p docs/images
for name in 01-main-menu 03-map-luxembourg 04-map-luxembourg-wider 05-map-search; do
  install -m 0644 "$out/result/$name.png" "docs/images/$name.png"
done
ls -l docs/images/
