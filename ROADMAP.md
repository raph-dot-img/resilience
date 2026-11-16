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
- ✅ Keyboard layout: US default, each account chooses its layout in
  Settings (helper `set-keymap`, no password at the console, reset before
  every login prompt; ADR 0017)
- ✅ User namespaces reviewed: refused to everyone on Ultra-Stable-only
  machines (live system), kept for the Nix sandbox elsewhere (ADR 0012)

## Phase 3 — Productivity and knowledge ✅
- ✅ Nano, Vim, Midnight Commander, calculator, units, calendar, notes, handbook
- ✅ Kiwix: `kiwix-serve` (127.0.0.1, one file) + lynx (restricted), links and full-text search tested (ADR 0013)
- ✅ Resource packs: choose by category/language with sizes, download (resumable) or copy from USB, hash-pinned catalog, unprivileged store (ADR 0013)
- ✅ Read packs directly from a stick (no installation; works on the live system), check them against the catalog
- ✅ Suggested sets of packs (Essential, Field guides, Knowledge, Extended documentation, Complete; Geography waits for maps)
- ✅ Built-in documentation pack: handbook, NixOS manual and options (searchable)
- ✅ First offline guides with provenance: practical and health packs from named sources (catalog `source`, `license`, medical notice)
- ✅ Manual pages from the TUI, less in secure mode, search of the manuals (ADR 0014)
- ✅ Find & view files: name search, text, PDF (pdftotext), CSV/TSV table, archive listing, SHA-256 (ADR 0014)

## Phase 4 — Cryptography ✅
Guided age/minisign/GnuPG workflows, checksums, random data, key management;
secrets never in argv or logs (ADR 0015).
- [x] Encrypt/decrypt files and messages (age; OpenPGP via GnuPG)
- [x] Own keys (stored encrypted), contacts' public keys, export/import
- [x] Sign files (minisign), verify signatures (minisign and OpenPGP)
- [x] Checksums: show, compare, check SHA256SUMS lists
- [x] Random passphrases, passwords and keys
- [x] Inspect encryption formats
- [ ] Unlock encrypted (LUKS) USB sticks in the media screens (later)
- [ ] OpenPGP signing in the interface (minisign signs; gpg on the command line)

## Phase 5 — Offline maps 🟡
Design: ADR 0016 (MBTiles with vector tiles, block-character viewer, search
inside the pack, reproducible pack builder, routing deferred to its own ADR).
Branch `feat/maps`.
- ✅ Resource packs accept map packs (`.mbtiles`, `maps` category, data date, coverage, attribution)
- ✅ Geographic maths, bounded vector-tile decoder, canvas, style table, labels
- ✅ Hardened pack reader (`resilience-maps`, SQLite opened as untrusted input)
- ✅ Viewer (`resilience-maps view`): reviewed (risky parts), run on the real Luxembourg pack in QEMU at 170×48 (1366×768); label fixes from that run
- 🟡 Pack builder (`resilience-mapmaker`, `nix/maps/`): merged into `feat/maps`; the Luxembourg pack builds and the hardened reader accepts it; code not reviewed line by line, reproducibility check not re-run
- ⬜ Built-in world overview, TUI "Maps & navigation", search, nearby and go-to, spot heights, `vm-maps`, catalog entries

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

### Planned for the final product: `build_resilience.sh` (requested 2026-11-14)
A self-contained script that builds a custom ISO from answers to plain
questions, for people who do not write Nix. Sketch:

- Questions, each with a sensible default and an explanation:
  - edition (minimal or standard);
  - accounts, and the security mode of each;
  - which account logs in automatically, if any;
  - language and keyboard layout, time zone;
  - Wi-Fi on or off;
  - which resource packs, or suggested sets, to **put on the image**
    ("Would you like to download Wikipedia?"), with sizes and the free
    space needed;
  - optional applications;
  - where to write the ISO.
- Writes the answers to a small generated Nix file (`custom.nix`, kept, so
  the same image can be rebuilt or edited later), shows a summary to
  confirm, then runs `nix build` for a `custom` configuration of the flake,
  and prints where the ISO is and how to write it to a stick.
- Requires only Nix with flakes. Never uses sudo; writing the ISO to a
  stick is left to the user, with instructions. Checks free space first
  (large packs make large images) and uses the pack catalog's pinned
  hashes to fetch packs at build time (fixed-output downloads).
- Packs on the image: either inside the squashfs (simple; image size grows)
  or on a second partition of the stick (better for tens of GiB). This is
  to be decided when it is built.
- Passwords are never asked by the script or stored in the Nix files.
  First-boot setup asks for them, or hashed passwords come from a file
  outside the repository.

## Phase 9 — Audit and release ⬜
Independent architecture and security reviews, `cargo deny`, license audit,
SBOM, physical hardware tests, release notes.

## Disk space (estimate, 2026-11-16)
Measured: the system closure (live-standard) is **2.3 GiB**, of which
`linux-firmware` 825 MiB, a nixpkgs source copy 197 MiB, Python 128 MiB.
An installed system needs room for a second generation during updates
(up to +2.3 GiB when everything changes), plus /var, logs, home and a
free-space margin: plan **about 8 GiB before content**. Content sizes: a
France map pack z0-z14 with search ~4.3 GB (ADR 0016 *estimate*);
Wikipedia FR "top" / Wiktionary FR / Wikipedia FR complete 1.4 / 4.6 /
12.0 GiB. Packs can also be read from a USB stick.

## Known limitations carried forward
See CLAUDE.md §15.
