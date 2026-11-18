# Resilience OS

**An offline-first, terminal-only operating system for x86_64 PCs, based on NixOS.**

Resilience OS is designed to stay useful — information, productivity, local
communication, security tools — during a long or complete loss of Internet
connectivity. It boots straight into a keyboard-driven text interface that
works at 80×24, without colour, and on modest hardware.

```
╔══════════════════════════════════════════════════════════════════════════════╗
║█▀█ █▀▀ █▀ █ █   █ █▀▀ █▄ █ █▀▀ █▀▀                                   12:49:23║
║█▀▄ ██▄ ▄█ █ █▄▄ █ ██▄ █ ▀█ █▄▄ ██▄                            STANDARD v0.4.0║
║OS :: AUTONOMOUS INFORMATION SYSTEM                         MODE: ULTRA-STABLE║
╚══════════════════════════════════════════════════════════════════════════════╝
┌ STATUS ────────────────────────────┐┌ MAIN MENU ─────────────────────────────┐
│SYSTEM  [OK] OPERATIONAL            ││> [1] KNOWLEDGE & DOCUMENTATION         │
│MODE    [--] ULTRA-STABLE           ││  [2] NETWORKING & MESH                 │
│NETWORK [--] OFFLINE                ││  [3] PRODUCTIVITY                      │
│POWER   [OK] BAT 71% DISCHARGING    ││  [4] ENTERTAINMENT                     │
│STORAGE [OK] 7.5 GiB FREE           ││  [5] SYSTEM MANAGEMENT                 │
```

> **Status: early prototype (v0.4.0).** It boots and is usable in a
> virtual machine (x86_64). It has booted once on physical hardware (a ThinkPad
> L15 Gen 1) and has otherwise run only in virtual machines. It runs as a live
> system and has a text installer (whole disk, optional encryption) and
> signed offline updates from a USB stick, both tested in virtual machines
> only. Maps cover the world overview and
> packs you build; local messaging and the mesh have been tested between
> virtual machines only; radio tools do not exist yet.
> See [ROADMAP.md](ROADMAP.md) and [docs/TESTING.md](docs/TESTING.md) for
> exactly what works.

## What works today

- Bootable live ISO (UEFI and BIOS) that boots into the TUI with no network.
- Main menu, live status panel (system health, mode, network, battery,
  storage, memory), search, context help, four themes including Monochrome.
- Three security modes enforced by the operating system (groups, sudo,
  Nix daemon access) — not by hidden menus. Modes can be changed from the
  interface with an administrator's password; every change is audited and
  survives reboots.
- Recovery section: system health, failed services, earlier system
  generations, exportable diagnostic report, emergency guide.
- Backups & USB sticks: encrypted (age) backups of your home folder on a USB
  stick, checked without the passphrase, restored into a new folder; safe
  eject. Internal disks are never offered.
- Offline handbook, private notes, network diagnostics with an opt-in,
  time-limited Internet probe, system status report, notification history.
- Launcher for verified text-mode programs (editors, file manager,
  calculator, calendar, process monitor, games) with restricted variants for
  beginners.
- Crash recovery: the session restarts the interface; other consoles and
  previous system generations remain available.
- Text installer (whole disk, encryption on by default, accounts and their
  modes, keyboard layout) and offline updates: a signed update on a USB
  stick is checked, installed with an administrator's password, and starts
  at the next restart with the previous version kept in the boot menu.
  Computers installed from 0.3.0 cannot take updates: reinstall them once.

## Downloads

Releases (ISO, offline update, source) are published on GitHub and on
<https://mirror.canevet.eu/files/resilience/>, with a `SHA256SUMS` file
signed with minisign. Check a download as explained in
[keys/README.md](keys/README.md) before using it.

## Quick start (developers)

Requires Nix with flakes; no root access is needed.

```sh
nix develop                  # Rust + Nix tooling
cargo test                   # Rust unit and interface tests
nix run .#vm                 # boot a development VM in the terminal
nix build .#iso              # build the live ISO (large, slow)
```

See [docs/BUILDING.md](docs/BUILDING.md) and [docs/TESTING.md](docs/TESTING.md).

## Documentation

| For | Read |
|---|---|
| Everyone | `handbook/` (also inside the system), [docs/RECOVERY.md](docs/RECOVERY.md), [docs/OFFLINE_OPERATIONS.md](docs/OFFLINE_OPERATIONS.md) |
| Administrators | [docs/INSTALLATION.md](docs/INSTALLATION.md), [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), [SECURITY.md](SECURITY.md) |
| Contributors | [CONTRIBUTING.md](CONTRIBUTING.md), [CLAUDE.md](CLAUDE.md), [docs/decisions/](docs/decisions/) |

## License

Resilience's own code is licensed under the
[Hippocratic License 2.1](LICENSE), an Ethical Source license: broad
permission to use, change and share it, on the condition that it is used
in a manner consistent with human rights. It is not a free software
license in the FSF/OSI sense. Bundled third-party software keeps its own
license; see [docs/COMPONENTS.md](docs/COMPONENTS.md).
