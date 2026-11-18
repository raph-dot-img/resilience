# ADR 0023: Offline updates (signed Nix binary cache on a USB stick)

Status: accepted (2026-11-18).

## Context

Since 0.3.0 Resilience can be installed (ADR 0022), but an installed
computer has no way to get a newer version: `nixos-rebuild` needs the
Internet or a build machine. The specification (§21) asks for an offline
mechanism: build on a connected machine, export the closure, a signed
manifest, carry it on removable media, verify integrity **and**
authenticity, import, install a new generation, keep rollback; established
signature mechanisms only, no custom protocol, a checksum alone is not
authenticity. §20 asks for recovery from failed updates.

What ADR 0022 already gives:

- Every installed computer of an edition runs one of four prebuilt,
  generic systems (`installed-<edition>-<uefi|bios>-<plain|luks>`).
  Everything that differs between machines (accounts, modes, keyboard)
  is state in `/var/lib/resilience`, untouched by a new system.
- An update is therefore "the same configuration, built from a newer
  Resilience": a new toplevel for the computer's own variant.
- Generations and `set-default-generation` (ADR 0011) already let an
  administrator start an earlier system.

Established mechanisms available offline on every Resilience system:

- **Nix binary caches** (`file://` stores) with **ed25519 signatures** on
  every store path (narinfo `Sig:` over the path, its NAR hash, size and
  references). `nix copy --from file://…` imports only paths that are
  missing, checks each NAR against its hash (integrity) and each path's
  signature against the trusted public keys (authenticity), and refuses
  unsigned or badly signed paths when signatures are required. This is
  how every NixOS system already trusts cache.nixos.org.
- minisign (ADR 0015) for files.

## Decision

1. **An update bundle is a signed Nix binary cache on a stick.** A folder
   `resilience-update/` at the root of a USB stick (or anywhere the
   administrator copies it) containing:
   - `nix-cache-info`, `*.narinfo`, `nar/…`: a `file://` binary cache with
     the closures of the four installed systems of one edition and of the
     manifest, **every path signed** with the Resilience release key;
   - `resilience-update.toml`: an unsigned pointer for people and the TUI
     (`edition`, `version`, `manifest = "/nix/store/…"`). Nothing in it
     is trusted; it only says which store path is the manifest.

2. **The manifest is a store path, signed like everything else.** A small
   TOML file built by the flake (`resilience-update-<edition>-<version>`):
   format version, edition, release version, and the toplevel store path
   of each installed variant (`installed-standard-uefi-luks = "/nix/store/…"`).
   It does not reference the toplevels (paths listed without string
   context), so importing it imports nothing else. One signature
   mechanism covers manifest and systems: the "signed manifest" of the
   specification is the manifest's narinfo signature.

3. **Trust: one release key, built into the system.** The release key is
   a Nix binary-cache key (`nix key generate-secret`). Its public part is
   in the flake (`nix/release-keys.nix`, option
   `resilience.updates.trustedKeys`); the secret part never enters Git,
   Nix expressions or any store: the owner keeps it offline and signs
   bundles on their build machine (`scripts/make-update-bundle.sh`).
   The key is passed to Nix explicitly for each import
   (`--option trusted-public-keys`, `--option require-sigs true`); it is
   **not** added to the system's global `nix.conf`, so it trusts nothing
   else. Until the owner has generated a release key, the list is empty
   and updates are not offered (the TUI hides the entry, the helper
   refuses). Key rotation: a new key is added to the list by an update
   signed with the old one; removing a key is the same.

4. **Integrity is distinguished from authenticity** in what the person
   sees: "Check this update" runs, as the user, `nix store verify` on the
   stick's cache with the release keys (every NAR re-hashed against its
   narinfo: integrity; every path's signature checked: authenticity) and
   reports both separately. Nothing is imported by checking.
   Checked with Nix 2.35 before this decision (a scratch store, no
   root): `nix store verify` exits with bit 1 set when a NAR does not
   match its hash and bit 2 when a path lacks a trusted signature;
   `nix copy` from a cache signed by another key fails with "lacks a
   signature by a trusted key", and from a tampered NAR with a hash
   mismatch, registering nothing in either case.

5. **Applying: one new helper operation, `resilience-admin apply-update
   DIR`** (polkit `org.resilience.admin.apply-update`, auth_admin, like
   ADR 0011). The helper, as root, with tools from admin.toml only:
   1. refuses unless the system is an installed one with release keys
      and a known variant (`resilience.updates.variant`, set by the flake
      for each installed configuration);
   2. checks `DIR` (canonical path, a directory, under `/run/media/`:
      removable media mounted by udisks) and reads the pointer (size
      bound, TOML, the manifest a plain `/nix/store/<hash>-<name>` path);
   3. imports **the manifest alone** with `nix copy --from file://DIR`
      and the signature options above, then reads it from the store:
      format, the **same edition**, a version **strictly greater** than
      the running one (no downgrade or replay of an older signed bundle;
      going back is done with generations), a toplevel for this variant;
   4. imports that toplevel the same way (only the missing paths are
      copied; a bad hash or signature stops it before anything is
      registered);
   5. checks the new toplevel announces the same version and edition as
      the manifest (`<toplevel>/etc/resilience/system.toml`);
   6. makes it the next system to start, as ADR 0011 does: `nix-env
      --profile /nix/var/nix/profiles/system --set <toplevel>`, then the
      new toplevel's `switch-to-configuration boot` (clean environment);
      if that fails, the profile is rolled back to the previous
      generation and its boot menu written again;
   7. audits every outcome (`op=apply-update`, from/to versions, result).
   The running system is not changed (never `switch`). After a restart,
   the new version runs; the previous one stays in the boot menu and in
   Recovery → System generations.

6. **What an update carries:** the system (programs, Resilience, kernel,
   handbook, built-in documentation and world map). Accounts, modes,
   keyboard layout, user files, messages, Wi-Fi profiles and installed
   resource packs are state and stay as they are. Resource packs keep
   their own catalog and stick import (ADR 0013); they are not in system
   bundles (§21 "separate updates"). State format changes, if a future
   version needs them, are done by the new system's activation, which
   must keep reading the older format.

7. **The TUI:** Recovery → "Update from a USB stick", shown when the
   system has release keys. It lists mounted removable media that hold
   `resilience-update/resilience-update.toml`, shows what the pointer
   claims (edition, version) as a claim, the running version, and two
   actions: `c` check (point 4) and Enter apply (summary, Cancel default,
   then `pkexec … apply-update DIR` in the foreground with `pkttyagent`,
   as the other operations). Live systems never show it.

## Consequences

- No new cryptography and no new trust format: Nix's binary-cache
  signatures, already used by every NixOS system, protect both the
  manifest and the systems. A tampered NAR, a missing signature, a path
  signed by another key, an older version or another edition are
  refused; `vm-update` tests each.
- Bundles are large (the whole closure of four systems, ~2 GB
  compressed for the standard edition) but only missing paths are
  imported; a computer needs free space for the new paths (Nix reports
  when it runs out; nothing is registered half-way).
- Old generations accumulate in the store; the boot menu keeps 10. A
  "delete old generations" operation is still missing (later work).
- The owner must create and keep the release key; losing it means a new
  key can only reach installed computers through a bundle signed with
  the old one (or a reinstall). Documented in docs/BUILDING.md.
- Systems installed from 0.3.0 have no release key: they cannot take
  updates and must be reinstalled once (0.3.0 was the first installable
  version and was tested in VMs only).
- Online updates (§21 "when Internet access is available") are the same
  bundle served over HTTPS later; not done now.
