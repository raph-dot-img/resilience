# Changelog

All notable changes are documented here. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- `resilience-admin`, the privileged helper (ADR 0007): `list`, `set-mode`,
  `apply-state`; polkit action `org.resilience.admin.set-mode` (auth_admin),
  server-side validation, persistent overrides re-applied after activation,
  audit log `/var/log/resilience/audit.log` + journal.
- TUI: *Users & security modes* (view all accounts, change a mode with
  consequences, confirmation and administrator password) and *Recovery &
  diagnostics* (health, failures, recent errors, generations, diagnostic
  report export, emergency guide).
- VM test `vm-admin`; security invariants for polkit admins, faillock and the
  helper configuration.
- *Backups & USB sticks* (ADR 0008): USB sticks and memory cards for every
  account through udisks2 (removable drives only, open/eject from the TUI);
  `resilience-backup create|check|restore`: the home folder as an
  age-encrypted `tar.zst` with a `sha256sum` file, checked without the
  passphrase, restored into a new `~/Restored/backup-<date>` folder.
  Handbook page "Backups and USB sticks"; VM test `vm-backup` (hot-plugged
  virtual stick); invariants for udisks2 polkit and mount location.
- *Date & time* (ADR 0009): set the clock from the TUI in every mode with an
  administrator's password; `resilience-admin set-time` (polkit action
  `org.resilience.admin.set-time`, auth_admin), strict UTC and bounds
  2025–2099 checked in the helper, audited with the previous time; timedated
  updates the hardware clock. Handbook section "The clock"; vm-admin covers
  refusals, wrong and right passwords, two administrators, the RTC.
- `/etc/resilience/system.toml` gained `[tools]` (absolute paths, checked at
  build time).

### Security
- PAM faillock on login, su, sudo and polkit-1 (5 failures / 15 min → 5 min).
- Demoting an account terminates its sessions.

### Fixed
- The security invariant "autologin account is not privileged" read
  `services.getty.autologinUser`, which is unused since autologin moved to the
  tty1 drop-in, so it could never fail; it now checks
  `resilience.interface.autologinUser`, and a new invariant keeps the getty
  option unset.
- pkexec could not complete a successful authentication on a text console
  (socket-activated polkit helper, upstream polkit issue 17); the TUI now
  registers `pkttyagent` for itself before running pkexec.

## [0.1.0] — 2026-11-14 — Phase 0 + Phase 1 prototype

### Security
- Root password explicitly locked; evaluation-time security invariants.
- Autologin restricted to tty1 (NixOS's `services.getty.autologinUser`
  would apply to every console).

### Fixed (found by booting the real ISO / VMs)
- Console font `ter-v16n` lacked the glyphs used by the interface; now
  `ter-i16n`, with a test that only console-font glyphs are drawn.
- Crashed interface left the terminal in raw mode; the session wrapper now
  resets the terminal before showing its diagnostics.

### Added
- Nix flake pinned to nixos-26.05 (nixpkgs `f1e5c356`, 2026-11-13).
- NixOS modules: core (catalog/system config generation with build-time
  verification), users and security modes, interface/session, security
  baseline, offline-first networking, boot, power, application groups.
- Editions: minimal, standard. Live ISO configurations and a development VM.
- `resilience-core` and `resilience` TUI (Rust, ratatui 0.30).
- `resilience-session` login shell with crash recovery.
- Offline handbook (first steps, security modes, offline/network, recovery).
- VM tests: offline boot and interface, security modes and local link.
- Project documentation and ADRs 0001–0006.
