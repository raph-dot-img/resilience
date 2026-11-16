# Checks on the map packs (ADR 0016 §8), for flake.nix's `checks`:
#
# - map-pack-reproducible: the test pack built twice in different ways
#   (one thread for osmium and tippecanoe, and the default) must have the
#   same SHA-256;
# - map-pack-testland: the test pack opened with the real reader
#   (`resilience-maps info --check`), its search rows read back with fixed
#   SQL, and the minimum zooms of a few classes checked in the tiles.
{
  lib,
  runCommand,
  sqlite,
  tippecanoe,
  resilience,
  maps,
}:

let
  pack = maps.packs.testland;
  single = maps.testlandSingleThread;
in
{
  map-pack-reproducible = runCommand "map-pack-reproducible" { } ''
    a=$(sha256sum < ${pack}/${pack.file} | cut -d ' ' -f 1)
    b=$(sha256sum < ${single}/${single.file} | cut -d ' ' -f 1)
    echo "default threads: $a"
    echo "one thread:      $b"
    if [ "$a" != "$b" ]; then
      echo "the test pack is not reproducible" >&2
      exit 1
    fi
    echo "$a  ${pack.file}" > "$out"
  '';

  map-pack-testland =
    runCommand "map-pack-testland"
      {
        nativeBuildInputs = [
          resilience
          tippecanoe
        ];
      }
      ''
        set -euo pipefail
        cp ${pack}/${pack.file} pack.mbtiles
        chmod u+w pack.mbtiles

        resilience-maps info --check pack.mbtiles | tee info.txt
        expect() {
          grep -qxF "$1" info.txt || { echo "info: missing '$1'" >&2; exit 1; }
        }
        expect "name: Testland"
        expect "profile: resilience-1"
        expect "data-date: 2026-01-01"
        expect "license: ODbL-1.0"
        expect "attribution: (c) OpenStreetMap contributors"
        expect "coverage: Testland (synthetic): the area west of 6.1 E only"
        expect "format: pbf"
        # No feature is drawn below z4 in so small a region (the lake is tiny).
        expect "zoom: 4-14"
        expect "center: 49.04000,6.05000,10"
        expect "coverage-area: 1 rings, 4 points"
        expect "search: available"
        expect "points-of-interest: available"
        expect "integrity: ok"
        grep -qx 'source: testland.osm.pbf sha256:[0-9a-f]\{64\}' info.txt
        grep -qx 'builder: resilience-mapmaker [0-9.]* (osmium-tool .*, tippecanoe .*)' info.txt

        q() { ${lib.getExe' sqlite "sqlite3"} -readonly -batch -noheader "file:pack.mbtiles?immutable=1" "$1"; }
        check() {
          got=$(q "$1")
          if [ "$got" != "$2" ]; then
            printf 'query: %s\ngot:   %s\nwant:  %s\n' "$1" "$got" "$2" >&2
            exit 1
          fi
        }
        check "SELECT count(*) FROM rs_place" 13
        check "SELECT group_concat(kind || ':' || name || ':' || ifnull(near, '-'), ',') FROM (SELECT * FROM rs_place ORDER BY id)" \
          "settlement:Testville:Petit-Essai,settlement:Petit-Essai:Testville,settlement:Les Granges:Testville,street:Grand-Rue:Testville,street:Route de Petit-Essai:Petit-Essai,street:Rue de l'Essai:Testville,street:Chemin des Vignes:Testville,poi:Pharmacie de la Gare:Testville,poi::Testville,poi:Café du Centre:Testville,natural:Lac de l'Essai:Testville,natural:La Testue:Testville,natural:Mont Essai:Testville"
        # Rue de l'Essai: two ways, one row, half-way along the first way,
        # with the box of both (6.048-6.054 E, 49.041 N).
        check "SELECT class || '|' || lon || '|' || lat || '|' || hex(bbox) FROM rs_place WHERE kind = 'street' AND fold = 'rue de l essai'" \
          "residential|6050000|49041000|00495C00684EEC0270605C00684EEC02"
        check "SELECT class || '|' || lon || '|' || lat || '|' || rank FROM rs_place WHERE name = 'Testville'" "town|6050000|49040000|159"
        # Unnamed drinking water: an empty name, listed by Nearby only.
        check "SELECT length(name) || '|' || length(fold) || '|' || lon || '|' || lat FROM rs_place WHERE class = 'drinking_water'" "0|0|6049800|49040500"
        check "SELECT fold FROM rs_place WHERE class = 'cafe'" "cafe du centre"
        check "SELECT lon || '|' || lat FROM rs_place WHERE class = 'peak'" "6030000|49020000"
        check "SELECT count(*) FROM rs_poi_rtree" 4
        check "SELECT group_concat(id, ',') FROM (SELECT id FROM rs_poi_rtree WHERE min_lon <= 6052000 AND max_lon >= 6048000 AND min_lat <= 49042000 AND max_lat >= 49038000 ORDER BY id)" "8,9,10"
        check "SELECT group_concat(name, ',') FROM (SELECT p.name FROM rs_place_fts f JOIN rs_place p ON p.id = f.rowid WHERE rs_place_fts MATCH 'essai*' ORDER BY p.id)" \
          "Petit-Essai,Route de Petit-Essai,Rue de l'Essai,Lac de l'Essai,Mont Essai"
        check "SELECT count(*) FROM rs_coverage" 1
        # The data reaches 6.12 E, beyond the coverage (6.1 E).
        q "SELECT value FROM metadata WHERE name = 'bounds'" | awk -F, '{ exit !($3 > 6.11 && $1 < 6.001) }'

        # Minimum zooms (style.rs at 49.07 N): towns from z5, peaks with
        # their elevation from z9, residential streets from z12.
        for z in 4 5 8 9 11 12 14; do
          tippecanoe-decode -Z $z -z $z pack.mbtiles > z$z.json
        done
        has() {
          grep -Eq "$2" "z$1.json" || { echo "z$1: no $2" >&2; exit 1; }
        }
        hasnt() {
          if grep -Eq "$2" "z$1.json"; then echo "z$1: unexpected $2" >&2; exit 1; fi
        }
        has 5 '"Testville"'
        hasnt 4 '"Testville"'
        has 9 '"ele": ?412'
        hasnt 8 '"Mont Essai"'
        has 12 "\"Rue de l'Essai\""
        hasnt 11 "\"Rue de l'Essai\""
        has 14 '"category": ?"water"'

        echo ok > "$out"
      '';
}
