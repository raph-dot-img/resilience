# Changelog

All notable changes are documented here. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed
- Resilience's own code is now licensed 0BSD (was GPL-3.0-or-later);
  `LICENSE` added. Bundled software keeps its own license.

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
- *Wi-Fi networks* (ADR 0010): list, join (password typed in the TUI and
  passed to nmcli on stdin), reconnect, disconnect and forget Wi-Fi
  networks in every mode; profiles private to the account; open-network
  warning; WEP and 802.1X not offered. VM test `vm-wifi` (simulated radios).
- *System generations* (ADR 0011): the Recovery list of generations now
  makes one the default for every boot, with an administrator's password;
  `resilience-admin set-default-generation N` (polkit action
  `org.resilience.admin.set-default-generation`, auth_admin) switches the
  system profile with nix-env and runs that generation's
  `switch-to-configuration boot`, undoing the profile switch on failure;
  audited. VM test `vm-generations` (systemd-boot, UEFI, real reboots).
- Option `resilience.security.userNamespaces` (ADR 0012); `admin.toml`
  `ultra_stable_only`: on Ultra-Stable-only machines `set-mode` refuses to
  raise an account and `apply-state` drops earlier raising overrides.
- The helper ignores Ctrl-C, Ctrl-Z and hang-up once a request is
  validated, so a change is never left half-done or unaudited.
- `/etc/resilience/system.toml` gained `[tools]` (absolute paths, checked at
  build time).

### Security
- User namespaces are refused to every process on Ultra-Stable-only
  machines, including the live system (`user.max_user_namespaces = 0`; the
  Nix sandbox is off there, nobody can build). Kept elsewhere (ADR 0012).
- NetworkManager's polkit defaults let any active console session switch
  networking off, share a hotspot or enable connectivity checks without a
  password, including Ultra-Stable accounts. An allowlist now limits accounts
  without the networkmanager group to scanning, connecting, their own
  profiles and the Wi-Fi radio (ADR 0010). The earlier vm-modes PASS for
  "cannot change networking" had only been tested without a session.
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
