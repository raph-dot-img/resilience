# Installation

## Current status (0.3.0)

You can:

1. **Try the live system** from a USB stick or in a VM (no changes to the disk).
2. **Install it on a whole disk with the text installer** of the live
   system (section 3; ADR 0022; since 0.3.0). Tested in VMs only.
3. **Install manually as an experienced NixOS user**, by importing the
   Resilience module into your own NixOS configuration (section 2).

Never test installation procedures on a machine whose data you need.

## 1. Live system

Build or obtain `resilience-standard-0.3.0-x86_64-linux.iso`, verify its
SHA-256 checksum against the one published with it, and either boot it in a
VM (`scripts/boot-iso.sh --gui`) or write it to a USB stick (docs/BUILDING.md).

The live system boots directly into the interface as the Ultra-Stable
`resilience` account. It has **no administrator account**: nothing on the
computer's disks is touched. Everything you do is lost at power-off.

Tested: QEMU with BIOS and UEFI firmware. **Not tested on physical hardware.**

## 2. Manual installation into a NixOS system (advanced)

Prerequisites: a working NixOS 26.05 installation using flakes, and
familiarity with `nixos-rebuild`. In your system flake:

```nix
{
  inputs.resilience.url = "path:/path/to/resilience";   # or a git URL you trust
  outputs = { nixpkgs, resilience, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        resilience.nixosModules.default
        "${resilience}/nix/profiles/standard.nix"
        {
          resilience.users.alice.mode = "ultra-stable";
          resilience.users.admin.mode = "advanced";
          # Set passwords with `passwd` after the first boot, or provide
          # hashedPasswordFile pointing OUTSIDE the Nix store.
        }
      ];
    };
  };
}
```

Then, as root on that machine: `nixos-rebuild switch --flake .#myhost`.

Notes:
- The module locks root's password and disables SSH by default.
  Make sure an `advanced` account with a password exists **before** switching,
  or you will lose administrative access (recover by booting a previous
  generation).
- `resilience.interface.autologinUser` is only accepted for an
  `ultra-stable` account and only applies to tty1.

## 3. The text installer (ADR 0022)

From the live system: SYSTEM > Install Resilience on this computer. The
handbook page "Installing Resilience on a computer" is the user guide;
this section is the technical summary.

- **What is installed.** The live ISO carries four prebuilt systems of
  its edition (`installed-<edition>-<uefi|bios>-<plain|luks>` in the
  flake). They are the same for every machine; the installer copies one
  with `nixos-install --system`, offline. Firmware is the one the live
  system booted with (`/sys/firmware/efi`).
- **Disk layout** (whole disk, GPT): 1 MiB BIOS boot partition; 1 GiB EFI
  system partition `RESBOOT` (FAT32, `/boot`); the rest `resilience-root`
  (ext4, or LUKS2 then ext4). The installed system finds them by label.
- **Boot loader.** UEFI: systemd-boot, without writing firmware variables
  (also installed as `EFI/BOOT/BOOTX64.EFI`). BIOS: GRUB, installed on
  the disk once by the installer.
- **Choices kept as state** in `/var/lib/resilience/`: `keyboard`
  (`KEYMAP=<id>`, also added to the initrd for the passphrase prompt),
  `accounts.toml` (accounts created on the machine) and `modes.toml`
  (their modes, managed by `resilience-admin` as usual). Accounts are
  created with `useradd` in the new system; passwords are typed into
  `passwd`, the passphrase into `cryptsetup`.
- **Safety.** Only whole, unused, writable disks of at least 16 GiB are
  offered; the live stick is in use and never offered. The TUI requires
  the disk's name to be typed. `resilience-install` (root, through pkexec,
  polkit action `org.resilience.install.run` allowed to the active local
  session of a live system only) checks everything again on its own: the
  disk must still have the size and model shown in the summary.
- **Not offered yet:** installing next to another system, other file
  systems, swap, choosing the host name or time zone (fixed: `resilience`,
  UTC).
