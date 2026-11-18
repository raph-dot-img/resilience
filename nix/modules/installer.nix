# The text installer on the live system (ADR 0022).
#
# `resilience-install` (root, through pkexec) copies one of the prebuilt
# installed systems onto a whole disk. Its configuration is generated here
# and root-owned (/etc/resilience/install.toml). The polkit action allows
# the person at the console of a live system (allow_active = yes): they
# already control the machine and there is no administrator to ask. The
# TUI shows the exact disk and requires typing its name before running it.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.resilience.installer;
  toml = pkgs.formats.toml { };
  installer = "${config.resilience.package}/bin/resilience-install";
  util = pkgs.util-linux;

  installConfig = toml.generate "resilience-install.toml" {
    edition = config.resilience.edition;
    inherit installer;
    lsblk = "${util}/bin/lsblk";
    wipefs = "${util}/bin/wipefs";
    sfdisk = "${util}/bin/sfdisk";
    mount = "${util}/bin/mount";
    umount = "${util}/bin/umount";
    udevadm = "${config.systemd.package}/bin/udevadm";
    mkfs_vfat = "${pkgs.dosfstools}/bin/mkfs.vfat";
    mkfs_ext4 = "${pkgs.e2fsprogs}/bin/mkfs.ext4";
    cryptsetup = "${pkgs.cryptsetup}/bin/cryptsetup";
    nixos_install = lib.getExe' config.system.build.nixos-install "nixos-install";
    nixos_enter = lib.getExe pkgs.nixos-enter;
    loadkeys = "${pkgs.kbd}/bin/loadkeys";
    grub_install = "${pkgs.grub2}/bin/grub-install";
    sync = "${pkgs.coreutils}/bin/sync";
    path = "/run/current-system/sw/bin";
    keymaps = lib.unique (
      [ config.resilience.keyboard.layout ] ++ map (l: l.id) config.resilience.keyboard.layouts
    );
    systems = lib.mapAttrs (_: s: "${s}") cfg.systems;
  };

  policy = pkgs.writeTextDir "share/polkit-1/actions/org.resilience.install.policy" ''
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE policyconfig PUBLIC "-//freedesktop//DTD PolicyKit Policy Configuration 1.0//EN"
      "http://www.freedesktop.org/standards/PolicyKit/1/policyconfig.dtd">
    <policyconfig>
      <vendor>Resilience OS</vendor>
      <action id="org.resilience.install.run">
        <description>Install Resilience on a disk</description>
        <message>Installing erases the chosen disk</message>
        <defaults>
          <allow_any>no</allow_any>
          <allow_inactive>no</allow_inactive>
          <allow_active>yes</allow_active>
        </defaults>
        <annotate key="org.freedesktop.policykit.exec.path">${installer}</annotate>
        <annotate key="org.freedesktop.policykit.exec.argv1">run</annotate>
      </action>
    </policyconfig>
  '';
in
{
  options.resilience.installer = {
    enable = lib.mkEnableOption "the text installer (live systems only, ADR 0022)";
    systems = lib.mkOption {
      type = lib.types.submodule {
        options = lib.genAttrs [ "uefi_plain" "uefi_luks" "bios_plain" "bios_luks" ] (
          n:
          lib.mkOption {
            type = lib.types.package;
            description = "Installed system toplevel (${n}).";
          }
        );
      };
      description = "The prebuilt installed systems of this edition, copied by the installer.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !config.resilience.localAccounts;
        message = "resilience.installer is for live systems, not installed ones.";
      }
    ];
    environment.etc."resilience/install.toml".source = installConfig;
    environment.systemPackages = [ policy ];
    # nixos-install and nixos-enter must be there (the NixOS default).
    system.disableInstallerTools = false;
  };
}
