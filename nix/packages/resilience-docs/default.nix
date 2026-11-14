# The built-in documentation pack (ADR 0013): a ZIM file made at build time
# from the Resilience handbook, the NixOS manual and the NixOS option list
# of the pinned nixpkgs. Read in the Offline library like any other pack.
#
# The NixOS manual and options come from nixpkgs' own release jobs
# (nixos/release.nix, with its default arguments). For the pinned nixpkgs
# these outputs were found in the binary cache on 2026-11-14 (2 MiB each);
# otherwise they are built locally, which takes a few minutes. They show the
# NixOS release (26.05), not the exact revision, and do not include the
# `resilience.*` options (documented in the handbook and docs/).
{
  lib,
  runCommand,
  python3,
  zim-tools,
  path,
  handbook,
}:

let
  release = import (path + "/nixos/release.nix") { };
  manual = "${release.manualHTML.${python3.stdenv.hostPlatform.system}}/share/doc/nixos";
  options = "${release.options}/share/doc/nixos/options.json";
  # A 48x48 PNG (dark green), required by zimwriterfs.
  icon = ./icon.png;
in
runCommand "resilience-docs-pack"
  {
    nativeBuildInputs = [
      python3
      zim-tools
    ];
    meta.description = "Resilience OS built-in documentation pack (ZIM)";
  }
  ''
    python3 -I ${./make-docs.py} ${handbook}/share/resilience/handbook ${manual} ${options} html
    cp ${icon} html/icon.png
    mkdir -p $out/share/resilience/packs
    zimwriterfs --welcome=index.html --illustration=icon.png --language=eng \
      --title="System documentation" \
      --description="Resilience handbook, NixOS manual and options" \
      --creator="Resilience OS" --publisher="Resilience OS" \
      --name=resilience_system_docs \
      html $out/share/resilience/packs/resilience-system-docs.zim
  ''
