# Resilience OS

**The operating system of Resilience, a standalone, offline-first cyberdeck.**

**Resilience** is a cyberdeck project: a self-contained, offline-first
laptop that we are building. **Resilience OS**, in this repository, is the
system that runs on it: NixOS, Rust, terminal-only.

Resilience OS is not a general-purpose distribution for any PC. It was
first planned as one, and some documents still read that way; it is now
developed for the cyberdeck. The cyberdeck's hardware is not described in
this repository yet.

Resilience OS is designed to stay useful — information, productivity, local
communication, security tools — during a long or complete loss of Internet
connectivity. It boots straight into a keyboard-driven text interface that
works at 80×24, without colour, and on modest hardware.

```
╔══════════════════════════════════════════════════════════════════════════════╗
║█▀█ █▀▀ █▀ █ █   █ █▀▀ █▄ █ █▀▀ █▀▀                                   12:49:23║
║█▀▄ ██▄ ▄█ █ █▄▄ █ ██▄ █ ▀█ █▄▄ ██▄                            STANDARD v0.1.0║
║OS :: AUTONOMOUS INFORMATION SYSTEM                         MODE: ULTRA-STABLE║
╚══════════════════════════════════════════════════════════════════════════════╝
┌ STATUS ────────────────────────────┐┌ MAIN MENU ─────────────────────────────┐
│SYSTEM  [OK] OPERATIONAL            ││> [1] KNOWLEDGE & DOCUMENTATION         │
│MODE    [--] ULTRA-STABLE           ││  [2] NETWORKING & MESH                 │
│NETWORK [--] OFFLINE                ││  [3] PRODUCTIVITY                      │
│POWER   [OK] BAT 71% DISCHARGING    ││  [4] ENTERTAINMENT                     │
│STORAGE [OK] 7.5 GiB FREE           ││  [5] SYSTEM MANAGEMENT                 │
```

> **Status: early prototype (v0.1.0).** It boots and is usable in a
> virtual machine (x86_64). It has **not** been tested on physical hardware,
> and never on the cyberdeck. It has no installer yet, and several planned
> features (maps are in progress; local messaging, radio) do not exist yet.
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

Resilience's own code is licensed under the [0BSD license](LICENSE)
(Zero-Clause BSD). Bundled third-party
software keeps its own license; see [docs/COMPONENTS.md](docs/COMPONENTS.md).
