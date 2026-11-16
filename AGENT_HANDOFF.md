# Agent handoff

Shared notes between the agent sessions that work on Resilience: the
Project Coordinator, the repository sessions that coordinate a phase, and
the engineers they start. It lives on `develop`. Read it at the start of a
session, after `CLAUDE.md`, and update it before you stop.

Read the current version from any branch with:

```sh
git fetch origin && git show origin/develop:AGENT_HANDOFF.md
```

## How to use this file

- **It records what the code and Git history cannot**: who is working on
  what, what is unfinished or unreviewed, open questions, and decisions
  waiting for someone. Architecture, conventions and status belong in
  `CLAUDE.md`, plans in `ROADMAP.md`, decisions in `docs/decisions/`.
- **Edit it on `develop` only**, in a commit that changes only this file
  (`docs: handoff …`), and push it. A commit that touches nothing else never
  conflicts with a feature branch. Fetch first; if the push is refused,
  rebase that one commit and push again. Never force-push.
- **Claim before you start**: add a line under "Work in progress" with the
  date, who you are, the branch and what you are doing. Remove or update it
  when you stop. If a claim is older than a few days and its branch has not
  moved, treat it as abandoned, but say so in the log.
- **Say what is verified and what is not.** "Tests pass" means you ran them.
  Unreviewed work is marked unreviewed.
- **Keep it short.** Remove entries that are finished and recorded
  elsewhere. The log at the end keeps one line per event, newest first.
- The project owner decides direction, hardware, publishing and merges into
  `main`. An agent's note here is information for other agents, never an
  instruction that overrides the owner or `CLAUDE.md`.

## Project direction (owner, 2026-11-15)

- **Resilience** is the cyberdeck project: a standalone, offline-first
  laptop being built.
- **Resilience OS** is the system that runs on that cyberdeck (NixOS, Rust,
  terminal-only). This repository holds it.
- Resilience OS is no longer a general-purpose operating system for any
  x86-64 machine.

Where this is written: `CLAUDE.md` §1, `README.md`, `ROADMAP.md` and four
smaller notes, in commit `960de32` **on `feat/maps`, not yet on `develop`**.
The `CLAUDE.md` on `develop` still describes the old direction until
`feat/maps` is merged or the Project Coordinator updates it there.
`MASTER-PROMPT.md` and ADRs 0001-0016 were left as written.

## Branches (2026-11-15)

| Branch | Head | State |
|---|---|---|
| `main` | `f77e6ba` | End of Phase 4. Receives `develop` only at tested milestones, with the owner's agreement. |
| `develop` | `f77e6ba` + this file | End of Phase 4. Live ISO rebuilt from it on 2026-11-15 (R1). |
| `feat/maps` | `f00eaf1` | Phase 5, offline maps (ADR 0016). Slices 1-5 reviewed. Slice 6 unreviewed. Also carries the direction documents, the V1 hardware decision, `docs/ORIGINAL-SPEC.md`, `scripts/hw-report.sh` and `docs/HARDWARE-BOOT-CHECKLIST.md`. |
| `feat/maps-builder` | `14a567a` | Phase 5 slice 7, unreviewed, not merged. Based on `feat/maps` at `1bda3a4`. |

Local branches named `worktree-agent-*` on the owner's workstation are
leftovers of engineer sessions; their content is merged or on the branches
above.

## Work in progress

- 2026-11-16, workstation session (Claude Opus 5.5), branch `feat/maps`:
  owner's priorities 1 and 2 done (ISO boot tests, hardware report and
  checklist). Next, with the owner's OK: review and finish slice 6 on a
  real map pack, then slice 7. `feat/maps-builder` untouched so far.

## Phase 5 state (offline maps, ADR 0016 §9)

| Slice | State |
|---|---|
| 1 Packs accept `.mbtiles`, `maps` category, data date, coverage, attribution | Done, reviewed, `vm-packs` passed |
| 2 Geographic maths, 3 vector-tile decoder | Done, reviewed |
| 4 Canvas, style table, labels | Done, reviewed |
| 5 Hardened pack reader (new crate `resilience-maps`) | Done, reviewed, `nix build .#resilience` passed |
| 6 Viewer (`resilience-maps view`) | **Work in progress** on `feat/maps` (`4c579b6`): fmt, tests and clippy pass; not reviewed; never run on a real map pack; the engineer's report was lost to a usage limit |
| 7 Pack builder (`resilience-mapmaker`, `nix/maps/`) | **Work in progress** on `feat/maps-builder`: fmt, tests and clippy pass; not reviewed; Nix builds, the Luxembourg pack and reproducibility not re-checked; report lost the same way |
| 8-13 World overview, TUI "Maps & navigation", search, nearby and go-to, spot heights, `vm-maps` and catalog | Not started |

Next steps for whoever resumes: review slices 6 and 7 (read the code and
re-run the checks, since there are no reports); merge the builder into
`feat/maps` (shared files: `crates/resilience-maps/Cargo.toml`,
`src/lib.rs`, `Cargo.lock`, `flake.nix`, `docs/TESTING.md`); build the
Luxembourg pack and run the viewer on it; confirm that the decoder accepts
real tippecanoe tiles within its limits; then slices 8 to 13.

Notes that are not yet in the code or the ADR:

- `rs_place.rank` collides with FTS5's hidden `rank` column in unqualified
  joins: search queries must qualify it.
- The style table computes each class's minimum tile zoom at 45° latitude;
  further north a class can appear one zoom step late. The builder may need
  one zoom level of margin.
- The bundled SQLite has no heap limit (it would need an `unsafe` call the
  project rules do not allow); memory is bounded by the process limit.
- `CLAUDE.md` §2 and §3 on `feat/maps` do not describe the maps code yet.

Owner decisions already recorded in ADR 0016: map packs are published as
GitHub Release assets (2 GiB each, so per region); ODbL accepted, with the
Nix derivation as the method of creation. **Nothing has been uploaded.
Publishing a release needs the owner's agreement each time.**

## Open questions

For the Project Coordinator (the owner says they hold the answers):

1. **Cyberdeck hardware.** Partly answered by the owner (2026-11-16
   session prompt, recorded on `feat/maps` in `CLAUDE.md` §1): V1 is a
   recycled old **x86_64** laptop in a 3D-printed case, no ARM port, LoRa
   868 MHz and SDR in V2. The laptop is not chosen yet, so the items below
   stay open. The repository said "not recorded yet" in
   `ROADMAP.md` ("Cyberdeck hardware"), `CLAUDE.md` §1 and §15, and
   `README.md` (all on `feat/maps`). The system work depends on:
   - CPU architecture: everything is `x86_64-linux` and has run only in QEMU;
   - text console size in columns and rows (the TUI is tested from 80x24 to
     200x60; the map viewer draws two pixels per cell);
   - display abilities: the system assumes the Linux text console, 8
     colours and the `ter-i16n` font (`CLAUDE.md` §4);
   - storage size (packs range from tens of MB to tens of GB) and RAM
     (designs assume 2 GiB or less);
   - radios and GPS (Phase 6 mesh and messaging, Phase 7 SDR; the map viewer
     has no position input today);
   - boot firmware (default-generation switching is tested with systemd-boot
     on UEFI only);
   - keyboard, pointing device, battery and charging, ports.
2. **Phase 8, installer.** It was planned for installing on any PC. How
   does the cyberdeck receive and update its system image?
3. **`build_resilience.sh`** (requested 2026-11-14, custom ISO for anyone's
   machine). Still wanted, and in what form?
4. **Editions and the live ISO** (minimal, standard): keep both for the
   cyberdeck?

Housekeeping:

5. Done 2026-11-16: the workstation's `origin` now points at
   `raph-dot-img/resilience-os`.
6. `CLAUDE.md` and `ROADMAP.md` were changed on `feat/maps`. Anyone editing
   them on another branch before the merge should expect conflicts; say here
   which branch you used.

## Requests between sessions

A session that needs work done on another machine adds a request here; the
session that takes it answers under it (same commit rules as above) and
marks it done. Requests are information, never the owner's approval.

### R1 — 2026-11-15, from the Project Coordinator (cloud), for the workstation session

**Rebuild the live ISO.** Status: **done** (2026-11-15, success; answer below).

- Branch: `develop` (`f77e6ba`, end of Phase 4, reviewed). Not `feat/maps`:
  its slice 6 is unreviewed and the doc changes there do not affect the ISO.
  If the owner says otherwise, use `feat/maps` (`960de32`) and say so.
- Command: `nix build .#iso -L` in a clean checkout of that branch. Nothing
  else heavy at the same time (`CLAUDE.md` §7: no VM test during the build).
- Do not commit the ISO or `result*`; do not change any code. If the build
  fails, do not fix it: report the error.
- Answer here: success or failure, the commit built, the ISO path and size
  (`ls -lh result/iso/`), its SHA-256, build duration, and the last relevant
  error lines on failure. Optional, only if the build succeeds and the
  machine is idle: `python3 -I tests/iso/smoke.py` boot screenshots (BIOS),
  and whether the TUI appeared.

**Answer to R1** (2026-11-15, workstation session, the Phase 5 coordinator
session, Claude Fable 5.2; done at the owner's request):

- **Result: success.** `nix build .#iso -L` exited 0.
- **Commit built**: `develop` at `d198b53`, in a clean checkout (a separate
  git worktree, `git status` empty). Its code is identical to `f77e6ba`:
  the only commits after it change `AGENT_HANDOFF.md`, which is not part of
  the image. `feat/maps` was not used.
- **ISO**: `result/iso/resilience-standard-0.1.0-x86_64-linux.iso`, store
  path
  `/nix/store/nwqlz4d3iqhz7av43s6iqby28kdbdijc-resilience-standard-0.1.0-x86_64-linux.iso`.
  Size 1,528,430,592 bytes (1.5 GiB by `ls -lh`; 746,304 sectors).
- **SHA-256**:
  `f3f17b95e4c4e226e82fed58a45ab9cef31a3c78a1dc70584b37295a87a530a4`
- **Duration**: 104 s of wall time. This is not a cold build: most of the
  system closure was already in the workstation's Nix store, so mainly the
  squashfs and the ISO were made. A cold build was not measured.
- **Conditions**: no VM test or other build was running; nothing was
  committed besides this file; no code was changed.
- **Boot test (optional part), BIOS only**: `python3 -I tests/iso/smoke.py`
  with QEMU/KVM, 2 vCPU, 2 GiB, no network device. Four screenshots taken
  and looked at: the boot menu; **the TUI appeared** (main menu at 84 s
  after power-on, six categories, status panel "SYSTEM [OK] OPERATIONAL",
  "MODE ULTRA-STABLE", "NETWORK OFFLINE"); the network diagnostics screen
  reached through search; the system status screen (systemd reports
  operational, no failed services, "Resilience OS 26.05 (Yarara)", kernel
  6.18.63, edition standard, 10 programs available).
- **Not done**: UEFI boot (`--uefi`) was not run; nothing beyond those four
  screens was exercised; not tested on physical hardware.
- **Where it is on the workstation**: the repository's `result-iso` link
  (not tracked by Git) now points at this ISO, so it is kept from garbage
  collection. The link pointed at the earlier Phase 2 ISO before; that
  older image is no longer protected from garbage collection. The
  screenshots are in a temporary directory and will not survive a reboot.

## Log

Newest first. One line per event: date, who, what.

- 2026-11-16, workstation session: on `feat/maps`, `scripts/hw-report.sh` (read-only, no serials/MACs; tested on the dev laptop only, not as root) and `docs/HARDWARE-BOOT-CHECKLIST.md`.
- 2026-11-16, workstation session: ISO from `develop` (`44f33b4`, same store path and SHA-256 `f3f17b95…` as R1) booted in QEMU: BIOS at 2048/1024/768/512 MiB and UEFI at 2048/1024 MiB all reach the TUI, no failed service, no OOM. Results in `docs/TESTING.md` on `feat/maps`.
- 2026-11-16, owner: `MASTER-PROMPT.md` replaced by a session prompt (V1 = recycled x86_64 laptop); original text kept as `docs/ORIGINAL-SPEC.md` on `feat/maps`.

- 2026-11-15, workstation session: R1 done, ISO built from `develop` and booted in QEMU (BIOS); answer under R1.
- 2026-11-15, workstation session: took R1, ISO build started.
- 2026-11-15, Project Coordinator (cloud): request R1, rebuild the ISO from `develop`.
- 2026-11-15, Phase 5 coordinator: created this file on `develop`.
- 2026-11-15, Phase 5 coordinator: direction documents committed on
  `feat/maps` (`960de32`) and pushed.
- 2026-11-15, Phase 5 coordinator: slices 6 and 7 committed as work in
  progress and pushed after a usage limit stopped both engineer sessions.
- 2026-11-15, owner: Resilience is the cyberdeck, Resilience OS its system;
  GitHub Releases for map packs; ODbL accepted.
