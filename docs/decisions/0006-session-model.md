# ADR 0006 — Session model

Status: accepted (2026-11-14)

## Decision
- getty on tty1–tty6. tty1 may autologin **only** an Ultra-Stable account
  (assertion), e.g. the live ISO's locked-password `resilience` user.
  `services.getty.autologinUser` is deliberately **not** used: NixOS applies
  it to every getty (tty1–6 and serial consoles), which would remove the
  recovery login prompts. A `getty@tty1` drop-in adds `--autologin` to tty1
  only (found while testing the dev VM; covered by vm-boot-offline).
- Login shell `resilience-session` (bash script, `nounset`/`pipefail`, not `-e`):
  refuses arguments, requires a TTY, sets `$SHELL` by mode, runs the TUI and
  on a non-zero exit shows a diagnostic screen and restarts after Enter, with
  a 30 s back-off after three crashes within 10 s each. Exit 0 = logout.
- The TUI logs to `$XDG_STATE_HOME/resilience/tui.log` (rotated at 1 MiB),
  never to the terminal.

## Consequences
If the TUI is broken, other ttys still offer a login prompt; booting an
earlier generation remains possible from the boot menu.
