# Reticulum (rns) and LXMF in one Python environment (ADR 0018), for the
# opt-in local messaging module.
#
# Both are under the Reticulum License: the MIT text plus two conditions
# (no use in systems able to purposefully harm people; no use for AI
# training datasets). It is not a free software licence and nixpkgs marks
# them unfree. They are allowed here by name, and nowhere else: nixpkgs is
# imported again with a predicate for exactly these two packages, because
# modules must work with read-only `pkgs` (CLAUDE.md §10).
#
# The environment also contains rnsh and rnx (remote shells and commands).
# Nothing puts its bin directory in PATH; services use absolute paths.
{
  lib,
  path,
  stdenv,
}:

let
  allowed = [
    "rns"
    "lxmf"
  ];
  pkgs = import path {
    inherit (stdenv.hostPlatform) system;
    config.allowUnfreePredicate = p: lib.elem (lib.getName p) allowed;
    overlays = [ ];
  };
  env = pkgs.python3.withPackages (ps: [
    ps.rns
    ps.lxmf
  ]);
in
env.overrideAttrs (old: {
  name = "resilience-reticulum-${pkgs.python3Packages.rns.version}";
  passthru = (old.passthru or { }) // {
    rnsVersion = pkgs.python3Packages.rns.version;
    lxmfVersion = pkgs.python3Packages.lxmf.version;
    unfreeNames = allowed;
  };
  meta = (old.meta or { }) // {
    description = "Reticulum and LXMF for Resilience local messaging (Reticulum License, not free software)";
  };
})
