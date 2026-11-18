# Security policy

## Reporting

Resilience OS is an early prototype with no public release yet. Report
suspected vulnerabilities privately to the project maintainer; do not open a
public issue. Include the commit, steps to reproduce and impact.

## Supported versions

None yet. v0.3.0 is a development prototype and must not be relied on for
safety-critical use.

## Security principles (summary)

- Modes are OS privileges (groups, sudo, polkit, Nix daemon), never UI flags.
  See docs/decisions/0003-security-modes.md.
- The interface never builds shell commands and never runs privileged code.
  The only privileged entry point is `resilience-admin`, with fixed typed
  operations, polkit `auth_admin` (administrator password every time),
  server-side validation and an audit log (docs/decisions/0007-privileged-helper.md).
  One exception: loading one of the system's listed keyboard layouts
  (`set-keymap`) needs no password for the person at the console
  (docs/decisions/0017-keyboard-layout.md).
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
| Private keys (age, minisign) | In `~/.local/share/resilience/keys` (0700), files 0600, **always encrypted with the user's passphrase**: the age key goes from `age-keygen` straight into `age --passphrase` through a pipe, and minisign encrypts its own key. Never logged, never in argv; the interface never replaces or deletes them (ADR 0015). Included in the user's encrypted backups. |
| Passphrases | Read from the terminal by the tool itself (age, minisign; later pinentry for GnuPG), never typed into the interface, never in argv, environment, logs or files. Resilience code never sees them (backups ADR 0008, cryptography ADR 0015). The Wi-Fi password is the one exception: typed in the TUI (masked) and given to nmcli on stdin (ADR 0010). |
| Decrypted data | Files go to `~/Decrypted` (0700, files 0600), never over an existing file; partial output is deleted on failure. Decrypted messages are kept in memory and shown in `less` (secure mode), never written. |
| Generated passwords | Shown once in a dialog; not stored, not logged, not in the notification history. |
| Temporary files | Created with `O_EXCL` and mode 0600 in a private directory; removed after use. |
| Logs | `~/.local/state/resilience/tui.log` contains navigation events and program names only. |

The development VM configuration contains public test passwords by design;
it must never be used for a real system.

## Known limitations

See docs/THREAT_MODEL.md §5. In particular: physical access defeats the mode
boundaries without full-disk encryption (not yet available), and there has
been no independent security review.
