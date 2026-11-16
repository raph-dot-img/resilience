# CLAUDE.md — Resilience OS project memory

Read this whole file at the start of every session. Keep it accurate: update it
whenever architecture, build process, conventions or status change. The full
original specification is `docs/ORIGINAL-SPEC.md` (authoritative for the
feature scope of the system; this file is authoritative for *project
direction, current state and decisions*). It was written before the change
of direction in §1 and still describes a general-purpose distribution:
where the two disagree, §1 wins. ADRs that cite "MASTER-PROMPT.md §N" mean
that file: on 2026-11-16 the owner replaced `MASTER-PROMPT.md` with a short
session prompt (priorities and rules for agent sessions), and the original
text was moved to `docs/ORIGINAL-SPEC.md` unchanged.

## 1. Identity and purpose

**Project direction (decided by the owner on 2026-11-15).** Two names, two
things:

- **Resilience** is the **cyberdeck project**: a standalone, offline-first
  laptop that we are building. It is the product.
- **Resilience OS** is the **system that runs on that cyberdeck**: NixOS,
  Rust, terminal-only. It is one part of Resilience, and this repository
  holds it.

Resilience OS is **no longer a general-purpose operating system for any
x86-64 machine**. That was the original idea, and older documents, ADRs and
`docs/ORIGINAL-SPEC.md` are written that way. What this means for a session:

- Design for one known machine, the cyberdeck, and not for unknown
  hardware. When a choice depends on the hardware (screen and console size,
  keyboard, storage, battery and power, radios, boot firmware), ask the
  owner or look for the hardware specification; do not assume generic PC
  hardware and do not invent the cyberdeck's characteristics.
- **V1 hardware (owner decisions, 2026-11-15 and 2026-11-16):** the first
  cyberdeck is a **recycled Lenovo ThinkPad E540**, rebuilt into a
  3D-printed case: Intel Core **i3-4000M** (Haswell, x86_64), **4 to 8 GB of
  RAM**, a **16 GB SSD** (upgradeable; see ROADMAP "Cyberdeck hardware" for
  the size estimate). Resilience OS stays **x86_64 only**: no ARM port. LoRa
  868 MHz and SDR come in **V2**, not V1. Keyboard: **French AZERTY** by
  default (`resilience.keyboard.layout`).
  **To confirm with `scripts/hw-report.sh` on the machine itself** (not
  verified, do not assume): screen resolution and therefore console size,
  Wi-Fi/Bluetooth card and its firmware, boot firmware (UEFI/CSM, Secure
  Boot), exact RAM and SSD, battery. Resilience OS has booted on one real
  laptop so far (a ThinkPad L15 Gen 1, not the E540).
- Work whose purpose was "runs on any PC" (broad hardware support, a
  generic installer, images for other people's machines) is not a goal on
  its own any more. Before starting such work, ask whether the cyberdeck
  needs it. Affected roadmap items are marked in ROADMAP.md.
- The principles below (offline-first, security modes, low resources,
  terminal-only) are unchanged.
- Use the names as defined: "Resilience" for the cyberdeck or the project
  as a whole, "Resilience OS" for the system. Program and crate names
  (`resilience`, `resilience-admin`, …) do not change.

**Resilience OS** is an offline-first, low-resource, terminal-only system
built on **NixOS** (x86_64 so far). It must stay useful — information,
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
   │            admin.nix       privileged helper: /etc/resilience/admin.toml, polkit actions
   │                            org.resilience.admin.{set-mode,set-time,set-default-generation} (auth_admin),
   │                            tmpfiles for state/audit, activation `apply-state`
   │            media.nix       udisks2 (removable media; its polkit defaults, no overrides)
   │            crypto.nix      gpg-agent (pinentry-curses, cache TTL 0); age/minisign on PATH
   │            packs.nix       resource packs (ADR 0013): /etc/resilience/packs.toml (catalog with
   │                            pinned size+SHA-256, default nix/packs/kiwix.nix), system user
   │                            resilience-packs owns /var/lib/resilience/packs, polkit action
   │                            org.resilience.packs.change (auth_admin), lynx restrictions
   │            users.nix       resilience.users.<n>.mode -> groups, login shell,
   │                            nix allowed-users
   │            interface.nix   resilience.keyboard.layout (console keymap, default "fr",
   │                            checked at build), getty branding, console font ter-i16n, tty1-only
   │                            autologin via getty@tty1 drop-in (NOT services.getty.autologinUser)
   │            security.nix    sudo (wheel+password), pam_faillock, sysctls, no ssh,
   │                            user namespaces off on Ultra-Stable-only machines (ADR 0012)
   │            networking.nix  NetworkManager (no connectivity check), polkit allowlist for
   │                            accounts without its group (ADR 0010), firewall, no NTP
   │            boot.nix power.nix applications/{productivity,system-tools,networking,entertainment}.nix
   ├─ nixosConfigurations: live-standard, live-minimal (ISO), dev-vm (9p VM)
   ├─ packages: resilience, resilience-session, resilience-handbook, iso, iso-minimal, vm
   └─ checks: resilience (cargo test), vm-boot-offline, vm-modes, vm-admin,
              vm-backup, vm-wifi, vm-generations, vm-packs, vm-documents, vm-crypto,
              security-invariants, eval-live-iso

crates/resilience-core  library: status, network (passive + on-demand probe),
                        mode detection, catalog, config, handbook, notes, launch, proc,
                        admin (pure mode-change rules), clock (time parsing/bounds),
                        recovery (generations, default-generation rules, report),
                        media (lsblk parsing, udisksctl mount/eject), files (name
                        index, kinds, text caps, CSV), manpage (names, secure env), crypto (key store,
                        key/format recognition, output names, checksums), openpgp
                        (gpg status/colons parsers), backup (tar|zstd|age
                        pipeline, checksum, restore into ~/Restored), wifi (nmcli parsing,
                        private profiles, password via stdin), packs (catalog, inventory,
                        stdin chunk framing, receive+hash, remove, reader URLs)
crates/resilience-admin binary `resilience-admin`: the ONLY privileged code (ADR 0007):
                        list | set-mode USER MODE | set-time UTC |
                        set-default-generation N | apply-state
crates/resilience-backup binary `resilience-backup` (UNPRIVILEGED, ADR 0008):
                        create MOUNT | check FILE | restore FILE
crates/resilience-library binaries (ADR 0013): `resilience-library` (as the user: list |
                        change [--from M] [--remove F].. [--get ID].. | open FILE [--search W])
                        and `resilience-pack-store` (as resilience-packs via pkexec --user,
                        never root: change; stdin bytes only hashed, never parsed)
crates/resilience-crypto binary `resilience-crypto` (UNPRIVILEGED, ADR 0015, foreground):
                        encrypt TO FILE | decrypt | write-message TO | read-message | sign |
                        verify | checksum | compare SUM FILE | check-list | inspect | new-keys |
                        add-contact [FILE] | export-keys DIR | new-openpgp-key;
                        src/pgp.rs = GnuPG paths (status-fd parsers in core openpgp.rs)
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

Backup flow: TUI Backups & USB sticks → lsblk lists removable drives only →
`udisksctl mount --no-user-interaction` (internal disks need auth → refused)
→ confirm → `resilience-backup create /run/media/U/X` in the foreground (age
reads the passphrase from /dev/tty itself) → `*.partial` renamed when done +
`.sha256`. Restore: checksum first, then into a NEW ~/Restored/backup-<stamp>.
Tool paths: `[tools]` in /etc/resilience/system.toml (checked at build).

Default generation flow: TUI Recovery → System generations (list) → Enter →
confirm (Cancel default) → pkttyagent + `pkexec <helper> set-default-generation
N` → helper: plan_set_default (exists, not default, closure in /nix/store with
switch-to-configuration) → `nix-env -p …/system --switch-generation N` →
`<gen>/bin/switch-to-configuration boot` (clean env; profile switched back on
failure) → audit. Never `switch`: the running system is untouched.

Resource packs flow: TUI Knowledge → Resource packs (Space ticks [+]/[-], f
language, d download, u USB stick) → confirm → `resilience-library change …`
in the foreground → pkttyagent + `pkexec --user resilience-packs
<store> change …` (one admin password) → store prints `send ID OFFSET`/`skip`
lines → library streams curl (HTTPS only, --continue-at) or the stick file as
[u32 len][bytes] chunks → store appends to .staging/F.part, sha256sum over
old+new bytes, rename only on match; early end keeps the part (resume).
Sets: `s` on Resource packs ticks a suggested set. Library also lists built-in
packs (`[[builtin]]`, the docs ZIM from nix/packages/resilience-docs) and packs on
mounted sticks (`u` opens one, `c` → `resilience-library check FILE`).
Reading: Offline library → `resilience-library open FILE` → kiwix-serve
127.0.0.1:<free port> + lynx with every restriction (`-restrictions=…`, not `all`) and `-localhost`.

Crypto flow (ADR 0015): TUI Security entries → Find & view files as picker
(`View::Files { pick: Some(Pick) }`) → "Encrypt for" checklist (passphrase | age
keys | OpenPGP keys, one kind) or checksum box → `resilience-crypto OP … FILE`
in the foreground → age / minisign / gpg (pinentry-curses, GPG_TTY) read
passphrases on the tty themselves; outputs `.partial` + RENAME_NOREPLACE;
decrypted files only in ~/Decrypted; keys in ~/.local/share/resilience/keys
(age identity = age-keygen | age -p, never in clear). Random passwords: TUI
runs xkcdpass/openssl and shows them in a dialog (never notified/logged).
Invariants kept from the ADR 0015 review (do not regress): a non-detached
OpenPGP signature `NAME.asc` beside `NAME` is refused (never VALID for NAME);
clear-signed text is shown exactly as signed; gpg always `--batch
--no-auto-key-retrieve --no-auto-key-locate`, file paths absolute, user id
after `--`, names start alphanumeric; gpg output into memory is bounded
(`pgp::capture`); the binary's SIGINT/SIGTERM/SIGHUP handler unlinks the
current `.partial` (one at a time, `PARTIAL` atomic) and stale partials of
dead PIDs are removed at start; checksum tools run with LC_ALL=C.

Wi-Fi flow (as the user): nmcli list → password in a TUI dialog → profile
with `connection.permissions user:U`, no secret → thread: `nmcli connection
up uuid X passwd-file /dev/stdin` (secret on a pipe) → failure deletes a
new profile. Never `nmcli --ask` (wrong password = inescapable prompt loop).

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
- Wi-Fi in VM tests: `mac80211_hwsim radios=2`, hostapd+dnsmasq on wlan0
  (NM unmanaged, firewall UDP 67 on wlan0), `networking.wireless.enable =
  mkOverride 0 true` (qemu-vm.nix forces it off), `dmesg -n 1` (kernel
  messages draw over the TUI). See vm-wifi.
- polkit checks must use a process in a real (active) session, e.g. the
  TUI's pid with `pkcheck --process`; `runuser` has no session and gets "no"
  for unrelated reasons.
- Never grep for a secret (or pgrep for a command) with a pattern that
  appears literally in the grep's own command line: `grep 'secre[t]'`.
- pkttyagent asks "Choose identity" when several Advanced accounts exist;
  VM tests must answer it (see `authenticate_as_carol` in vm-admin).
- Generations in VM tests: `virtualisation.useBootLoader` + `useEFIBoot` +
  systemd-boot; a second toplevel via `extendModules` (guarded by a test
  option so it does not nest) in `system.extraDependencies`, installed with
  `nix-env --set` + `switch-to-configuration boot`; `machine.shutdown()` +
  `start()` keeps the disk. TUI lists wrap around: use End/Home, not
  repeated Down. See vm-generations.
- Resource packs in VM tests: never commit `.zim` files (gitignored); build a
  test ZIM with `zimwriterfs` at build time and append its catalog entry to
  `config.resilience.packs.configFile` (mkForce the etc file). A second node
  serves HTTPS with a test certificate (`security.pki.certificateFiles`) and
  `limit_rate` to interrupt a download. See vm-packs.
- Crypto in VM tests: type passphrases into age/minisign/pinentry prompts on
  tty1 (`type_secret`); grep for secrets with `grep -F -f FILE` (patterns in a
  file, never in grep's argv). In Nix `''` strings an empty shell argument
  must be written `""`, not `''`. Peer keys (Bob) are made by root with
  `gpg --homedir /root/bob --batch --pinentry-mode loopback`. See vm-crypto.
- pkexec in VM tests: run it under `setsid -w` when no agent is wanted,
  otherwise its text agent writes into the driver's terminal protocol.
- USB sticks in VM tests: `-device qemu-xhci,id=xhci` + QEMU monitor
  `drive_add`/`device_add usb-storage` (new ids per plug), see vm-backup.
- The age round-trip unit test needs age in PATH (Nix build has it; on the
  host prepend the store path of `nixpkgs#age`, otherwise it prints SKIPPED).
- ISO: `python3 -I tests/iso/smoke.py ISO OUTDIR [--uefi OVMF.fd] [--keymap fr|us]`
  takes screenshots; look at them (the Read tool displays PNGs).
- Keyboard: the system default is AZERTY (`resilience.keyboard.layout =
  "fr"`), but the NixOS test driver types US key codes, so `flake.nix`
  sets `defaults.resilience.keyboard.layout = "us"` for every VM test node.
  smoke.py translates typed text for the ISO's layout (`--keymap`, default fr).
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
`chore:`, `refactor:`). Branches: `develop` is the development branch (work
happens there, or on `feat/<topic>` branches created from `develop` and merged
back into it); `main` only receives `develop` at tested milestones. Both are
on `origin` (GitHub); push only when the user asks.
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

## 14. Current implementation status (2026-11-15)

Direction changed on 2026-11-15: Resilience is the cyberdeck, Resilience OS
its system (§1). Only documentation was changed for it; no code.

Phase 5 (offline maps, ADR 0016) is in progress on branch `feat/maps`
(pushed): slices 1 to 5 of ADR 0016 §9 are done and reviewed (packs accept
`.mbtiles`, geographic maths, MVT decoder, canvas/style/labels, hardened
pack reader in the new crate `resilience-maps`); slice 6 (viewer) is
committed there as work in progress, not reviewed and never run on a real
pack; slice 7 (pack builder `resilience-mapmaker`, `nix/maps/`) is on branch
`feat/maps-builder` as work in progress, not reviewed and not merged.
Slices 8 to 13 are not started. The rest of this section and §2 describe
the state at the end of Phase 4 and do not list the maps code yet.


Phase 0 (foundation), Phase 1 (core), Phase 2, Phase 3 and Phase 4 done
(Phase 3: resource packs, sets, built-in docs, packs on sticks, ADR 0013;
Find & view files and manual pages, ADR 0014. Phase 4: guided cryptography
with age, minisign, checksums, random data and GnuPG interop, ADR 0015).
A future `build_resilience.sh` (interactive custom ISO) is described in ROADMAP. Phase 2:
privileged helper with set-mode, TUI Users & security modes, TUI Recovery &
diagnostics, faillock (ADR 0007); removable media + encrypted backups (ADR
0008); Date & time via `set-time` (ADR 0009); Wi-Fi for every account and
the NetworkManager polkit allowlist (ADR 0010); making an earlier generation
the default via `set-default-generation` (ADR 0011); user namespaces off on
Ultra-Stable-only machines (ADR 0012). See ROADMAP.md.
Done: flake, modules, editions (minimal/standard), live ISO config, dev VM,
TUI (main menu, status panel, categories, search, help, settings/themes,
notes, handbook, network diagnostics, system status, notifications, power and
logout with confirmation), session wrapper with crash recovery, group-based
modes with Nix-daemon and sudo restrictions, VM tests.
See docs/TESTING.md for what has actually been executed and passed.
Git state (2026-11-15): `main` = `develop` = end of Phase 4, both pushed to
origin; last full `nix flake check -L --max-jobs 1` passed (9 VM tests,
26 invariants × 3 configurations). Rust: 121 core, 3 library, 1 store,
60 TUI tests; clippy clean. The ISO was not rebuilt since Phase 2.

## 15. Known bugs and limitations

- No installer yet; the ISO is live-only and has no admin account (by design).
- Mode changes need an Advanced account to authorise; the live ISO has none,
  so modes cannot be changed there (by design).
- pkexec's built-in text agent fails on success with NixOS 26.05's
  socket-activated polkit helper ("No session for cookie"); always register
  `pkttyagent` first (launch::run_with_polkit_agent).
- Default generation: tested with systemd-boot (UEFI) only; GRUB untested.
  Deleting old generations is not offered. Backups are full copies (no incremental), no rotation;
  LUKS-encrypted sticks are not supported.
- Wi-Fi: no hidden, 802.1X or WEP networks; a wrong password is reported
  only after the 45 s timeout (NetworkManager keeps retrying with it).
- Ultra-Stable restrictions on launched programs rely on restricted modes of
  those programs; they are usability measures, not a sandbox.
- Emergency/rescue shell requires root, which is locked, by design: layered
  recovery (TUI Recovery, other ttys + Advanced/sudo, generations, live ISO).
- Resource packs: installing needs an administrator (none on the live ISO;
  packs can be read from a stick there); Kiwix deletes old files,
  so ageing catalog entries become USB-only. No maps, comms or SDR yet.
- Cryptography: no LUKS sticks, no OpenPGP signing in the TUI (minisign
  signs), no secure deletion of originals (the user is told); contacts'
  keys are only as trustworthy as the user's own check.
- Power profiles not implemented (schedutil governor only). Not measured yet.
- Not tested on physical hardware, and in particular never on the cyberdeck,
  whose hardware is not recorded here yet (§1).

## 16. Roadmap

See ROADMAP.md. Current: Phase 5 (offline maps), see §14. The roadmap is
read in the light of §1: the target is the cyberdeck.

## 17. Important decisions (docs/decisions/)

- 0001 NixOS 26.05 stable + flakes, single nixpkgs input
- 0002 Rust TUI with ratatui, synchronous design, no tokio
- 0003 Security modes = per-account group membership
- 0004 Offline-first networking: passive detection, explicit probe only
- 0005 Application catalog generated by Nix, verified at build time
- 0006 Session model: login shell wrapper, autologin only for ultra-stable
- 0007 Privileged helper (pkexec + typed ops), persistent mode overrides, audit, recovery layers
- 0008 Removable media via udisks2 (no overrides), unprivileged age-encrypted backups, restore into a new folder
- 0009 Clock set via helper `set-time` (own auth_admin action, bounds, audit) → timedatectl
- 0010 Wi-Fi as the user via nmcli (private profiles, password on stdin); NM polkit allowlist without its group
- 0011 Default generation via helper `set-default-generation` (nix-env + that generation's switch-to-configuration boot)
- 0012 User namespaces: off (max_user_namespaces=0, no Nix sandbox) when every account is Ultra-Stable
- 0013 Resource packs: hash-pinned catalog, unprivileged store via pkexec --user, kiwix-serve + restricted lynx
- 0014 Find & view files inside the TUI (read-only viewers); man in less secure mode
- 0015 Guided cryptography: resilience-crypto (age, minisign, coreutils, xkcdpass, openssl, GnuPG), secrets only typed into the tools

## 18. How to continue safely

1. Read this file, `git status`, `git log --oneline -20`, ROADMAP.md, docs/TESTING.md.
2. Run `cargo test` and `nix flake check --no-build` before changing anything.
3. Work in small vertical slices; add tests; update docs and this file.
4. Commit coherent milestones. Never weaken a test to make it pass.
