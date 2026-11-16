# ADR 0017: Choosing the console keyboard layout

Status: accepted (2026-11-16).

## Context

Resilience OS is a general-purpose system for x86_64 PCs, and their
keyboards differ (US, UK, French AZERTY, German QWERTZ, ...). The owner
decided (2026-11-16): **US by default**, and each person can choose their
layout in **Settings**.

Constraints found while building it:

- The Linux console has **one keymap for all its consoles**. Changing it
  in one session changes it on every console, and it stays after logout.
- The kernel lets only processes with `CAP_SYS_TTY_CONFIG` change the
  keymap: `loadkeys` run by an ordinary user fails with "Keymap 0:
  Permission denied" (observed in vm-boot-offline). This is a real
  boundary, because a keymap can also bind whole strings to keys: an
  arbitrary keymap could make a key type a command line into the next
  person's session.
- A keymap left behind by one person is dangerous for the next one: a
  password typed with the wrong layout fails, and pam_faillock then locks
  the account.
- The live ISO has no administrator account (ADR 0007), and Ultra-Stable
  users may have none at hand, yet both need to type with their own
  keyboard.

## Decision

1. **System layout.** `resilience.keyboard.layout` (default `"us"`) sets
   `console.keyMap`: used at boot, by every login prompt and by accounts
   that never chose a layout.
2. **Closed list.** `resilience.keyboard.layouts` (13 common layouts by
   default, each with a label) is what Settings offers. The build fails if
   any name, or the system layout, is not a kbd keymap.
3. **A new helper operation, `resilience-admin set-keymap LAYOUT`**, under
   the rules of ADR 0007 (typed operation, own polkit action, argument
   validated in the helper, tool path from root-owned `admin.toml`,
   audited). It accepts only a valid name listed in `admin.toml`
   (`keymaps`) and runs `loadkeys -q -- NAME` with that fixed argv. It
   does not persist anything system-wide.
4. **Authorisation: no password for the person at the console.** The
   polkit action `org.resilience.admin.set-keymap` is `allow_active=yes`,
   `allow_inactive=no`, `allow_any=no`. **This is the only Resilience
   action without `auth_admin`**, an exception to CLAUDE.md §9 accepted by
   the owner (2026-11-16) because: only listed, standard layouts can be
   loaded (no string bindings of the user's choosing); the effect is
   temporary (see 5); nothing is persisted outside the user's own
   settings; every call is audited; and the alternative (an administrator
   password) would make the layout impossible to change on the live ISO.
5. **Reset before every login prompt.** A drop-in on `getty@.service`
   (`ExecStartPre=-loadkeys -q -- <system layout>`, as root) loads the
   system layout again whenever a login prompt starts, so nobody types a
   password with another person's layout.
6. **Per-account choice.** Settings → "Keyboard  <layout>" calls
   `pkexec <helper> set-keymap ID` (SHELL set to the session shell) and,
   on success, saves `keyboard = "ID"` in the user's settings file. At the
   start of each session the TUI loads the saved layout again with the
   same call, unless it is the system layout; a saved layout no longer
   offered falls back to the system layout with a warning.

## Consequences

- Layouts work on the live ISO and for Ultra-Stable accounts, without an
  administrator.
- Limitation (kernel): when several people use different consoles at the
  same time, they share one keymap; the last change wins, and a login
  prompt starting on any console resets it to the system layout. The TUI
  loads its user's layout again only at the start of a session.
- Someone at the console can change the keymap for others logged in on
  other consoles, among the listed layouts only. This is an annoyance,
  not an escalation, and it is audited.
- VM tests type US key codes; the system default being US, no test
  override is needed any more. smoke.py keeps `--keymap fr|us`.

## Tested

vm-boot-offline: polkit implicit authorisations (`set-keymap` active=yes,
any=no; `set-mode` still auth_admin); an unlisted name refused and
audited; French chosen in Settings without a password changes the kernel
keymap and is audited with the caller; restarting a getty puts US back;
the next session of the same account loads French again. Unit tests: name
validation, `plan_set_keymap`, settings round trip, the Settings screen
and the session-start reload (TUI tests with a fake pkexec).
