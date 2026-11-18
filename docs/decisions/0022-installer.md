# ADR 0022: The text installer (one prebuilt system, choices kept as state)

Status: accepted (2026-11-17).

## Context

Resilience runs only as a live system: nothing is kept across a reboot.
Messages, keys, Wi-Fi profiles and packs are lost, and the live system has
no administrator, so modes cannot be changed and packs cannot be
installed. The specification (§26) asks for a text installer for
non-experts: hardware detection, disk selection, a partition overview,
file system and encryption options, profile, user creation with security
modes, bootloader, progress, summary and error handling. It must never
erase a disk without showing the exact device and the consequences and
getting an explicit confirmation, and it must be tested in disposable VMs.

Constraints that shape the design:

- **Installing must work offline**, from the ISO alone.
- A NixOS system built for one machine (its accounts, keyboard,
  partitions in Nix) can only be built where its build tools are. The
  live ISO holds the runtime closure, not compilers, `makeInitrd` inputs
  or nixpkgs' sources. Building on the target is either impossible
  offline or needs gigabytes of build dependencies on the ISO.
- Offline updates (spec §19, the next part of Phase 8) bring a new system
  built elsewhere and copied with `nix copy`. That only works if the same
  system closure fits every installed machine.
- Spec §8: Advanced must not be enabled by default for new installations.

## Decision

1. **The installed system is prebuilt and the same for every machine.**
   The flake gains installed configurations of each edition:
   `installed-<edition>-<boot>-<disk>` where boot is `uefi` (systemd-boot)
   or `bios` (GRUB) and disk is `plain` or `luks`. They are generic:
   - partitions by label: the boot partition is the FAT file system
     labelled `RESBOOT` (`/boot`), the root is the ext4 file system
     labelled `resilience-root`; with LUKS, the LUKS2 container is the
     GPT partition labelled `resilience-root` and opens as `/dev/mapper/root`;
   - all-hardware initrd modules, redistributable firmware and both CPU
     microcode updates, as on the live ISO (hardware detection is the
     kernel's, at every boot);
   - host name `resilience`, time zone UTC, no autologin (ADR 0006: an
     installed system has real accounts with passwords).
   The live ISO carries the four toplevels of its own edition in its
   store; they share almost everything with the live system (two initrds
   and a few small files more). Installing copies one of them.
   Updates later replace it by the same configuration built anew.

2. **What is chosen at install time is state, not Nix:**
   - **Accounts** are created with `useradd` in the new system
     (`users.mutableUsers = true` on installed systems, the NixOS
     default), login shell `/run/current-system/sw/bin/resilience-session`
     (stable across generations). Their modes go into the existing
     `/var/lib/resilience/modes.toml` (ADR 0007); `resilience-admin`
     treats an account that is listed there and not declared in Nix as a
     **local account** whose base mode is Ultra-Stable, so `set-mode` and
     `apply-state` work on it the same way. Passwords are typed into
     `passwd` running in the new system, never seen by the installer.
   - **The system keyboard layout** goes into
     `/var/lib/resilience/keyboard` (one layout id from the offered list,
     ADR 0017). Getty's `ExecStartPre` and the TUI's session start read it
     (falling back to the built-in layout if it is missing or not
     listed). The initrd needs it too, for the LUKS passphrase: it is
     added with `boot.initrd.secrets` (a small extra initrd made from the
     file when the bootloader is installed; it is not secret), and the
     initrd loads that layout (the offered keymaps are in the initrd)
     before asking for the passphrase.

3. **One disk layout, the whole disk.** GPT:
   1. `bios` 1 MiB, BIOS boot partition (used only by GRUB on BIOS);
   2. `RESBOOT` 1 GiB, EFI system partition, FAT32, mounted at `/boot`;
   3. `resilience-root`, the rest, ext4 (or LUKS2 then ext4).
   The same layout boots with either firmware. No swap partition (zram
   later if measurements ask for it). Installing next to another system
   (dual boot), other file systems and manual partitioning are not
   offered yet: the spec's "file system options" are ext4 with or without
   encryption for now.

4. **Bootloaders.** UEFI: systemd-boot with `canTouchEfiVariables =
   false` (no firmware NVRAM writes; systemd-boot is also installed as the
   removable fallback `EFI/BOOT/BOOTX64.EFI`, which firmware finds on its
   own). BIOS: GRUB with `devices = [ "nodev" ]` in the toplevel, so later
   generations only rewrite `grub.cfg`; the installer runs `grub-install
   --target=i386-pc` on the chosen disk once. The installer picks UEFI
   when the live system booted with UEFI (`/sys/firmware/efi`), BIOS
   otherwise.

5. **Encryption is offered and on by default** (spec: "optional
   encryption configuration"). `cryptsetup luksFormat` asks for the
   passphrase itself, twice, on the console, after the installer has
   loaded the chosen keyboard layout: the same layout the initrd will
   load at boot. The installer never sees the passphrase.

6. **A privileged program, `resilience-install`, only on the live ISO.**
   - Planning is pure and unit-tested in `resilience-core/src/install.rs`:
     which disks may be chosen (whole disks only; never the disk the live
     system booted from; never a disk with a mounted partition; at least
     16 GiB), the partition table, the account and layout rules, the steps.
   - The TUI's Install screens (live ISO only) gather the answers, show
     the summary with the exact device (path, model, size, the partitions
     it holds now) and its consequence ("everything on it is erased"),
     and require typing the device name to confirm. Then they run
     `pkexec resilience-install run …` in the foreground, the same way as
     the other helpers.
   - polkit: action `org.resilience.install.run`, **allow_active = yes**,
     defined only in the live ISO's configuration. Whoever sits at a
     running live system already controls the machine (they booted it
     from their stick); there is no administrator to ask. Inactive and
     remote sessions get "no".
   - `resilience-install` (root) re-checks every argument and the disk
     rules itself (the TUI's checks are not trusted), refuses if the disk
     changed since the summary (size and model given on the command line
     must match), then prints numbered steps: wipe signatures, partition
     (`sfdisk`), format, LUKS, mount, `nixos-install --system <toplevel>
     --no-root-passwd --no-channel-copy`, accounts, passwords, state files,
     GRUB on BIOS, unmount. Every tool by absolute path from a root-owned
     `/etc/resilience/install.toml`. On failure it stops, unmounts, says
     which step failed and that the disk is not usable yet; nothing is
     retried silently.
   - The root account stays locked on the installed system, as on every
     Resilience system.

7. **Accounts proposed by default:** one everyday account in Stable mode
   and one administrator account in Advanced mode, used only to authorise
   (polkit, sudo). The person may rename them, change the everyday mode,
   add up to 6 accounts or drop the administrator, with a warning that
   nobody could then change modes, install packs or update.

## Consequences

- One system closure per edition and firmware type: installs, offline
  updates and VM tests all use the same tested artefacts.
- The ISO grows by two initrds and a few store paths per toplevel; the
  standard ISO only installs the standard edition (and the minimal ISO
  the minimal one).
- `resilience-admin` gains local accounts; vm-admin keeps testing
  declared ones, vm-install tests local ones.
- ADR 0012 counts local accounts: an installed system is never
  "Ultra-Stable only" (its accounts are unknown at build time and may be
  raised), so user namespaces and the Nix sandbox stay on there
  (`security-invariants` checks it).
- Host name, time zone and file system choices are fixed for now.
- The live system gains a polkit action with allow_active = yes for a
  destructive operation. It is limited to the live ISO, to the person at
  the console, and to a disk they confirmed by typing its name.
- Tested in VMs (`vm-install`: install on an empty virtual disk, reboot
  from it, log in, LUKS passphrase, modes of local accounts), UEFI and
  BIOS. Real machines: the owner's tests.
