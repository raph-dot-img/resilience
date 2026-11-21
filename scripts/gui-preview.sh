#!/bin/sh
# Screenshot the GUI on this machine without a display: headless cage
# (pixman renderer) + grim. Development only (ADR 0026); no root.
#   scripts/gui-preview.sh OUT.png [WIDTHxHEIGHT] [KEYS...]
# KEYS: wtype arguments run one after another before the shot, e.g.
#   scripts/gui-preview.sh s.png 1366x768 'k' '-k Return'   (each item
#   is split on spaces and given to wtype; 1 s between items).
set -eu
out=${1:?usage: gui-preview.sh OUT.png [WxH]}
size=${2:-1366x768}
p() { nix build --no-link --print-out-paths --inputs-from . "nixpkgs#$1" | tail -1; }
cage=$(p cage); grim=$(p grim); foot=$(p foot); wtype=$(p wtype)
wl=$(p wayland.out); xkb=$(p libxkbcommon.out)
fonts=$(mktemp -d)
inter=$(p inter)/share/fonts/truetype; dv=$(p dejavu_fonts)/share/fonts/truetype
for f in "$inter/InterVariable.ttf" "$dv/DejaVuSans.ttf" "$dv/DejaVuSansMono.ttf" "$dv/DejaVuSansMono-Bold.ttf"; do
  ln -s "$(readlink -f "$f")" "$fonts/"
done
cargo build -q -p resilience-gui
gui=$PWD/target/debug/resilience-gui
run=$(mktemp -d); chmod 700 "$run"
export XDG_RUNTIME_DIR="$run" WLR_BACKENDS=headless WLR_RENDERER=pixman \
  WLR_HEADLESS_OUTPUTS=1 WLR_LIBINPUT_NO_DEVICES=1 \
  LD_LIBRARY_PATH="$wl/lib:$xkb/lib" RESILIENCE_GUI_FONTS="$fonts" \
  RESILIENCE_TERMINAL="$foot/bin/foot"
unset WAYLAND_DISPLAY DISPLAY
"$cage/bin/cage" -- "$gui" >"$run/cage.log" 2>&1 &
pid=$!
sock=""
for _ in $(seq 50); do
  sock=$(ls "$run" | grep '^wayland-[0-9]*$' | head -1 || true)
  [ -n "$sock" ] && break; sleep 0.2
done
sleep 6
shift; [ $# -gt 0 ] && shift
for k in "$@"; do
  # shellcheck disable=SC2086
  WAYLAND_DISPLAY=$sock "$wtype/bin/wtype" $k
  sleep 1
done
WAYLAND_DISPLAY=$sock "$grim/bin/grim" "$out" || { cat "$run/cage.log"; }
kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
rm -rf "$fonts" "$run"
echo "$out ($size)"
