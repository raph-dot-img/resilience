# Security policy

## Reporting

Resilience OS is an early prototype with no public release yet. Report
suspected vulnerabilities privately to the project maintainer; do not open a
public issue. Include the commit, steps to reproduce and impact.

## Supported versions

None yet. v0.1.0 is a development prototype and must not be relied on for
safety-critical use.

## Security principles (summary)

- Modes are OS privileges (groups, sudo, polkit, Nix daemon), never UI flags.
  See docs/decisions/0003-security-modes.md.
- The interface never builds shell commands and never runs privileged code.
  The only privileged entry point is `resilience-admin`, with fixed typed
  operations, polkit `auth_admin` (administrator password every time),
  server-side validation and an audit log (docs/decisions/0007-privileged-helper.md).
- No background network traffic; no listening services by default.
- No custom cryptography: only established tools (age, minisign, GnuPG,
  cryptsetup, OpenSSL).
- Root is locked; administration only through sudo with a password.
  Password guessing is rate-limited by pam_faillock.
- Invariants are checked on every `nix flake check`
  (tests/eval/security-invariants.nix).

## Handling of secrets

| Secret | Rule |
|---|---|
| Passwords | Set interactively (`passwd`) or via `hashedPasswordFile` outside the Nix store. Never in Nix expressions, Git or logs. |
| Private keys (age, GnuPG, minisign) | Stored in the user's home with mode 0600 (future tools enforce it); never logged; never passed in argv. |
| Passphrases | Read from the terminal or passed through stdin/fd to the tool, never as command-line arguments. |
| Temporary files | Created with `O_EXCL` and mode 0600 in a private directory; removed after use. |
| Logs | `~/.local/state/resilience/tui.log` contains navigation events and program names only. |

The development VM configuration contains public test passwords by design;
it must never be used for a real system.

## Known limitations

See docs/THREAT_MODEL.md §5. In particular: physical access defeats the mode
boundaries without full-disk encryption (not yet available), and there has
been no independent security review.
