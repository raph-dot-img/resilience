# Changelog

All notable changes are documented here. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
