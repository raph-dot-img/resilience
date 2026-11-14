# Roadmap

Legend: ✅ done and tested · 🟡 partial · ⬜ not started. "Tested" means a
test in docs/TESTING.md actually ran and passed (VM only so far).

## Phase 0 — Project foundation ✅
- ✅ Environment inspection, Git, CLAUDE.md, documentation set, ADRs 0001–0006
- ✅ Flake pinned to nixos-26.05, dev shell, formatter, checks
- ✅ Build and test scripts (`nix flake check`, `tests/iso/smoke.py`, `scripts/boot-iso.sh`)

## Phase 1 — Resilience Core ✅
- ✅ Minimal NixOS configuration, editions minimal/standard
- ✅ Bootable x86_64 ISO (BIOS + UEFI), boot-tested in QEMU
- ✅ Starts and works without any network
- ✅ Rust TUI: main navigation, status panel, launcher, search, help,
  settings/themes, notifications, error dialogs, confirmations
- ✅ Logging (per-user log file) and crash recovery (session wrapper)
- 🟡 Text boot presentation: quiet kernel, systemd status on failure,
  branded getty and BIOS menu; UEFI GRUB still uses default colours

## Phase 2 — User and security architecture ✅
- ✅ Three modes as real privileges (groups, sudo, Nix daemon, polkit defaults)
- ✅ Account separation, private homes, locked root, permission VM tests
- ✅ Evaluation-time security invariants
- ✅ `resilience-admin`: typed helper (`set-mode`), polkit `auth_admin`
  every time, server-side validation, audit log + journal (ADR 0007)
- ✅ Guided mode switching in the TUI: consequences, confirmation
  (Cancel default), administrator password, persistence across reboots,
  sessions closed on demotion, new rights at next login
- ✅ Recovery access design for locked root (layered, ADR 0007); TUI Recovery
  section: health, failures, recent errors, generations, diagnostic report,
  emergency guide
- ✅ PAM faillock (login, su, sudo, polkit)
- ✅ Setting the clock from the TUI: `resilience-admin set-time`, own polkit
  action (auth_admin), bounds, audit, hardware clock (ADR 0009)
- ✅ Wi-Fi for every account from the TUI (unprivileged nmcli, private
  profiles, password via stdin); NetworkManager polkit allowlist for accounts
  without its group (ADR 0010)
- ✅ Making an earlier generation the default from the TUI:
  `resilience-admin set-default-generation`, own polkit action (auth_admin),
  boot only, undo on failure, audit (ADR 0011)
- ✅ Removable media for every account (udisks2, removable drives only) and
  encrypted backup/check/restore of the home folder (age, ADR 0008)
- ✅ User namespaces reviewed: refused to everyone on Ultra-Stable-only
  machines (live system), kept for the Nix sandbox elsewhere (ADR 0012)

## Phase 3 — Productivity and knowledge 🟡
- ✅ Nano, Vim, Midnight Commander, calculator, units, calendar, notes, handbook
- ✅ Kiwix: `kiwix-serve` (127.0.0.1, one file) + lynx (restricted), links and full-text search tested (ADR 0013)
- ✅ Resource packs: choose by category/language with sizes, download (resumable) or copy from USB, hash-pinned catalog, unprivileged store (ADR 0013)
- ⬜ Read packs directly from a stick on the live system (no installation)
- ⬜ First offline guides with provenance; man/NixOS docs pack
- ⬜ Document viewing (pdftotext), CSV tool, file search in the TUI

## Phase 4 — Cryptography ⬜
Guided age/minisign/GnuPG workflows, checksums, random data, key management;
secrets never in argv or logs.

## Phase 5 — Offline maps ⬜
MBTiles/GeoPackage storage, block-character renderer, place search, routing study.

## Phase 6 — Networking and communications 🟡
- ✅ Passive diagnostics + on-demand probe; LOCAL LINK detection tested
- ⬜ Reticulum evaluation for local messaging; store-and-forward; delivery states
- ⬜ Yggdrasil / BATMAN-adv profiles; Tox, signal-cli (documented as unofficial)

## Phase 7 — Radio and additional applications 🟡
- ✅ First offline games (nudoku, solitaire, ninvaders, moon-buggy, NetHack)
- ⬜ Receive-only SDR tools with hardware detection and legal notices
- ⬜ Power profiles (Maximum Efficiency / Balanced / Performance) and measurements on hardware

## Phase 8 — Updates and installation ⬜
Text installer, offline update bundles (`nix copy` + minisign manifest),
signed pack verification, rollback tests, installation docs.

## Phase 9 — Audit and release ⬜
Independent architecture and security reviews, `cargo deny`, license audit,
SBOM, physical hardware tests, release notes.

## Known limitations carried forward
See CLAUDE.md §15.
