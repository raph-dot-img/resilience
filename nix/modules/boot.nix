# Text boot: quiet kernel (errors still shown), systemd status shown only for
# failures and slow units, so problems are visible and never hidden.
{ lib, ... }:

{
  boot.consoleLogLevel = lib.mkDefault 3;
  boot.kernelParams = [
    "systemd.show_status=auto"
    "consoleblank=300"
  ];
  boot.loader.timeout = lib.mkDefault 5;

  # Keep the closure small: no NixOS HTML manual (it can come as a resource
  # pack), but keep man pages.
  documentation = {
    enable = true;
    man.enable = true;
    # The whatis database: "Manual pages" searches descriptions (man -k).
    man.cache.enable = true;
    info.enable = false;
    doc.enable = false;
    nixos.enable = false;
  };
  # Drop NixOS's implicit default packages (perl, rsync, strace).
  environment.defaultPackages = lib.mkForce [ ];
}
