# Testing

Every result below is **PASS**, **FAIL**, **BLOCKED** or **NOT TESTED**.
A test is only PASS if it was actually executed. "VM" means QEMU/KVM on the
development host — **no test has been run on physical hardware**.

Last full run (`nix flake check -L --max-jobs 1`, all VM tests): 2026-11-14 on the Phase 2 commits (privileged helper, Users & modes, Recovery, backups, Date & time, Wi-Fi, default generation, user namespaces), nixpkgs `f1e5c356`. ISO not rebuilt since the helper build (SHA-256 `3d28aeb2…a9f591`) (bit-for-bit reproducibility not verified).

## How to run

| What | Command | Time on dev host |
|---|---|---|
| Rust unit + TUI tests | `cargo test` | ~5 s |
| Lints | `cargo clippy --all-targets` (must be clean), `nix fmt` | ~10 s |
| Screen dump for review | `RESILIENCE_DUMP=1 cargo test dump_screens -- --nocapture` | ~2 s |
| Evaluate everything | `nix flake check --no-build` | ~30 s |
| Everything incl. VM tests | `nix flake check -L --max-jobs 1` | ~5 min cold |
| One VM test | `nix build .#checks.x86_64-linux.vm-modes -L` (also `vm-admin`, `vm-backup`, `vm-wifi`, `vm-generations`, `vm-boot-offline`) | ~1–2 min |
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
| resilience-core: mode, status, network, catalog, config, handbook, notes, launch (incl. env overrides, polkit agent ordering), proc (incl. `run_visible`: clean environment, timeout kills the whole process group, with a positive control), admin (names, modes, config, refusals, overrides/staleness, Ultra-Stable-only machines, group ops, audit lines, timestamps), recovery (generations, generation numbers, default-generation rules: existing, not default, closure directly in the store with switch-to-configuration; report files), media (lsblk parsing/escapes, removable-only selection, device validation, udisks errors), backup (names, checksums, listing, size estimate, pipeline failure cause, **real age round trip**: encrypt, check, restore, no overwrite, wrong key, damaged with/without checksum), clock (local and strict UTC parsing, impossible dates, bounds, zone names), wifi (nmcli terse escapes, security kinds, scan dedupe/sort/saved match, profiles, private profile argv without secret, password only on stdin with a fake nmcli, password rules, failure hints) | 87 | PASS (round trip needs age in PATH: runs in the Nix build; SKIPPED on a host without age) |
| resilience-tui: navigation, rendering 80×24…200×60, modes, restricted commands, missing programs, confirmations, Ctrl-C, handbook, search, notes, themes, shell gating, wrapping, console glyphs; Users & modes (consequences, Cancel default, fixed pkexec argv + SHELL + agent, refusals, no administrator, corrupt config); Recovery (health, guide, report 0600 / no overwrite); System generations (list and states at all sizes, empty list, current default and incomplete generation explained, confirmation with Cancel default, fixed pkexec argv + SHELL + agent, list refreshed after the helper, no administrator); Backups & USB sticks (internal disk never listed, unusable devices, mount via `--no-user-interaction`, Cancel default, backup/check/restore argv, eject busy / safe, lsblk failure); Date & time (prefilled, input filter, errors kept in the dialog, Cancel default, local→UTC argv, no administrator); Wi-Fi (list at 80×24, password dialog masked, nothing created before confirm / on Esc, private profile, secret on stdin only, wrong password → nothing saved, saved network without password, refused saved password → dialog, forget + Cancel default, admin profile not deletable, open warning, 802.1X/WEP refused, polkit refusal, radio off, no adapter, no nmcli) | 44 | PASS |
| `cargo clippy --all-targets` | — | PASS (0 warnings) |

### Nix

| Check | Result |
|---|---|
| Flake evaluation (`nix flake check --no-build`) | PASS |
| `eval-live-iso`: live-standard, live-minimal, dev-vm evaluate | PASS |
| `security-invariants`: 22 invariants × 3 configurations | PASS |
| Invariant checker rejects a live configuration with user namespaces forced on | PASS (manual, once) |
| Invariant checker rejects a deliberately broken configuration (ssh on, root password) | PASS (manual, once) |
| Invariant checker rejects a privileged autologin account and an extra polkit admin identity | PASS (manual, once; this found that the autologin invariant had been vacuous) |
| Build-time catalog check (every app command executable) | PASS (part of every system build) |
| Build-time tool check (`[tools]` in system.toml executable) | PASS (part of every system build) |
| ISO derivation builds (standard) | PASS — 1.48 GB |
| ISO derivation builds (minimal) | NOT TESTED (evaluates only) |

### VM: `vm-boot-offline` — no network hardware at all

| # | Scenario (spec §28) | Result |
|---|---|---|
| 1 | Boot with no network cable (no NIC at all) | PASS |
| — | systemd reaches `running`, no failed units | PASS |
| — | Ultra-Stable-only machine (ADR 0012): `user.max_user_namespaces` = 0, `unshare --user` fails for alice (ENOSPC) and root, no unit failed; `ultra_stable_only = true` in admin.toml and `set-mode alice stable` refused | PASS |
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
| Ultra-Stable: cannot write /etc or the store, stop units, read other homes | PASS |
| Ultra-Stable: `nmcli networking off` refused — **only from a session-less `runuser`**; the console session is covered by vm-wifi (before ADR 0010 it was in fact allowed there) | PASS (scope corrected 2026-11-14) |
| Stable: Nix daemon allowed; sudo refused; cannot write /etc or stop units | PASS |
| Machine with Stable/Advanced accounts keeps user namespaces (`max_user_namespaces` > 0) | PASS |
| Advanced: sudo works with the password, refuses a wrong one (`sudo -k`) | PASS |
| 3 | Isolated LAN without gateway/DNS detected as LOCAL LINK, gateway none | PASS |
| TUI on tty2 as Ultra-Stable: search for "shell" finds nothing; logout works | PASS |
| TUI on tty3 as Stable: terminal shell offered, runs as the user, returns | PASS |
| Root password locked | covered by `security-invariants` (test VMs deliberately unlock root) |

### VM: `vm-admin` — privileged helper (ADR 0007, 0009), driven through the TUI

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
| `set-time`: second polkit action installed; without root exits 77; refused and audited (6×): before 2025, 2100, 30 February, space instead of `T`, `@seconds`, trailing `;id`; extra argument → usage (64); clock unchanged | PASS |
| TUI *Date & time* (Ultra-Stable on tty1, two administrators → pkttyagent "Choose identity" answered): wrong password → clock unchanged, nothing audited as ok | PASS |
| TUI: correct password → clock 2031-03-01T10:00Z, `/sys/class/rtc/rtc0` updated, audit + journal `op=set-time caller=alice … from=<old> to=… result=ok` | PASS |
| Clock kept by the RTC across a reboot; RTC in local-time mode; NTP-on refusal | NOT TESTED |
| Override dropped when the Nix declaration changes | unit tests only (needs a second system configuration in the VM) |

### VM: `vm-backup` — removable media and backups (ADR 0008), driven through the TUI

Ultra-Stable account on tty1; a 128 MiB raw image hot-plugged as a QEMU
`usb-storage` device on an xHCI controller; a 64 MiB virtio disk as the
"internal" disk.

| Check | Result |
|---|---|
| udisks2 installed but not running at boot (D-Bus activated) | PASS |
| polkit for the TUI's session: `filesystem-mount` authorised; `filesystem-mount-system` not (`auth_admin_keep`) | PASS |
| Hot-plugged stick listed; internal disk (ext4, label INTERNAL) not listed | PASS |
| Opening the stick mounts it without a password, `nosuid,nodev`, owned by the user | PASS |
| Backup: confirmation, age asks and confirms the passphrase on the console, archive + `.sha256` (no partial file), `sha256sum -c` passes, age header present, marker text absent from the raw stick device | PASS |
| Eject → SAFE TO UNPLUG, unmounted; unplug + replug (QEMU monitor), refresh, reopen | PASS |
| Check after replug: intact | PASS |
| Restore with a wrong passphrase: "Nothing was restored", `~/Restored` empty | PASS |
| Restore with the right passphrase: new `~/Restored/backup-<stamp>` (alice, 0700), files identical (sha256), originals unchanged, `.cache` not saved | PASS |
| Damaged copy (16 bytes overwritten): check says DAMAGED; restore refused before asking for a passphrase; nothing new restored | PASS |
| Physical USB sticks, SD card readers, exFAT/NTFS sticks, a stick removed during a backup | NOT TESTED |

### VM: `vm-wifi` — Wi-Fi for an Ultra-Stable account (ADR 0010), driven through the TUI

Two simulated radios (`mac80211_hwsim`): `wlan0` is a WPA2 access point
(hostapd + dnsmasq, test only), `wlan1` is managed by NetworkManager.

| Check | Result |
|---|---|
| polkit, TUI process on tty1 (no networkmanager group): scan, network-control, settings.modify.own, enable-disable-wifi allowed; enable-disable-network, -connectivity-check, -statistics, -wwan, wifi.share.open/protected, settings.modify.system/hostname/global-dns, reload, checkpoint-rollback → `result=no` (not auth_admin) | PASS |
| Stable account's TUI on tty2: enable-disable-network and settings.modify.system still allowed | PASS |
| Network listed in the TUI (WPA2) | PASS |
| Esc in the password dialog: no profile created | PASS |
| Wrong password: "NOT CONNECTED … probably wrong … Nothing was", no profile left, NetworkManager stops trying (wlan1 disconnected) — takes ~46 s (timeout) | PASS |
| Right password: connected, DHCP address, profile `permissions=user:alice:;`, root 0600 | PASS |
| Password absent from the journal, every `/proc/*/cmdline`, alice's home; profile unreadable by alice; invisible to bob (positive control: found in the profile file) | PASS |
| Disconnect (`d`), reconnect without a password dialog | PASS |
| Forget: profile file removed, disconnected | PASS |
| Physical Wi-Fi adapters, WPA3-only, open networks, hidden networks, roaming, weak signal | NOT TESTED |

### VM: `vm-generations` — default generation (ADR 0011), systemd-boot on UEFI, driven through the TUI

Generation 1 is the image's system; generation 2 is the same system plus a
marker file, installed in the test with `nix-env --set` and
`switch-to-configuration boot`, as `nixos-rebuild boot` does.

| Check | Result |
|---|---|
| Setup: generation 2 becomes the default (`loader.conf`), reboot into it (marker present) | PASS |
| Helper: as alice exit 77; `0`, `01`, `-1`, `abc`, `1;id`, `../system-1-link`, `99`, `2` (already default) refused with exit 2 and the right reason; extra argument → usage; 8 refusals audited; nothing changed | PASS |
| polkit: `pkcheck` on the TUI process → challenge (2); `pkaction`: auth_admin for any/inactive/active (not `_keep`) | PASS |
| TUI (Ultra-Stable, Recovery → System generations): list shows `started, default`; confirmation; wrong administrator password → profile and boot menu unchanged, no `result=ok` | PASS |
| TUI: carol's password → "Generation 1 is now started by default"; profile → `system-1-link`, `loader.conf` → generation 1; running system unchanged; audit `caller=alice … from=2 to=1 result=ok` + journal; list refreshed | PASS |
| Reboot: generation 1 runs (no marker), generation 2 and its boot entry kept, system `running` | PASS |
| Root (as with sudo) makes generation 2 the default again; audited `caller=root` | PASS |
| Ctrl-C during the operation, boot-loader timeout, concurrent `nixos-rebuild`, GRUB/BIOS, physical hardware | NOT TESTED |

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
| ISO size (standard) | 1.48 GB | build (Phase 2 helper build; not rebuilt with udisks2) |
| Closure growth from removable media + backups | +62 MiB (2231.6 → 2293.9 MiB; age 23 MiB, udisks2 + libblockdev deps ≈ 30 MiB) | `nix path-info -S`, main vs branch |
| TUI binary | 1.5 MiB (stripped, LTO) | build |

## Specification scenarios not yet covered

| # | Scenario | Status |
|---|---|---|
| 2 | Boot with Wi-Fi unavailable | NOT TESTED as a boot scenario (vm-wifi uses simulated radios; logic shared with #1) |
| 6 | Search locally installed resources (resource packs) | BLOCKED — resource packs not implemented |
| 7 | Open installed maps | BLOCKED — maps not implemented |
| 9 | Cryptographic operations | BLOCKED — Phase 4 |
| 10 | Access local files (file manager) | NOT TESTED in VM (mc launch untested) |
| 11 | Import a resource pack from USB | BLOCKED — Phase 3 |
| 12 | Verify an offline update bundle | BLOCKED — Phase 8 |
| 13 | Communicate across a local network | BLOCKED — Phase 6 |
| — | Installer, rollback to previous generation | BLOCKED — no installer yet |
| — | Recovery section with real generations; diagnostic report on a real system | NOT TESTED in VM (unit/TUI tests with fake profiles only) |
| — | Backup / restore with a physical USB stick | NOT TESTED (QEMU usb-storage only) |
