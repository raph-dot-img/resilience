# ADR 0013: Resource packs and the offline library

Status: accepted (2026-11-14).

## Context

Phase 3 brings large offline content to Resilience: encyclopedias,
dictionaries, health and practical guides. This content is far too big for
the ISO or Git (the English Wikipedia without pictures is 49 GiB). The
user must be able to **choose easily what to get**: see what exists, how
big it is and in which language, tick what they want, and get it either by
downloading it while a network is available or by copying it from a USB
stick prepared on another computer.

Kiwix publishes this kind of content as ZIM files, with a server
(`kiwix-serve`) that renders it as web pages. Every file on
download.kiwix.org has a metalink (`.meta4`) giving its exact size and
SHA-256. Kiwix does not sign its files, and it removes old versions from
its servers after a few months.

The earlier draft (`docs/RESOURCE_PACKS.md`) planned our own archive format
with a manifest and minisign signatures, extracted by the privileged
helper. That means parsing archives as root, and it is not needed for
single-file content like ZIM.

## Decision

### The catalog is the trust anchor

The packs that can be installed are declared in the NixOS configuration
(`resilience.packs.catalog`, by default `nix/packs/kiwix.nix`). Each entry
pins the file name, **the exact size and the SHA-256**, the download
addresses (HTTPS only), the license, the source and an optional notice (for
example medical packs). It is generated into `/etc/resilience/packs.toml`,
which is root-owned and checked at build time by the same Rust parser that
uses it (`resilience-library check-config`).

A file is installed only if its bytes have exactly that hash. Whoever built
the system therefore chose the content, its source and its exact bytes, the
same way Nix pins fixed-output downloads. HTTPS protects privacy and gives
early errors; the hash gives integrity and authenticity. A stick prepared by
anyone can be used, because a file that does not match is refused. The
catalog now has 29 packs (English and French), with sizes and hashes
from Kiwix's metalinks of 2026-11-14.

Only content that reads well in a text browser is listed. Wikimedia
projects, Stack Exchange and static websites (zimit) were tried with lynx.
Kiwix's JavaScript PDF libraries (`zimgit-*`) and the Gutenberg index were
not readable, so they are not listed.

### Shared packs, written by an unprivileged account

Installed packs are shared by every account. They live in
`/var/lib/resilience/packs` (mode 0755), owned by a new system account,
`resilience-packs`. Only `resilience-pack-store` writes there. It starts
through `pkexec --user resilience-packs` after **an administrator's
password** (polkit action `org.resilience.packs.change`, `auth_admin` for
any, inactive and active sessions, never `_keep`). The store:

- refuses to run as anyone but `resilience-packs`, and never as root;
- reads only the fixed `/etc/resilience/packs.toml`, and ignores the
  environment;
- takes typed arguments: `change [--remove FILE]... [--get ID]...`. File
  names are plain `[A-Za-z0-9._-]+.zim`; ids must be in the catalog;
- **never parses the content.** It receives bytes on its standard input in
  length-prefixed chunks (at most 1 MiB each). It appends them to
  `.staging/<file>.part` and feeds them to `sha256sum` along with the bytes
  already there. It renames the file into place only if the hash matches,
  deletes it on a mismatch or on extra bytes, and keeps it when the data
  stops early, so the next attempt continues from there;
- checks free space first, keeping 1% (at least 64 MiB) free;
- holds a lock, so only one change runs at a time;
- removes only regular files directly in its directory;
- logs every step to the journal (`resilience-pack-store`, authpriv).

The bytes come from `resilience-library change`, which runs as the user.
It downloads with `curl` (HTTPS only, also after redirects; `~/.curlrc`
ignored; a resumed download uses `--continue-at`, which fails if the server
ignores ranges), or reads the file from a mounted stick. The store answers
first with one line per pack: `send ID OFFSET` (the bytes already received)
or `skip ID REASON`. One password covers the whole selection. Ctrl-C stops
everything, and the next attempt continues where it stopped.

An independent review of the first version led to these protections:

- after an early refusal or a mismatch, the store skips the rest of that
  pack's chunks, so the next pack is not fed someone else's bytes;
- a full-size partial file that was never installed is started again
  instead of blocking the pack;
- the library checks that the local port really belongs to its own server
  (owner in `/proc/net/tcp`) before opening the browser. Otherwise another
  account could take the port and show its own pages;
- the polkit message includes the exact command line, so the administrator
  sees which packs are added or removed;
- the store resets the file-size limit and `SIGXFSZ` inherited through
  pkexec.

A bug in the store can at worst damage the packs directory, never the rest
of the system. Unprivileged code (curl) does the network work, so a flaw
there gives nothing an attacker could not get as the user.

### Reading

`resilience-library open FILE [--search WORDS]` (as the user) starts
`kiwix-serve` for that one file on 127.0.0.1, on a free port, with
`--blockexternal` and `--attachToProcess`. It then runs **lynx with every
restriction and `-localhost`**: no shell escape, no saving or
printing, no external programs, no address to type, no remote host. The
restrictions are listed one by one (`nix/modules/packs.nix`), because
`-restrictions=all` also refuses the server's redirects, and `-localhost`
accepts only the name `localhost` (not 127.0.0.1). The VM test found both. When lynx exits,
the server stops. A pack's notice is shown first. Full-text search uses
the index inside the ZIM file. Reading works in every mode, Ultra-Stable
included.

### In the interface

- **Knowledge → Resource packs: choose what to get.** Every pack is listed
  by category, with its size and state: `[*]` installed, `[ ]` not
  installed, `n% done`. A line shows how much will be added, how much
  removed, and the free space. `f` shows one language at a time. Space
  ticks a pack: `[+]` will be added, `[-]` will be removed. Enter shows
  the details (license, source, file name, SHA-256). `d` downloads, and is
  refused, with an explanation, when there is no route to the Internet.
  `u` copies from a USB stick: the stick is chosen and mounted as for
  backups, and the packs are found at its top level or in
  `Resilience-packs/`. With nothing ticked, `u` offers every pack on the
  stick. A confirmation lists everything that will change (Cancel is the
  default). Older versions left by a catalog update are listed too, and
  can be removed.
- **Knowledge → Offline library**: the installed packs. Enter reads one,
  `s` searches its full text (the words are one argument, never a shell),
  `p` goes to Resource packs.
- Both entries are shown only when the system has a catalog.

### Suggested sets, built-in documentation, packs on a stick

Added the same day, after the first version:

- **Suggested sets** (`resilience.packs.sets`, default `nix/packs/sets.nix`):
  Essential (English), Essentiel (français), Field guides, Knowledge,
  Connaissances, Extended documentation, Complete. On the Resource packs
  screen, `s` lists them with what is still missing and its size. Enter
  ticks those packs; nothing is downloaded until `d` or `u`. Sets keep only
  the packs present in the catalog, so a smaller catalog still works. The
  roadmap's "Geography" set waits for maps (Phase 5).
- **A built-in documentation pack**, about 4 MiB, shipped in the system
  (`resilience.packs.builtinDocs`, on by default, also on the live system).
  It holds the Resilience handbook, the NixOS manual and all NixOS options,
  one page per option group, so pages stay small and search finds an
  option by name or description. It is built at build time
  (`nix/packages/resilience-docs`) from nixpkgs' own manual and
  `options.json`, which are in the binary cache, with `zimwriterfs`. It sits
  in the Nix store, so it is as trusted as the rest of the system.
- **Reading packs on a stick without installing them** (the live system has
  no administrator). The Offline library also lists:
  - catalog files on mounted sticks, matched by name and size, at the top
    or in `Resilience-packs/`;
  - any other `.zim` file in that folder, marked as not in the catalog.

  `u` opens a stick. `c` checks a stick file with `resilience-library
  check`, which hashes it against the catalog: matches, does not match, or
  unknown with its SHA-256 shown. Opening an unchecked file is the user's
  choice: libzim parses it as the user, as with any document. The
  description says it was not checked.
- Eight more catalog packs, one reason for the new **Documentation**
  category:
  - documentation: Linux manual pages, Bash and Nix (DevDocs);
  - Stack Exchange: home improvement, vehicle repair, earth science and
    weather, amateur radio, electronics.

## Alternatives considered

- **Our own `.rpack` archive with minisign signatures** (the earlier
  draft): extraction means parsing archives, which would run as a
  privileged account. Pinning the hash in the catalog gives the same
  guarantee for single files with less code. Signed packs may come back
  for content that changes faster than system updates (Phase 8, offline
  updates).
- **The root helper (`resilience-admin`) installs packs**: it would read
  user-supplied bytes as root. A dedicated unprivileged account limits any
  mistake to the packs directory.
- **A per-user download, then a copy by the store**: twice the disk space,
  and a time-of-check gap on a file the user still controls. Streaming on
  standard input avoids both: what is hashed is what is on disk.
- **Packs per user, in the home folder**: no privileges at all, but the
  same 50 GiB once per account.
- **Kiwix's own downloader (aria2, Kiwix desktop apps)**: graphical or
  bigger, with no pinned hashes.
- **w3m instead of lynx**: w3m has no restricted mode. Its `!` command
  starts a shell, which Ultra-Stable accounts must not have.

## Consequences

- An administrator chooses, with one password, what every account can read.
  Ultra-Stable users can choose packs; the administrator authorises.
- On the live system (no administrator) packs cannot be installed; they can
  be read directly from a stick.
- The catalog ages: Kiwix deletes old files, and those packs can then only
  be installed from a stick, until a system update brings newer entries.
  A failed download says so. Updating the catalog without a system update
  is not possible (a design choice: the catalog is part of the trusted
  system).
- `kiwix-serve` on 127.0.0.1 can be reached by other local accounts while a
  pack is open. The content is public and shared anyway, and there is no
  write access.
- An administrator can add their own packs (for example a ZIM file of
  local documents) by declaring them with their hash in the configuration.
- The browser's restrictions are usability measures, the same as for other
  restricted programs. They are not a sandbox.
- Not tested: real Kiwix downloads in the VM test (no Internet there: a
  local HTTPS mirror is used); multi-GiB packs; a stick removed during a
  copy; two administrators changing packs at the same time (the lock
  refuses the second one).

Tested by: `tests/vm/packs.nix` and unit tests in `packs.rs`, the store and
the library, plus the TUI tests.
