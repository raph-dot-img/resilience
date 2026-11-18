# ADR 0024: The licence is shown and its reading recorded at first use

Status: accepted (2026-11-18).

## Context

Resilience's own code is under the Hippocratic License 2.1 (`LICENSE`,
owner's decision 2026-11-16). The owner asked (2026-11-18) that every
person sees a plain-language summary and the original text of the
licence the first time they log in, and at every start of the live ISO;
that they certify they have read it; and that a timestamped record of it
is kept on the system.

Facts that shape the design:

- The licence states its own acceptance: "Licensee accepts this License
  and agrees to its terms and conditions by taking any action with the
  Software that, absent this License, would infringe". A screen does not
  change that; what it adds is proof that the text was shown and the
  person said they read it.
- The licence covers Resilience's own programs only. The system also
  carries other free software under its own licences (GPL and others).
  The GPL (v2 §6, v3 §10) forbids adding conditions on running those
  programs, so the screen must not present the Hippocratic License as a
  condition for using the whole computer.
- Offline computers may have a wrong clock (ADR 0009): a timestamp is
  the system clock's value, not proof of the real time.
- An account must not be able to forge or erase its own record, so the
  record cannot live in the account's home folder.

## Decision

1. **The licence screen** opens before the main menu when the account
   has no record for the licence text the system carries. It shows a
   summary (what the licence allows, its conditions, that the original
   text prevails, that it covers Resilience's own programs only and that
   other programs keep their own licences), then the full original text
   from `LICENSE`, unchanged. Search, help, status and the menu cannot be
   reached while it is shown. The person must open the full text, tick
   "I certify that I have read the Hippocratic License 2.1" and choose
   Accept. Declining ends the session; the next login asks again.
2. **The record:** `resilience-admin record-licence SHA256` (polkit
   `org.resilience.admin.record-licence`, allowed without a password to
   the active local session, like `set-keymap`, ADR 0017). The helper
   checks that the hash is the SHA-256 of the licence file named in
   admin.toml (so a record always names the exact text shown), then
   appends one line to `/var/lib/resilience/licence-acceptances.log`
   (root-owned, 0644, opened with `O_APPEND | O_NOFOLLOW`): UTC time from
   the system clock, account name and UID, the text's SHA-256, licence
   name and version (`time=… user=… uid=… sha256=… licence="…"
   version=…`). The same operation goes to the audit log and the journal
   (`op=record-licence`). The hash is computed when the system is built
   (`builtins.hashFile`) and written to system.toml and admin.toml with
   the licence file's store path.
3. **Who has read it** is read from that file by the interface (it is
   world-readable: it holds account names and times, no secret). A
   changed licence text has a new hash: everyone is asked again.
4. **Live ISO:** its state is in memory, so the record disappears at
   shutdown and the licence is asked at every start, as requested.
   Installed systems keep it across reboots and updates (state, ADR
   0023 point 6).
5. **Tests only:** `resilience.licence.prompt` (default true) can turn the
   screen off for VM tests that are about something else (the flake's
   VM test wrapper does, with `mkDefault`); an evaluation invariant
   requires it on for every real system. `vm-licence` covers the notice;
   `vm-install` goes through it on the real installed system.

## Consequences

- One more step at first login, also for Ultra-Stable accounts that log
  in automatically.
- If recording fails (polkit or the helper unavailable), the screen says
  so and offers to try again or log out: the interface does not open
  without a record.
- The record proves the screen was shown and the box ticked, at the
  clock's time. It is not a signature and does not change the licence's
  own terms of acceptance.
