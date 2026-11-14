# Resource packs — design (not implemented yet)

Status: **design draft for Phase 3.** Nothing in this document exists in
code yet. Large content (Wikipedia ZIMs, maps, guides) is never put in the
ISO or in Git; it is delivered as resource packs on external media.

## Goals

Discover, verify, install, update, remove and index packs **without
Internet**; a corrupt or malicious pack must be rejected without affecting
other installed resources.

## Format

A pack is two files side by side:

```
wikipedia-en-mini-2026-10.rpack          TAR archive, zstd-compressed (.tar.zst content)
wikipedia-en-mini-2026-10.rpack.minisig  minisign signature of the .rpack file
```

Archive layout:
```
manifest.toml       first member
LICENSES/…          license texts
content/…           payload (e.g. *.zim, *.mbtiles, *.md)
```

`manifest.toml`:
```toml
format = 1
id = "wikipedia-en-mini"          # [a-z0-9-]
version = "2026-10"
type = "knowledge"                # knowledge | geography | field-guide | documentation
title = "Wikipedia (English, mini)"
description = "…"
installed_size = 1234567890       # bytes, checked against free space first
requires = []                     # other pack ids
min_resilience = "0.3.0"
[provenance]
source = "https://download.kiwix.org/zim/wikipedia/…"
retrieved = "2026-10-12"
upstream_checksum = "sha256:…"
[license]
spdx = "CC-BY-SA-4.0"
attribution = "Wikipedia contributors"
[[files]]
path = "content/wikipedia_en_mini.zim"
sha256 = "…"
size = 1234567890
```

## Verification — integrity vs authenticity

1. **Authenticity**: `minisign -V` against publisher public keys pinned in
   the system configuration (`resilience.packs.trustedKeys`). Unsigned or
   unknown-key packs are refused by default; an Advanced user may explicitly
   accept them with a warning.
2. **Integrity**: SHA-256 of every file against the signed manifest, during
   extraction into a private staging directory.

A checksum alone never establishes who published a pack.

## Safe installation

- Stream-extract into `/var/lib/resilience/packs/.staging/<id>` (root-owned,
  via the privileged helper); reject absolute paths, `..`, symlinks, hard
  links, device nodes, and files not listed in the manifest.
- Check free space before extracting; atomic rename into place; the previous
  version stays until the new one is verified.
- Index (search metadata) rebuilt per pack, so one bad pack cannot corrupt
  others' indexes.

## Planned packs

Essential, Knowledge, Geography (per region), Field Guides, Extended
Documentation, Complete (a set of the others). Medical and safety guides
carry their source, date and a prominent limitation notice.
