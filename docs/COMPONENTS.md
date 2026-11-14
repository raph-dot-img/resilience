# Component inventory

Third-party components, as present in the pinned nixpkgs (`nixos-26.05`,
rev `f1e5c356`, 2026-11-13). Versions and licenses were read from nixpkgs
metadata (`nix eval`), not assumed. "TUI" = usable without any graphical
environment. Maintenance status reflects nixpkgs packaging and upstream
activity as known on 2026-11-14; re-check before each release.

## Integrated (in the system today)

| Component | Version | License | Upstream | Offline | Integration |
|---|---|---|---|---|---|
| NixOS / nixpkgs | 26.05 | MIT (expressions) | https://github.com/NixOS/nixpkgs | yes | base system |
| Linux kernel | 6.18.63 | GPL-2.0 | https://kernel.org | yes | default kernel of 26.05 |
| systemd | 260.5 | LGPL-2.1+ | https://systemd.io | yes | init, logind, journald, timedated (`timedatectl set-time` from the helper) |
| NetworkManager (nmtui) | 1.56.0 | GPL-2.0+ | https://networkmanager.dev | yes (LAN) | connectivity check disabled; app `nmtui` (Stable+); `nmcli` used by the TUI's Wi-Fi screen (every mode, polkit allowlist without the group, ADR 0010) |
| sudo | (nixpkgs) | ISC | https://www.sudo.ws | yes | wheel only, password |
| polkit (pkexec, pkttyagent) | 127 | LGPL-2.0+ | https://github.com/polkit-org/polkit | yes | authorises `resilience-admin set-mode` and `set-time` (auth_admin); text agent on the console |
| Linux-PAM (pam_faillock, faillock) | 1.7.2 | BSD-3-Clause | https://github.com/linux-pam/linux-pam | yes | rate limiting on login, su, sudo, polkit-1 |
| udisks2 (udisksctl) | 2.11.2 | GPL-2.0+ / LGPL-2.0+ | https://github.com/storaged-project/udisks | yes | mounts removable media for the console user (D-Bus activated, its polkit defaults) |
| kiwix-tools (kiwix-serve) | 3.8.2 | GPL-3.0-or-later | https://github.com/kiwix/kiwix-tools | yes | serves one installed ZIM file on 127.0.0.1 while it is read (ADR 0013) |
| libkiwix, libzim, Xapian | 14.2.1, 9.7.0, 1.4.31 | GPL-3.0+, GPL-2.0+, GPL-2.0+ | https://kiwix.org | yes | dependencies of kiwix-serve (ZIM reading, full-text search) |
| Lynx | 2.9.2 | GPL-2.0-or-later | https://lynx.invisible-island.net | yes | text browser of the offline library, `-restrictions=… -localhost` |
| curl | 8.23.0 | curl | https://curl.se | downloads only | downloads resource packs on request, HTTPS only, resumable |
| Kiwix ZIM content (catalog) | 2024–2026 files | CC-BY-SA-4.0 and others (per pack) | https://download.kiwix.org | yes | not shipped: chosen by the user, hashes pinned in `nix/packs/kiwix.nix` |
| age | 1.3.2 | BSD-3-Clause | https://age-encryption.org | yes | backup encryption (passphrase, read from the terminal by age) |
| zstd | 1.5.7 | BSD-3-Clause / GPL-2.0 | https://facebook.github.io/zstd | yes | backup compression |
| GNU tar | 1.35 | GPL-3.0+ | https://www.gnu.org/software/tar | yes | backup archives |
| shadow (gpasswd) | 4.19.4 | BSD-3-Clause | https://github.com/shadow-maint/shadow | yes | group changes by `resilience-admin` |
| Terminus console font | (nixpkgs) | OFL-1.1 | https://terminus-font.sourceforge.net | yes | `ter-i16n` |
| GNU nano | 9.3 | GPL-3.0+ | https://nano-editor.org | yes | editor (Stable+), notes editor (`--restricted` for Ultra-Stable) |
| Vim | 9.2.1252 | Vim | https://www.vim.org | yes | Stable+ |
| Midnight Commander | 4.8.33 | GPL-3.0+ | https://midnight-commander.org | yes | file manager, Stable+ |
| GNU bc | 1.08.2 | GPL-3.0+ | https://www.gnu.org/software/bc | yes | calculator |
| GNU units | 2.27 | GPL-3.0+ | https://www.gnu.org/software/units | yes | unit converter (its currency updater needs Internet; not offered) |
| util-linux `cal` | (nixpkgs) | GPL-2.0+ | https://github.com/util-linux/util-linux | yes | calendar |
| htop | 3.5.1 | GPL-2.0-only | https://htop.dev | yes | `--readonly` for Ultra-Stable |
| ncdu | 2.9.2 | MIT | https://dev.yorhel.nl/ncdu | yes | `-rr` (read-only, no shell) for Ultra-Stable |
| nudoku | 5.0.0 | GPL-3.0-only | https://github.com/jubalh/nudoku | yes | game |
| tty-solitaire | 1.4.1 | MIT | https://github.com/mpereira/tty-solitaire | yes | game |
| ninvaders | 0.1.2 | GPL-2.0+ | https://ninvaders.sourceforge.net | yes | game |
| moon-buggy | 1.1.0 | GPL-2.0-only | https://www.seehuhn.de/pages/moon-buggy | yes | game |
| NetHack | 3.6.7 | NGPL | https://www.nethack.org | yes | game |

## Rust crates (Cargo.lock, hash-pinned)

ratatui 0.30 (MIT), crossterm 0.29 (MIT, via ratatui), serde 1 (MIT/Apache-2.0),
toml 1 (MIT/Apache-2.0), tracing 0.1 / tracing-subscriber 0.3 (MIT),
thiserror 2 (MIT/Apache-2.0), libc 0.2 (MIT/Apache-2.0). 108 crates total
including transitive dependencies. `cargo deny check licenses advisories`
is to be run before each release (cargo-deny is in the dev shell).

## Evaluated, available in nixpkgs, planned

| Component | Version | License | Purpose | Notes |
|---|---|---|---|---|
| kiwix-tools | 3.8.2 | GPL-3.0+ | ZIM serving (Phase 3) | `kiwix-serve` on 127.0.0.1 + text browser |
| w3m / lynx / links2 / elinks / chawan | 0.5.6 / 2.9.2 / 2.30 / 0.19.1 / 0.3.3 | MIT / GPL-2.0+ / GPL-2.0+ / GPL-2.0 / Unlicense | text browser for Kiwix | to compare (UTF-8, tables, link navigation) |
| minisign | 0.12 | ISC | signatures for packs/updates (Phase 3/8) | |
| GnuPG | 2.4.9 | GPL-3.0+ | OpenPGP (Phase 4) | |
| cryptsetup | 2.8.6 | GPL-2.0+ | LUKS (installer) | |
| OpenSSL | 3.6.5 | Apache-2.0 | already in closure | |
| poppler-utils (pdftotext) | 26.10.0 | GPL-2.0+ | PDF text reading | |
| visidata | 3.3 | GPL-3.0 | CSV/spreadsheet | Python closure: evaluate size |
| sc-im | 0.8.5 | BSD-4-Clause | spreadsheet | advertising clause: review |
| glow / mdcat | 2.1.2 / 2.7.1 | MIT / MPL-2.0 | Markdown viewing | |
| yggdrasil | 0.5.13 | LGPL-3.0 | mesh (Phase 6) | |
| rns (Reticulum) | 1.2.9 | (license metadata missing in nixpkgs — verify upstream) | local/mesh messaging (Phase 6) | |
| batctl | 2026.4 | GPL-2.0-only | BATMAN-adv (Phase 6) | |
| toxic (Tox) | 0.16.1 | GPL-3.0-only | Internet messaging TUI (Phase 6) | |
| signal-cli | 0.14.7 | GPL-3.0 | **unofficial** Signal client (Phase 6) | Java; unofficial; may break with server changes |
| rtl-sdr / rtl_433 / soapysdr | 1.3.5 / 25.12 / 0.8.1-unstable | GPL-2.0+ / GPL-2.0+ / BSL-1.0 | receive-only SDR (Phase 7) | |
| gpsd | 3.27.5 | BSD-2-Clause | GPS for maps (Phase 5) | |
| osmium-tool, gdal | 1.19.1, 3.12.4 | GPL-3.0+…, MIT | map data processing (build side) | |
| frotz, angband, chess-tui, gnuchess | 2.55, 4.2.6, 2.7.0, 6.3.0 | GPL-2.0+, GPL-2.0, MIT, GPL-3.0+ | entertainment (Phase 7) | |
| tealdeer | 1.8.1 | MIT/Apache-2.0 | command examples | its cache must be shipped offline |

## Rejected or deferred

| Component | Reason |
|---|---|
| calcurse | Pulls ~128 MiB of Python (CalDAV sync helper) into the closure. Revisit with an override without the helper. |
| ModemManager (service) | Resident service unused by most machines; off by default (library stays in NetworkManager's closure). |
| simplex-chat-desktop | Graphical client only in nixpkgs; a CLI exists upstream but is not packaged — re-evaluate. |
| zathura, qalculate-gtk | Graphical. |
| `nomadnet`, `navit`, `dump1090`, `zimwriterfs`, `sudoku-tui` | Not in the pinned nixpkgs under these names (checked). |
