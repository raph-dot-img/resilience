# ADR 0007 — Privileged helper, mode changes, recovery access

Status: accepted (2026-11-14). Supersedes the "Phase 2 plan" section of ADR 0003.

## Context

Phase 2 needs administrative actions from the TUI, starting with changing
an account's security mode, without a general privileged command runner
(ORIGINAL-SPEC §10, §17). Constraints found on the pinned NixOS 26.05:

- `users.mutableUsers = true` and `update-users-groups.pl` **reset the group
  membership of every declared user** to the declaration on each activation
  (boot and switch). A plain `gpasswd` change would be reverted.
- polkit 127 uses a socket-activated `polkit-agent-helper`. With pkexec's
  built-in text agent, a *successful* authentication fails with "No session
  for cookie": the helper reports pkexec's effective uid (0), but polkitd
  expects the calling user (upstream polkit issue 17). Wrong passwords fail
  as they should, so this is only visible when testing the success path.
- pkexec refuses to start if `$SHELL` is not in `/etc/shells`;
  Ultra-Stable sessions set `$SHELL` to nologin on purpose.
- Root is locked, so systemd's emergency and rescue shells are unusable.

## Decision

### One helper binary with typed operations

`resilience-admin` (crate `crates/resilience-admin`, installed with the TUI
package) accepts exactly:

| Command | Who | Effect |
|---|---|---|
| `list` | anyone | read-only table of managed accounts |
| `set-mode USER MODE` | root (via pkexec or sudo) | change one account's mode |
| `apply-state` | root (system activation) | re-apply recorded modes |

Any other argv is a usage error (exit 64). The helper never runs a shell and
only ever executes `gpasswd` and `loginctl`, through absolute paths read from
the root-owned, Nix-generated `/etc/resilience/admin.toml`. It ignores the
environment, apart from pkexec's `PKEXEC_UID` and sudo's `SUDO_UID`, which it
reads only to record who asked. All checks run in the helper, whatever the
TUI already checked (`admin::plan_set_mode`, which is pure and unit-tested):

- strict user-name syntax, and only accounts declared in `resilience.users`;
- exact mode names;
- the autologin account must stay Ultra-Stable;
- the last Advanced account cannot be demoted;
- no-op changes are refused.

Changes only ever touch the three mode groups (`resilience-stable`,
`networkmanager`, `wheel`).

### Authorisation: polkit `auth_admin`, every time

The action `org.resilience.admin.set-mode` matches the helper's store path
(`exec.path`) and `exec.argv1 = set-mode`. All three defaults are
`auth_admin`, never `auth_admin_keep`. polkit administrators are exactly
`unix-group:wheel`, i.e. Advanced accounts. An Ultra-Stable user at the TUI
can therefore *request* a change, and an administrator authorises it by
typing their password.

The TUI runs this fixed argv:

```
pkexec <helper> set-mode <user> <mode>
```

It sets `SHELL` to the Resilience login shell, which is in `/etc/shells` and
refuses `-c`. It first starts polkit's own text agent,
`pkttyagent --process <tui pid> --notify-fd N --fallback`, which runs as the
user, waits until the agent has registered, and stops it afterwards
(`launch::run_with_polkit_agent`). No password passes through our code.

### Persistence: overrides recorded with the declaration they override

`/var/lib/resilience/modes.toml` (root, 0644) stores per account the new
mode, the declared mode at the time of the change, the time, and the caller.
An activation script that runs after `users`, `groups` and `etc` calls
`apply-state`, which re-adds the recorded groups. An override whose recorded
declaration differs from the current one is **dropped**, because the
administrator edited the Nix configuration later. The most recent explicit
change wins, and the drop is audited. Going back to the declared mode
removes the override. A corrupt state file makes the helper refuse to change
anything, and the declared modes stay in effect.

### Sessions

- **Promotion** applies at the account's next login.
- **Demotion** ends all of the account's sessions (`loginctl terminate-user`)
  as the last step, so no session keeps rights that were removed. The TUI's
  confirmation says so, including when it is the user's own session.

### Audit

Every `set-mode` (ok, refused or error) and every `apply-state` change is
appended to `/var/log/resilience/audit.log`, one logfmt line per event, with
free text quoted and control characters removed. The log is root:wheel 0640
in a 0750 directory; each entry is also sent to the journal (authpriv,
identifier `resilience-admin`). Denied authorisations are logged by polkit.

### Password guessing

`pam_faillock` (preauth / authfail / account) is applied to login, su, sudo
and polkit-1: 5 failures within 15 minutes lock the account for 5 minutes.
Tallies live in /run and reset at reboot. Administrators can inspect or lift
a lock with `faillock --user NAME [--reset]`.

### Recovery access with root locked (THREAT_MODEL T18)

No password-less emergency shell and no recovery root password. Recovery is
layered, and each layer is documented in the handbook:

1. The TUI's **Recovery** section, available to every account and read-only:
   - health and failed units;
   - recent errors (all of them for wheel members, otherwise only the
     account's own);
   - the list of generations, with boot-menu instructions;
   - a diagnostic report saved to `~/Diagnostics` (0600; no addresses or
     file names);
   - the emergency guide.
2. Other consoles (Ctrl-Alt-F2…F6) always offer a normal login; an Advanced
   account has sudo.
3. Earlier generations in the boot menu. System rollback never touches user
   data.
4. The live ISO, as rescue media.

Installed systems must therefore have at least one Advanced account; the
installer (Phase 8) will require one. A physical attacker can already boot
other media (T17), so a recovery root password would add attack surface for
little benefit.

## Consequences

- One small, auditable privileged surface. Wi-Fi, time and pack import will
  be added as further typed sub-commands, each with its own polkit action.
- pkexec is setuid (part of NixOS's polkit). The generic
  `org.freedesktop.policykit.exec` action remains `auth_admin`, so a user
  who knows an administrator password could run any program as root. That is
  equivalent to that administrator's own sudo.
- An administrator typing their password into another user's session must
  trust that session. A Stable user's programs could fake the prompt. The
  handbook tells administrators to authorise only on a screen they trust;
  Ultra-Stable sessions cannot run arbitrary programs, which reduces this
  risk.
- Selecting a generation as the *permanent* default (`switch-to-configuration
  boot` for an older generation) is not implemented yet; the boot menu is the
  supported path.

Tested by: `tests/vm/admin.nix`, unit tests in `admin.rs` and `launch.rs`,
TUI tests, and `security-invariants`.
