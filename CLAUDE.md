# CLAUDE.md — Resilience OS project memory

Read this whole file at the start of every session. Keep it accurate: update it
whenever architecture, build process, conventions or status change. The full
original specification is `MASTER-PROMPT.md` (authoritative for scope; this
file is authoritative for *current state and decisions*).

## 1. Identity and purpose

**Resilience OS** is an offline-first, low-resource, terminal-only Linux
distribution built on **NixOS** (x86_64). It must stay useful — information,
productivity, local communication, security tools — with no Internet at all.
Primary interface: a Rust TUI (`resilience`). No X11, Wayland, desktop or WM.

Guiding rule: *Resilience must not depend on the availability of the
infrastructure it is designed to survive without.*

Decision priorities (never reorder to go faster): security > correctness >
reliability > offline > maintainability > low resources > usability >
performance > visual polish.

## 2. Current architecture (summary — details in docs/ARCHITECTURE.md)

```
flake.nix ── nixosModules.default = nix/modules ── imported by every system
   │            core.nix        options: resilience.{edition,apps,probe,package...}
   │                            generates /etc/resilience/{system,apps}.toml
   │                            (build fails if an app command is not executable)
   │            admin.nix       privileged helper: /etc/resilience/admin.toml, polkit
   │                            action org.resilience.admin.set-mode (auth_admin),
   │                            tmpfiles for state/audit, activation `apply-state`
   │            users.nix       resilience.users.<n>.mode -> groups, login shell,
   │                            nix allowed-users
   │            interface.nix   getty branding, console font ter-i16n, tty1-only
   │                            autologin via getty@tty1 drop-in (NOT services.getty.autologinUser)
   │            security.nix    sudo (wheel+password), pam_faillock, sysctls, no ssh
   │            networking.nix  NetworkManager (no connectivity check), firewall, no NTP
   │            boot.nix power.nix applications/{productivity,system-tools,networking,entertainment}.nix
   ├─ nixosConfigurations: live-standard, live-minimal (ISO), dev-vm (9p VM)
   ├─ packages: resilience, resilience-session, resilience-handbook, iso, iso-minimal, vm
   └─ checks: resilience (cargo test), vm-boot-offline, vm-modes, vm-admin,
              security-invariants, eval-live-iso

crates/resilience-core  library: status, network (passive + on-demand probe),
                        mode detection, catalog, config, handbook, notes, launch, proc,
                        admin (pure mode-change rules), recovery (generations, report)
crates/resilience-admin binary `resilience-admin`: the ONLY privileged code (ADR 0007):
                        list | set-mode USER MODE | apply-state
crates/resilience-tui   binary `resilience`: app.rs (state machine, returns Effects),
                        ui.rs (rendering), theme.rs, help.rs, tests.rs
nix/packages/resilience-session  login shell: runs TUI, restarts after crash, refuses -c
handbook/*.md           offline user handbook shipped in the system
tests/vm/*.nix          NixOS VM tests (QEMU/KVM via nix build .#checks...)
```

Runtime flow: getty (tty1 autologin or login prompt) → login shell
`resilience-session` → `resilience` TUI → programs launched with fixed argv
(never via a shell) → back to TUI. tty2–tty6 always offer normal logins.

Mode change flow: TUI Users screen → consequences + confirm (Cancel default)
→ `pkttyagent` registered for the TUI pid + `pkexec <helper> set-mode U M`
(SHELL set to the session shell: pkexec rejects nologin) → admin password →
helper validates, writes /var/lib/resilience/modes.toml, gpasswd, audit log,
terminates sessions on demotion. Activation (`users` resets groups) →
`resilienceModes` script → `apply-state` re-applies overrides.

## 3. Directory structure

| Path | Purpose |
|---|---|
| `flake.nix`, `flake.lock` | Entry point; nixpkgs pinned to `nixos-26.05` |
| `Cargo.toml`, `Cargo.lock` | Rust workspace (edition 2024) |
| `crates/` | Rust code (see above) |
| `nix/modules/` | NixOS modules (one concern per file) |
| `nix/packages/` | Derivations for our own software |
| `nix/profiles/` | Editions: `minimal.nix`, `standard.nix` (extended: not yet) |
| `nix/systems/` | Concrete systems: `common.nix`, `live-iso.nix`, `dev-vm.nix` |
| `nix/overlays/` | Overlay exposing our packages (used by the flake's `pkgs`) |
| `handbook/` | Offline handbook pages (`NN-name.md`, sorted by prefix) |
| `tests/vm/` | NixOS VM integration tests |
| `scripts/` | Developer helper scripts (no root, ever) |
| `docs/` | All documentation; ADRs in `docs/decisions/` |

## 4. Technology choices

- NixOS 26.05 stable, flakes, single input (nixpkgs). No flake-utils/crane: fewer inputs.
- Rust stable (host: 1.98 via rustup; Nix build: nixpkgs rustc). ratatui 0.30 (crossterm
  backend, re-exported as `ratatui::crossterm` — do not add a separate crossterm dep),
  serde + toml, tracing, thiserror, libc. **No tokio**: threads + timeouts suffice.
- Only the 8 basic ANSI colours (Linux console has no 256/RGB). Non-ASCII
  glyphs limited to `▀▄█═║╔╗╚╝─│┌┐└┘░▒▓` (console font `ter-i16n`, CP437
  repertoire, 256 glyphs so the VT keeps 16 colours). Enforced by the test
  `only_console_font_glyphs_are_drawn`. ter-v16n lacks ▀▄ and double lines.
- See `docs/COMPONENTS.md` for every third-party component, version and license.

## 5. Build

```sh
cargo build && cargo test                 # fast Rust loop on the host
nix build .#resilience                    # Nix build (runs cargo test too)
nix flake check --no-build                # evaluate everything (seconds)
nix build .#checks.x86_64-linux.vm-boot-offline -L   # VM test (minutes)
nix build .#iso                           # live ISO -> result/iso/*.iso (heavy)
nix run .#vm                              # dev VM on the serial console
scripts/boot-iso.sh result/iso/*.iso      # boot ISO in QEMU (2 vCPU, 2 GiB)
```
Details: docs/BUILDING.md. The ISO build is the heaviest operation: never run
it concurrently with VM tests.

## 6. Tests

- VM tests read the screen through `/dev/vcsuN` (helper `tty_text`/`wait_tty`
  in each test): the driver's `get_tty_text()` mis-decodes CP437 glyphs.
  Long TUI texts wrap: wait for short fragments.
- pkexec in VM tests: run it under `setsid -w` when no agent is wanted,
  otherwise its text agent writes into the driver's terminal protocol.
- ISO: `python3 -I tests/iso/smoke.py ISO OUTDIR [--uefi OVMF.fd]` takes
  screenshots; look at them (the Read tool displays PNGs).
- Rust: `cargo test` (unit + TUI behaviour via ratatui TestBackend) and
  `cargo clippy --all-targets` must be clean. `RESILIENCE_DUMP=1 cargo test dump_screens -- --nocapture` prints screens.
- Nix/VM: `nix flake check` runs everything including VM tests.
- Report each test as PASS / FAIL / BLOCKED / NOT TESTED in docs/TESTING.md.
  Never mark untested things as passing.

## 7. Development environment constraints (HARD RULES)

Host: Fedora, Ryzen 5 PRO 4650U (6c/12t), 12 GB RAM, ordinary user.
Nix 2.35 multi-user daemon (user is NOT trusted; cache.nixos.org works;
`kvm` and `nixos-test` system features available; max-jobs 2, cores 2).

- Never use sudo/su/doas/pkexec, never modify the host system, its bootloader,
  firewall, networking, disks or anything outside this repo (and user-local
  caches). If root is required, write instructions for the user instead.
- Never install Resilience on the host. Destructive tests only in disposable VMs.
- ≤ 2 concurrent heavyweight jobs; never two VM tests or a VM test + ISO build
  at the same time. Default test VM: 2 vCPU, ≤ 2 GiB RAM.

## 8. Git conventions

Conventional commits (`feat:`, `fix:`, `security:`, `test:`, `docs:`,
`chore:`, `refactor:`). Main branch `main`; feature branches for larger work.
Never commit ISOs, disk images, `result*` links, large archives, keys or
secrets (see `.gitignore`). Never force-push, never push unless asked, never
run destructive git operations without explicit authorization.
Commit trailers: `Co-Authored-By: Claude …` as instructed by the session.

## 9. Security requirements (summary — docs/THREAT_MODEL.md, SECURITY.md)

- Modes are OS privileges (groups), never UI flags. Hidden menus are not a boundary.
- The TUI never builds shell commands; programs run from fixed argv.
- No privileged command runner. Privileged actions go only through
  `resilience-admin`: one sub-command + one polkit action (auth_admin, never
  _keep) per operation, every argument validated in the helper, absolute tool
  paths from root-owned admin.toml, environment ignored, every outcome
  audited (/var/log/resilience/audit.log + journal). Add new operations the
  same way (pure rule in resilience-core/admin.rs + unit tests + vm-admin).
- No secrets in Git, Nix expressions, the Nix store, logs or world-readable temp files.
- No cryptography of our own. Use age, minisign, GnuPG, cryptsetup, OpenSSL.
- No background network traffic. The Internet probe runs only on user request,
  with timeouts, and is skipped without a default route.

## 10. NixOS module conventions

- One concern per module file; every option under `resilience.*` with a
  description. Use `lib.mkDefault` for policy defaults users may override;
  hard security settings are plain definitions.
- Modules must work with read-only `pkgs` (NixOS tests): **never set
  `nixpkgs.overlays` in a module**; default packages via `pkgs.callPackage`.
- Applications: declare `resilience.apps.<id>` with absolute store paths
  (`lib.getExe pkg` if `meta.mainProgram` is set, else `"${pkg}/bin/x"`).
  Verify the package exists in the pinned nixpkgs before adding it.
  Provide `restrictedCommand` when a safer mode exists for Ultra-Stable users.
- Format with `nix fmt` (nixfmt).

## 11. Rust conventions

- `resilience-core` has no UI code; every system reader has a pure parser
  with unit tests. `unsafe` only for libc calls, each with a `// SAFETY:` comment.
- No `unwrap()` outside tests (clippy warns); `expect` only for invariants.
- Anything that touches the network or another process has a timeout.
- The TUI state machine (`App::handle_key`) returns `Effect`s; only `main.rs`
  touches the terminal. Add a test in `tests.rs` for every new screen/flow.
- `cargo fmt`, `cargo clippy --all-targets` clean before committing.

## 12. Resource management

See §7. Prefer `nix flake check --no-build` and `cargo test` for fast loops;
build the ISO only for milestones. Reuse the store; don't garbage-collect it
needlessly. Watch `free -h` during VM tests.

## 13. Application integration guidelines

1. Verify upstream, license, maintenance, nixpkgs presence (pinned rev),
   no graphical dependencies, offline behaviour. Record in docs/COMPONENTS.md.
2. Add it to the right `nix/modules/applications/*.nix` group with a beginner
   description that says how to quit it.
3. Choose `minMode`: anything that can spawn a shell or edit arbitrary files
   is `stable` unless a restricted variant exists.
4. The build-time catalog check ensures the binary exists. Add a VM test
   assertion if the app is important.

## 14. Current implementation status (2026-11-14)

Phase 0 (foundation) and Phase 1 (core) done; Phase 2 mostly done (branch
`feat/phase2-admin-helper`): privileged helper with set-mode, TUI Users &
security modes, TUI Recovery & diagnostics, faillock, ADR 0007. Remaining in
Phase 2: user-data backups, more helper operations. See ROADMAP.md.
Done: flake, modules, editions (minimal/standard), live ISO config, dev VM,
TUI (main menu, status panel, categories, search, help, settings/themes,
notes, handbook, network diagnostics, system status, notifications, power and
logout with confirmation), session wrapper with crash recovery, group-based
modes with Nix-daemon and sudo restrictions, VM tests.
See docs/TESTING.md for what has actually been executed and passed.

## 15. Known bugs and limitations

- No installer yet; the ISO is live-only and has no admin account (by design).
- Mode changes need an Advanced account to authorise; the live ISO has none,
  so modes cannot be changed there (by design).
- pkexec's built-in text agent fails on success with NixOS 26.05's
  socket-activated polkit helper ("No session for cookie"); always register
  `pkttyagent` first (launch::run_with_polkit_agent).
- Making an older generation the permanent default is not implemented
  (boot menu only). No user-data backup tool yet.
- Ultra-Stable restrictions on launched programs rely on restricted modes of
  those programs; they are usability measures, not a sandbox.
- Emergency/rescue shell requires root, which is locked, by design: layered
  recovery (TUI Recovery, other ttys + Advanced/sudo, generations, live ISO).
- No resource packs, Kiwix, maps, crypto workflows, comms or SDR yet.
- Power profiles not implemented (schedutil governor only). Not measured yet.
- Not tested on physical hardware.

## 16. Roadmap

See ROADMAP.md. Next: finish Phase 2 (encrypted user-data backups with
removable media, Wi-Fi/time helper operations), then Phase 3 (Kiwix, resource packs).

## 17. Important decisions (docs/decisions/)

- 0001 NixOS 26.05 stable + flakes, single nixpkgs input
- 0002 Rust TUI with ratatui, synchronous design, no tokio
- 0003 Security modes = per-account group membership
- 0004 Offline-first networking: passive detection, explicit probe only
- 0005 Application catalog generated by Nix, verified at build time
- 0006 Session model: login shell wrapper, autologin only for ultra-stable
- 0007 Privileged helper (pkexec + typed ops), persistent mode overrides, audit, recovery layers

## 18. How to continue safely

1. Read this file, `git status`, `git log --oneline -20`, ROADMAP.md, docs/TESTING.md.
2. Run `cargo test` and `nix flake check --no-build` before changing anything.
3. Work in small vertical slices; add tests; update docs and this file.
4. Commit coherent milestones. Never weaken a test to make it pass.
