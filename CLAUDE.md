# CLAUDE.md — Resilience OS project memory

Read this whole file at the start of every session. Keep it accurate: update it
whenever architecture, build process, conventions or status change. The full
original specification is `docs/ORIGINAL-SPEC.md` (authoritative for the
feature scope; this file is authoritative for *current state and
decisions*). Older commits cite it as "MASTER-PROMPT §N"; the sections
are numbered the same.

## 1. Identity and purpose

**Resilience OS** is an offline-first, low-resource, terminal-only Linux
distribution for **x86_64 PCs**, built on **NixOS**. It must stay useful — information,
productivity, local communication, security tools — with no Internet at all.
Primary interface: a Rust TUI (`resilience`). No X11, Wayland, desktop or WM.
Editions: **standard** is the main one (the live ISO we build and test);
**minimal** is kept as a secondary edition.

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
   │                            org.resilience.admin.{set-mode,set-time,set-default-generation} (auth_admin)
   │                            and org.resilience.admin.set-keymap (allow_active=yes, ADR 0017),
   │                            tmpfiles for state/audit, activation `apply-state`
   │            media.nix       udisks2 (removable media; its polkit defaults, no overrides)
   │            crypto.nix      gpg-agent (pinentry-curses, cache TTL 0); age/minisign on PATH
   │            packs.nix       resource packs (ADR 0013): /etc/resilience/packs.toml (catalog with
   │                            pinned size+SHA-256, default nix/packs/kiwix.nix), system user
   │                            resilience-packs owns /var/lib/resilience/packs, polkit action
   │                            org.resilience.packs.change (auth_admin), lynx restrictions
   │            users.nix       resilience.users.<n>.mode -> groups, login shell,
   │                            nix allowed-users
   │            interface.nix   resilience.keyboard.{layout (system keymap, default "us"), layouts
   │                            (offered in Settings)}, all checked against kbd at build; getty@
   │                            ExecStartPre loads the system layout before every login prompt;
   │                            getty branding, console font ter-i16n, tty1-only
   │                            autologin via getty@tty1 drop-in (NOT services.getty.autologinUser)
   │            security.nix    sudo (wheel+password), pam_faillock, sysctls, no ssh,
   │                            user namespaces off on Ultra-Stable-only machines (ADR 0012)
   │            networking.nix  NetworkManager (no connectivity check), polkit allowlist for
   │                            accounts without its group (ADR 0010), firewall, no NTP
   │            comms.nix       local messaging network (ADR 0018): resilience.comms.enable
   │                            (default off; the STANDARD profile turns it on)
   │                            -> resilience-reticulum.service (rnsd, DynamicUser, AutoInterface
   │                            only, UDP 29716+42671), /run/resilience-reticulum/client/config
   │                            (instance name + RPC key, world-readable by design), and the
   │                            systemd USER service resilience-messages (nix/packages/
   │                            resilience-messages: Python around LXMF.LXMRouter; per-account
   │                            identity + messages in ~/.local/share/resilience/messages;
   │                            outbox/ID.json + cancel/ID in, messages/ID.json out). rns and lxmf
   │                            are UNFREE (Reticulum License): allowed by name only in
   │                            nix/packages/resilience-reticulum (its own nixpkgs import)
   │            boot.nix power.nix applications/{productivity,system-tools,networking,entertainment}.nix
   ├─ nixosConfigurations: live-standard, live-minimal (ISO), dev-vm (9p VM)
   ├─ packages: resilience, resilience-session, resilience-handbook, resilience-world-map, iso, iso-minimal, vm
   ├─ legacyPackages.mapPacks.{luxembourg,testland}: map packs (nix/maps, build machines only)
   └─ checks: resilience (cargo test), vm-boot-offline, vm-modes, vm-admin,
              vm-backup, vm-wifi, vm-generations, vm-packs, vm-documents, vm-crypto, vm-maps, vm-comms,
              security-invariants, eval-live-iso, map-pack-testland, map-pack-reproducible, map-world,
              comms-messages (the messaging program with real Reticulum + LXMF, no VM)

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
                        stdin chunk framing, receive+hash, remove, reader URLs), keyboard
                        (layout list, names, loadkeys argv), messages (local messages store:
                        bounded JSON records, peers, contacts, outbox/cancel files, text
                        cleaning; ADR 0018), maps/ (geo, mvt decoder, canvas,
                        style, labels: pure, ADR 0016)
crates/resilience-admin binary `resilience-admin`: the ONLY privileged code (ADR 0007):
                        list | set-mode USER MODE | set-time UTC |
                        set-default-generation N | set-keymap LAYOUT | apply-state
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
crates/resilience-maps  binaries (ADR 0016, UNPRIVILEGED): `resilience-maps` (view [--pack F]..
                        [--at LAT,LON] [--scale M] | info [--check] F: hardened MBTiles reader,
                        reader thread, tile cache, 1 GiB address-space limit) and
                        `resilience-mapmaker` (classify | natural-earth | finish; build
                        machines only, not installed in the system).
                        nix/packages/resilience-world-map: the built-in world overview
                        (Natural Earth, z0-7), a `[[builtin]]` maps entry of packs.toml
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

Keyboard flow (ADR 0017): boot and every login prompt load the system
layout (console.keyMap; getty@ ExecStartPre). TUI Settings → "Keyboard
<layout>" → `pkexec <helper> set-keymap ID` (SHELL = session shell; polkit
allows the active local session without a password) → helper: ID in
admin.toml `keymaps` → `loadkeys -q -- ID` → audit → TUI saves `keyboard =
"ID"` in the user's settings. Session start: the TUI loads the saved
layout again the same way (unless it is the system layout). The console
has ONE keymap for all ttys: unprivileged loadkeys fails (CAP_SYS_TTY_CONFIG).

Maps flow (ADR 0016): TUI Maps & navigation → "Map" → `resilience-maps view
--pack F...` in the foreground as the user (`[tools] maps`): built-in maps
(`[[builtin]]` with category maps: the world overview), then installed
catalog map packs, at most 8. "Maps on this computer & USB sticks" =
`View::Library { kind: PackKind::Map }` (the Offline library's list for
`.mbtiles`): Enter opens the map on the chosen file over the overview, `c`
→ `resilience-library check`. The viewer opens SQLite hardened, lowers its
memory limit, writes only ~/.local/state/resilience/maps.toml.

Local messages flow (ADR 0018, only with resilience.comms.enable): TUI
Communications → Local messages (shown when /run/resilience-reticulum/
client/config exists) reads ~/.local/share/resilience/messages every 2 s
(`resilience_core::messages::Store`: records, peers, contacts, status) and
writes `outbox/ID.json` (tmp + rename), `cancel/ID`, `contacts.json`. The
user service `resilience-messages` takes the request, sends through the
machine's Reticulum instance and rewrites `messages/ID.json` (queued →
sent → delivered only on LXMF's proof; failed). The TUI never touches the
network; names from the network are shown as claims (`"name"?`).

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
| `nix/maps/` | Map pack derivations (osmium, tippecanoe, mapmaker), checks, test OSM data |
| `nix/packs/` | Resource pack catalog (Kiwix) |
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
- Resilience's own code: **Hippocratic License 2.1** (`LICENSE`, owner's decision
  2026-11-16; SPDX `Hippocratic-2.1` in Cargo.toml; Nix `meta.license = import
  nix/lib/license.nix`, which leaves `free` unset so nixpkgs still builds it).

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
- ISO: `python3 -I tests/iso/smoke.py ISO OUTDIR [--uefi OVMF.fd] [--keymap us|fr]`
  takes screenshots; look at them (the Read tool displays PNGs). `--keymap`
  is the ISO's layout (default us): QEMU sends physical keys.
- Keyboard: the NixOS test driver types US key codes, which matches the
  system default (`us`). A test that switches the layout must check the
  kernel keymap with `dumpkeys -k` (keycode 16 = q on US, a on AZERTY) and
  type nothing afterwards in that layout. See vm-boot-offline.
- The NixOS test driver type-checks the test script (`Optional` results
  must be checked before use; `machine.sleep` takes an int).
- Map viewer on a real pack: `nix build .#mapPacks.luxembourg`, then a VM
  with `boot.kernelParams = [ "video=1366x768" ]` and `-vga std` gives a
  170x48 console. QEMU's screendump of a 1366-wide framebuffer is skewed
  (1360-pixel PNG): re-stride the pixels to 1366 to look at it.
- World overview: `nix build .#resilience-world-map`, then
  `RESILIENCE_WORLD_PACK=$(echo result/share/resilience/packs/*.mbtiles) cargo test -p
  resilience-maps --test world -- --ignored --nocapture` prints real screens (text only:
  land and sea differ by colour, which the test counts).
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
  _keep; the single exception is `set-keymap`, allow_active=yes for listed
  layouts only, ADR 0017) per operation, every argument validated in the helper, absolute tool
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

## 14. Current implementation status (2026-11-16)

**Now: Phase 6 (networking and communications), branch `feat/comms`.**
Maps (Phase 5) were parked by the owner on 2026-11-16 after slice 9: do
not continue slices 10 to 13 unless asked. Phase 6 so far: ADR 0018
(Reticulum + LXMF, opt-in because their licence is not free: owner's
decision); slice 1 = `nix/modules/comms.nix`, `nix/packages/
resilience-reticulum`, `vm-comms`; slice 2 = the per-user program
`resilience-messages` (file formats in ADR 0018), check `comms-messages`.
Slice 3 = `resilience-core/src/messages.rs` and the TUI screens
(`crates/resilience-tui/src/chat_ui.rs`: `Builtin::Chat` "Local
messages" under Communications, views Chats / Nearby / Chat, dialog
`Modal::ChatInput`), handbook `60-local-messages.md`, vm-comms through
the TUI. Open: whether the standard edition turns the module on (owner);
a propagation node (store-and-forward); file exchange; multi-line
messages; Internet messengers (toxic, signal-cli); Yggdrasil/BATMAN.
`resilience.comms.enable` is ON by default in the standard edition
(owner, 2026-11-16; `nix/profiles/standard.nix`, mkDefault) and off in
minimal. Not yet done after that change: ISO rebuild, full flake check.

Phase 5 state when parked:

Phases 0 to 4 are done (see ROADMAP.md). Phase 5 (offline maps, ADR 0016):
slices 1 to 9 are in `develop`. Slices 1 to 7 came with PR #1; slice 8
(built-in world overview: `resilience-mapmaker natural-earth`,
`nix/packages/resilience-world-map`, `[[builtin]]` entry, check
`map-world`) and slice 9 (TUI Maps & navigation: `Builtin::Map`,
`Builtin::MapFiles`, `[tools] maps`, handbook `45-maps.md`, first part of
`vm-maps`) were merged from `feat/maps-overview`, not reviewed, ISO not
rebuilt with them. Slices 1 to 5 done and reviewed (packs accept `.mbtiles`, geographic
maths, MVT decoder, canvas/style/labels, hardened pack reader); slice 6
(viewer `resilience-maps view`) reviewed in its risky parts and run on the
real Luxembourg pack in QEMU at 170x48 (label fixes followed); slice 7
(pack builder `resilience-mapmaker`, `nix/maps/`) merged, builds the
Luxembourg pack, code not reviewed line by line. Slices 10 to 13 (search,
nearby/go-to, spot heights, the rest of `vm-maps`, catalog) are not
started (parked). If resumed: slice 10 (search: `maps/search.rs`, the
viewer's dialog).

Also done with `feat/maps` (2026-11-16): keyboard layouts (ADR 0017: US default,
choice in Settings through `set-keymap`); `scripts/hw-report.sh` and
`docs/HARDWARE-BOOT-CHECKLIST.md` for trying real PCs; the original
specification as `docs/ORIGINAL-SPEC.md`.

Earlier phases: Phase 2 privileged helper and modes (ADR 0007), removable
media and backups (0008), clock (0009), Wi-Fi (0010), default generation
(0011), user namespaces (0012); Phase 3 resource packs and library (0013),
files and manual pages (0014); Phase 4 guided cryptography (0015).
See docs/TESTING.md for what has actually been executed and passed.

Git state (2026-11-16): `main` = end of Phase 4; `develop` = `feat/maps`
and `feat/maps-overview` merged (local, not pushed); `feat/comms` = Phase 6
work, local, from `develop`. Last full `nix flake check -L --max-jobs 1 --keep-going` on
`feat/maps` (before the keyboard work) passed: 9 VM tests, security
invariants, ISO evaluation, both map pack checks. Rust: 198 core, 71 TUI,
134 maps (+1 on demand), 5 library/store tests; clippy clean. The live ISO was rebuilt
and boot-tested in QEMU (BIOS and UEFI, 512 MiB to 2 GiB) on 2026-11-16 and
booted once on a ThinkPad L15 Gen 1.

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
  so ageing catalog entries become USB-only. No comms or SDR yet.
- Cryptography: no LUKS sticks, no OpenPGP signing in the TUI (minisign
  signs), no secure deletion of originals (the user is told); contacts'
  keys are only as trustworthy as the user's own check.
- Local messaging: on in standard, off in minimal; two machines tested, in
  VMs only (with the minimal profile plus the option; the standard image
  with it has only been evaluated, not built or booted);
  no store-and-forward node (both computers must be on at the same time at
  some point within 7 days); one-line messages; no "read" state; received
  messages are not announced outside the messages screens.
- Power profiles not implemented (schedutil governor only). Not measured yet.
- Keyboard: all consoles share one keymap (kernel); a login prompt starting
  on any console resets it to the system layout, and the TUI reloads its
  user's layout only at session start (ADR 0017).
- Maps: no search, nearby or go-to yet; no regional pack in the catalog
  (packs are built with `nix build .#mapPacks.ID` and opened from a
  stick); the world overview has no elevation points and draws disputed
  borders like any other; too many point-of-interest names at city scales
  (style question open); osmium tags-filter needs ~2 GB RAM to build a pack.
- Physical hardware: the live ISO has booted once on a ThinkPad L15 Gen 1
  (owner, 2026-11-16); nothing else has been tested outside QEMU.

## 16. Roadmap

See ROADMAP.md. Current: Phase 6 (networking and communications), see §14.
Phase 5 (maps) is parked.

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
- 0016 Offline maps: MBTiles vector packs (profile resilience-1) built reproducibly in Nix, hardened reader, block-character viewer
- 0017 Keyboard layout: US default, choice in Settings via helper `set-keymap` (allow_active=yes, listed layouts only), reset before every login prompt
- 0018 Local messaging: Reticulum + LXMF (Reticulum License, not free: opt-in module), one unprivileged instance per machine on the local network only, per-user identities and file-based message store (program not written yet)

## 18. How to continue safely

1. Read this file, `git status`, `git log --oneline -20`, ROADMAP.md, docs/TESTING.md.
2. Run `cargo test` and `nix flake check --no-build` before changing anything.
3. Work in small vertical slices; add tests; update docs and this file.
4. Commit coherent milestones. Never weaken a test to make it pass.
