# ADR 0001 — NixOS 26.05 stable, flakes, a single pinned input

Status: accepted (2026-11-14)

## Context
Resilience must be reproducible, rollback-capable and maintainable offline.
The specification mandates NixOS, flakes, a committed lock file and a stable
branch for production.

## Decision
- Base: `github:NixOS/nixpkgs/nixos-26.05` (latest stable branch at the time;
  verified with `git ls-remote`). Locked in `flake.lock` (rev `f1e5c356…`, 2026-11-13).
- The flake has **one input** (nixpkgs). No flake-utils, crane, home-manager:
  every extra input is another supply-chain and maintenance dependency.
- Rust is built with nixpkgs' `rustPlatform.buildRustPackage` and
  `cargoLock.lockFile`, so Cargo dependencies are pinned by hash.
- Upgrades (e.g. to 26.11) are evaluated on a branch, tested with the full
  VM test suite, then merged and recorded in CHANGELOG.md.

## Consequences
+ Reproducible builds, generations and rollback for free.
+ Offline updates can reuse `nix copy` closures (Phase 8).
− nixpkgs' rustc may lag the host toolchain; code must compile on both
  (rust-version 1.85 minimum declared).
