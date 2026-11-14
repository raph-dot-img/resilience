# Bootable live ISO (UEFI + legacy BIOS, USB-bootable).
#
# Boots straight into the TUI as the Ultra-Stable `resilience` account.
# The live system has no administrative account at all: it is for exploring
# Resilience, not for administration (the installer is a later milestone).
{
  config,
  lib,
  modulesPath,
  ...
}:

let
  version = config.resilience.package.version;
in
{
  imports = [
    "${modulesPath}/installer/cd-dvd/iso-image.nix"
    # Kernel modules for common hardware (storage, USB, network controllers).
    "${modulesPath}/profiles/all-hardware.nix"
  ];

  isoImage = {
    volumeID = "RESILIENCE_${lib.replaceStrings [ "." ] [ "_" ] version}";
    edition = config.resilience.edition;
    makeEfiBootable = true;
    makeUsbBootable = true;
    forceTextMode = true;
    # Level 19 (the default) is very slow on a 2-core budget for little gain.
    squashfsCompression = "zstd -Xcompression-level 9";
    appendToMenuLabel = " (live)";
  };
  image.fileName = "resilience-${config.resilience.edition}-${version}-x86_64-linux.iso";

  networking.hostName = "resilience-live";

  resilience.users.resilience.description = "Resilience live session";
  # Locked password: the account is reachable only through autologin on tty1.
  users.users.resilience.hashedPassword = "!";
  resilience.interface.autologinUser = "resilience";

  # Wi-Fi and many network cards need firmware blobs (redistributable only).
  hardware.enableRedistributableFirmware = true;
}
