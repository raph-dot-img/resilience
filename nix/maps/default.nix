# Map packs of profile resilience-1, built reproducibly (ADR 0016 §2):
#
#   PBF (fixed-output fetch, SHA-256 pinned)
#     -> osmium tags-filter (filter.txt)
#     -> osmium export -f geojsonseq (osmium-export.json)
#     -> resilience-mapmaker classify
#     -> tippecanoe
#     -> resilience-mapmaker finish
#     => $out/map-osm-ID_YYYY-MM.mbtiles (+ .sha256)
#
# For build machines only: none of these tools is in the system image. The
# derivation is the ODbL "method of creation" of the packs it builds.
# Anyone with this flake gets the same bytes: `nix build .#mapPacks.ID`.
# The build log gives the time and peak memory of each step and the sizes
# and largest tiles of the pack (see resilience-mapmaker's report).
{
  lib,
  runCommand,
  fetchurl,
  writeText,
  osmium-tool,
  tippecanoe,
  time,
  resilience,
}:

let
  # The pack builder alone, from the same sources as the system package
  # (which does not install it). Its tests run in checks.resilience.
  mapmaker = resilience.overrideAttrs (old: {
    pname = "resilience-mapmaker";
    cargoBuildFlags = [
      "-p"
      "resilience-maps"
      "--bin"
      "resilience-mapmaker"
    ];
    doCheck = false;
    postInstall = "";
    meta = old.meta // {
      description = "Map pack builder of Resilience OS (build machines only)";
      mainProgram = "resilience-mapmaker";
    };
  });

  tools = "osmium-tool ${osmium-tool.version}, tippecanoe ${tippecanoe.version}";

  # One pack. `dataDate` must match the PBF header's replication
  # timestamp (the build fails otherwise). `singleThread` runs osmium and
  # tippecanoe on one thread: the same bytes must come out (the
  # reproducibility check builds both).
  mkPack =
    {
      id,
      name,
      description,
      coverage,
      pbf,
      sourceName,
      poly,
      dataDate,
      singleThread ? false,
    }:
    let
      file = "map-osm-${id}_${lib.substring 0 7 dataDate}.mbtiles";
    in
    runCommand file
      {
        nativeBuildInputs = [
          osmium-tool
          tippecanoe
          mapmaker
          time
        ];
        passthru = { inherit file mapmaker; };
        meta = {
          description = "Map pack ${name} (resilience-1, OpenStreetMap ${dataDate})";
          license = lib.licenses.odbl;
        };
      }
      ''
        set -euo pipefail
        ${lib.optionalString singleThread ''
          export TIPPECANOE_MAX_THREADS=1 OSMIUM_POOL_THREADS=1
        ''}
        step() { command time -f "time: $1: %e s, peak %M KiB" "''${@:2}"; }

        ts=$(osmium fileinfo -g header.option.osmosis_replication_timestamp ${pbf})
        if [ "''${ts%%T*}" != "${dataDate}" ]; then
          echo "the PBF header says '$ts', not ${dataDate}" >&2
          exit 1
        fi
        sha=$(sha256sum ${pbf} | cut -d ' ' -f 1)

        step tags-filter osmium tags-filter --no-progress -t \
          -e ${./filter.txt} -o filtered.osm.pbf ${pbf}
        step export osmium export --no-progress -c ${./osmium-export.json} \
          -f geojsonseq -o - filtered.osm.pbf \
          | step classify resilience-mapmaker classify --poly ${poly} \
              --rows rows.jsonl --output features.geojsonseq -
        rm filtered.osm.pbf
        step tippecanoe tippecanoe --quiet --no-progress-indicator \
          --output=tiles.mbtiles --read-parallel \
          --minimum-zoom=0 --maximum-zoom=14 \
          --no-tile-size-limit --no-feature-limit --drop-rate=1 \
          --no-tile-stats --temporary-directory="$TMPDIR" \
          features.geojsonseq
        ls -l features.geojsonseq rows.jsonl tiles.mbtiles >&2
        rm features.geojsonseq
        step finish resilience-mapmaker finish \
          --tiles tiles.mbtiles --rows rows.jsonl --poly ${poly} \
          --output pack.mbtiles \
          --name ${lib.escapeShellArg name} \
          --description ${lib.escapeShellArg description} \
          --data-date ${dataDate} \
          --source "${sourceName} sha256:$sha" \
          --coverage ${lib.escapeShellArg coverage} \
          --tools ${lib.escapeShellArg tools}

        mkdir "$out"
        mv pack.mbtiles "$out/${file}"
        (cd "$out" && sha256sum ${file} > ${file}.sha256)
        cat "$out/${file}.sha256" >&2
      '';

  test = import ./test-osm.nix { inherit writeText; };

  testlandPbf =
    runCommand "testland.osm.pbf"
      {
        nativeBuildInputs = [ osmium-tool ];
      }
      ''
        osmium cat --no-progress \
          --output-header=osmosis_replication_timestamp=2026-01-01T00:00:00Z \
          -o "$out" -f pbf ${test.osm}
      '';

  testland = {
    id = "testland";
    name = "Testland";
    description = "Synthetic test region of Resilience OS (not a real place)";
    coverage = "Testland (synthetic): the area west of 6.1 E only";
    pbf = testlandPbf;
    sourceName = "testland.osm.pbf";
    poly = test.poly;
    dataDate = "2026-01-01";
  };

  # Geofabrik's extract of 1 January 2026 and its boundary. The .poly has
  # no dated name; if Geofabrik changes it, this fetch fails and the hash
  # (and the pack) must be updated together.
  luxembourg = {
    id = "luxembourg";
    name = "Luxembourg";
    description = "OpenStreetMap data for Luxembourg (Geofabrik extract of 2026-01-01)";
    coverage = "Luxembourg, whole country (Geofabrik extract boundary)";
    pbf = fetchurl {
      url = "https://download.geofabrik.de/europe/luxembourg-260101.osm.pbf";
      hash = "sha256-2PPh2rtNnVSJUAw1Qy6zkwicMiIs//jPUHBoUvBpaoA=";
    };
    sourceName = "luxembourg-260101.osm.pbf";
    poly = fetchurl {
      url = "https://download.geofabrik.de/europe/luxembourg.poly";
      hash = "sha256-1fcG5KPSXZ9Rc2ZZuWsBiIPn/Pj+d1LoTWxW2Kv7Uho=";
    };
    dataDate = "2026-01-01";
  };

  # Bretagne (owner's choice for 0.6.0, ADR 0016): Geofabrik's extract of
  # 18 November 2026 and the region's boundary (same caveat for the .poly).
  bretagne = {
    id = "bretagne";
    name = "Bretagne";
    description = "OpenStreetMap data for Bretagne, France (Geofabrik extract of 2026-11-18)";
    coverage = "Bretagne (France), the whole region: Côtes-d'Armor, Finistère, Ille-et-Vilaine, Morbihan (Geofabrik extract boundary)";
    pbf = fetchurl {
      url = "https://download.geofabrik.de/europe/france/bretagne-261118.osm.pbf";
      hash = "sha256-S+Cx7OTjFa/OL92mnRuy7rUWchAJyhGm2NQHvq86jPc=";
    };
    sourceName = "bretagne-261118.osm.pbf";
    poly = fetchurl {
      url = "https://download.geofabrik.de/europe/france/bretagne.poly";
      hash = "sha256-yVBSJ1ZNQbWrAN6BqTzMuFlxeiLQPDvPV4Iqv+r2PU0=";
    };
    dataDate = "2026-11-18";
  };
in
{
  inherit mkPack mapmaker;

  packs = {
    testland = mkPack testland;
    luxembourg = mkPack luxembourg;
    bretagne = mkPack bretagne;
  };

  # The same test pack built on one thread (TIPPECANOE_MAX_THREADS=1,
  # OSMIUM_POOL_THREADS=1): checks.map-pack-reproducible compares the two.
  testlandSingleThread = mkPack (testland // { singleThread = true; });
}
