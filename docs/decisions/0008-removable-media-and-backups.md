# ADR 0008: Removable media and encrypted backups of user data

Status: accepted (2026-11-14).

## Context

The original specification (docs/ORIGINAL-SPEC.md) asks for user-data backups and restore, kept separate from
system rollback (§20), usable by beginners, offline, with no cryptography of
our own (§17). Until now only an administrator could mount a USB stick. Phase
3 resource packs and Phase 8 offline updates will also need removable media.

Constraints:

- Ultra-Stable accounts have no shell. Everything must work from the TUI,
  and nothing may give them more than access to their own files and sticks.
- No daemon may run when nobody uses it (low resources).
- Internal disks must never be offered or mounted by ordinary users.
- A passphrase must not pass through our code, our logs or the process list.
- Cheap sticks fail silently. A backup must be verifiable, and a half-written
  backup must never look complete.

## Decision

### Removable media: udisks2, with its own polkit policy

`services.udisks2` is enabled (`nix/modules/media.nix`, option
`resilience.media.enable`). It is D-Bus activated: it starts only when the
interface first lists or mounts a stick.

- **Listing:** `lsblk -P -b` (unprivileged), parsed by the pure function
  `media::removable_volumes`. A file system is offered only if its device, or
  a device above it, is removable or hot-plugged. Encrypted (LUKS) and
  unsupported file systems are shown as unusable. Volumes mounted outside
  `/run/media/` (for example the live ISO's own stick) are shown as in use by
  the system.
- **Mount and eject:** `udisksctl mount|unmount|power-off
  --no-user-interaction --block-device DEV`. udisks2's polkit defaults:
  - removable media, active local session: allowed;
  - internal ("system") disks: `auth_admin_keep`, an administrator's password
    (upstream default, observed in vm-backup; a Stable user's `udisksctl`
    from a shell would ask for it, the TUI never does);
  - inactive or remote sessions: an administrator's password.

  `--no-user-interaction` turns every password request into a refusal, so the
  TUI cannot mount an internal disk even if one slipped through the list.
  Mounts are `nosuid,nodev`, belong to the user, and live under
  `/run/media/<user>` (ACL-protected, per user, gone at reboot). No polkit
  rule widens these defaults; a security invariant checks that.
- **Eject** unmounts, which writes everything out, then powers the drive off.
  If power-off fails, the stick is still safe to unplug once unmounted, and
  the TUI says so. If unmount fails (busy), the TUI says "Do not unplug it
  yet".

### Backups: `resilience-backup`, unprivileged, tar | zstd | age

A new binary, `resilience-backup` (crate `crates/resilience-backup`), runs as
the user. It has no setuid bit, no polkit action and no special groups. The
TUI starts it in the foreground like any other program:

| Command | Effect |
|---|---|
| `create MOUNTPOINT` | `tar -C ~ . \| zstd -3 \| age --passphrase` → `MOUNTPOINT/Resilience-backups/home-<UTC stamp>.tar.zst.age`, then `sha256sum` → `<file>.sha256` |
| `check FILE` | compare the file with its `.sha256` (no passphrase needed) |
| `restore FILE` | check, then `age --decrypt \| zstd -d \| tar -x` into a new `~/Restored/backup-<stamp>` |

- **Encryption:** age 1.3 in passphrase mode. age reads and confirms the
  passphrase on the terminal itself (`/dev/tty`), and can generate a strong
  one when the field is left empty. The passphrase never reaches our
  process, argv, environment or logs. age authenticates every 64 KiB chunk
  and the end of the stream, so truncation and tampering are detected on
  restore even without the checksum file. Key derivation is scrypt; the
  work factor is age's default.
- **Integrity:** the `.sha256` file is in `sha256sum` format, so
  `sha256sum -c` checks it on any Linux system. It detects a damaged copy
  without the passphrase. It is computed right after writing; checking after
  re-plugging reads the data back from the stick. It is not a signature: an
  attacker who can rewrite the stick can rewrite both files. That case is
  caught by age's authentication on restore.
- **Atomicity:** the archive is written to `*.partial` (0600, `O_EXCL`),
  fsync'd, renamed, and the directory fsync'd. Only complete files are ever
  listed. On any failure the partial file is removed.
- **Scope:** the whole home folder, on one file system, without following
  symbolic links, excluding `~/.cache`. Unreadable files are skipped with a
  warning (`--ignore-failed-read`) instead of failing the whole backup.
- **Restore never overwrites:** extraction goes into a freshly created 0700
  folder (`backup-<stamp>`, then `-2`, `-3` and so on). GNU tar runs as the
  user with `--no-same-owner --no-same-permissions --no-overwrite-dir`, and
  by default refuses member names containing `..` and strips leading `/`. On
  failure (wrong passphrase, damage), an empty destination is removed and a
  partial one is renamed `-incomplete`, and the tool says so.
- **Damaged copies are refused** for restore when the checksum does not match.
  An Advanced user can still try a manual extraction (handbook).
- **Root cause on failure:** pipeline stages killed by SIGPIPE are not
  reported as the cause, so the user sees age's "incorrect passphrase", not a
  tar error.

Tool paths (`lsblk`, `udisksctl`, `tar`, `zstd`, `age`, `sha256sum`,
`resilience-backup`) are absolute store paths in `/etc/resilience/system.toml`
(`[tools]`). The system build fails if one is not executable. age and zstd
are also installed for the command line.

### Interface

A new System entry, **Backups & USB sticks**, is available in every mode. It
has two screens:

- **Sticks:** Enter opens a stick (mounts it), `e` ejects it, `r` refreshes.
- **Backups on a stick:** the first row makes a new backup. Enter on a backup
  restores it, `c` checks it, `e` ejects the stick.

New backups and restores need a confirmation, with Cancel as the default.
The confirmation says what will happen, how large the home folder is, how
much space the stick has, that the passphrase cannot be recovered, and where
restored files will go.

## Alternatives considered

- **A privileged helper operation for mounting:** rejected. udisks2 already
  implements removable versus system disks with polkit, and is maintained
  and audited upstream. Our own mount helper would add privileged surface.
- **LUKS-encrypted sticks (cryptsetup):** strong, but formatting a stick
  needs root or a udisks `auth_admin` action, and the stick becomes unreadable
  on other systems without cryptsetup. Possible later, for whole-stick
  encryption.
- **GnuPG symmetric encryption:** works, but has a larger attack surface and
  more confusing prompts. age is simpler, and its format is versioned and
  specified.
- **restic or borg:** good tools with deduplication, but they bring a
  repository format beginners cannot inspect, plus more code. A single
  archive per backup is easy to explain ("one file per backup, keep two
  sticks").
- **Restoring in place:** rejected. It is too easy to overwrite newer files.
  Users move what they need out of `~/Restored`.

## Consequences

- Every account, Ultra-Stable included, can mount USB sticks and memory cards
  from its console session, and read and write files on them. Ultra-Stable
  accounts still cannot run programs (no shell). For Stable accounts this
  adds nothing they could not do by copying a program into their home folder
  (mounts are `nosuid,nodev` either way).
- Kernel file system drivers parse untrusted media (THREAT_MODEL T23). This
  is the usual risk of plugging in a stick. Only common file systems are
  offered, and auto-mounting is not used: the user decides.
- A forgotten passphrase means the backup is lost. The interface says so
  before every backup.
- Backups are full copies (no incremental backups). They are fine for the
  small home folders this system targets. Large media collections are better
  copied directly.
- The live-standard system closure grows by 62 MiB (2231.6 → 2293.9 MiB,
  measured). age is 23 MiB (static Go binaries, including its plugins).
  udisks2 with what libblockdev pulls in (parted, xfsprogs, mdadm,
  thin-provisioning-tools, libnvme, ntfs3g, nss) is about 30 MiB. The rest is
  rebuilt paths (hwdb, system-path). A udisks2 build with fewer libblockdev
  plugins could reduce this later.

Tested by: `tests/vm/backup.nix`, unit tests in `media.rs` and `backup.rs`
(including a real age round trip in the Nix build), TUI tests, and
`security-invariants`.
