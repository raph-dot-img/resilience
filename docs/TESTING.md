# Testing

Every result below is **PASS**, **FAIL**, **BLOCKED** or **NOT TESTED**.
A test is only PASS if it was actually executed. "VM" means QEMU/KVM on the
development host — **no test has been run on physical hardware**.

Last full run: 2026-11-14 on the commit that added this file (`docs: complete Phase 0/1 documentation…`), nixpkgs `f1e5c356`. ISO SHA-256 of that build: `d8e0335c…3e2f6f` (not reproducible bit-for-bit yet: not verified).

## How to run

| What | Command | Time on dev host |
|---|---|---|
| Rust unit + TUI tests | `cargo test` | ~5 s |
| Lints | `cargo clippy --all-targets` (must be clean), `nix fmt` | ~10 s |
| Screen dump for review | `RESILIENCE_DUMP=1 cargo test dump_screens -- --nocapture` | ~2 s |
| Evaluate everything | `nix flake check --no-build` | ~30 s |
| Everything incl. VM tests | `nix flake check -L --max-jobs 1` | ~5 min cold |
| One VM test | `nix build .#checks.x86_64-linux.vm-modes -L` | ~1–2 min |
| ISO boot smoke test | `nix build .#iso -o result-iso && python3 -I tests/iso/smoke.py result-iso/iso/*.iso /tmp/shots [--uefi OVMF.fd]` | ~2 min |

Always pass `--max-jobs 1` to `nix flake check` on the 12 GB development
host so the two VM tests never run at the same time.

VM tests read the console through `/dev/vcsuN` (UTF-32) instead of the test
driver's `get_tty_text()`: the latter decodes 8-bit glyph indices as UTF-8,
which corrupts text next to box-drawing characters with our console font.

## Results

### Rust (`cargo test`, also run inside `nix build .#resilience`)

| Suite | Tests | Result |
|---|---|---|
| resilience-core: mode, status, network, catalog, config, handbook, notes, launch, proc | 40 | PASS |
| resilience-tui: navigation, rendering 80×24…200×60, modes, restricted commands, missing programs, confirmations, Ctrl-C, handbook, search, notes, themes, shell gating, wrapping, console glyphs | 17 | PASS |
| `cargo clippy --all-targets` | — | PASS (0 warnings) |

### Nix

| Check | Result |
|---|---|
| Flake evaluation (`nix flake check --no-build`) | PASS |
| `eval-live-iso`: live-standard, live-minimal, dev-vm evaluate | PASS |
| `security-invariants`: 12 invariants × 3 configurations | PASS |
| Invariant checker rejects a deliberately broken configuration (ssh on, root password) | PASS (manual, once) |
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

### ISO boot (`tests/iso/smoke.py`, screenshots reviewed by a human/agent)

| Firmware | Result |
|---|---|
| Legacy BIOS (SeaBIOS, syslinux) — boots to TUI, offline, menus render | PASS |
| UEFI (OVMF from pinned nixpkgs, GRUB text mode) — boots to TUI | PASS |
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
