# Development VM (`nix run .#vm`). Shares the host's Nix store over 9p, so it
# builds in seconds and needs no disk image.
#
# TEST-ONLY ACCOUNTS: the passwords below are public and exist only so the
# three modes can be tried in a throw-away VM. Never reuse this file for a
# real system.
{ lib, ... }:

{
  networking.hostName = "resilience-dev";

  resilience.users = {
    demo.mode = "ultra-stable";
    stable.mode = "stable";
    admin.mode = "advanced";
  };
  users.users = {
    demo.hashedPassword = "!"; # autologin only
    stable.initialPassword = "stable";
    admin.initialPassword = "admin";
  };
  resilience.interface.autologinUser = "demo";

  virtualisation.vmVariant.virtualisation = {
    memorySize = 2048;
    cores = 2;
    graphics = false;
    diskSize = 2048;
  };
  # Needed by nixos-rebuild build-vm style configurations without a disk.
  boot.loader.grub.device = lib.mkDefault "nodev";
  fileSystems."/" = lib.mkDefault {
    device = "/dev/disk/by-label/nixos";
    fsType = "ext4";
  };
}
