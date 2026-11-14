# ADR 0002 — Rust TUI with ratatui, synchronous design

Status: accepted (2026-11-14)

## Decision
- `ratatui` 0.30 with its re-exported crossterm backend.
- No async runtime. The interface polls input every 250 ms; slow work
  (network probe) runs on a worker thread reporting through an mpsc channel;
  every external command has a hard timeout (`resilience_core::proc`).
- Split: `resilience-core` (testable logic, no UI) and `resilience-tui`.
  The TUI state machine returns `Effect`s (`None`, `Quit`, `Run`) so that
  only `main.rs` touches the terminal, and all navigation is unit-testable
  with ratatui's `TestBackend`.
- Programs are launched with a fixed argv via `execve` semantics, never a
  shell. While a child runs the TUI ignores SIGINT/SIGQUIT/SIGTSTP.
- Colours limited to the 8 ANSI colours; every state also has a text marker.

## Alternatives considered
- tokio: unnecessary complexity and binary size for a handful of timeouts.
- cursive / ncurses bindings: C dependency, less control over rendering.
- A shell-script dialog UI (whiptail): fragile quoting, hard to test.

## Consequences
Binary ≈ 1.5 MiB stripped; ~108 crates in Cargo.lock (mostly ratatui's
dependency tree) — reviewed with `cargo deny` before releases.
