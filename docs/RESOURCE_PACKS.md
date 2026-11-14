# Resource packs

Status: **implemented for Kiwix ZIM files** (Phase 3, ADR 0013). This page
describes what exists; the decision record has the reasoning and the
alternatives.

## What a pack is

One file (today: a Kiwix ZIM file) declared in the system configuration
with its exact size and SHA-256:

```nix
resilience.packs.catalog = [
  {
    id = "wikivoyage-en";                         # stable across versions
    title = "Wikivoyage (English): travel guide, text only";
    description = "Places to go, getting around, staying safe.";
    category = "travel";   # encyclopedia dictionary health practical travel computing
    language = "en";
    file = "wikivoyage_en_all_nopic_2026-09.zim";
    size = 272495142;
    sha256 = "4305c505…";
    urls = [ "https://download.kiwix.org/zim/wikivoyage/wikivoyage_en_all_nopic_2026-09.zim" ];
    license = "CC-BY-SA-4.0";
    source = "Wikivoyage contributors; packaged by Kiwix (download.kiwix.org)";
    notice = null;                                # shown before opening
  }
];
```

The default catalog is `nix/packs/kiwix.nix`. Sizes and hashes come from
Kiwix's metalinks (`<url>.meta4`). To add a pack: take the file's size and
SHA-256 from its `.meta4` (or `stat -c %s` and `sha256sum` of a copy you
trust), check that it reads well in lynx, and add an entry. A pack with no
`urls` can only be installed from a USB stick.

## Where things are

| Path | What |
|---|---|
| `/etc/resilience/packs.toml` | the catalog and tool paths (root, read-only) |
| `/var/lib/resilience/packs/` | installed packs, owner `resilience-packs`, 0755 |
| `/var/lib/resilience/packs/.staging/` | unfinished downloads (`<file>.part`) |
| journal, identifier `resilience-pack-store` | every install and removal |

## Commands

```text
resilience-library list
resilience-library change [--from MOUNT] [--remove FILE]... [--get ID]...
resilience-library open FILE [--search WORDS]
```

`change` asks for an administrator's password (pkexec, action
`org.resilience.packs.change`) and runs `resilience-pack-store` as
`resilience-packs`. The store protocol (stdin chunks, plan lines) is
documented in `crates/resilience-core/src/packs.rs`.

## Sets, built-in packs, sticks

- `resilience.packs.sets`: suggested selections (`nix/packs/sets.nix`);
  unknown ids are dropped when the catalog is replaced.
- `resilience.packs.builtinDocs`: the "System documentation" ZIM built from
  the handbook and nixpkgs' NixOS manual and `options.json`
  (`nix/packages/resilience-docs/make-docs.py`), listed as `[[builtin]]`.
- Packs on mounted sticks are listed in the Offline library and can be
  checked with `resilience-library check FILE`.

## Not yet

- Maps (Phase 5), documentation packs of our own, signed packs that can be
  updated without a system update (Phase 8).
