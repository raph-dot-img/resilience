# Testing

Every result below is **PASS**, **FAIL**, **BLOCKED** or **NOT TESTED**.
A test is only PASS if it was actually executed. "VM" means QEMU/KVM on the
development host — physical hardware: one boot of the live ISO on a ThinkPad L15 Gen 1 (see "ISO boot").

Last full run (`nix flake check -L --max-jobs 1 --keep-going`, all 19 checks incl. 13 VM tests): 2026-11-17 on `develop` at `af4b661` (mesh network, ADR 0021), every check PASS in 13 min; `vm-comms`, `vm-mesh` and `comms-messages` were taken from their passing builds of the same derivations in the run just before (at `7039e06`). That run and the one before it each had one failure, both fixed: `vm-crypto` (GnuPG asked a third time for the passphrase while creating a key on the loaded host: `max-cache-ttl 0` expires the creation's cache nonce; the tool now says so and the test answers it) and `vm-modes` (`stty` on tty2 failed with EIO while its getty started; every test's `tty_text` now retries it). Earlier full run: 2026-11-17 on `feat/comms-files` at `73754e8` (multi-line messages, ADR 0020 files), every check PASS in 17 min; `vm-comms` and `comms-messages` were taken from their passing builds of the same derivations earlier that day (vm-comms 318 s), the other 10 VM tests ran. Earlier full run: 2026-11-17 on `feat/comms` at `ba62020` (ADR 0019), every check PASS in 36 min. Previous full run 2026-11-16/17 (slice 3 + messaging on in standard): `vm-crypto` failed on the first pass (it pressed `3` for Security, which slice 9's Maps & navigation had moved to `5`); fixed in `7f1a0ac`, then PASS. ISO last built and boot-tested 2026-11-17 from `develop` at `af4b661` (see "ISO boot"; bit-for-bit reproducibility not verified).

## How to run

| What | Command | Time on dev host |
|---|---|---|
| Rust unit + TUI tests | `cargo test` | ~5 s |
| Lints | `cargo clippy --all-targets` (must be clean), `nix fmt` | ~10 s |
| Screen dump for review | `RESILIENCE_DUMP=1 cargo test dump_screens -- --nocapture` | ~2 s |
| Evaluate everything | `nix flake check --no-build` | ~30 s |
| Everything incl. VM tests | `nix flake check -L --max-jobs 1` | ~5 min cold |
| One VM test | `nix build .#checks.x86_64-linux.vm-modes -L` (also `vm-admin`, `vm-backup`, `vm-wifi`, `vm-generations`, `vm-packs`, `vm-documents`, `vm-crypto`, `vm-boot-offline`) | ~1–2 min |
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
| resilience-core: mode, status, network, catalog, config, handbook, notes, launch (incl. env overrides, polkit agent ordering; the fake agent is retried on ETXTBSY, a test-only race with other test threads forking, fixed 2026-11-17 in `9d3ebd2`), proc (incl. `run_visible`: clean environment, timeout kills the whole process group, with a positive control), admin (names, modes, config, refusals, overrides/staleness, Ultra-Stable-only machines, group ops, audit lines, timestamps), recovery (generations, generation numbers, default-generation rules: existing, not default, closure directly in the store with switch-to-configuration; report files), media (lsblk parsing/escapes, removable-only selection, device validation, udisks errors), backup (names, checksums, listing, size estimate, pipeline failure cause, **real age round trip**: encrypt, check, restore, no overwrite, wrong key, damaged with/without checksum), clock (local and strict UTC parsing, impossible dates, bounds, zone names), wifi (nmcli terse escapes, security kinds, scan dedupe/sort/saved match, profiles, private profile argv without secret, password only on stdin with a fake nmcli, password rules, failure hints), packs (ids, file names, https-only URLs, version stems, strict catalog, plan/outcome lines, chunk framing limits, receive with resume, wrong offset skipped in step, mismatch and extra bytes deleted, sender vanishing keeps the bytes, full-size part restarted, free-space margin, other versions, safe removal (symlink refused), packs found on a stick, reader URLs, sets and built-in entries; map packs (ADR 0016): `.mbtiles` names and refused look-alikes (`-wal`, `-journal`, `.part`, case), kind by extension, `maps` category and its order, optional `data_date`/`coverage`/`attribution` absent on ZIM entries and present on maps, map entries refused without them, in another category, with a bad date, empty or too long texts, ZIM refused in `maps`, built-in map category, older map versions by stem and kind, map files received/removed like any pack, found on a stick), proc (output above the 64 KiB pipe buffer, non-UTF-8), files (index skips hidden entries, links and other file systems; kinds; text cap and control characters; CSV quotes and column cap), manpage (names, sections, searches; nothing read as an option; secure pager environment), crypto (base64, minisign key numbers from real keys and signatures, public keys in many forms, secret keys detected (a signature is not one), contact names, key store round trip and 0700, format recognition: age binary/armored with recipient types, minisign, OpenPGP armored/binary packets, SSH, PEM, OpenSSL, LUKS, ZIP encrypted flag, checksum lists; output names, free names, dangling links, O_EXCL, RENAME_NOREPLACE; expected checksums in many forms; trusted comments; xkcdpass output), openpgp (real gpg 2.4.9 status lines: good, bad, unknown key, revoked, GOODSIG without VALIDSIG not trusted, lines not from the status protocol ignored; decryption integrity, no secret key, bad passphrase, signed messages; imports; colon listings with subkeys and escapes; recipients of a message; user ids) | 125 | PASS (round trip needs age in PATH: runs in the Nix build; SKIPPED on a host without age) |
| resilience-core maps::geo (ADR 0016 slice 2): position validation and longitude wrapping (±180, ±540, NaN, infinities); Web Mercator round trips at known points, clamp at the poles and beyond the square; tile numbers of known places (OSM wiki Brandenburg Gate z17 70406/42987, Luxembourg, Paris, Sydney), poles and 180° edges; TMS row flip both ways and validation; zoom ladder steps in/out, sticky ends, scales between steps, NaN/negative scales; tile zoom at every ladder step at 45°, exact at the equator, clamped to a pack's range (overzoom); tiles per view bounded for every step × 7 latitudes × 5 longitudes × 6 sizes (≤ 64 always, ≤ 6 on 170×96 up to 70°), too detailed min zoom refused, oversized views clamped; antimeridian tiles touching, repeated world, no rows past the poles; tile transform puts the centre on the screen centre, tile span, overzoom, saturating pixels; pixel/position round trip, nearest copy across 180°; panning across 180° and stopping at 85°; distances Paris–London, New York–London, Luxembourg–Esch, antipodes, poles, across 180°; bearings and 8-point compass sector boundaries; distance text table; "17 km SW"; decimal and DMS formatting (rounding never shows S/W for 0, carries); parsing table (34 accepted forms) and bad-input table (41 refusals with their error), 500 random format→parse round trips, 5,000 random strings never panic; scale-bar lengths for each step and every scale from 5 m to 200 km, odd scales, bad inputs | 21 | PASS |
| resilience-core maps::mvt (ADR 0016 slice 3), tiles built with the test-only encoder: a sample tile with every geometry and value type (parts, roles, attributes by key and key index, ids, extents); the geometry examples of the MVT 2.1 specification §4.3.5 (point, multipoint, line, multiline, polygon, multipolygon with a hole); empty tile; unknown fields of every wire type skipped at tile, layer, feature and value level; layers of version 1, 3 and without version skipped and counted (still checked), unknown geometry types skipped and counted; structural errors (odd tags, key/value index out of range, missing name, duplicate names, values with zero/two fields, invalid UTF-8, wrong wire types for known fields at each level, group and undefined wire types, field number 0 and above 2^29); geometry errors (missing/extra commands, zero counts, huge counts refused without looping, rings under 3 points, ClosePath count, unknown command), i32 coordinate extremes accepted and exceeded by one, zero-area ring dropped, unpacked repeated fields; varints (u64::MAX in 10 bytes, overlong 5-byte encoding accepted, 11 bytes / 10th byte > 1 / unterminated refused, u32 fields above u32, lengths past the end and past usize), zigzag extremes; every truncation of a one-layer tile is an error and the sample tile decodes only at layer boundaries; each bound at its value and one above (64 layers, 65,536 keys, 65,536 values, 200,000 features, 2,000,000 geometry integers, 2,000,000 tag integers, 1 KiB strings in names/keys/values, extent 256..65,536, 8 MiB tile); 2,000 random byte strings, 4,000 mutated sample tiles and 2,000 structurally plausible random tiles never panic, and whatever decodes is consistent | 12 | PASS |
| resilience-core maps (ADR 0016 slice 4, compared as text): canvas (half-block composition `▀`/`▄`/space, lines in every direction, Liang-Barsky clipping on every side, dashed and dotted patterns, dashes continuous across 1-pixel and 0.25-pixel segments and across the off-canvas part, even-odd fill with holes of either winding and several exteriors, polygons far outside or covering the canvas, nearly flat edges, `░` shading inside/outside a polygon and of a union, detail kept under shading, overlays clipped and sanitised; hostile geometry: NaN, ±inf, f64/i64/i32 extremes, a line from -1e300 to 1e300, a 20,000-vertex zigzag, degenerate rings, zero and oversized canvases, all fast); style (visibility of each ADR class at all 15 zoom steps against an independent transcription of the ADR table, colours and styles, scale rounding, nonsense scales, label details: POI names at 10 m, peak elevation at 100 m, street names at 5/10 m, priority order, distinct POI letters, every class name round-trips and has exactly one rule except farmland, boundary levels, drawing order, builder min zooms; legend: every rule in exactly one entry with its scale, words and colour, entries shown at each step exactly when drawn, shading and coverage edge on opposite sides of 500 m); labels (folding table for Latin-1, Latin Extended-A, Romanian, ligatures, typographic quotes, fullwidth, `©`→`(c)`; control characters, escape sequences, bidi overrides, zero-width, combining marks, Arabic/CJK/Cyrillic/Greek, emoji, 40-character cap with `...`, 10 MB inputs in bounded time; every code point U+0000-U+2FFFF folds to printable ASCII; placement: positions, margin, priority, edges, reserved cells, marker-only POIs, 3,000 random candidates never overlap or touch, order-independent, 50,000 candidates capped at 4,096 highest priorities, huge texts and extreme anchors; street names on the longest flat stretch, only on-canvas length, too short, vertical, NaN, back and forth; whole scenes at 10, 50 and 500 m drawn by the style table contain only console glyphs) | 27 | PASS |
| resilience-maps (ADR 0016 slice 5), synthetic packs written at test time (tippecanoe schema, gzip MVT tiles from the core test encoder, the `resilience-1` tables with rows), every test that opens a file asserting the directory is byte-for-byte unchanged and that it returned promptly: valid pack reads back (metadata, tiles through TMS and XYZ numbering, absent tile and zoom, coverage ring, capabilities, tile count, quick_check); FTS5 search and R*Tree lookup work under the authorizer; refusals each with its specific error: random bytes and empty file (not SQLite), truncated at 5 lengths, symlink (our O_NOFOLLOW and SQLite's own NOFOLLOW; a symlinked directory is resolved), directory and FIFO, WAL mode, recursive view as `tiles` (and a hardened connection cannot run it, recursive CTE denied), `metadata` as a view with an expensive function, view calling `load_extension`, `load_extension`/ATTACH/`readfile`/pragma writes/INSERT/unverified table on a hardened connection, trigger, generated column, computed default, missing column, expression and partial indexes, FTS5 virtual table as `tiles`, unknown collation (crafted schema text), schema row whose type contradicts its text, 300 schema objects; mbutil/tilemaker (`map`+`images`) and Planetiler (`tiles_shallow`+`tiles_data`) layouts read through their tables with a never-ending or expensive `tiles` view; an `rs_` statement differing by one character disables search only (map, coverage, nearby still work; its tables unreadable); no profile → features "not our profile"; gzip bomb tile (64 MiB of zeros) stopped at 8 MiB; tile over 4 MiB refused by SQLite's LENGTH limit; PNG tile and invalid MVT; metadata missing, reversed zooms, bounds outside the world, 1 MiB name, raster format, bad data date, missing licence; hostile strings (escape sequences, line breaks, C1 CSI, bidi override) cleaned; progress handler stops an unindexed scan (small budget), a triple cross join (2 s budget) and quick_check; quick_check reports a damaged index page with cleaned messages; read-only directory; 60 randomly mutated packs never panic or hang; reader in a child process under RLIMIT_AS 1 GiB (2 GiB reservation fails there); bundled SQLite compiled with OMIT_LOAD_EXTENSION and MAX_MMAP_SIZE=0; header checks (magic, page sizes, WAL bytes, format version, payload fractions, header page count); file URIs; the profile schema with its shadow tables; inflate (CRC-32 check value, gzip/zlib/uncompressed accepted, PNG/JPEG/WebP refused, optional gzip header fields and header CRC, damaged checksum/length/trailing data/reserved flags/method, bombs in gzip and zlib, exactly 8 MiB accepted, 3,000 random and mutated blobs); metadata, schema, profile blobs and text cleaning unit tests | 56 | PASS (host and `nix build .#resilience`, 2026-11-15) |
| resilience-maps viewer (ADR 0016 slice 6), synthetic packs written at test time (a detailed "Testland" with roads, a lake, a border, a town and a POI in every tile of zooms 0-13; a "World overview" with land, a city and a border at zooms 0-7), the reader driven synchronously and screens drawn with ratatui's TestBackend: first screen on the overview (readout `49.00000 N  5.00000 E` from its centre, scale bar, `Natural Earth`, `Map:` line, hints) and on a detailed pack (its centre and zoom 12 snapped to 20 m, `(c) OpenStreetMap contributors`; POI and street names at 10 m, town at 200 m); zoom in/out keeps the readout, pan a quarter screen east moves the readout by 20 pixels of 100 m and the road along a known longitude from the crosshair column to a quarter screen left, back again, one cell with Shift (two pixels), `d` to degrees-minutes-seconds; outside the coverage at 50 m "No detailed map here: world overview only" and `░`, none inside, none at 5 km where the dotted edge is drawn; only the overview → shaded; only detailed packs (no overview) work, and far away say no open map covers the place; two overlapping detailed packs drawn by data date whatever the command-line order (newer replaces the older inside its coverage, the older's lake shows outside, the older's label inside the newer is dropped, `Map:` names the newer); a pack without a coverage polygon: role approximate, status says it does not state its area, its outer limit drawn at fine scales, information screen says so, without bounds it lifts no shading; information screen with title, "checked against the catalog of this system" for a file in the packs directory of a catalog, "not checked against the catalog ... passed its integrity check" for another, data date, coverage, source, licence, attribution and the OpenStreetMap sentence, scrolling at 40×12; legend at 500 m (only what is drawn), `a` all scales with their ranges; help (`?`, F1), `q`/Esc back and quit; reserved keys `/`, `n`, `g` do nothing and are not in the hints; below 40×12 a plain message, every size from 40×12 to 250×60 draws; no pack at all says how to get one (exit 0); damaged tiles (bad gzip, bad MVT) leave the good tile drawn, one notice that expires, never asked again; a random-bytes file and a missing file are explained while the good pack shows, a truncated file alone gives the failure page and exit status 2; a damaged unchecked file is not drawn while checked, then the failure page; a large unchecked file is offered the check (`c` queues it); a reader that does not answer leaves the viewer responsive ("Reading the map...", then "slow" after 1 s), a read over its budget is reported once, a lost reader thread is said; the last position restored, `--at` and `--scale` each override their half, nothing to save before the start position is known, `--search` kept for later; renderer compared as text: a road cut at a tile edge is continuous, a dashed border cut at a tile edge keeps its 5-pixel period (and drawing each piece from phase 0 breaks it, so the test sees the artefact), overzoom past the pack's maximum zoom draws the same line 2.5× and 2× longer, underzoom below its minimum zoom draws nothing and asks for no tile, a pack's area far away asks for none, names folded (`Évry` + OSC/CSI escapes + bidi override + Cyrillic → `Evry]0;pwned[2Jx ?`) and hostile titles and HTML attribution cleaned; console glyph test on every screen (map at 5 m, 50 m, 5 km, 200 km, damaged pack, no pack, info, legend, all-scales legend, help, DMS) at 80×24, 60×20 and 160×50; argument table (accepted forms, 28 refusals, non-UTF-8 paths, escape sequences kept out of messages); last-position file (round trip, mode 0600 and directory 0700, no temporary file left, 15 corrupt texts, too large, not UTF-8, symlink not followed and replaced on save, directory in its place); tile cache (count and byte bounds, LRU order, pinned tiles never evicted, absent and bad tiles remembered, slow ones retried after 10 s, side tables bounded); coverage areas (point in polygon with a hole, rings across the antimeridian, bounds rectangles); reader (checked only in the packs directory or built in, read from a real catalog file; small unchecked pack checked before use; the thread answers tiles and its check runs on a thread of its own); in resilience-core, `Canvas::polyline_from` (a start phase shifts dashes and dots; whole periods and non-finite phases are phase 0) | 40 + 1 | PASS (host, 2026-11-15) |
| Luxembourg map pack (`nix build .#mapPacks.luxembourg`, Geofabrik 2026-01-01): builds in ~2 min (tags-filter 2.4 s with **2.1 GB peak RAM**, export 3.8 s, classify 3.8 s, tippecanoe 16.3 s, finish 1.9 s); 29,315,072 bytes, 1,897 tiles, SHA-256 `975fc460c38770caf2c36b712d115b98f829a107b58eddc9928aac27baa015dd`; largest tile 458 KB compressed / 882 KB decompressed; `resilience-maps info --check`: profile resilience-1, coverage polygon, search and points of interest available, integrity ok | 1 | PASS (host, 2026-11-16) |
| Viewer on the real Luxembourg pack, NixOS VM, Linux console on a 1366×768 framebuffer (ter-i16n, **170×48**), 2 vCPU, 2 GiB: starts at the pack's centre (2 km bar), zoom in to street scale, pan, zoom out to 200 km, information and legend screens, quit; position file written; viewer peak memory (VmHWM) 30 MiB. Screenshots looked at. Found and fixed: POI letters touching their names, the capital's name dropped under the crosshair, a town named twice. Not tested: performance on a slow CPU, the overview (slice 8 not done) | ad hoc test (not committed; becomes `vm-maps`, slice 12) | PASS (2026-11-16, after the label fixes) |
| resilience-maps `view` (the real binary): usage errors exit 64 before anything is opened; without a terminal it exits 1 with a message, nothing written beside the pack | 1 + 3 cases | PASS (host) |
| resilience-maps `view` under a pseudo-terminal (throwaway Python driver, 170×48, release build, dense pack of the perf test): pans, zooms, legend, info; `q` and SIGTERM both leave the alternate screen and exit 0, `maps.toml` written 0600 in a fake HOME; a random-bytes pack → exit 2 | manual | PASS (host, 2026-11-15) |
| resilience-maps `info` (the real binary, which lowers RLIMIT_AS to 1 GiB first): valid pack, keys in their fixed order and values, `--check` → integrity ok; hostile strings and an escape sequence in the file name come out cleaned, the forged `profile:` line stays inside the name; random bytes and WAL → exit 2 "Refused", missing file → exit 1, usage errors → exit 64; directory unchanged | 3 | PASS (host and Nix build) |
| resilience-crypto (host end-to-end, two accounts, real age 1.3.2 / minisign 0.12 / gpg 2.4.9 under a pty): keys created (age identity armored+encrypted, 0600), export, add contact (merge, private key refused), encrypt for a contact and with a passphrase, decrypt by the recipient, wrong passphrase → nothing written, no terminal → clear message, sign/verify/tampered/unknown signer, checksum/compare (match, mismatch, MD5 warning), SHA256SUMS list (found the translated-output bug: now LC_ALL=C), inspect, messages both ways, OpenPGP import/verify/encrypt/inspect, mixing age and OpenPGP refused | manual | PASS |
| resilience-library and resilience-pack-store: strict store arguments (ids, file names incl. `.mbtiles` and refused look-alikes, at most 64), library arguments, chunked copy stops at the size, loopback listener owner from /proc/net/tcp, `open` refuses a map pack with the Maps & navigation message before any server starts | 4 + 1 | PASS |
| resilience-tui: navigation, rendering 80×24…200×60, modes, restricted commands, missing programs, confirmations, Ctrl-C, handbook, search, notes, themes, shell gating, wrapping, console glyphs; Users & modes (consequences, Cancel default, fixed pkexec argv + SHELL + agent, refusals, no administrator, corrupt config); Recovery (health, guide, report 0600 / no overwrite); System generations (list and states at all sizes, empty list, current default and incomplete generation explained, confirmation with Cancel default, fixed pkexec argv + SHELL + agent, list refreshed after the helper, no administrator); Backups & USB sticks (internal disk never listed, unusable devices, mount via `--no-user-interaction`, Cancel default, backup/check/restore argv, eject busy / safe, lsblk failure); Date & time (prefilled, input filter, errors kept in the dialog, Cancel default, local→UTC argv, no administrator); Wi-Fi (list at 80×24, password dialog masked, nothing created before confirm / on Esc, private profile, secret on stdin only, wrong password → nothing saved, saved network without password, refused saved password → dialog, forget + Cancel default, admin profile not deletable, open warning, 802.1X/WEP refused, polkit refusal, radio off, no adapter, no nmcli); Resource packs (grouped by category, headers skipped, wrap-around, states and percentages, language filter, ticks [+]/[-], partial download continue/discard cycle, totals, offline refusal, confirmation with Cancel default, fixed `resilience-library change` argv, refresh after the run, stick chosen and mounted, packs found on a stick, missing ones named); Offline library (installed and older versions, open and search argv as single arguments, hidden without a catalog, also from search); suggested sets; packs on a stick (listed, checked, opened); map packs (ADR 0016: `maps` category row, details as a scrolling view with the whole coverage text, data date and attribution on the first 80×24 page, ZIM details without the new fields, older map version; the Offline library and its stick listing never offer a map file to the reader); Manual pages (validation, missing page, secure pager with a clean environment, search in the TUI); Find & view files (hidden files not searched, text, CSV table with column scrolling, PDF, archive listing, details and F3 checksum); Security & cryptography (every mode; picker titles and Enter chooses; "Encrypt for": passphrase only without keys, me/contacts/OpenPGP never mixed, nothing ticked refused, Esc runs nothing; decrypt/verify/inspect/sign argv; sign without a key explained; messages; checksum input validated before running; random secrets shown and never in notifications; Keys & contacts: create, add by file/typed, export to a stick, details, remove with Cancel default; OpenPGP keys listed only when ~/.gnupg exists, removed with gpg --homedir, offered as recipients) | 62 | PASS |
| resilience-maps mapmaker `natural-earth` and `finish --center` (ADR 0016 slice 8): Natural Earth land → `landmass` with Natural Earth's own minimum zoom, features beyond zoom 7 left out (lines, areas) or kept at zoom 7 (places), wrong geometry type and `null` geometry skipped and counted; country and region border lines with their levels, upper-case property names read, region borders never before the style draws them; named lakes and rivers give search rows (centre, box, rivers merged), unnamed ones none; places are cities (capital or 100,000 inhabitants) or towns, names cleaned, ranks by population, unnamed places left out; invalid JSON is an error, positions outside the world or with altitude skipped, odd property types ignored; `finish` with a given centre writes it and refuses one outside the map | 6 | PASS (host, 2026-11-16) |
| The real world overview pack drawn by the viewer code (`tests/world.rs`, on demand: needs the built pack): first screen 170×48 on the whole world with sea and land both drawn (blue cells between a quarter and three quarters), `Natural Earth` and `World overview` on the status line; Europe at 5 km (Paris, Munich, Zurich named), Luxembourg at 500 m (name, `░` shading, "No detailed map here"), Japan at 2 km (Tokyo), Brasilia at 2 km on 80×24. Text output looked at; **colours not looked at on a real console** | 1 (ignored by default) | PASS (host, 2026-11-16) |
| resilience-tui Maps & navigation (ADR 0016 slice 9): the section has "Map" and "Maps on this computer & USB sticks" in every mode, found by search ("atlas"), absent (and the section with it) without a packs catalog; Map with no map at all explains and runs nothing; with the built-in overview: fixed argv `resilience-maps view --pack WORLD`, no agent, no pause; with an installed pack: overview first, then the pack, the older version not opened by default; at most 8 `--pack` with ten packs installed; the list shows only maps (built in, installed, older version, a file on a stick) at 80×24, 60×20 and 160×50 with console glyphs only; Enter on each kind gives its argv (overview alone; overview + pack; overview + older + current; overview + stick file + installed); `c` runs `resilience-library check` on the stick file; `s` does nothing; F1 shows the maps help; `p` opens Resource packs; the home screen now lists MAPS & NAVIGATION | 3 (+2 changed) | PASS (host, 2026-11-16) |
| resilience-core messages (ADR 0018): addresses (32 lower-case hex; typed with groups, dashes, colons, upper case; formatting in groups of four); text made safe to draw (control characters, escape sequences, direction overrides, zero-width characters, length caps, tabs); records read with bounds and sorted, skipping a wrong id, an unknown state, a bad peer, junk, a symbolic link, a 2 MiB file; markers (`waiting`, `delivered`, `from them`, `from them, NOT VERIFIED`); conversations with counts; status from the program's file (running, stale, never run, damaged file); peers with cleaned claimed names; contacts saved 0600, cleaned, removed; queue refuses a bad address, an empty or too long text and one's own address, writes only the finished file (0600, directory 0700) with the text as JSON; cancel refuses odd ids; rough times | 7 | PASS (host, 2026-11-16) |
| resilience-tui Local messages (ADR 0018): absent from the menu and from search without the machine's messaging service, present in every mode with it; a program that is not running and a missing address are said plainly, the address shown in groups; writing to an address: not an address and one's own address refused in the dialog, an address typed in groups and upper case accepted, empty message refused, the request file holds the exact text (quotes, `$(...)`) and the notice says "saved", never sent or delivered; a conversation follows the program's records by itself (waiting → sent → delivered), shows a verified and a NOT VERIFIED incoming message, a FAILED one, a hostile text without its escape sequence or direction override, at 160×50, 80×24 and 60×20 with console glyphs only; `c` cancels only a waiting message; naming a person changes the titles and lists; `i` shows the whole address; people nearby with a claimed name quoted, questioned and cleaned; help page | 5 | PASS (host, 2026-11-16) |
| resilience-tui files in messages (ADR 0020): `f` in a conversation opens the file picker; a 900,001-byte file → "CANNOT SEND THIS FILE" with the 900 KB limit, nothing written; a chosen file → "SEND A FILE" with its name and size, the text typed, then `outbox/ID.file` (exact bytes) before `outbox/ID.json` with `"file"` and the text, the notice says a copy is saved; a received file shown as `[file] NAME (SIZE)  o open, s save` with a hostile name cleaned, one not kept as `not kept`; `o` shows it in the pager; `s` moves it to `~/Received/` under a safe name (`_..` prefix, ` (2)` before the extension) beside the user's own file, which is untouched; then `o` and `s` say it is no longer here; core: copies taken through one descriptor, 0600, folders and oversize files refused, `data` names outside `ID[.ext]` (including `.data`) ignored, `../` and `.ssh` paths never followed | 2 TUI + 2 core | PASS (host and Nix build, 2026-11-17) |
| resilience-tui multi-line messages: Ctrl-N, Ctrl-J and Alt-Enter start a new line in the message dialog (Enter still sends), each typed line drawn on its own line with the cursor after the last; Backspace removes a line break; the request file holds the text with `\n`; only line breaks → "the message is empty"; a contact name stays on one line; at most 40 lines, the end being typed stays on screen | 1 | PASS (host and Nix build, 2026-11-17) |
| resilience-core admin `plan_set_mesh` (ADR 0021): only `on` and `off`, only the configured unit (a `.service` name of safe characters, never starting with `-`), an absolute `systemctl`; without `mesh_unit` the operation is unavailable; the plan is exactly `systemctl start|stop -- UNIT` | 1 | PASS (host and Nix build, 2026-11-17) |
| resilience-tui Mesh network (ADR 0021): absent (even from search) without `mesh_unit` in admin.toml, present with it and found by search ("mesh", "router"); its confirmation says it is off and that the card cannot join an ordinary network meanwhile, Cancel focused (Enter runs nothing); "Turn on" runs `pkexec HELPER set-mesh on` with `SHELL` = the session shell and reports it on; with `/run/resilience-mesh/devices` written it shows "ON (wlan0)" and "Turn off" runs `set-mesh off` | 1 | PASS (host and Nix build, 2026-11-17) |
| resilience-core install (ADR 0022): disk list from lsblk (whole disks only; loop, CD, zram, partitions left out; in use when anything below is mounted, read-only, under 16 GiB), partition paths (sda3, nvme0n1p2), the sfdisk script and its labels, request ↔ arguments round trip and refused arguments, the disk must still match size and model, keymap offered, account rules (names, reserved and taken names, duplicates, full names, at most 8), state written for local accounts (overrides of Ultra-Stable), install.toml paths; admin local accounts (base mode Ultra-Stable, overrides, a declared account kept, bad files refused); keyboard state file (`KEYMAP=<offered id>` only) | 14 | PASS (2026-11-17) |
| resilience-tui Install (ADR 0022): absent without install.toml, present with it; defaults (encryption on, BIOS here, current layout, user Stable + administrator Advanced); Install without a disk refused; the live stick not choosable; disk, encryption, keyboard, account name (bad name refused), mode cycling, full name, add and remove; the summary shows the disk, its partitions and the no-administrator warning; a wrong disk name does nothing; the exact pkexec argv and SHELL | 2 | PASS (2026-11-17) |
| `cargo clippy --all-targets` | — | PASS (0 warnings) |

### Nix

| Check | Result |
|---|---|
| Flake evaluation (`nix flake check --no-build`) | PASS |
| `eval-live-iso`: live-standard, live-minimal, dev-vm evaluate | PASS |
| `security-invariants`: 33 invariants × 11 configurations (the 8 installed systems of ADR 0022 added; new: installed systems keep user namespaces and the sandbox, have no autologin and keep root locked, the installer exists on live systems only, the session shell listed in /etc/shells is installed) (2026-11-17: new "the mesh comes only with local messaging and starts on request": mesh only with comms, never `autoStart` in the shipped systems, `bat0` unmanaged by NetworkManager; the firewall one now allows exactly UDP 29716 and 42671 where local messaging is on, and nothing else; two earlier ones: the messaging service is unprivileged with nothing of Reticulum in `PATH`, and the minimal edition has no local messaging) | PASS (2026-11-17) |
| Invariant checker rejects a live configuration with user namespaces forced on | PASS (manual, once) |
| Invariant checker rejects a deliberately broken configuration (ssh on, root password) | PASS (manual, once) |
| Invariant checker rejects a privileged autologin account and an extra polkit admin identity | PASS (manual, once; this found that the autologin invariant had been vacuous) |
| Build-time catalog check (every app command executable) | PASS (part of every system build) |
| Build-time tool check (`[tools]` in system.toml executable) | PASS (part of every system build) |
| ISO derivation builds (standard) | PASS — 1.48 GB |
| ISO derivation builds (minimal) | NOT TESTED (evaluates only) |
| `comms-messages`: `resilience-messages` with the real Reticulum 1.2.9 and LXMF 0.9.8 in the build sandbox (one instance, two accounts' programs): refuses to start without the client configuration or with a wrong command, creating nothing; store 0700 and identity 0600; a bad address and an empty text → `failed` with the reason and no text kept; a message to an address never heard stays `queued`; cancel → `failed` "cancelled"; the queue and the address survive a restart of the program; when the recipient appears the message becomes `delivered`, arrives once, `verified`, with the sender's address; the recipient's claimed name is recorded; outbox and cancel directories emptied; a reply is delivered and verified; the cancelled message never arrives | PASS (2026-11-16) |
| `comms-messages` with the reply on two lines: the line break arrives through LXMF unchanged (`"OK, see you there\nwith cans"`) | PASS (2026-11-17) |
| `comms-messages`, files (ADR 0020): a 300 KB file sent directly arrives byte for byte, kept by the recipient as `files/ID.pdf` (0600, the only file there) with the name that came with it (`../../Liste des besoins.PDF` → `Liste des besoins.PDF`) only in the record, verified; the sender's copy is removed once delivered; a 900,001-byte file and a request whose file is missing are refused with their reason and leave nothing behind; a 400 KB file (above LXMF's default 256 KB store limit) is left in the machine's store and collected intact after the sender is stopped | PASS (2026-11-17) |
| `comms-messages`, store-and-forward (ADR 0019): `node` refuses a missing `--dir` or a 0 MB limit, `run` refuses node options; without the instance a program exits 75 ("not running yet") instead of becoming the instance; the store starts (identity 0600, address file) and the programs pick it up; Bob's program stopped, Ann's message is `sent` with `stored` true after 2 minutes and the store holds it; Ann stopped; Bob's program starts, collects it once, `verified` (key kept from before) | PASS (2026-11-17) |
| `comms-messages` with announces again 30 s and 90 s after a program or store starts and 10, 30, 90 s after the instance restarts (client configuration rewritten): every scenario above unchanged | PASS (2026-11-17) |
| `map-world`: the built-in world overview (`nix build .#resilience-world-map`, Natural Earth 5.1.2: 10 land features, 515 country and 4,616 region border lines, 1,355 lakes, 1,454 rivers, 7,342 places; 11,362 tiles, zooms 0-7, largest tile 50 KB compressed) read by `resilience-maps info --check` (profile, data date, licence, attribution, coverage, zoom 0-7, centre, search available, integrity ok), size between 8 and 16 MB (**11,735,040 bytes**), 7,342 settlements, Luxembourg a city at its position, Rhine and Lake Victoria searchable, prefix search without accents finds Reykjavík, one zoom-0 tile and 8 zooms | PASS (2026-11-16) |
| World overview in a system image: opened from the TUI in `vm-maps` (minimal system) | PASS (2026-11-16) |
| ISO rebuilt with the world overview, closure growth measured | NOT TESTED (live-standard and live-minimal evaluate) |
| Standard edition with local messaging on by default: live-standard and dev-vm evaluate with the service, live-minimal without it | PASS (evaluation only, 2026-11-16) |
| Standard image with local messaging built and booted; messaging on the live ISO | NOT TESTED |
| World overview reproducibility (two builds, same bytes) | NOT TESTED |
| `vm-packs` re-run with the built-in world map in `packs.toml` (the system of the test holds the map; packs screens, library and stick listings unchanged) | PASS (2026-11-16) |

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

### VM: `vm-packs` — resource packs and the offline library (ADR 0013), driven through the TUI

Two machines: an HTTPS mirror (nginx, test certificate, 256 KiB/s) and the
Resilience machine (alice Ultra-Stable on tty1, carol Advanced). The test
ZIM file is built with `zimwriterfs` during the build.

| Check | Result |
|---|---|
| Store: packs directory `resilience-packs` 0755; the store refuses alice and root ("must run as resilience-packs"), bad ids (usage) and unknown ids; alice cannot write there; `pkaction` auth_admin for any/inactive/active; `pkcheck` on the TUI → challenge | PASS |
| TUI: catalog grouped by category with sizes; language filter | PASS |
| USB: stick hot-plugged, `u`, mounted through udisks, pack found in `Resilience-packs/`, confirmation, polkit prompt shows the exact store command line, carol's password → "installed and checked"; file `resilience-packs` 0644, SHA-256 equal to the catalog; journal `op=get … result=ok` | PASS |
| Library: notice shown, pack opened in lynx, kiwix-serve on 127.0.0.1 only, link followed, `!` → "Spawning is currently disabled" and no shell process, `g` refused; server stopped after quitting; full-text search finds the marker word | PASS |
| Wrong administrator password → "Not authorised", nothing staged | PASS |
| Download interrupted with Ctrl-C (part kept, between 0 and the size), shown as "% done", resumed ("continuing from"), mirror log shows `206 bytes=<part>-`, SHA-256 equal, part removed | PASS |
| Mirror serves other bytes for a pack → "not the expected file", nothing installed or staged, journal `result=mismatch` | PASS |
| Ticked installed pack removed with the administrator's password; other packs kept; system `running` | PASS |
| Library: built-in "System documentation" listed first, opened, full-text search works; a pack still on the stick is listed as "USB stick", `c` → "MATCHES", opened from the stick by kiwix-serve running as alice | PASS |
| Map pack (ADR 0016 slice 1; fake bytes with a pinned size and hash, entry through the Nix option with `data_date`, `coverage`, `attribution`): `-- MAPS --` on the packs screen; details show the whole coverage, the data date and the attribution; installed from the stick with the ZIM (`resilience-packs` 0644, SHA-256 equal); not listed in the Offline library (installed or on the stick); `resilience-library open` on it (installed and on the stick) → exit 2 with the Maps & navigation message, no kiwix-serve; `check` → MATCHES; removed with the ZIM (2026-11-15) | PASS |
| Real Kiwix downloads, multi-GiB packs, stick removed during a copy, two concurrent changes, physical hardware; a real MBTiles file (none exists yet) | NOT TESTED |

### VM: `vm-documents` — Find & view files and manual pages (ADR 0014), driven through the TUI

| Check | Result |
|---|---|
| Find & view files (Ultra-Stable): text file found by name and shown; CSV as a table; a real PDF (Ghostscript) through pdftotext; a zip listed, nothing extracted; F3 shows the right SHA-256 | PASS |
| Manual pages: `ls` opens in less with `LESSSECURE=1` in its environment; `!` → "Command not available", no shell process; "copy files" (apropos --and) lists `cp (1)`; system `running` | PASS |
| Very large PDFs and archives on real hardware | NOT TESTED |

### VM: `vm-crypto` — cryptography (ADR 0015), driven through the TUI by an Ultra-Stable account

| Check | Result |
|---|---|
| Keys created through the TUI (passphrases typed into age and minisign); key folder 0700, key files 0600, age identity armored and encrypted, minisign key encrypted; `AGE-SECRET-KEY` nowhere on disk | PASS |
| Encrypt for "me", decrypt with the key into ~/Decrypted (0700/0600, identical); wrong passphrase → message, nothing written | PASS |
| Encrypt with a passphrase, decrypt with it | PASS |
| Inspect: age file, opens with a passphrase | PASS |
| Sign, check (VALID, own key), file changed → INVALID | PASS |
| Compare a file with its SHA-256 → MATCH | PASS |
| Encrypted message for me: saved armored, not in clear; read back in less, "not saved" | PASS |
| Random passphrase shown with its strength | PASS |
| OpenPGP with a real GnuPG peer: unknown signer → NOT CHECKED; key file added (fingerprint shown, confirmed); signature VALID with the fingerprint; own OpenPGP key created with pinentry-curses; peer's file for Alice decrypted; Alice's file for the peer decrypted by the peer | PASS |
| No passphrase (age, minisign, file, OpenPGP) nor the message text in the journal, the home folder, /tmp or /var/tmp | PASS |
| Very large files, sticks without RENAME_NOREPLACE, physical hardware | NOT TESTED |

### VM: `vm-maps` — Maps & navigation (ADR 0016), driven through the TUI by an Ultra-Stable account, no network

First part of the test planned in ADR 0016 §8 (its step 1).

| Scenario | Result |
|---|---|
| The world overview is in `/etc/resilience/packs.toml` and in the store of a minimal system | PASS (2026-11-16) |
| MAPS & NAVIGATION is on the main menu; Map opens the viewer on the overview: `Natural Earth`, `World overview` and a coordinate readout on the console; between 1/8 and 7/8 of the console cells are blue (sea and land both drawn, read from `/dev/vcsa1`) | PASS |
| The viewer runs as the user with exactly `resilience-maps view --pack <the store file>`; four `+` change the scale bar; pan; `i` shows the data date; Esc returns to the map | PASS |
| Viewer peak memory under 200 MB (VmHWM), no TCP or UDP socket; `q` returns to the menu, no viewer left; `maps.toml` written, owner only (0600) | PASS |
| "Maps on this computer & USB sticks" (through search) lists the overview as "Built in"; Enter opens it; `q` returns to the list | PASS |
| Search finds the map ("atlas"); `systemctl is-system-running` is `running` at the end | PASS |
| Installing a detailed pack from a stick, search, nearby, outside-coverage message on real console, hostile files, nothing written on the stick (steps 2 to 8 of the ADR) | NOT TESTED (later slices) |
| The map looked at by a person on a real or emulated console (colours, legibility at 80×25) | NOT TESTED |

### VM: `vm-comms` — local messaging network (ADR 0018, slice 1), two machines on an isolated network

The network service, each account's messaging program and the TUI
screens are tested. The first message is sent by a test program
(`tests/vm/comms-node.py`), the others through `resilience-messages`,
the last ones from the interface.

| Scenario | Result |
|---|---|
| `resilience-reticulum.service` runs on both machines as a non-root user; state directory 0700; client configuration 0644 with a 64-hex-digit key; the configurations in the Nix store hold no key; each machine has its own key; no `rnsh`/`rnx` process, and no Reticulum program in an account's `PATH` | PASS (2026-11-16) |
| Each machine sees the other on the local network (`rnstatus` through the client configuration, as the ordinary account: `AutoInterface[Local network]`, "1 reachable") within seconds; no TCP listener outside loopback | PASS |
| A message from one machine to the other through the two shared instances: state `outbound`, then `delivered` once the proof is back (0.1 s after sending); received with the sender's address and `verified=True` | PASS |
| The other machine's service stopped: the message is `failed` after 72 s and never `delivered` | PASS |
| The key is the same after a service restart; system `running` | PASS |
| Each account's program (`resilience-messages`, systemd user service of the logged-in Ultra-Stable account, not root): store 0700, identity 0600, nothing in it readable by others; a request file in the outbox on one machine → `delivered` there, `received` with `verified` true and the right sender on the other; each lists the other in `peers.json` with its claimed name | PASS (2026-11-16) |
| The other machine's program stopped: the message stays `queued` or `sent` for 20 s (not delivered, not lost); program started again → `delivered`, and the text is on the other machine | PASS |
| Through the interface on both consoles (Ultra-Stable accounts): COMMUNICATIONS on the main menu; Local messages says messaging is running; `m` shows my address in groups; people nearby lists the other machine as `"alice"?` with its address (and the test program of an earlier step as "bob"); `n` with the address typed as read aloud (groups, upper case), a message typed → `You [delivered]` with the text on the first screen; on the other machine the conversation appears, `Them [from them]` with the text and not "NOT VERIFIED"; a reply typed there is `[delivered]` there and appears on the first screen by itself | PASS (2026-11-16) |
| A file through the interface (ADR 0020): on alpha, `f`, "water-list" typed in the picker, Enter, a text → `You [delivered]` with the text and `[file] water-list.txt`, alpha's outgoing copy removed; on beta `[file] water-list.txt (271 KB)  o open, s save` (verified); `s` → "Saved as ~/Received/water-list.txt", same SHA-256 as on alpha, owner alice, mode 600, nothing left in beta's messages folder | PASS (2026-11-17, whole test 318 s) |
| Message store (ADR 0019) on both machines: unprivileged user, address file 0644 with 32 hex digits, state 0700, identity 0600, no TCP/UDP socket of its own (`ss -p`), a different address per machine; restarted with the instance | PASS (2026-11-17) |
| Store-and-forward: beta's account logged out of messaging; alpha's message → `sent` with `stored` true after ~2 minutes; beta's store has the copy within 34 s (stores peered by themselves); the text is in neither store (encrypted); alpha's screen shows "wait in a message store" and `You [sent, kept on the network]`; alpha switched off; beta's program starts and collects it in 4 s, once, `verified` true; beta's screen `Them [from them]`, not NOT VERIFIED; system `running` | PASS (2026-11-17) |
| Three or more machines; a link that drops mid-message; a machine switched off for hours; Wi-Fi; hostile peers or messages; a physical network | NOT TESTED |
| A system without `resilience.comms.enable` contains no Reticulum code | PASS once, by hand (2026-11-17): the live-minimal toplevel's derivation closure has no `reticulum`/`rns`/`lxmf` path (live-standard and dev-vm, which use the standard profile, have 7); not a flake check |
| The standard live ISO (comms on by default) | PASS (2026-11-17, see "ISO boot") |

### VM: `vm-mesh` — mesh network (ADR 0021), three machines in a chain and one with simulated Wi-Fi

alpha (vlan 1) ── beta (vlans 1 and 2) ── gamma (vlan 2): alpha and gamma
share no link. The mesh runs over the cables (`resilience.mesh.devices`);
then delta joins the Wi-Fi ad hoc cell with a `mac80211_hwsim` radio, a
second radio in a network namespace playing the other computer.

| Scenario | Result |
|---|---|
| Off by default: no `resilience-mesh.service`, no `bat0`; alpha hears beta (announces again after start) but never gamma; a message to gamma stays `queued` | PASS (2026-11-17) |
| `set-mesh`: polkit allows the TUI's process (active session) without a password, refuses a process of the same account outside the session, still refuses `set-mode`; bad arguments (`maybe`, `restart`, `on --force`) refused; the account itself running the helper is refused and audited; root `set-mesh on` on the three → "The mesh network is on.", audited `result=ok` | PASS (2026-11-17) |
| `bat0`: up, an IPv6 link-local address only (no global IPv6, no IPv4), `accept_ra` and `autoconf` 0, no default route through it; `bat0` and the member cards unmanaged by NetworkManager; beta has both others as BATMAN-adv neighbours; alpha pings gamma's link-local address through beta | PASS (2026-11-17) |
| The queued message reaches gamma through beta after the instances restarted: `delivered` on alpha (proof back), received and `verified` on gamma with alpha's address | PASS (2026-11-17, 11.5 s) |
| beta turns it off: unit inactive, `bat0` and the state file gone, each card's NetworkManager state as before; alpha can no longer ping gamma; a new message is not delivered within 40 s | PASS (2026-11-17) |
| Wi-Fi (delta): `set-mesh on` with no device configured takes the first card (wlan0): type IBSS, joined cell `02:52:45:53:4d:31`; a BATMAN-adv neighbour (the other radio); ping6 to it over `bat0` | PASS (2026-11-17) |
| Wi-Fi: `set-mesh off` → no `bat0`, the card back to type managed and to its NetworkManager state | PASS (2026-11-17) |
| Real Wi-Fi cards (IBSS on iwlwifi and others), radio range, several hops over the air, battery use | NOT TESTED |

### VM: `vm-install` — the text installer (ADR 0022), driven through the TUI

`installer`: a live-like system (autologin, no administrator, installer
enabled), running from vdb; vda (20 GiB, empty) is the target. `target`:
the same disk booted with SeaBIOS. The installed system is the real
`installed-standard-bios-luks` plus the NixOS test instrumentation only.

| Scenario | Result |
|---|---|
| Install is offered (live only); the disk list shows vda, and vdb (the running system) "in use", not choosable | PASS (2026-11-17) |
| French keyboard and encryption chosen on the Install screen, the two proposed accounts kept | PASS (2026-11-17) |
| The summary names the disk; a wrong name ("vdb") is refused and the disk's first 4 MiB are unchanged; "vda" starts `resilience-install` through pkexec | PASS (2026-11-17) |
| Passphrase typed twice into `cryptsetup luksFormat` and once into `cryptsetup open`, passwords twice into `passwd` for each account (French layout loaded by the installer); "Resilience is installed"; nothing left mounted or open | PASS (2026-11-17, ~3 min) |
| The disk boots (GRUB on BIOS); the initrd loads the French layout and keeps it after udev restarts the console setup; the passphrase typed on the keyboard unlocks the disk; `/` on `/dev/mapper/root`; login prompts use the French layout (`dumpkeys`) | PASS (2026-11-17) |
| Local accounts: shell `/run/current-system/sw/bin/resilience-session`; `user` Stable, `administrator` Advanced (wheel) as overrides of Ultra-Stable in `resilience-admin list`; no autologin; user namespaces on | PASS (2026-11-17) |
| `user` logs in on tty1 with the password typed during the installation; the interface starts (MODE: STABLE) | PASS (2026-11-17) |
| Minimal edition; installing next to another system (not offered) | NOT TESTED |

Found and fixed by this test: the initrd keymap unit could not start (its
interpreter is not in the systemd initrd); udev's restart of
systemd-vconsole-setup then put the US layout back before the prompt;
`/run/current-system/sw/bin/resilience-session` (in `/etc/shells`) did not
exist. Each would have left an installed French machine unusable.

### VM: `vm-install-uefi` — the same test with UEFI firmware, no encryption

Same script (`tests/vm/install.nix` with `firmware = "uefi"`). Both
machines have OVMF firmware: the installer starts its kernel directly
through OVMF (`/sys/firmware/efi` exists, so the installer chooses UEFI),
the target boots the disk with an **empty** variable store (the installer
never writes firmware variables). Installed system:
`installed-standard-uefi-plain` plus the test instrumentation.

| Scenario | Result |
|---|---|
| The live system sees UEFI firmware; the form says UEFI; encryption turned off on the form ("off: anyone with the disk can read it"), French keyboard, the two proposed accounts | PASS (2026-11-18) |
| Wrong disk name refused, first 4 MiB unchanged; "vda" starts the installer; no passphrase asked; passwords typed into `passwd`; "Resilience is installed"; nothing left mounted | PASS (2026-11-18, ~3 min) |
| The disk boots through OVMF's fallback path `EFI/BOOT/BOOTX64.EFI` (systemd-boot) with no boot entry in the firmware; `/` on `/dev/vda3` (no mapper device); `/boot` = `/dev/vda2` with NixOS loader entries; login prompts in the French layout | PASS (2026-11-18) |
| Accounts and modes as in `vm-install`; `user` logs in on tty1, MODE: STABLE | PASS (2026-11-18) |

### ISO boot (`tests/iso/smoke.py`, screenshots reviewed by a human/agent)

| Firmware | Result |
|---|---|
| Legacy BIOS (SeaBIOS, syslinux) — boots to TUI, offline, menus render, system OPERATIONAL | PASS (end-of-Phase-4 build, 2026-11-16) |
| UEFI (OVMF 202602 from pinned nixpkgs, GRUB 2.12 text mode) — boots to TUI, same screens | PASS (end-of-Phase-4 build, 2026-11-16) |
| Low memory: BIOS at 2048, 1024, 768 and 512 MiB; UEFI at 2048 and 1024 MiB — TUI, network diagnostics and system status shown, no failed service, no OOM message on the serial console | PASS (2026-11-16) |
| Console keymap French AZERTY on an ISO built with `resilience.keyboard.layout = "fr"`: search typed with AZERTY key positions (`--keymap fr`) opens network diagnostics (BIOS, 2 GiB) | PASS (2026-11-16, `feat/maps` build; the default is now `us`) |
| Build refuses an unknown keymap (`resilience.keyboard.layout = "azerty-nope"`) | PASS (manual, once) |
| `feat/comms` build (local messaging on): BIOS 2048 MiB, UEFI 2048 MiB, BIOS 1024 MiB — TUI, Maps & navigation and Communications on the main menu, diagnostics, status; search "messages" finds Local messages (tool and handbook page); it opens with `[OK] Messaging is running. My address: …` (the per-account service and the machine's Reticulum instance start on the live system) | PASS (2026-11-17) |
| `feat/comms` build with the message store (ADR 0019, `9d3ebd2`): BIOS 2048 MiB, UEFI 2048 MiB, BIOS 1024 MiB: Local messages shows `[OK] Messaging is running` and "Messages for people who are away wait in a message store on this network" (the machine's store started and wrote its address); system OPERATIONAL, no failed service, no OOM or failed unit on the serial console; 351 MiB used of 964 MiB after boot at 1 GiB | PASS (2026-11-17) |
| Milestone build from `develop` at `af4b661` (mesh network, ADR 0021; 1.55 GB): BIOS 2048 MiB, UEFI 2048 MiB (OVMF), BIOS 1024 MiB — TUI, system OPERATIONAL with no failed service, diagnostics, status, Local messages `[OK] Messaging is running` with the message store; ~610 MiB of 964 MiB available at 1 GiB; no OOM or failed unit on the serial console | PASS (2026-11-17) |
| Release 0.2.0 build from `release/0.2.0` (the Phase 6 milestone of `main` with the version raised; `resilience-standard-0.2.0-x86_64-linux.iso`, 1,554,186,240 bytes, SHA-256 `6507ee52025eef134ed39848d3c06fc40dce6d95681bfd379eaab7defa130f30`): BIOS 2048 MiB, UEFI 2048 MiB (OVMF 202608), BIOS 1024 MiB — TUI shows v0.2.0, system OPERATIONAL with no failed service, status, network, Local messages running | PASS (2026-11-17) |

### Keyboard layout (ADR 0017), in vm-boot-offline

| Check | Result |
|---|---|
| polkit: `set-keymap` implicit active yes, any no; `set-mode` still auth_admin | PASS (2026-11-16) |
| Helper refuses an unlisted layout (`../../tmp/evil`) and audits `result=refused` | PASS (2026-11-16) |
| Ultra-Stable user chooses French in Settings (no password): kernel keymap becomes AZERTY (keycode 16 = a), audit `caller=alice result=ok`, `keyboard = "fr"` saved | PASS (2026-11-16) |
| A login prompt starting (getty@tty2 restart) loads US again | PASS (2026-11-16) |
| Next session of the same account loads French again | PASS (2026-11-16) |
| Physical keyboard, several people on several consoles at once | NOT TESTED |
| Heavy use at low memory (Kiwix reader, backups, map viewer at ≤1 GiB) | NOT TESTED |
| Physical hardware: Lenovo ThinkPad L15 Gen 1, ISO built from `develop` (end of Phase 4) — boots without problem | PASS (owner, 2026-11-16; firmware mode, ISO hash and checklist items not recorded) |
| Physical hardware: other machines | NOT TESTED |

Last ISO run (2026-11-17): `feat/comms` at `9d3ebd2`,
`resilience-standard-0.1.0-x86_64-linux.iso`, 1,554,186,240 bytes, SHA-256
`87583b65d36f6947a22932b3d0af431123d1564ef995018b44f4aa0f1061cf7f`
(previous run at `7f1a0ac`: `bc3cdeff…8df8d`, same size).
The smoke script now also searches "messages" and opens the first hit
(screens 05 and 06). Previous run (2026-11-16): end of Phase 4,
1,528,430,592 bytes, SHA-256 `f3f17b95e4c4e226e82fed58a45ab9cef31a3c78a1dc70584b37295a87a530a4`. QEMU/KVM, 2 vCPU, no
network device, `-vga std` (160×50 console). Only the four smoke-test
screens were exercised. The smoke script waits a fixed 75 s, so time to
TUI is not measured (R1 saw the menu at 84 s, BIOS, 2 GiB).

The live root is a tmpfs of half the RAM: writable space for the user and
the system is ~980 MiB at 2 GiB, ~480 MiB at 1 GiB, ~230 MiB at 512 MiB.
Resource packs cannot be installed there anyway (no administrator), but a
modest machine running the live ISO has little room for files.

### Measurements (VM, 2 vCPU, 1.5–2 GiB RAM; reported, not asserted)

| Metric | Value | Where |
|---|---|---|
| TUI resident memory | 3.5 MiB | vm-boot-offline |
| Memory used after boot (incl. test backdoor) | 275 MiB | vm-boot-offline |
| Memory used on live ISO after boot (tmpfs root) | ~300 MiB of 1.9 GiB; ~280 MiB of 959 MiB; ~240 MiB of 712 MiB; ~120 MiB of 460 MiB ("available" as reported by the TUI) | ISO screenshots 2026-11-16 |
| Boot to multi-user (kernel + initrd + userspace) | 0.9 + 3.6 + 4.3 = 8.8 s | vm-boot-offline |
| Running services | journald, udevd, logind, dbus-broker, NetworkManager, nscd, oomd (+ transient) | vm-boot-offline |
| System closure (live-standard) | 2.2 GiB, of which linux-firmware 825 MiB, nixpkgs source 197 MiB | `nix path-info -S` |
| ISO size (standard) | 1.53 GB (1,528,430,592 bytes) at the end of Phase 4; 1.55 GB (1,554,186,240 bytes) with maps (world overview) and local messaging | builds of 2026-11-16 and 2026-11-17 |
| Memory used on live ISO after boot with local messaging on (rnsd + per-account Python service) | 339 MiB of 1.9 GiB; 330 MiB of 964 MiB (about +40 to +50 MiB over the end of Phase 4) | ISO screenshots 2026-11-17 |
| Closure growth from removable media + backups | +62 MiB (2231.6 → 2293.9 MiB; age 23 MiB, udisks2 + libblockdev deps ≈ 30 MiB) | `nix path-info -S`, main vs branch |
| TUI binary | 1.5 MiB (stripped, LTO) | build |
| Map viewer, dense view (4 tiles of zoom 14, 384 KiB of MVT and ~84,000 vertices each, 170×45 map): decode 2.9 ms per tile; cold frame 25-31 ms (read + inflate + decode 19-23 ms of it); warm frame 4.5-6.8 ms (worst 7.3 ms) at 5, 10 and 20 m per pixel; cache 5.6 MiB for the 4 tiles | release build, Ryzen 5 PRO 4650U | `cargo test --release -p resilience-maps --test perf -- --ignored --nocapture` (dev host, not a VM) |
| Map viewer peak memory on that view while panning and zooming (VmHWM) | 31 MiB (VmPeak 137 MiB of address space) | real binary under a pty on the dev host |

## Specification scenarios not yet covered

| # | Scenario | Status |
|---|---|---|
| 2 | Boot with Wi-Fi unavailable | NOT TESTED as a boot scenario (vm-wifi uses simulated radios; logic shared with #1) |
| 6 | Search locally installed resources (resource packs) | PASS (vm-packs: full-text search in an installed pack) |
| 7 | Open installed maps | PASS for the built-in world overview and a map from the list (vm-maps); a detailed pack from a stick NOT TESTED in VM |
| 9 | Cryptographic operations | PASS (vm-crypto) |
| 10 | Access local files (file manager) | NOT TESTED in VM (mc launch untested) |
| 11 | Import a resource pack from USB | PASS (vm-packs, QEMU usb-storage) |
| 12 | Verify an offline update bundle | BLOCKED — Phase 8 |
| 13 | Communicate across a local network | PASS between two VMs (vm-comms), and across a relay with no shared network (vm-mesh, cables and simulated Wi-Fi); physical network NOT TESTED |
| — | Installer, rollback to previous generation | BLOCKED — no installer yet |
| — | Recovery section with real generations; diagnostic report on a real system | NOT TESTED in VM (unit/TUI tests with fake profiles only) |
| — | Backup / restore with a physical USB stick | NOT TESTED (QEMU usb-storage only) |
