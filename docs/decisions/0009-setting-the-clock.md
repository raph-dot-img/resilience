# ADR 0009: Setting the clock without network time

Status: accepted (2026-11-14).

## Context

Resilience does not synchronise the clock (no NTP, ADR 0004). Computers
without a working RTC battery, or after a long time in storage, start with
a wrong date. A wrong clock makes backup names, logs, the audit log and
faillock lock-outs misleading. Later it will also make signature and
certificate checks misleading (Phase 4 and Phase 8).

Before this change only an Advanced user could fix the clock, with
`sudo date -s` in a shell. That does not update the hardware clock. The
interface had no way to do it.

Constraints:

- Setting the clock is security-relevant. Moving it backwards hides events
  in the logs and shortens lock-outs. It must not be possible without an
  administrator, and it must be recorded.
- Ultra-Stable users must be able to ask for it from the TUI, with an
  administrator typing their password (like mode changes).
- No new privileged code beyond a typed operation; no own RTC handling.

## Decision

A second operation of the privileged helper, under the rules of ADR 0007:

```
resilience-admin set-time YYYY-MM-DDTHH:MM:SSZ
```

- **Authorisation:** its own polkit action `org.resilience.admin.set-time`,
  `auth_admin` for active, inactive and remote sessions. Every attempt needs
  an administrator's password; there is no `_keep`.
- **Validation in the helper:** exactly one argument, in strict UTC RFC 3339
  form, with a real calendar date, and with 2025-01-01 ≤ time < 2100-01-01
  (`resilience-core/src/clock.rs`). The lower bound refuses the typical
  reset dates (1970, 2000) and dates before this system existed. Anything
  else is refused with exit code 2 and audited.
- **Effect:** the helper runs systemd's
  `timedatectl --no-ask-password set-time @SECONDS`, using the absolute path
  from the root-owned `admin.toml`. timedated sets the system clock and
  writes the hardware clock in the configured mode (UTC or local). It
  refuses while NTP is active, and the helper reports that.
- **Audit:** `op=set-time caller=… caller_uid=… from=<clock before> to=<new
  time> result=ok|refused|error`, in the audit log and the journal. Entries
  after the change carry the new time. The `from` field keeps the step
  visible.
- **Interface:** a System entry **Date & time**, available in every mode
  (like Users and modes). It shows the current local time and the time zone,
  and takes a local `YYYY-MM-DD HH:MM`. The TUI converts it to UTC with the C
  library's time-zone rules and shows the UTC value in a confirmation (Cancel
  is the default). It then runs `pkexec <helper> set-time <UTC>` with
  `pkttyagent`. When no administrator exists, the entry explains this
  instead of starting pkexec.

The time zone stays declared in Nix (`time.timeZone`, default UTC).
Changing it from the TUI is not part of this decision.

## Alternatives considered

- **timedated's own polkit action (`org.freedesktop.timedate1.set-time`)**
  from the TUI: its upstream default is `auth_admin_keep`, so a second
  change within minutes would need no password. Overriding that default
  would be a polkit rule, which we avoid (security invariant). It would also
  bypass our audit log and bounds checks.
- **Letting every console user set the clock without a password:** simpler
  for a single-user machine, but any local user could then move the clock to
  hide log entries or shorten faillock lock-outs.
- **`clock_settime` and `hwclock` in the helper:** more code of our own. It
  would also need to know whether the RTC runs in local time.
  timedated already handles this.

## Consequences

- An administrator must be present to fix the clock. The live ISO has none
  (by design), so its clock can only be fixed in the firmware setup.
- Audit-log timestamps can jump. Each `set-time` entry records the time
  before and after.
- The bounds will need updating before 2100, a limit we accept.

Tested by: `tests/vm/admin.nix` (refusals; wrong and right administrator
password through the TUI; system clock and `/sys/class/rtc/rtc0`; audit and
journal), unit tests in `clock.rs`, TUI tests, and `security-invariants`
(no polkit rule relaxes our actions or timedated's).
