# Recovery

Resilience separates **system rollback** (the operating system) from
**user-data restoration** (your files). They are independent operations.

## Start here: the Recovery section

SYSTEM MANAGEMENT → **Recovery & diagnostics** works in every mode and changes nothing:

| Item | What it shows |
|---|---|
| System health & recent failures | systemd state, failed units with result and time, the last 30 error messages of this boot (the whole system journal for Advanced accounts, otherwise only the account's own messages) |
| System generations | number, date, NixOS version, kernel; which one is running, which one was booted, which one is the default; how to start an older one from the boot menu |
| Export a diagnostic report | writes `~/Diagnostics/resilience-report-YYYYMMDD-HHMMSS.txt` (directory 0700, file 0600, never overwritten) with version, mode, status, interface names and link state (no IP or MAC addresses), health, generations and the last 50 lines of the interface log |
| Emergency guide | handbook page 40-recovery |

*Tested: TUI unit tests (rendering, report permissions, no overwrite). Not
tested on an installed system with several generations (no installer yet).*

## The interface stopped

The session wrapper shows "RESILIENCE INTERFACE STOPPED UNEXPECTEDLY",
the exit code and the log path (`~/.local/state/resilience/tui.log`), then
restarts the interface when you press Enter. After three quick failures it
waits 30 seconds between attempts so it cannot spin. *Tested in VM.*

## You need another login

Press **Alt+F2** … **Alt+F6**: each console offers a login prompt, even
when tty1 logs in automatically. *Tested in VM (tty2).*

## The system misbehaves after a change or update — system rollback

Every configuration change creates a new **generation**; old ones stay on
disk until deliberately deleted.

1. Restart the computer.
2. In the boot menu, choose an older entry.
3. If it works, an administrator can make it permanent:
   `sudo nixos-rebuild switch --rollback`, or `sudo /nix/var/nix/profiles/system-<N>-link/bin/switch-to-configuration switch`.

*Not yet tested: requires an installed system (no installer yet).*

## The system does not boot at all

Boot the Resilience live ISO from USB. From there an administrator with
NixOS experience can mount the installed system and inspect it. Your files
are under `/home` on the installed disk.

**By design, root is locked**, so systemd's emergency shell cannot be
entered. Recovery is layered (ADR 0007): the Recovery section, other
consoles with an Advanced account (sudo), earlier generations, and the live
ISO. Installed systems must have at least one Advanced account; the
installer will require one (Phase 8).

## Someone is locked out after wrong passwords

After 5 failures within 15 minutes, an account is locked for 5 minutes
(pam_faillock on login, su, sudo and polkit). Wait, or as an administrator:
`faillock --user NAME` to inspect, `sudo faillock --user NAME --reset` to unlock.
The counters reset at reboot. *Tested in VM (vm-admin).*

## A security mode is wrong

SYSTEM MANAGEMENT → Users & security modes (administrator password
required), or as an administrator `sudo resilience-admin set-mode NAME MODE`.
`resilience-admin list` shows the declared mode, the effective mode and the
active groups. Every change is recorded in `/var/log/resilience/audit.log`
(readable by administrators) and the journal (`journalctl -t resilience-admin`).
If `/var/lib/resilience/modes.toml` is damaged, the helper refuses changes
and the declared modes apply. An administrator can inspect the file, or
delete it to go back to the configured modes.

## User data

Backups and restoration tools are planned (Phase 2/4: encrypted backups with
age to external media, integrity verification). Until then, copy your home
folder to external media regularly (Stable mode: `cp -a`, or Midnight
Commander).

## Diagnostic information to give a helper

Recovery & diagnostics → Export a diagnostic report (see above). SYSTEM
MANAGEMENT → System status (or F5) shows the same live information.
