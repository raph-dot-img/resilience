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

## Phase 2 — User and security architecture 🟡
- ✅ Three modes as real privileges (groups, sudo, Nix daemon, polkit defaults)
- ✅ Account separation, private homes, locked root, permission VM tests
- ✅ Evaluation-time security invariants
- ⬜ `resilience-privileged`: typed polkit-authorised helper (set mode, Wi-Fi,
  time, pack import) with server-side validation and audit log
- ⬜ Guided mode switching with administrator authentication and re-login
- ⬜ Recovery access design for locked root (rescue target / recovery key)
- ⬜ PAM faillock; review `unprivileged_userns_clone` for Ultra-Stable machines
- ⬜ Backup/restore of user data (age-encrypted, external media)

## Phase 3 — Productivity and knowledge 🟡
- ✅ Nano, Vim, Midnight Commander, calculator, units, calendar, notes, handbook
- ⬜ Kiwix: `kiwix-serve` (localhost) + evaluated text browser, search & links tested
- ⬜ Resource-pack manager (docs/RESOURCE_PACKS.md), USB discovery
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
