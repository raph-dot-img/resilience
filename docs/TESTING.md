# Testing

Every result below is **PASS**, **FAIL**, **BLOCKED** or **NOT TESTED**.
A test is only PASS if it was actually executed. "VM" means QEMU/KVM on the
development host — **no test has been run on physical hardware**.

Last full run: 2026-11-14 on branch `feat/phase2-admin-helper` (Phase 2: privileged helper, Users & modes, Recovery), nixpkgs `f1e5c356`. ISO SHA-256 of that build: `3d28aeb2…a9f591` (bit-for-bit reproducibility not verified).

## How to run

| What | Command | Time on dev host |
|---|---|---|
| Rust unit + TUI tests | `cargo test` | ~5 s |
| Lints | `cargo clippy --all-targets` (must be clean), `nix fmt` | ~10 s |
| Screen dump for review | `RESILIENCE_DUMP=1 cargo test dump_screens -- --nocapture` | ~2 s |
| Evaluate everything | `nix flake check --no-build` | ~30 s |
| Everything incl. VM tests | `nix flake check -L --max-jobs 1` | ~5 min cold |
| One VM test | `nix build .#checks.x86_64-linux.vm-modes -L` (also `vm-admin`, `vm-boot-offline`) | ~1–2 min |
| ISO boot smoke test | `nix build .#iso -o result-iso && python3 -I tests/iso/smoke.py result-iso/iso/*.iso /tmp/shots [--uefi OVMF.fd]` | ~2 min |

Always pass `--max-jobs 1` to `nix flake check` on the 12 GB development
host so the VM tests never run at the same time.

VM tests read the console through `/dev/vcsuN` (UTF-32) instead of the test
driver's `get_tty_text()`: the latter decodes 8-bit glyph indices as UTF-8,
which corrupts text next to box-drawing characters with our console font.

## Results

### Rust (`cargo test`, also run inside `nix build .#resilience`)

| Suite | Tests | Result |
|---|---|---|
| resilience-core: mode, status, network, catalog, config, handbook, notes, launch (incl. env overrides, polkit agent ordering), proc, admin (names, modes, config, refusals, overrides/staleness, group ops, audit lines, timestamps), recovery (generations, report files) | 57 | PASS |
| resilience-tui: navigation, rendering 80×24…200×60, modes, restricted commands, missing programs, confirmations, Ctrl-C, handbook, search, notes, themes, shell gating, wrapping, console glyphs; Users & modes (consequences, Cancel default, fixed pkexec argv + SHELL + agent, refusals, no administrator, corrupt config); Recovery (health, generations, guide, report 0600 / no overwrite) | 26 | PASS |
| `cargo clippy --all-targets` | — | PASS (0 warnings) |

### Nix

| Check | Result |
|---|---|
| Flake evaluation (`nix flake check --no-build`) | PASS |
| `eval-live-iso`: live-standard, live-minimal, dev-vm evaluate | PASS |
| `security-invariants`: 16 invariants × 3 configurations | PASS |
| Invariant checker rejects a deliberately broken configuration (ssh on, root password) | PASS (manual, once) |
| Invariant checker rejects a privileged autologin account and an extra polkit admin identity | PASS (manual, once; this found that the autologin invariant had been vacuous) |
| Build-time catalog check (every app command executable) | PASS (part of every system build) |
| ISO derivation builds (standard) | PASS — 1.48 GB |
| ISO derivation builds (minimal) | NOT TESTED (evaluates only) |

### VM: `vm-boot-offline` — no network hardware at all

| # | Scenario (spec §28) | Result |
|---|---|---|
| 1 | Boot with no network cable (no NIC at all) | PASS |
| — | systemd reaches `running`, no failed units | PASS |
| 4 | TUI starts on tty1 (autologin), shows OFFLINE and ULTRA-STABLE | PASS |
| — | Autologin limited to tty1; tty2 shows a login prompt, no tty2 session | PASS |
| — | `resilience status` reports mode and network correctly | PASS |
| — | Every catalog program visible to the account is present | PASS |
| 5 | Open local documentation (handbook page) | PASS |
| — | Internet probe without a route returns "not attempted" in ~1 s | PASS |
| 8 | Launch a local program (calendar) and return to the TUI | PASS |
| — | Ctrl-C does not leave the TUI; no shell is spawned | PASS |
| 14 | TUI crash (SIGABRT) → session wrapper shows diagnostics, restarts | PASS |
| — | No listening TCP/UDP sockets | PASS |

### VM: `vm-modes` — isolated VLAN, three accounts

| Check | Result |
|---|---|
| Modes reported from real group membership (3 accounts) | PASS |
| Ultra-Stable: no privileged groups, `sudo` refused (with and without password) | PASS |
| Ultra-Stable: Nix daemon refuses the connection | PASS |
| Ultra-Stable: login shell refuses `su - alice -c id` | PASS |
| Ultra-Stable: cannot write /etc or the store, stop units, change networking, read other homes | PASS |
| Stable: Nix daemon allowed; sudo refused; cannot write /etc or stop units | PASS |
| Advanced: sudo works with the password, refuses a wrong one (`sudo -k`) | PASS |
| 3 | Isolated LAN without gateway/DNS detected as LOCAL LINK, gateway none | PASS |
| TUI on tty2 as Ultra-Stable: search for "shell" finds nothing; logout works | PASS |
| TUI on tty3 as Stable: terminal shell offered, runs as the user, returns | PASS |
| Root password locked | covered by `security-invariants` (test VMs deliberately unlock root) |

### VM: `vm-admin` — privileged helper (ADR 0007), driven through the TUI

| Check | Result |
|---|---|
| Audit log root:wheel 0640 in 0750 dir; unreadable by Ultra-Stable/Stable, readable by Advanced; state not writable by users; polkit action installed | PASS |
| `resilience-admin list` works for any user; `set-mode` without root exits 77 (Ultra-Stable and Advanced) | PASS |
| pkexec with no authentication agent: refused, nothing changes, nothing audited as ok | PASS |
| Server-side refusals: unmanaged account (root), invalid names (`bob;id`, `../carol`), unknown modes (`root`, `Advanced`), last administrator, autologin account, no-op; extra argument → usage (64); 8 refusals audited | PASS |
| TUI (Ultra-Stable on tty1): wrong administrator password → polkit refuses, groups unchanged | PASS |
| TUI: correct administrator password → bob promoted, polkit action `org.resilience.admin.set-mode` shown, audit + journal entry with caller=alice, state recorded | PASS |
| New rights at next login (bob's TUI shows ADVANCED) | PASS |
| Demotion terminates all of the account's sessions; returning to the declared mode removes the override | PASS |
| Recorded override survives a reboot (re-applied after activation, audited) | PASS |
| pam_faillock: 5 wrong sudo passwords lock the account (correct password refused); `faillock --reset` unlocks | PASS |
| Override dropped when the Nix declaration changes | unit tests only (needs a second system configuration in the VM) |

### ISO boot (`tests/iso/smoke.py`, screenshots reviewed by a human/agent)

| Firmware | Result |
|---|---|
| Legacy BIOS (SeaBIOS, syslinux) — boots to TUI, offline, menus render, system OPERATIONAL | PASS (re-run with Phase 2 build) |
| UEFI (OVMF from pinned nixpkgs, GRUB text mode) — boots to TUI | PASS (Phase 0/1 build; not re-run for Phase 2) |
| Physical hardware | NOT TESTED |

### Measurements (VM, 2 vCPU, 1.5–2 GiB RAM; reported, not asserted)

| Metric | Value | Where |
|---|---|---|
| TUI resident memory | 3.5 MiB | vm-boot-offline |
| Memory used after boot (incl. test backdoor) | 275 MiB | vm-boot-offline |
| Memory used on live ISO (tmpfs root) | ~300 MiB of 1.9 GiB | ISO screenshot |
| Boot to multi-user (kernel + initrd + userspace) | 0.9 + 3.6 + 4.3 = 8.8 s | vm-boot-offline |
| Running services | journald, udevd, logind, dbus-broker, NetworkManager, nscd, oomd (+ transient) | vm-boot-offline |
| System closure (live-standard) | 2.2 GiB, of which linux-firmware 825 MiB, nixpkgs source 197 MiB | `nix path-info -S` |
| ISO size (standard) | 1.48 GB | build |
| TUI binary | 1.5 MiB (stripped, LTO) | build |

## Specification scenarios not yet covered

| # | Scenario | Status |
|---|---|---|
| 2 | Boot with Wi-Fi unavailable | NOT TESTED (no Wi-Fi emulation; logic shared with #1) |
| 6 | Search locally installed resources (resource packs) | BLOCKED — resource packs not implemented |
| 7 | Open installed maps | BLOCKED — maps not implemented |
| 9 | Cryptographic operations | BLOCKED — Phase 4 |
| 10 | Access local files (file manager) | NOT TESTED in VM (mc launch untested) |
| 11 | Import a resource pack from USB | BLOCKED — Phase 3 |
| 12 | Verify an offline update bundle | BLOCKED — Phase 8 |
| 13 | Communicate across a local network | BLOCKED — Phase 6 |
| — | Installer, rollback to previous generation | BLOCKED — no installer yet |
| — | Recovery section with real generations; diagnostic report on a real system | NOT TESTED in VM (unit/TUI tests with fake profiles only) |
| — | Backup / restore of user data | BLOCKED — not implemented |
