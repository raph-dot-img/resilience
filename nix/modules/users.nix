# Resilience accounts and security modes (see docs/decisions/0003-security-modes.md).
#
# A mode is a set of real OS privileges, expressed as group membership:
#   ultra-stable : no extra groups; no Nix daemon access; no sudo.
#   stable       : resilience-stable (+ networkmanager): unprivileged shell, Nix daemon access.
#   advanced     : additionally wheel: sudo with the account's password.
# The TUI derives the mode from these groups at runtime; it never stores it.
{
  config,
  lib,
  ...
}:

let
  inherit (lib) mkOption types;
  cfg = config.resilience;

  groupsFor =
    mode:
    {
      "ultra-stable" = [ ];
      "stable" = [
        "resilience-stable"
        "networkmanager"
      ];
      "advanced" = [
        "resilience-stable"
        "networkmanager"
        "wheel"
      ];
    }
    .${mode};

  userModule = {
    options = {
      mode = mkOption {
        type = types.enum [
          "ultra-stable"
          "stable"
          "advanced"
        ];
        default = "ultra-stable";
        description = "Security mode of the account. New accounts default to the least-privileged mode.";
      };
      description = mkOption {
        type = types.str;
        default = "";
        description = "Full name or comment.";
      };
      extraGroups = mkOption {
        type = types.listOf types.str;
        default = [ ];
        description = "Additional groups (hardware access such as audio or dialout). Must not grant privileges.";
      };
      hashedPasswordFile = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = ''
          Path (outside the Nix store) to a file containing the hashed password.
          Never put plaintext or hashed passwords into Nix expressions.
        '';
      };
    };
  };

  privilegedGroups = [
    "wheel"
    "resilience-stable"
    "networkmanager"
    "root"
    "systemd-journal"
    "nixbld"
    "docker"
    "libvirtd"
    "kvm"
    "disk"
  ];
in
{
  options.resilience.users = mkOption {
    type = types.attrsOf (types.submodule userModule);
    default = { };
    example = {
      alice.mode = "stable";
    };
    description = "Human accounts managed by Resilience.";
  };

  config = {
    assertions = lib.mapAttrsToList (name: u: {
      assertion = lib.intersectLists u.extraGroups privilegedGroups == [ ];
      message = ''
        resilience.users.${name}.extraGroups contains a privilege-granting group
        (${toString (lib.intersectLists u.extraGroups privilegedGroups)}). Use `mode` instead.
      '';
    }) cfg.users;

    users.groups.resilience-stable = { };

    users.users = lib.mapAttrs (
      _name: u:
      {
        isNormalUser = true;
        inherit (u) description;
        shell = cfg.sessionPackage;
        extraGroups = groupsFor u.mode ++ u.extraGroups;
      }
      // lib.optionalAttrs (u.hashedPasswordFile != null) { inherit (u) hashedPasswordFile; }
    ) cfg.users;

    environment.shells = [ cfg.sessionPackage ];

    # Ultra-Stable accounts may not talk to the Nix daemon at all, so they
    # cannot build or install software or fill the store.
    nix.settings.allowed-users = [
      "root"
      "@wheel"
      "@resilience-stable"
    ];
    nix.settings.trusted-users = [ "root" ];
  };
}
