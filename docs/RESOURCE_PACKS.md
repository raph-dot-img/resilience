# Resource packs

Status: **implemented for Kiwix ZIM files** (Phase 3, ADR 0013). The pack
machinery also accepts **map packs** (`.mbtiles`, ADR 0016). A world
overview map is built in (`resilience.packs.builtinWorldMap`, a `[[builtin]]`
entry of the `maps` category, Natural Earth, 11.7 MB); the map viewer is not
in the interface yet and no regional map pack is in the catalog. This page describes
what exists; the decision records have the reasoning and the alternatives.

## What a pack is

One file declared in the system configuration with its exact size and
SHA-256: a Kiwix ZIM file (`NAME.zim`, read in the text browser), or a map
pack (`map-osm-REGION_YYYY-MM.mbtiles`, category `maps`, opened from Maps &
navigation, never by the text browser):

```nix
resilience.packs.catalog = [
  {
    id = "wikivoyage-en";                         # stable across versions
    title = "Wikivoyage (English): travel guide, text only";
    description = "Places to go, getting around, staying safe.";
    category = "travel";   # encyclopedia dictionary health practical travel maps computing documentation
    language = "en";
    file = "wikivoyage_en_all_nopic_2026-09.zim";
    size = 272495142;
    sha256 = "4305c505…";
    urls = [ "https://download.kiwix.org/zim/wikivoyage/wikivoyage_en_all_nopic_2026-09.zim" ];
    license = "CC-BY-SA-4.0";
    source = "Wikivoyage contributors; packaged by Kiwix (download.kiwix.org)";
    notice = null;                                # shown before opening
    data_date = null;                             # optional: "YYYY-MM-DD" or "YYYY-MM"
    coverage = null;                              # optional: what is covered and what is not
    attribution = null;                           # optional: the credit the license requires
  }
];
```

File names: ASCII letters, digits, `.`, `_` and `-`, at most 200
characters, not starting with a dot, no `..`, ending in `.zim` or
`.mbtiles`. A trailing `_YYYY-MM` marks the version: a file with the same
name before the date and the same extension is recognised as an older
version of a catalog pack.

`data_date`, `coverage` and `attribution` are shown in the pack details
(Enter on the Resource packs screen), coverage in full (at most 1000
characters; attribution at most 300). A map pack must have all three and
be in the `maps` category, and only map packs may be in it; the build
fails otherwise. The coverage text is where a map says what it leaves out.

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
- ZIM packs on mounted sticks are listed in the Offline library and can be
  checked with `resilience-library check FILE` (map files too).
- The Offline library lists only ZIM files. `resilience-library open` on an
  `.mbtiles` file refuses with a message pointing to Maps & navigation; it
  never starts kiwix-serve for it.

## Not yet

- Maps in the interface and regional map packs in the catalog (Phase 5, ADR 0016), documentation packs of our own, signed packs that can be
  updated without a system update (Phase 8).
