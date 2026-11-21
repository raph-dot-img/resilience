# The built-in world overview (ADR 0016 §2): a small map pack of profile
# resilience-1, zooms 0 to 7, made at build time from Natural Earth and
# shipped in the system image (also on the live ISO). It has coastlines,
# country and region borders, large lakes and rivers, about 7,300 towns
# and 711 spot heights (major summits with their height); no streets.
#
#   Natural Earth GeoJSON (fixed-output fetches of the v5.1.2 tag)
#     -> resilience-mapmaker natural-earth
#     -> tippecanoe
#     -> resilience-mapmaker finish
#     => $out/share/resilience/packs/map-world-natural-earth_2022-05.mbtiles
#
# Natural Earth is in the public domain (naturalearthdata.com/about/terms-of-use).
{
  lib,
  callPackage,
  runCommand,
  fetchurl,
  writeText,
  tippecanoe,
  resilience,
}:

let
  inherit (callPackage ../../maps { inherit resilience; }) mapmaker;

  version = "5.1.2";
  # The day the v5.1.2 release was published.
  dataDate = "2022-05-13";
  file = "map-world-natural-earth_${lib.substring 0 7 dataDate}.mbtiles";

  ne =
    name: hash:
    fetchurl {
      url = "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/v${version}/geojson/ne_10m_${name}.geojson";
      inherit hash;
    };
  land = ne "land" "sha256-GskHlkCLxq1pEdaUSEhdPE2/IZA3AIA2igmXbhyfdBY=";
  countries = ne "admin_0_boundary_lines_land" "sha256-dNnBYinAlf3mWUOpkZ4zdoLwRLzrzLEgdk847fO3D0o=";
  regions = ne "admin_1_states_provinces_lines" "sha256-Gh8wzKr0zJxL3jQmbwuMu5VdOkzyVLdWkSJV8ux8dbY=";
  lakes = ne "lakes" "sha256-LQNvU97exXgAHFwwwpWe59TuvBMGkA+kNnxJkp7I8tk=";
  rivers = ne "rivers_lake_centerlines" "sha256-u4VKkA7L07QI30bV4W4+D5dLpVmT+di1wm6FUnPAkFo=";
  places = ne "populated_places_simple" "sha256-/T+oZ6Mgy9XFtrtbxVCv7sKTn7LO9ojlCABygqVaxC8=";
  # 711 summits, spot elevations, depressions and passes (ADR 0016 §5).
  elevations = ne "geography_regions_elevation_points" "sha256-+YoWhnhnFG7EFG1tSxjII+7tsoJZR95mYRbPmk4/Q8s=";

  # The whole world as Web Mercator shows it.
  world = writeText "world.poly" ''
    world
    1
       -180 -85
       180 -85
       180 85
       -180 85
    END
    END
  '';

  coverage = "The whole world at overview detail (1:10 million): coasts, borders, large lakes and rivers, towns, major summits with their heights. No streets.";
in
runCommand "resilience-world-map-${version}"
  {
    nativeBuildInputs = [
      tippecanoe
      mapmaker
    ];
    passthru = {
      inherit
        file
        dataDate
        coverage
        version
        ;
      path = "share/resilience/packs/${file}";
    };
    meta = {
      description = "World overview map of Resilience OS (Natural Earth ${version})";
      license = lib.licenses.publicDomain;
    };
  }
  ''
    set -euo pipefail
    resilience-mapmaker natural-earth \
      --land ${land} --countries ${countries} --regions ${regions} \
      --lakes ${lakes} --rivers ${rivers} --places ${places} \
      --elevations ${elevations} \
      --rows rows.jsonl --output features.geojsonseq
    tippecanoe --quiet --no-progress-indicator \
      --output=tiles.mbtiles --read-parallel \
      --minimum-zoom=0 --maximum-zoom=7 \
      --no-tile-size-limit --no-feature-limit --drop-rate=1 \
      --no-tile-stats --temporary-directory="$TMPDIR" \
      features.geojsonseq
    ls -l features.geojsonseq rows.jsonl tiles.mbtiles >&2
    resilience-mapmaker finish \
      --tiles tiles.mbtiles --rows rows.jsonl --poly ${world} \
      --output pack.mbtiles \
      --name "World overview" \
      --description "Natural Earth ${version}, 1:10 million" \
      --data-date ${dataDate} \
      --source "Natural Earth ${version} (github.com/nvkelso/natural-earth-vector, tag v${version})" \
      --coverage ${lib.escapeShellArg coverage} \
      --license "Public domain" \
      --attribution "Natural Earth" \
      --center 0,20,0 \
      --tools "tippecanoe ${tippecanoe.version}"
    mkdir -p "$out/share/resilience/packs"
    mv pack.mbtiles "$out/share/resilience/packs/${file}"
    ls -l "$out/share/resilience/packs" >&2
  ''
