# ADR 0011: Making an earlier system generation the default

Status: accepted (2026-11-14).

## Context

Every system change on NixOS creates a new *generation*. The boot menu lists
all of them. Until now, Resilience explained how to start an older generation
**once** from the boot menu. The next restart went back to the newest one.
When a new generation is broken in a way that only shows after a while (a
driver, a service, a setting), the user has to choose the old entry at every
start. A beginner may not manage that, and the machine may not even show the
menu (timeout of 5 seconds).

NixOS does this from the command line: `nixos-rebuild --rollback boot`, or
`nix-env -p /nix/var/nix/profiles/system --switch-generation N` followed by
`/nix/var/nix/profiles/system/bin/switch-to-configuration boot`. Both need
root and the knowledge of the commands.

## Decision

A new typed operation of the privileged helper (ADR 0007):

```
resilience-admin set-default-generation N
```

with its own polkit action `org.resilience.admin.set-default-generation`
(`auth_admin` for any, inactive and active sessions: an administrator's
password every time, never `_keep`).

The helper, as root, with `/run/resilience-admin.lock` held:

1. Reads the generations from `/nix/var/nix/profiles` (the same pure code
   the TUI uses, `recovery::generations`).
2. Checks the request with the pure rule `recovery::plan_set_default`:
   - `N` is decimal digits only, no sign, no leading zero, at most 9 digits;
   - generation `N` exists and is not already the default;
   - its target is a direct child of `/nix/store` and contains
     `bin/switch-to-configuration`.
   A refusal is audited and exits with status 2.
3. Runs `nix-env --profile /nix/var/nix/profiles/system --switch-generation N`
   (absolute path from the root-owned `admin.toml`).
4. Runs **that generation's own** `bin/switch-to-configuration boot`, which
   writes the boot menu with `N` as the default. `boot`, never `switch`:
   the running system is not changed, so nothing restarts under the user.
5. If step 4 fails, switches the profile back to the previous default and
   says so, but only if the profile still points to `N`. A `nixos-rebuild`
   running at the same time is never undone. If the profile no longer
   points to `N` after step 4, the helper reports an error instead of
   success. Every outcome is audited (`op=set-default-generation caller=…
   from=… to=… result=ok|refused|error`).

Both programs run with a **cleared environment** (only `PATH` set to the
target's `sw/bin`, and `HOME=/root`), with stdin closed and their output
shown on the terminal (`proc::run_visible`). Time limits are 60 s and
300 s. Each program runs in its own process group, and on timeout the whole
group is killed, including a boot-loader installer it started.

Once a request is validated, the helper ignores Ctrl-C, Ctrl-\\, Ctrl-Z and
hang-up (as do the programs it starts), so a change is never left half-done
or unaudited. This applies to `set-mode` and `set-time` too. A request can
still be cancelled at the password prompt, and the TUI's banner says so.

These protections were added after an independent review of the first
version.

In the TUI, *Recovery & diagnostics → System generations* becomes a list
(number, date, state) with details. Enter on a generation:

- the current default, or an incomplete generation: an explanation only;
- no administrator account on the machine: an explanation only;
- otherwise a confirmation (Cancel focused) that says it changes system
  software and settings only, never personal files, and nothing changes
  until the next restart. Then `pkexec <helper> set-default-generation N`
  with `pkttyagent`, as for the other helper operations.

Every account can ask (it is a recovery tool). The administrator's password
is the authorisation.

## Alternatives considered

- **`nixos-rebuild --rollback boot`**: it only goes one generation back, and
  it needs `nixos-rebuild` and its dependencies in the helper's path.
- **Writing the symlink ourselves** instead of `nix-env`: little code, but
  `nix-env` is what NixOS itself uses and keeps Nix's own conventions.
- **`switch-to-configuration switch`** (change the running system too): it
  restarts services under the user, possibly including the interface, and
  an old generation may not switch cleanly. A restart is the predictable
  path.
- **Editing the boot loader's default only** (`bootctl set-default`, GRUB
  `default`): the profile would still point to the newest generation, so
  the next system change would bring it back unexpectedly.

## Consequences

- An administrator, or anyone with the administrator's password at hand,
  can make any kept generation permanent from the interface, and undo it
  the same way (newer generations stay in the boot menu).
- **An older generation also brings back older software and older system
  settings**, including fixed security problems and the declared modes of
  that time. This is the administrator's informed choice: the confirmation
  says so. Mode changes made with the helper stay recorded in
  `/var/lib/resilience/modes.toml` and are re-applied at boot. When the
  declared mode of an account differs in the older generation, the existing
  rule applies (ADR 0007): the stale override is dropped and audited.
- The next `nixos-rebuild` creates a new generation after the newest one, as
  usual. Deleting generations (garbage collection) is not offered.
- The helper runs code from the chosen generation as root
  (`switch-to-configuration`). That code is part of a system closure that
  root put in the system profile, so it is trusted to the same degree as the
  generation itself, which would run as root at boot anyway.
- Not tested: Ctrl-C during the operation, a timeout of the boot-loader
  installer, a concurrent `nixos-rebuild`. The process-group kill is
  covered by a unit test.
- Tested with systemd-boot on UEFI only. GRUB (BIOS) uses the same
  `switch-to-configuration boot` path but is not tested.

Tested by: `tests/vm/generations.nix` (two real generations, refusals,
polkit, a wrong and the right administrator password through the TUI, the
running system unchanged, a reboot into the chosen generation, and back),
unit tests in `recovery.rs` and `proc.rs`, and TUI tests.
