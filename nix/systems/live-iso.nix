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

    # Text-only boot menus: no NixOS artwork, green on black.
    splashImage = ./assets/black-800x600.png;
    efiSplashImage = ./assets/black-800x600.png;
    grubTheme = null;
    syslinuxTheme = ''
      MENU TITLE RESILIENCE OS :: BOOT
      MENU RESOLUTION 800 600
      MENU CLEAR
      MENU ROWS 6
      MENU CMDLINEROW -4
      MENU TIMEOUTROW -3
      MENU TABMSGROW  -2
      MENU HELPMSGROW -1
      MENU HELPMSGENDROW -1
      MENU MARGIN 0

      #                                FG:AARRGGBB  BG:AARRGGBB   shadow
      MENU COLOR BORDER       32;40      #00000000    #00000000   none
      MENU COLOR SCREEN       32;40      #FF33FF33    #00000000   none
      MENU COLOR TABMSG       32;40      #FF1FAA1F    #00000000   none
      MENU COLOR TIMEOUT      1;32;40    #FF33FF33    #00000000   none
      MENU COLOR TIMEOUT_MSG  32;40      #FF33FF33    #00000000   none
      MENU COLOR CMDMARK      1;32;40    #FF33FF33    #00000000   none
      MENU COLOR CMDLINE      32;40      #FF33FF33    #00000000   none
      MENU COLOR TITLE        1;32;40    #FF33FF33    #00000000   none
      MENU COLOR UNSEL        32;40      #FF33FF33    #00000000   none
      MENU COLOR SEL          7;32;40    #FF000000    #FF33FF33   none
    '';
  };
  # iso-image.nix defines baseName itself, hence mkForce.
  image.baseName = lib.mkForce "resilience-${config.resilience.edition}-${version}-x86_64-linux";

  networking.hostName = "resilience-live";

  resilience.users.resilience.description = "Resilience live session";
  # Locked password: the account is reachable only through autologin on tty1.
  users.users.resilience.hashedPassword = "!";
  resilience.interface.autologinUser = "resilience";

  # Wi-Fi and many network cards need firmware blobs (redistributable only).
  hardware.enableRedistributableFirmware = true;
}
