# Threat model

Version 0.2 (2026-11-14, Phase 2: privileged helper). Reviewed whenever a security-relevant feature is
added. "Implemented" means implemented **and tested**; see docs/TESTING.md.

## 1. What we protect

| Asset | Why it matters |
|---|---|
| System integrity (Nix store, /etc, bootloader, generations) | A broken system may be unrecoverable without the Internet |
| User data (home directories, notes, keys) | Irreplaceable; private |
| Private keys and passphrases (age, GnuPG, minisign) — future | Authenticity of communications and updates |
| Availability of the interface | The machine is useless to beginners without it |
| Authenticity of updates and resource packs — future | Offline media are an easy injection vector |

## 2. Actors and capabilities

| Actor | Capabilities | In scope |
|---|---|---|
| **Inexperienced user** (Ultra-Stable) | Keyboard at the TUI; may press anything | Yes — primary design driver: *mistakes* must not damage the system |
| **Curious/hostile local user** (Ultra-Stable/Stable account) | Arbitrary code as their own UID (assume shell escapes exist) | Yes — must not gain root, read others' files, change the system, or fill the Nix store (Ultra-Stable) |
| **Malicious program or document** (e.g. a crafted handbook page, ZIM, map tile, resource pack) | Data parsed by our code or by integrated tools | Yes — parsers must fail safely; packs verified before install (Phase 3/8) |
| **Local-network attacker** | Packets on the LAN / Wi-Fi | Yes — no listening services by default; future local messaging must authenticate peers |
| **Supply chain** | Compromised upstream source or binary cache | Partly — pinned nixpkgs, hash-locked Cargo deps, signed caches; we cannot audit all of nixpkgs |
| **Physical attacker** | Boots other media, removes the disk, keyboard at boot menu | **Out of scope for integrity** unless full-disk encryption and firmware passwords are used. Documented, not hidden. |
| **Remote Internet attacker** | Nothing listens; outbound connections only on request | Yes, but surface is minimal |

## 3. Trust boundaries

1. **Kernel/UID boundary** between accounts. The only real boundary for
   modes. Enforced by Linux permissions, sudo (wheel + password), polkit,
   the Nix daemon's `allowed-users`.
2. **TUI ↔ launched programs**: the TUI runs programs from fixed argv taken
   from a root-owned catalog; it never interprets user input as a command.
3. **System ↔ external media** (future): resource packs and update bundles
   are untrusted until their signature is verified.
4. **Machine ↔ network**: default deny inbound (firewall, no services);
   outbound only on explicit request.

## 4. Threats and mitigations

| # | Threat | Mitigation | Status |
|---|---|---|---|
| T1 | Beginner deletes or corrupts system files | Ultra-Stable has no write access outside home; Nix store read-only; no sudo | Implemented (vm-modes) |
| T2 | Beginner reaches a shell and runs damaging commands | No shell entry; login shell refuses `-c`; `$SHELL=nologin` for program escapes; **even with a shell, no privileges** | Implemented (vm-modes, vm-boot-offline) |
| T3 | Ultra-Stable user installs or builds software, fills disk via Nix | Nix daemon refuses non-allowed users | Implemented (vm-modes) |
| T4 | Privilege escalation via the TUI | TUI runs unprivileged; no command construction; power actions via logind policy with `--no-ask-password`; the only privileged paths are `pkexec resilience-admin set-mode` and `set-time`, each of which needs an administrator's password every time | Implemented (vm-admin) |
| T5 | Stable user changes system configuration | No sudo; /etc and store read-only; polkit denies unit management | Implemented (vm-modes) |
| T6 | Advanced user's password guessed by another local user | sudo/polkit require the Advanced user's password; pam_faillock on login, su, sudo, polkit-1 (5 failures / 15 min → 5 min lock) | Implemented (vm-admin, security-invariants) |
| T7 | Root login | Root password locked ("!"), no SSH | Implemented (security-invariants) |
| T8 | Users read each other's files | Home directories mode 0700 (NixOS default); notes and settings created 0600 | Implemented (vm-modes, unit tests) |
| T9 | Background network traffic leaks presence/metadata | No NTP, no NM connectivity check, no update checks; probe only on keypress, configurable targets | Implemented (security-invariants, vm-boot-offline `ss` check) |
| T10 | Hang or freeze when the network is absent/broken | Passive detection; probe skipped without route; timeouts on every external command | Implemented (unit tests, vm-boot-offline) |
| T11 | TUI crash locks user out | Session wrapper restarts it; ttys 2–6; previous generations in boot menu | Implemented (vm-boot-offline crash test) |
| T12 | Malformed catalog/config breaks the interface | Strict parsing, defaults on failure, built-ins keep working | Implemented (unit tests) |
| T13 | Path traversal via note names | Strict name validation, no `/`, no leading `.`, no `..` | Implemented (unit tests) |
| T14 | Secrets in logs, store, process lists | Passphrases are typed into age/minisign themselves on the terminal (backups, cryptography): never in our process, argv, env or logs; rules in SECURITY.md | Implemented (vm-backup, vm-crypto greps the journal, home and /tmp for every passphrase) |
| T15 | Malicious resource pack or offline update | minisign signature verification against pinned keys before extraction; extraction in a private staging dir; no symlinks/absolute paths | Planned (Phase 3/8) |
| T16 | Kernel info leaks / ptrace abuse | sysctls: kptr_restrict, dmesg_restrict, ptrace_scope=1, unprivileged BPF off, kexec off | Implemented (config) |
| T17 | Physical access bypass | Out of scope without FDE; installer will offer LUKS | Documented |
| T18 | Recovery needs root but root is locked | Layered recovery without a root password (ADR 0007): TUI Recovery section, other consoles + Advanced account with sudo, earlier generations, live ISO; installer must create an Advanced account | Designed; Recovery section tested (unit/TUI); installed-system path NOT TESTED (no installer) |
| T19 | Abuse of the privileged helper (argument injection, unmanaged accounts, lock-out of all admins, privileges kept after demotion) | Fixed sub-commands; every argument validated in the helper; only accounts in `resilience.users`; only the three mode groups touched; absolute tool paths from root-owned config, environment ignored; last administrator protected; autologin stays Ultra-Stable; demotion terminates sessions; refusal on corrupt state | Implemented (vm-admin, unit tests) |
| T20 | Unaudited security-sensitive changes | Append-only logfmt audit log (root:wheel 0640) + journal (authpriv) for every set-mode/set-time/apply-state outcome (set-time records the clock before and after); polkit logs denied authorisations | Implemented (vm-admin) |
| T21 | Fake password prompt captures an administrator's password in another user's session | Inherent to "over-the-shoulder" authorisation; Ultra-Stable sessions cannot run arbitrary programs; handbook tells administrators to authorise only on a trusted screen | Accepted risk, documented |
| T22 | Mode overrides silently reverted or stale | Recorded overrides re-applied after every activation; overrides older than a changed Nix declaration are dropped and audited | Implemented (vm-admin reboot test; staleness: unit tests) |
| T23 | Malicious or damaged removable media (USB stick) | Only removable/hot-plugged drives offered; mounted only on user request (no automount), by udisks2, `nosuid,nodev`, per user under /run/media; internal disks need an administrator (udisks polkit, `--no-user-interaction`); common file systems only. Kernel file-system drivers still parse the stick | Implemented (vm-backup); kernel parsing: accepted risk |
| T24 | Lost or stolen backup stick exposes user files | Backups encrypted with age (passphrase, scrypt); passphrase typed into age on the terminal, never in our argv, env or logs; no plaintext written to the stick (vm-backup scans the raw device) | Implemented (vm-backup, unit tests) |
| T25 | Restoring a backup destroys newer files, or a bad backup goes unnoticed | Restore only into a new 0700 folder under ~/Restored; checksum checked first, damaged copies refused; age authenticates every chunk; partial backups never listed (`.partial` + rename); failed restores marked `-incomplete` | Implemented (vm-backup, unit tests) |
| T26 | Clock moved to hide log entries, shorten faillock lock-outs or (later) accept expired signatures | Only via `resilience-admin set-time` with an administrator's password every time (own polkit action, auth_admin); strict UTC format and bounds 2025 ≤ t < 2100 checked in the helper; audited with the previous time; no polkit rule relaxes timedated; no NTP in the background | Implemented (vm-admin, unit tests, security-invariants). Root and Advanced users can still use `date`/`timedatectl` directly |
| T27 | Ultra-Stable account disrupts or reconfigures networking (switches it off, hotspot, connectivity checks = background traffic) through NetworkManager's permissive polkit defaults for active sessions | polkit allowlist for accounts without the networkmanager group: only wifi.scan, network-control, settings.modify.own, enable-disable-wifi; everything else NO (ADR 0010) | Implemented (vm-wifi pkcheck on the TUI process, security-invariants). Before 2026-11-14 these defaults applied unnoticed |
| T28 | Wi-Fi password exposed | Typed in the TUI (masked), passed to nmcli on stdin (`passwd-file /dev/stdin`), never argv/env/logs/files we write; stored by NetworkManager root 0600; profile private to the account | Implemented (vm-wifi, unit test with fake nmcli) |
| T29 | Old, vulnerable or broken generation made permanent, or the boot menu left broken | Only via `resilience-admin set-default-generation` with an administrator's password every time (own polkit action, auth_admin); number validated, generation must exist and be a system closure in /nix/store; `boot` only (running system unchanged); profile switched back if writing the boot menu fails; audited; clean environment for nix-env and switch-to-configuration | Implemented (vm-generations, unit tests). Tested with systemd-boot (UEFI) only |
| T30 | Kernel privilege escalation through user namespaces | `user.max_user_namespaces = 0` on Ultra-Stable-only machines (live system), checked by an invariant; kept elsewhere for the Nix sandbox (ADR 0012) | Implemented (vm-boot-offline, vm-modes, security-invariants). Not restricted on mixed machines |
| T31 | Malicious, damaged or substituted resource pack (mirror, network, USB stick) | Catalog in the root-owned system configuration pins each file's size and SHA-256; the store installs only on an exact match (mismatch or extra bytes: deleted); HTTPS only, also after redirects; files from sticks checked the same way (ADR 0013) | Implemented (vm-packs: tampered download refused, stick checked; unit tests) |
| T32 | Pack installation used to gain privileges or damage the system | No root: `resilience-pack-store` runs as the unprivileged `resilience-packs` account via `pkexec --user` (refuses any other uid), with an administrator's password every time (own action, auth_admin); typed arguments (catalog ids, plain `.zim` names); content never parsed, only hashed; O_NOFOLLOW, regular files only, lock, free-space margin; journal log | Implemented (vm-packs, unit tests) |
| T34 | Shell from a file viewer or the manual pager for Ultra-Stable accounts; terminal escape sequences in a viewed file | Files are shown inside the TUI (text read directly, control characters replaced; PDF via pdftotext; archives only listed by bsdtar -t; nothing extracted or run); man runs less with LESSSECURE=1 and LESSOPEN/MANOPT/BROWSER/EDITOR emptied; typed names validated (no options, no shell) (ADR 0014) | Implemented (vm-documents, unit tests) |
| T35 | Private key stolen from a copied home folder, backup or lost laptop | Own keys always stored encrypted with the user's passphrase (age identity via `age-keygen \| age -p`, minisign's own encryption); key folder 0700, files 0600; private keys refused when "importing a contact" | Implemented (vm-crypto: no `AGE-SECRET-KEY` on disk, modes; unit tests). Brute force of a weak passphrase: scrypt slows it, a strong passphrase is the defence |
| T36 | Encryption or signature results misunderstood (false sense of security) | Results in words with their limits: valid signature ≠ safe file; unknown signer = "NOT CHECKED"; checksum match only as good as its source; MD5/SHA-1 flagged; weak formats named; original left in place and the user told; lost passphrase = lost data, said at every creation | Implemented (unit tests, vm-crypto) |
| T37 | Substituted contact key (man in the middle when exchanging keys) | Short key forms to compare by voice; the user is asked to check in person or by phone; a contact whose key changes is refused until removed; no automatic key fetching (offline) | Implemented (host end-to-end, unit tests). Depends on the user really checking |
| T38 | Decrypted data left behind or overwriting files | Decrypted files only in ~/Decrypted (0700/0600), O_EXCL + RENAME_NOREPLACE, partial output deleted on failure (wrong passphrase leaves nothing); messages decrypted in memory only | Implemented (vm-crypto, unit tests) |
| T39 | GnuPG leaks presence, fetches keys, keeps passphrases, or trusts keys for the user | Always `--batch --no-auto-key-retrieve --no-auto-key-locate`, no key server; gpg-agent cache TTL 0 (invariant); fingerprint shown and confirmation asked before any import (`--show-keys`), private keys refused; `--trust-model always` only for keys the user chose; messages without integrity protection refused | Implemented (vm-crypto with a real GnuPG peer, security-invariants, unit tests on real gpg status output) |
| T40 | A signature made to look like it covers a file it does not (clear-signed `NAME.asc` next to `NAME`), or options injected into gpg | Such signatures refused next to a same-named file; clear-signed text shown exactly as signed; "not a detached signature" is a refusal; names typed for keys cannot start with `-`, `--` before the user id; absolute paths everywhere (ADR 0015 review) | Implemented (vm-crypto regression step, host reproduction of the review's cases, unit tests) |
| T41 | Console keymap used against another person: a key bound to a typed command line, or a login password typed with the wrong layout until faillock locks the account | The kernel allows keymap changes only with CAP_SYS_TTY_CONFIG; the only path for users is `resilience-admin set-keymap` with a name from the root-owned list in admin.toml (no user-supplied keymap files, so no string bindings of their choosing), `loadkeys -q -- NAME` with fixed argv, audited; polkit allows it without a password to the active local session only (ADR 0017); the system layout is loaded again before every login prompt (getty@ ExecStartPre) | Implemented (vm-boot-offline, unit tests). Concurrent sessions on several consoles share one keymap (kernel) |
| T33 | Shell or Internet access from the offline library for Ultra-Stable accounts | lynx with `-restrictions=… -localhost` (no shell escape, no external programs, no typed address, no remote host); kiwix-serve on 127.0.0.1 only, `--blockexternal`, one file, stopped with the browser | Implemented (vm-packs: `!` refused, no shell process, loopback only). Usability measure, not a sandbox |

## 5. Known gaps (honest list)

- The privileged helper offers `set-mode`, `set-time` and
  `set-default-generation` so far. The time zone can only be changed in the
  Nix configuration.
- Making an older generation the default brings back its older software,
  including fixed vulnerabilities (ADR 0011); the confirmation says so, and
  only an administrator's password allows it.
- Ultra-Stable accounts keep NetworkManager's `network-control` (they can
  disconnect any connection, including an administrator's Ethernet) and
  `settings.modify.own` (profiles for themselves with any settings, e.g. a
  static address or cloned MAC). The TUI creates only plain Wi-Fi profiles.
- The TUI holds a typed Wi-Fi password in memory until nmcli has read it;
  wiping our copies is best effort.
- The audit log is append-only by convention (root writes with O_APPEND);
  root can still edit it. No remote/WORM log copy.
- pkexec is setuid and its generic action allows any program with an
  administrator's password (equivalent to that administrator's sudo).
- No full-disk encryption (no installer yet). No Secure Boot integration.
- Launched third-party programs may contain their own shell escapes; this
  is acceptable only because accounts have no privileges (T2).
- Resource packs are opened by libzim/kiwix-serve and rendered by lynx as
  the user; a pack that matches the catalog is trusted to the degree the
  catalog is. `resilience-library open` also opens any `.zim` file the user
  names (their own risk, as with any document). While a pack is open, other
  local accounts can read it through the loopback port (public content).
- The pack catalog ages with the system: Kiwix deletes old files, so some
  packs can then only be installed from a stick until a system update.
- User namespaces stay available on machines with Stable or Advanced
  accounts (the Nix sandbox needs them), so Ultra-Stable accounts there can
  create them too. Ultra-Stable-only machines, including the live system,
  refuse them to everyone (ADR 0012). The mainline kernel has no
  `unprivileged_userns_clone` switch.
- Backup checksums are not signatures: someone who can rewrite a stick can
  replace both files. age's authentication still detects tampering at restore
  time, but cannot tell an attacker's validly encrypted archive (made with a
  known passphrase) from yours.
- Backups have no rotation or retention policy; old backups stay until the
  user deletes them from the stick.
- Not reviewed by an independent security auditor.
