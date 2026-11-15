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
| `develop` | `f77e6ba` + this file | End of Phase 4. |
| `feat/maps` | `960de32` | Phase 5, offline maps (ADR 0016). Slices 1-5 reviewed. Slice 6 unreviewed. Also carries the direction documents. |
| `feat/maps-builder` | `14a567a` | Phase 5 slice 7, unreviewed, not merged. Based on `feat/maps` at `1bda3a4`. |

Local branches named `worktree-agent-*` on the owner's workstation are
leftovers of engineer sessions; their content is merged or on the branches
above.

## Work in progress

- 2026-11-15, repository session "Phase 5 coordinator" (Claude Fable 5.2,
  with Claude Opus 5.5 engineers), branches `feat/maps` and
  `feat/maps-builder`: **paused**, the workstation was powered off. Nobody
  is working on these branches until a session says so here.

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

1. **Cyberdeck hardware.** The repository says "not recorded yet" in
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

5. GitHub reports that the repository moved to
   `raph-dot-img/resilience-os`; the workstation's `origin` URL still uses
   the old name (pushes are redirected).
6. `CLAUDE.md` and `ROADMAP.md` were changed on `feat/maps`. Anyone editing
   them on another branch before the merge should expect conflicts; say here
   which branch you used.

## Log

Newest first. One line per event: date, who, what.

- 2026-11-15, Phase 5 coordinator: created this file on `develop`.
- 2026-11-15, Phase 5 coordinator: direction documents committed on
  `feat/maps` (`960de32`) and pushed.
- 2026-11-15, Phase 5 coordinator: slices 6 and 7 committed as work in
  progress and pushed after a usage limit stopped both engineer sessions.
- 2026-11-15, owner: Resilience is the cyberdeck, Resilience OS its system;
  GitHub Releases for map packs; ODbL accepted.
