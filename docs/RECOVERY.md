# Recovery

Resilience separates **system rollback** (the operating system) from
**user-data restoration** (your files). They are independent operations.

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
are under `/home` on the installed disk. A guided rescue mode is planned
(Phase 2/8).

**Known gap:** the root account is locked, so systemd's emergency shell
cannot be entered. Recovery today relies on previous generations and the
live ISO. See THREAT_MODEL.md T18.

## User data

Backups and restoration tools are planned (Phase 2/4: encrypted backups with
age to external media, integrity verification). Until then, copy your home
folder to external media regularly (Stable mode: `cp -a`, or Midnight
Commander).

## Diagnostic information to give a helper

SYSTEM MANAGEMENT → System status (or F5) lists systemd state, failed
services, storage, memory, power and unavailable programs. Exporting this
report to a file is planned.
