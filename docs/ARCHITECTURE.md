# Architecture

Version 0.1.0. Decisions and their rationale live in `docs/decisions/`.

## 1. Layers

```
┌──────────────────────────────────────────────────────────────────────┐
│ Launched programs (nano, mc, htop, games, nmtui ...)  fixed argv      │
├──────────────────────────────────────────────────────────────────────┤
│ resilience TUI (Rust, ratatui)        unprivileged, per user          │
│   app.rs state machine → Effect { None | Quit | Run }                 │
│   ui.rs rendering   help.rs   theme.rs                                │
├──────────────────────────────────────────────────────────────────────┤
│ resilience-core (Rust library)                                        │
│   status  network  mode  catalog  config  handbook  notes  launch proc│
├──────────────────────────────────────────────────────────────────────┤
│ resilience-session (login shell) — restart on crash, refuse -c        │
├──────────────────────────────────────────────────────────────────────┤
│ NixOS 26.05: systemd, getty, logind, polkit, sudo, NetworkManager,    │
│ nix-daemon — configured by nix/modules (declarative, reproducible)    │
├──────────────────────────────────────────────────────────────────────┤
│ Linux kernel (LTS from nixpkgs), firmware (redistributable, ISO)      │
└──────────────────────────────────────────────────────────────────────┘
```

## 2. Boot and session sequence

1. Bootloader (GRUB on the ISO in text mode; systemd-boot planned for
   installed UEFI systems), with all previous generations listed.
2. Kernel at `loglevel=3`: errors still visible. systemd `show_status=auto`:
   silent when healthy, shows failing or slow units.
3. multi-user.target. No display manager. NetworkManager starts but sends
   nothing until an interface is configured; no connectivity check.
4. getty on tty1–6 with the Resilience banner. tty1 may autologin an
   Ultra-Stable account (live ISO); other ttys always prompt.
5. Login shell `resilience-session` → `resilience`.
6. The TUI shows system health from `systemctl is-system-running` and
   failed units on the main screen, so a degraded boot is never hidden.

## 3. Configuration flow

```
nix/modules/*.nix ──(nixos-rebuild / ISO build)──► /etc/resilience/system.toml
resilience.apps.<id> ─(build-time executable check)► /etc/resilience/apps.toml
handbook/*.md ───────► /run/current-system/sw/share/resilience/handbook
user preferences ────► ~/.config/resilience/settings.toml   (0600, theme only)
TUI log ─────────────► ~/.local/state/resilience/tui.log    (rotated at 1 MiB)
```
The TUI only *reads* system files. Anything that changes the system goes
through Nix (today: an administrator rebuilding; later: a typed privileged
helper — see §6).

## 4. Security modes

Mode = Unix group membership, computed at runtime (ADR 0003):

| | ultra-stable | stable | advanced |
|---|---|---|---|
| Groups | — | resilience-stable, networkmanager | + wheel |
| Shell in TUI | no | yes (bash, unprivileged) | yes |
| sudo | no | no | yes, password |
| Nix daemon | refused | yes | yes |
| Network config | no | nmtui | nmtui, nmcli |
| Restricted variants (htop --readonly, ncdu -rr, nano --restricted) | yes | no | no |

## 5. Networking state model

Passive: `/sys/class/net`, `getifaddrs`, `/proc/net/{route,ipv6_route,arp}`,
`/etc/resolv.conf` → OFFLINE / LOCAL LINK / ROUTE UP. Active probe on request
only, bounded by timeouts, skipped without a route (ADR 0004).

## 6. Planned components (not implemented)

| Component | Phase | Sketch |
|---|---|---|
| `resilience-privileged` | 2 | systemd service exposing typed operations over varlink/D-Bus (SetMode, ConnectWifi, SetTime, ImportPack, ApplyUpdate), polkit-authorised, audit-logged, every argument validated server-side |
| `resilience-resource-manager` | 3 | discover/verify (minisign)/install/remove TAR+zstd packs with manifests on external media |
| Kiwix integration | 3 | `kiwix-serve` on 127.0.0.1 + text browser, or a native ZIM reader |
| Crypto workflows | 4 | guided age/minisign/GnuPG operations, secrets via stdin/fd |
| Maps | 5 | MBTiles/GeoPackage rendering with block characters |
| Local messaging | 6 | evaluate Reticulum (rns, in nixpkgs) before writing anything |
| Installer | 8 | text installer, explicit disk confirmation, optional LUKS |
| Offline updates | 8 | `nix copy` closures + signed manifest on removable media |

## 7. Resource budget (to be measured — see ROADMAP)

Targets, not measurements: idle RAM < 200 MiB without optional services; TUI
RSS < 15 MiB; boot to TUI < 20 s on SSD; no periodic wakeups from our code
except the 250 ms input poll and a 5 s status refresh.
