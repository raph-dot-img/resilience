# ADR 0012: User namespaces on Ultra-Stable-only machines

Status: accepted (2026-11-14).

## Context

Unprivileged user namespaces let any process become "root" inside its own
namespace. They are needed by the Nix build sandbox, rootless containers and
some sandboxing tools. They also expose kernel code that used to be
reachable only by root (network stack configuration, mounts, many file
systems), and they have been a common step in kernel privilege-escalation
exploits.

The roadmap asked us to review `unprivileged_userns_clone` for
Ultra-Stable-only machines. That sysctl is a Debian and `linux-hardened`
patch. **It does not exist in the mainline kernel NixOS 26.05 uses**
(6.18). Mainline has one switch: `user.max_user_namespaces`, a limit per
user that applies to everyone, root included. `0` disables user namespaces
entirely. NixOS exposes this as `security.allowUserNamespaces = false`, and
asserts that the Nix sandbox is then off too, because it needs them.

Who needs user namespaces on Resilience:

- **Stable and Advanced accounts** use the Nix daemon, whose build sandbox
  needs them. They also have a shell, where tools may need them.
- **Ultra-Stable accounts** have no shell and are refused by the Nix
  daemon. Nothing in the catalog uses user namespaces.
- **System services**: only units with `PrivateUsers=` need them. In the
  pinned systemd (260.5) that is only the unused "portable" profiles. NixOS
  sets `PrivateUsers=false` for wpa_supplicant. Other sandboxing settings
  of system services (`PrivateTmp=`, `ProtectSystem=`…) do not need user
  namespaces, because the system manager is root.

## Decision

A new option, `resilience.security.userNamespaces` (`nix/modules/security.nix`):

- **Default `false`** when every account in `resilience.users` is
  Ultra-Stable and no other login account exists. The live system is such
  a machine. The kernel then refuses user namespaces to everyone
  (`user.max_user_namespaces = 0`), and the Nix sandbox is off. Nobody can
  build there: there is no administrator, root is locked, and Ultra-Stable
  accounts are refused by the daemon.
- **Default `true`** otherwise. This is the kernel and NixOS default, kept
  for the Nix sandbox and Stable users.
- An administrator can override it either way in the configuration.

A security invariant checks that an Ultra-Stable-only configuration has
`user.max_user_namespaces = 0` and `ultra_stable_only` in the helper's
configuration. `vm-boot-offline` (an Ultra-Stable-only
machine) checks that `unshare --user` fails for the account and for root,
and that no unit fails. `vm-modes` checks that a machine with Stable
accounts keeps user namespaces.

A mode change cannot widen this at runtime. `admin.toml` carries
`ultra_stable_only = true` on such machines, and then:

- `set-mode` refuses to raise any account above Ultra-Stable;
- `apply-state`, at every boot and switch, drops earlier overrides that
  raised an account, with an audit entry.

Without this, an account promoted at runtime (overrides survive while its
own declared mode is unchanged, ADR 0007) would keep `wheel` or the Nix
daemon on a machine whose configuration later became Ultra-Stable-only, and
it would then build without the sandbox. An independent review found that
case.

Setting the option to `false` while a Stable or Advanced account is
declared is an evaluation error. It would silently remove the sandbox for
those accounts. After a configuration change adds a Stable or Advanced
account, the next build turns user namespaces back on.

## Alternatives considered

- **Disable user namespaces everywhere:** this breaks the Nix sandbox. Then
  Stable users' builds would run unsandboxed, which is worse.
- **Block them only for Ultra-Stable sessions** with a seccomp filter
  installed by the session wrapper: processes started through the user's
  systemd instance (`systemd-run --user`, D-Bus activation) would not
  inherit the filter, unless `user@.service` were restricted too, which
  needs per-UID configuration and has to follow mode changes. It would also
  mean writing and maintaining BPF filter code. Rejected for now; it is the
  path to look at if Ultra-Stable accounts on mixed machines need this
  protection.
- **AppArmor's `apparmor_restrict_unprivileged_userns`** (as in Ubuntu):
  this needs AppArmor enabled and profiles for every program that needs
  user namespaces. Too heavy for this one setting.
- **Switch to `linux-hardened`:** that kernel lags behind mainline releases
  and changes more than this one setting. That is a separate decision.

## Consequences

- On the live system and on Ultra-Stable-only installs, a whole class of
  kernel attack surface is closed for every process, including root.
- On those machines the Nix sandbox is off. A later offline update applied
  there (Phase 8) has to be built elsewhere and copied, not built locally.
  That is the planned design anyway.
- On mixed machines, Ultra-Stable accounts can still create user
  namespaces, as on most Linux distributions. This is listed as a known
  gap.
- Root can raise the limit at runtime (`sysctl`). This is not a boundary
  against root.
