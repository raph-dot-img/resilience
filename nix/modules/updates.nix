# Offline updates (ADR 0023).
#
# An installed system takes a newer system of its own variant from a signed
# Nix binary cache on a USB stick: `resilience-admin apply-update DIR`
# (polkit org.resilience.admin.apply-update, auth_admin) imports it with
# `nix copy`, signatures required and checked against `trustedKeys` only,
# then makes it the system started next (switch-to-configuration boot).
# The keys are passed to Nix for each import; they are not added to
# nix.conf, so they trust nothing else.
{
  config,
  lib,
  ...
}:

let
  cfg = config.resilience;
  u = cfg.updates;
  keyPattern = "[A-Za-z0-9._-]{1,64}:[A-Za-z0-9+/]{43}=";
  enabled = u.trustedKeys != [ ] && u.variant != null && cfg.localAccounts;
in
{
  options.resilience.updates = {
    trustedKeys = lib.mkOption {
      type = lib.types.listOf (lib.types.strMatching keyPattern);
      default = [ ];
      description = ''
        Nix public keys (`name:base64`) of the Resilience release key.
        Update bundles must be signed by one of them. Empty: updates are
        not offered.
      '';
    };
    variant = lib.mkOption {
      type = lib.types.nullOr (lib.types.strMatching "[a-z0-9-]{1,64}");
      default = null;
      example = "installed-standard-uefi-luks";
      description = ''
        The installed configuration this system is (ADR 0022); an update's
        manifest gives the new system of each variant.
      '';
    };
    settings = lib.mkOption {
      type = lib.types.nullOr lib.types.attrs;
      internal = true;
      readOnly = true;
      default =
        if enabled then
          {
            nix = "${config.nix.package}/bin/nix";
            trusted_keys = u.trustedKeys;
            inherit (u) variant;
            inherit (cfg) edition version;
          }
        else
          null;
      description = "The [updates] section of system.toml and admin.toml (null: no updates).";
    };
  };
}
