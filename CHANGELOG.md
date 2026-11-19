# Changelog

All notable changes are documented here. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/).
Git tags: `vX.Y.Z` on the commit of each release.

## [Unreleased]

## [0.5.0] — 2026-11-19 — Community network

Computers nearby can share their library, resource packs and maps, a
folder of files and simple web pages, without the Internet, and a
computer can become a Wi-Fi network for others. This is the first
release that can run a network server: it is offered in the standard
edition but nothing is shared until an administrator turns it on, and
nothing starts by itself at boot. Tested in virtual machines only.

### Added
- Community network (ADR 0025), Communications > Community network:
  - finding the computers that share nearby (Reticulum announces while
    they share; names are shown as claims), or typing an address; only
    addresses on this computer's own networks are used;
  - reading their library and pages in the text browser, through a
    relay on this computer (the browser still reaches no remote host);
  - getting packs from them: every pack is checked against this
    system's catalog before it is installed, as for downloads;
  - copying a file from their shared folder into Received (exact size,
    never replacing a file, never run);
  - sharing this computer's packs, library, shared folder and pages on
    one network, with an administrator's password every time
    (`resilience-admin set-share`): read-only nginx on TCP port 80 of
    that network only (firewall and kernel), only world-readable files,
    no uploads, limits per address, no access log; it stops when the
    network changes and at restart (`resilience.share.autoStart` for a
    computer left on in a shelter); a line on every screen while on.
- Hotspot (`resilience-admin set-hotspot`, administrator's password):
  this computer as a WPA2 Wi-Fi network with a new password each time,
  DHCP and DNS for `home.arpa` only (`resilience.home.arpa`; more names
  with `resilience.share.hotspot.dnsRecords`), never routing anyone to
  other networks. The mesh and the hotspot exclude each other.
- Handbook page "Community network"; VM tests `vm-share` (two
  machines through the interface) and `vm-hotspot` (a simulated phone).

### Security
- An independent review of the community network before release; its
  fixes are in ADR 0025 (sharing never follows a card onto another
  network, hotspot activated by its own identifier, nginx kept to the
  shared interface by the kernel, no forwarding at all while the hotspot
  is on, both loopbacks held for the browser, a new announce identity
  each time). Accepted limitations are listed in THREAT_MODEL.md.

## [0.4.0] — 2026-11-18 — Offline updates

Installed computers can now be updated from a USB stick, without the
Internet. Computers installed from 0.3.0 cannot: reinstall them once.

### Added
- Offline updates (ADR 0023): a signed update on a USB stick, checked
  (integrity and authenticity, as the user) and installed from Recovery →
  "Update from a USB stick" with an administrator's password; it starts
  at the next restart and the previous version stays in the boot menu.
  Bundles are made with `scripts/make-update-bundle.sh`. VM test
  `vm-update`; also tested end to end in QEMU from a system installed
  from the ISO, with the real release key, including going back to the
  previous version.
- The Resilience release key `resilience-release` (Nix binary-cache key,
  `nix/release-keys.nix`), built into installed systems; the secret part
  is kept offline by the owner.
- Release publishing: `scripts/make-release.sh` builds a release folder
  (ISO, offline update archive per edition, source archive, `SHA256SUMS`)
  and signs the checksums with a separate minisign key
  (`keys/resilience-release.minisign.pub`, ID `5CC1E51F72AC25E7`; see
  keys/README.md). Mirror layout `releases/VERSION/` + `LATEST`; second
  official mirror <https://mirror.canevet.eu/files/resilience/>.
- Licence notice (ADR 0024): before an account first uses the interface
  (and at every start of the live ISO), a plain summary and the original
  text of the Hippocratic License 2.1; the person certifies having read
  it, and the time is recorded in a root-owned file.

## [0.3.0] — 2026-11-18 — Text installer

Resilience can now be installed on a whole disk from the live system.
Tested in virtual machines only.

### Added
- *Install Resilience on this computer* (ADR 0022, Phase 8): a text
  installer on the live system. Whole disk, GPT, encryption (LUKS2) on by
  default, keyboard layout, accounts and their modes; nothing is changed
  before the disk's name is typed. Copies one of four prebuilt installed
  systems per edition (UEFI/systemd-boot or BIOS/GRUB, with or without
  encryption) offline. Accounts created on the machine are managed by
  `resilience-admin` like declared ones. VM tests `vm-install` (BIOS,
  encryption, French layout) and `vm-install-uefi` (UEFI, no encryption);
  `tests/iso/install.py` installs from the ISO in QEMU.

### Changed
- Systems installed by the installer keep user namespaces and the Nix
  sandbox on, whatever the accounts' modes (ADR 0012: their accounts are
  not known when the system is built).
- The full check is run as separate evaluations plus one build on the
  development host (`nix flake check` ran out of memory there; see
  docs/TESTING.md).

### Fixed
- `/run/current-system/sw/bin/resilience-session`, listed in
  `/etc/shells` and passed as `SHELL` to pkexec, did not exist: the
  session package is now installed system-wide.

## [0.2.0] — 2026-11-17 — Phases 2 to 6 (local part)

Accounts and security modes, backups, clock, Wi-Fi, generations, resource
packs and offline library, files, guided cryptography, keyboard layouts,
offline maps (Phase 5, parked after its first part), local messaging with
store-and-forward, files in messages and a Wi-Fi mesh (Phase 6, local
part). Live system only: there is no installer yet.

### Changed
- Console keyboard: US by default; each account chooses its layout in
  Settings (13 layouts by default, `resilience.keyboard.layouts`). Loaded
  by the new helper operation `resilience-admin set-keymap` without a
  password at the console; the system layout is loaded again before every
  login prompt (ADR 0017).
- Resilience's own code is now licensed under the Hippocratic License 2.1
  (was 0BSD, and GPL-3.0-or-later before that); see `LICENSE`. Bundled
  software keeps its own license.

### Added
- *Maps & navigation* (ADR 0016, Phase 5, first part): MBTiles vector map
  packs built reproducibly from OpenStreetMap (`nix build .#mapPacks.ID`,
  `resilience-mapmaker`), a hardened read-only pack reader, and the
  block-character viewer `resilience-maps view` (pan, zoom, labels). A
  built-in world overview (Natural Earth, zoom 0 to 7). TUI: *Map* and
  *Maps on this computer & USB sticks*. No search, nearby or go-to yet.
- *Local messages* (ADR 0018 to 0020, Phase 6): messaging between nearby
  computers without the Internet, over Reticulum and LXMF (Reticulum
  License, not free software: opt-in module `resilience.comms.enable`, on
  in the standard edition, off in minimal). One unprivileged instance per
  machine on the local network only; per-account identities; per-message
  state (waiting, sent, delivered on proof). Store-and-forward: a message
  store on every machine keeps messages for recipients who are away.
  Multi-line messages; one file per message up to 900 KB, saved on
  request into `~/Received`. Handbook page and VM test `vm-comms`; check
  `comms-messages` with the real libraries.
- *Mesh network* (ADR 0021): Wi-Fi without a router, BATMAN-adv over Wi-Fi
  ad hoc, turned on and off at the console (`resilience-admin set-mesh`,
  no password for the active local session); local messages are relayed
  across it. VM test `vm-mesh` (wired links and simulated Wi-Fi radios);
  never tried on real Wi-Fi cards.
- *Security & cryptography* (ADR 0015, Phase 4), for every account. New
  unprivileged tool `resilience-crypto`, run in the foreground by the TUI:
  encrypt a file (age: passphrase, or own key and contacts' keys) next to
  it; decrypt into `~/Decrypted`; encrypted messages (typed, saved armored
  in `~/Messages`, read in memory only); sign files and check signatures
  (minisign); checksums (show, compare with a published one, check a
  SHA256SUMS list); "what is this file?" (age, OpenPGP, LUKS, ZIP,
  OpenSSL, keys, checksum lists); random passphrases (xkcdpass, EFF and
  French lists) and passwords/keys (openssl rand); Keys & contacts (own
  keys always stored encrypted, contacts' public keys checked, export to a
  stick). OpenPGP interoperability through GnuPG and pinentry-curses:
  signatures (.sig/.asc), decryption, encryption to imported keys, key
  import with fingerprint confirmation, own OpenPGP key; no key server,
  no passphrase cache. Passphrases are always typed into age, minisign or
  pinentry, never into the interface. Find & view files doubles as the
  file picker. Handbook page "Encryption, signatures and checksums"; VM
  test `vm-crypto`; two new security invariants.
- *Offline library* and *Resource packs* (ADR 0013, Phase 3): a catalog of
  20 Kiwix packs (Wikipedia, Wiktionary, medicine, practical guides,
  Wikivoyage, computing; English and French) with pinned sizes and
  SHA-256. In the TUI, tick packs by category and language, see sizes and
  free space, then download them (resumable, HTTPS only) or copy them from
  a USB stick, with one administrator password; remove packs and older
  versions. Packs are written by `resilience-pack-store` as the
  unprivileged `resilience-packs` account (pkexec --user, polkit action
  `org.resilience.packs.change`), installed only if the hash matches.
  Reading: kiwix-serve on 127.0.0.1 and lynx with all restrictions, with
  full-text search. New tool `resilience-library`; handbook page "The
  offline library"; VM test `vm-packs` (HTTPS mirror, USB stick, Ctrl-C
  and resume, tampered file, wrong password, removal).
- *Find & view files* (ADR 0014): search file names in the home folder and on
  open sticks; view text, PDF text (pdftotext), CSV/TSV tables, archive
  listings (bsdtar -t, never extracted) inside the TUI; size, date and
  SHA-256. *Manual pages*: `man` in less's secure mode, `apropos --and` search
  (whatis database now built). Handbook page "Files and manuals"; VM test
  `vm-documents`.
- Suggested sets of packs (`s` on Resource packs: Essential, Field guides,
  Knowledge, Extended documentation, Complete...); a built-in "System
  documentation" pack (handbook, NixOS manual, every NixOS option,
  searchable); packs read directly from a USB stick and checked with
  `resilience-library check` (`u` and `c` in the Offline library); eight
  more catalog packs (DevDocs man/Bash/Nix; Stack Exchange DIY, vehicles,
  earth science, ham radio, electronics).
- `resilience-admin`, the privileged helper (ADR 0007): `list`, `set-mode`,
  `apply-state`; polkit action `org.resilience.admin.set-mode` (auth_admin),
  server-side validation, persistent overrides re-applied after activation,
  audit log `/var/log/resilience/audit.log` + journal.
- TUI: *Users & security modes* (view all accounts, change a mode with
  consequences, confirmation and administrator password) and *Recovery &
  diagnostics* (health, failures, recent errors, generations, diagnostic
  report export, emergency guide).
- VM test `vm-admin`; security invariants for polkit admins, faillock and the
  helper configuration.
- *Backups & USB sticks* (ADR 0008): USB sticks and memory cards for every
  account through udisks2 (removable drives only, open/eject from the TUI);
  `resilience-backup create|check|restore`: the home folder as an
  age-encrypted `tar.zst` with a `sha256sum` file, checked without the
  passphrase, restored into a new `~/Restored/backup-<date>` folder.
  Handbook page "Backups and USB sticks"; VM test `vm-backup` (hot-plugged
  virtual stick); invariants for udisks2 polkit and mount location.
- *Date & time* (ADR 0009): set the clock from the TUI in every mode with an
  administrator's password; `resilience-admin set-time` (polkit action
  `org.resilience.admin.set-time`, auth_admin), strict UTC and bounds
  2025–2099 checked in the helper, audited with the previous time; timedated
  updates the hardware clock. Handbook section "The clock"; vm-admin covers
  refusals, wrong and right passwords, two administrators, the RTC.
- *Wi-Fi networks* (ADR 0010): list, join (password typed in the TUI and
  passed to nmcli on stdin), reconnect, disconnect and forget Wi-Fi
  networks in every mode; profiles private to the account; open-network
  warning; WEP and 802.1X not offered. VM test `vm-wifi` (simulated radios).
- *System generations* (ADR 0011): the Recovery list of generations now
  makes one the default for every boot, with an administrator's password;
  `resilience-admin set-default-generation N` (polkit action
  `org.resilience.admin.set-default-generation`, auth_admin) switches the
  system profile with nix-env and runs that generation's
  `switch-to-configuration boot`, undoing the profile switch on failure;
  audited. VM test `vm-generations` (systemd-boot, UEFI, real reboots).
- Option `resilience.security.userNamespaces` (ADR 0012); `admin.toml`
  `ultra_stable_only`: on Ultra-Stable-only machines `set-mode` refuses to
  raise an account and `apply-state` drops earlier raising overrides.
- The helper ignores Ctrl-C, Ctrl-Z and hang-up once a request is
  validated, so a change is never left half-done or unaudited.
- `/etc/resilience/system.toml` gained `[tools]` (absolute paths, checked at
  build time).

### Security
- User namespaces are refused to every process on Ultra-Stable-only
  machines, including the live system (`user.max_user_namespaces = 0`; the
  Nix sandbox is off there, nobody can build). Kept elsewhere (ADR 0012).
- NetworkManager's polkit defaults let any active console session switch
  networking off, share a hotspot or enable connectivity checks without a
  password, including Ultra-Stable accounts. An allowlist now limits accounts
  without the networkmanager group to scanning, connecting, their own
  profiles and the Wi-Fi radio (ADR 0010). The earlier vm-modes PASS for
  "cannot change networking" had only been tested without a session.
- PAM faillock on login, su, sudo and polkit-1 (5 failures / 15 min → 5 min).
- Demoting an account terminates its sessions.

### Fixed
- The security invariant "autologin account is not privileged" read
  `services.getty.autologinUser`, which is unused since autologin moved to the
  tty1 drop-in, so it could never fail; it now checks
  `resilience.interface.autologinUser`, and a new invariant keeps the getty
  option unset.
- pkexec could not complete a successful authentication on a text console
  (socket-activated polkit helper, upstream polkit issue 17); the TUI now
  registers `pkttyagent` for itself before running pkexec.

## [0.1.0] — 2026-11-14 — Phase 0 + Phase 1 prototype

### Security
- Root password explicitly locked; evaluation-time security invariants.
- Autologin restricted to tty1 (NixOS's `services.getty.autologinUser`
  would apply to every console).

### Fixed (found by booting the real ISO / VMs)
- Console font `ter-v16n` lacked the glyphs used by the interface; now
  `ter-i16n`, with a test that only console-font glyphs are drawn.
- Crashed interface left the terminal in raw mode; the session wrapper now
  resets the terminal before showing its diagnostics.

### Added
- Nix flake pinned to nixos-26.05 (nixpkgs `f1e5c356`, 2026-11-13).
- NixOS modules: core (catalog/system config generation with build-time
  verification), users and security modes, interface/session, security
  baseline, offline-first networking, boot, power, application groups.
- Editions: minimal, standard. Live ISO configurations and a development VM.
- `resilience-core` and `resilience` TUI (Rust, ratatui 0.30).
- `resilience-session` login shell with crash recovery.
- Offline handbook (first steps, security modes, offline/network, recovery).
- VM tests: offline boot and interface, security modes and local link.
- Project documentation and ADRs 0001–0006.
