# Removable media and encrypted backups (ADR 0008).
#
# * udisks2 (D-Bus activated: it only runs while something asks it) mounts
#   USB sticks and memory cards for the user at the console. Its polkit
#   defaults allow removable media for an active local session without a
#   password and require an administrator for internal disks; the interface
#   calls udisksctl with --no-user-interaction, so internal disks are refused
#   instead of prompting. Mounts are nosuid,nodev under /run/media/<user>.
# * Backups are made by the unprivileged `resilience-backup` (tar | zstd |
#   age); its tool paths come from /etc/resilience/system.toml (core.nix).
{ config, lib, ... }:

let
  cfg = config.resilience.media;
in
{
  options.resilience.media.enable = lib.mkOption {
    type = lib.types.bool;
    default = true;
    description = "USB sticks and memory cards (udisks2) for the Backups & USB sticks screen.";
  };

  config = lib.mkIf cfg.enable {
    services.udisks2 = {
      enable = true;
      # /run/media/<user>: per-user, not shared, gone at reboot.
      mountOnMedia = false;
    };
  };
}
