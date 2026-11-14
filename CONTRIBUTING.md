# Contributing to Resilience OS

Read [CLAUDE.md](CLAUDE.md) first: it is the authoritative description of
the current architecture, conventions and constraints.

## Workflow

1. `git switch -c feat/<topic>` from `main`.
2. Make a small, working vertical slice.
3. Run `cargo fmt && cargo clippy --all-targets && cargo test` and
   `nix fmt && nix flake check --no-build`. Run the relevant VM test
   (`nix build .#checks.x86_64-linux.<name> -L`) for system changes.
4. Update docs (TESTING.md results, CHANGELOG.md, CLAUDE.md if conventions or
   status changed; an ADR for significant decisions).
5. Commit with a conventional message (`feat:`, `fix:`, `security:`, `test:`,
   `docs:`, `chore:`, `refactor:`).

Never commit ISOs, disk images, `result*` links, large archives, keys or
secrets. Never weaken or delete a test to make a change pass.

## Adding an application

Follow CLAUDE.md §13: verify upstream, license, maintenance and presence in
the pinned nixpkgs; add it to `nix/modules/applications/*.nix` with a
beginner description saying how to quit; pick `minMode` conservatively;
record it in docs/COMPONENTS.md.

## Code standards

- Rust: no `unwrap()` outside tests, `// SAFETY:` on every `unsafe`, a pure
  parser + unit test for every system reader, a timeout on anything external,
  a `tests.rs` case for every new TUI flow.
- Nix: one concern per module, options under `resilience.*` with
  descriptions, no `nixpkgs.overlays` inside modules, no secrets ever.
- Interface: works at 80×24, in Monochrome, keyboard-only; only glyphs from
  the console font (enforced by a test).

## Release process (draft)

`nix flake check -L --max-jobs 1` → build both ISOs → `tests/iso/smoke.py`
(BIOS + UEFI) → `cargo deny check` → update CHANGELOG/TESTING → tag
`vX.Y.Z` → publish ISO with SHA-256 and a minisign signature.
