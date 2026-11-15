# Installation

Scope (2026-11-15): Resilience OS is now developed for the Resilience
cyberdeck, not as a general-purpose distribution (CLAUDE.md §1). How the
cyberdeck receives its system is not decided yet. What follows describes
what exists today, on generic x86_64 hardware and in virtual machines.

## Current status (v0.1.0)

**There is no installer yet** (Phase 8). Today you can:

1. **Try the live system** from a USB stick or in a VM (no changes to the disk).
2. **Install manually as an experienced NixOS user**, by importing the
   Resilience module into your own NixOS configuration (below).

Never test installation procedures on a machine whose data you need.

## 1. Live system

Build or obtain `resilience-standard-0.1.0-x86_64-linux.iso`, verify its
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

## 3. Planned installer (Phase 8)

Text-based, from the live system: hardware detection, explicit target-disk
selection showing model, size and existing partitions, a typed confirmation
before any write, optional LUKS encryption, edition and account/mode setup,
progress and a summary. It will never erase or repartition a disk
automatically, and it will be tested only on disposable VM disks.
