# ADR 0003 — Security modes are per-account group membership

Status: accepted (2026-11-14); mode-change tooling pending (Phase 2)

## Context
The spec requires three modes (Ultra-Stable, Stable, Advanced) that differ
in *real* privileges, never just in menu visibility.

## Decision
A mode is a property of a Unix account, expressed as groups that the kernel,
sudo, polkit and the Nix daemon already enforce:

| Mode | Groups | Shell | sudo | Nix daemon | NetworkManager |
|---|---|---|---|---|---|
| ultra-stable | (none) | `resilience-session` (TUI only) | no | **refused** | read-only |
| stable | resilience-stable, networkmanager | TUI; bash on request | no | yes (unprivileged) | yes |
| advanced | + wheel | TUI; bash on request | yes, password | yes | yes |

- Declared with `resilience.users.<name>.mode` (default `ultra-stable`).
  Privilege-granting groups in `extraGroups` are rejected by an assertion.
- The TUI computes the mode from `getgroups()` + `/etc/group`; it can never
  show more capability than the account actually has. `wheel` always means
  Advanced. Unreadable data → Ultra-Stable (fail-safe).
- The login shell for all Resilience accounts is `resilience-session`, which
  refuses `-c` invocations. For Ultra-Stable accounts `$SHELL` is `nologin`,
  so programs' shell escapes fail. **This is UX hardening, not a boundary**:
  the boundary is the absence of privileges.
- Root is locked; administration is only via sudo by Advanced accounts.

The spec's suggested `nix/profiles/{ultra-stable,stable,advanced}.nix` (system
-wide modes) was rejected: a household or school machine needs different
modes for different people at once, and a system-wide mode cannot be both
"beginner-safe" and "administrable".

## Mode switching (current and planned)
- Now: an administrator edits `resilience.users.<name>.mode` and rebuilds.
  This is authorised (root), persistent and recorded in the system generation.
- Phase 2: `resilience-admin` privileged helper (D-Bus/varlink service,
  polkit `auth_admin`) exposing a typed `SetMode(user, mode)` operation,
  with an append-only audit log and a re-login requirement.

## Consequences
+ Modes are testable as OS facts (tests/vm/modes.nix).
− With `users.mutableUsers = true` (needed so users can `passwd`), group
  changes made outside Nix may be reverted at activation — intended.
