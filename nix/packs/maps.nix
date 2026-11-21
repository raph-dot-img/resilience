# Map packs offered for download (ADR 0016): built from OpenStreetMap with
# `nix build .#mapPacks.ID` (nix/maps), published on the project's mirror.
# Size and SHA-256 are those of the built file; the store installs only a
# file whose bytes match. Like every pack, they can also come from a stick.
let
  mirror = file: [ "https://mirror.canevet.eu/files/resilience/packs/maps/${file}" ];
  osm = "OpenStreetMap contributors (Geofabrik extract), built by Resilience OS";
  attribution = "© OpenStreetMap contributors";
in
[
  {
    id = "map-luxembourg";
    title = "Map: Luxembourg";
    description = "Detailed map of Luxembourg: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "lb";
    file = "map-osm-luxembourg_2026-01.mbtiles";
    size = 29315072;
    sha256 = "2c07ad41ce6b01c187d9795b6391a7f770900a568d4038053c6bcf64bd6f7f60";
    urls = mirror "map-osm-luxembourg_2026-01.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-01-01";
    coverage = "Luxembourg, whole country (Geofabrik extract boundary). OpenStreetMap data of 1 January 2026: anything built or changed later is missing, and some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-bretagne";
    title = "Map: Bretagne (France)";
    description = "Detailed map of Bretagne: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-bretagne_2026-11.mbtiles";
    size = 177532928;
    sha256 = "ca72abfd5f05d4ee2cf24db38c01948dd8f846f37074ca4d6d5e7291149b5a43";
    urls = mirror "map-osm-bretagne_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-18";
    coverage = "Bretagne (France), the whole region: Côtes-d'Armor, Finistère, Ille-et-Vilaine, Morbihan (Geofabrik extract boundary; Loire-Atlantique is not included). OpenStreetMap data of 18 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
]
