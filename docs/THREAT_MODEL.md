# Threat model

Version 0.1 (2026-11-14). Reviewed whenever a security-relevant feature is
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
| T4 | Privilege escalation via the TUI | TUI runs unprivileged; no privileged helper exists yet; no command construction; power actions via logind policy with `--no-ask-password` | Implemented |
| T5 | Stable user changes system configuration | No sudo; /etc and store read-only; polkit denies unit management | Implemented (vm-modes) |
| T6 | Advanced user's password guessed by another local user | sudo requires the Advanced user's password; rate limiting via PAM faillock | Partly (faillock not configured yet) |
| T7 | Root login | Root password locked ("!"), no SSH | Implemented (security-invariants) |
| T8 | Users read each other's files | Home directories mode 0700 (NixOS default); notes and settings created 0600 | Implemented (vm-modes, unit tests) |
| T9 | Background network traffic leaks presence/metadata | No NTP, no NM connectivity check, no update checks; probe only on keypress, configurable targets | Implemented (security-invariants, vm-boot-offline `ss` check) |
| T10 | Hang or freeze when the network is absent/broken | Passive detection; probe skipped without route; timeouts on every external command | Implemented (unit tests, vm-boot-offline) |
| T11 | TUI crash locks user out | Session wrapper restarts it; ttys 2–6; previous generations in boot menu | Implemented (vm-boot-offline crash test) |
| T12 | Malformed catalog/config breaks the interface | Strict parsing, defaults on failure, built-ins keep working | Implemented (unit tests) |
| T13 | Path traversal via note names | Strict name validation, no `/`, no leading `.`, no `..` | Implemented (unit tests) |
| T14 | Secrets in logs, store, process lists | No secrets handled yet; rules in SECURITY.md; future crypto must pass secrets via stdin/fd, never argv | Design rule |
| T15 | Malicious resource pack or offline update | minisign signature verification against pinned keys before extraction; extraction in a private staging dir; no symlinks/absolute paths | Planned (Phase 3/8) |
| T16 | Kernel info leaks / ptrace abuse | sysctls: kptr_restrict, dmesg_restrict, ptrace_scope=1, unprivileged BPF off, kexec off | Implemented (config) |
| T17 | Physical access bypass | Out of scope without FDE; installer will offer LUKS | Documented |
| T18 | Recovery needs root but root is locked | Previous generations; live ISO. Rescue design pending | **Open** (Phase 2) |

## 5. Known gaps (honest list)

- No privileged helper yet, so Ultra-Stable users cannot do safe administrative
  tasks (join Wi-Fi, change time) through the TUI. Stable users use `nmtui`.
- No PAM faillock configuration, no audit log of security-sensitive changes.
- No full-disk encryption (no installer yet). No Secure Boot integration.
- Launched third-party programs may contain their own shell escapes; this
  is acceptable only because accounts have no privileges (T2).
- `kernel.unprivileged_userns_clone` is not restricted (needed by the Nix
  sandbox for Stable users); reconsider for Ultra-Stable-only machines.
- Not reviewed by an independent security auditor.
