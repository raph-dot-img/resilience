# Testland: the synthetic region of ADR 0016 §8, written here so that no
# map data is committed. Tests assert these coordinates (lon, lat):
#
#   Testville (town, pop. 12000)       6.0500, 49.0400
#   Petit-Essai (village)              6.0900, 49.0600
#   Les Granges (hamlet)               6.0200, 49.0650
#   Rue de l'Essai (residential, two ways merged into one search row near
#     Testville; its point is the middle of the first way)
#                                      6.0500, 49.0410
#   Grand-Rue (primary), Route de Petit-Essai (secondary), Chemin des
#     Vignes (track), the motorway A 99 (ref only), a railway
#   Cafe du Centre (amenity=cafe)      6.0505, 49.0402
#   Pharmacie de la Gare               6.0510, 49.0395
#   drinking water (no name)           6.0498, 49.0405
#   Mont Essai (peak, ele "412 m")     6.0300, 49.0200
#   Lac de l'Essai (multipolygon with an island), 6.07-6.08, 49.020-49.028
#   Bois de Test (forest)              6.005-6.020, 49.055-49.070
#   La Testue (river)                  6.000, 49.050 -> 6.120, 49.055
#   Testville commune (admin_level 8)  6.035-6.070, 49.030-49.050
#
# The data reaches lon 6.12; the coverage polygon stops at lon 6.10, so the
# east of the map's bounding box is outside the coverage.
{ writeText }:
{
  osm = writeText "testland.osm" ''
    <?xml version="1.0" encoding="UTF-8"?>
    <osm version="0.6" generator="resilience testland">
      <node id="1" version="1" lon="6.0500" lat="49.0400">
        <tag k="place" v="town"/>
        <tag k="name" v="Testville"/>
        <tag k="population" v="12 000"/>
      </node>
      <node id="2" version="1" lon="6.0900" lat="49.0600">
        <tag k="place" v="village"/>
        <tag k="name" v="Petit-Essai"/>
      </node>
      <node id="3" version="1" lon="6.0200" lat="49.0650">
        <tag k="place" v="hamlet"/>
        <tag k="name" v="Les Granges"/>
      </node>
      <node id="4" version="1" lon="6.0505" lat="49.0402">
        <tag k="amenity" v="cafe"/>
        <tag k="name" v="Café du Centre"/>
      </node>
      <node id="5" version="1" lon="6.0510" lat="49.0395">
        <tag k="amenity" v="pharmacy"/>
        <tag k="name" v="Pharmacie de la Gare"/>
      </node>
      <node id="6" version="1" lon="6.0498" lat="49.0405">
        <tag k="amenity" v="drinking_water"/>
      </node>
      <node id="7" version="1" lon="6.0300" lat="49.0200">
        <tag k="natural" v="peak"/>
        <tag k="name" v="Mont Essai"/>
        <tag k="ele" v="412 m"/>
      </node>

      <node id="10" version="1" lon="6.0480" lat="49.0410"/>
      <node id="11" version="1" lon="6.0500" lat="49.0410"/>
      <node id="12" version="1" lon="6.0520" lat="49.0410"/>
      <node id="13" version="1" lon="6.0540" lat="49.0410"/>
      <node id="20" version="1" lon="6.0400" lat="49.0395"/>
      <node id="21" version="1" lon="6.0500" lat="49.0395"/>
      <node id="22" version="1" lon="6.0600" lat="49.0395"/>
      <node id="30" version="1" lon="6.0300" lat="49.0450"/>
      <node id="31" version="1" lon="6.0350" lat="49.0480"/>
      <node id="40" version="1" lon="6.0600" lat="49.0420"/>
      <node id="41" version="1" lon="6.0900" lat="49.0600"/>
      <node id="50" version="1" lon="6.0000" lat="49.0100"/>
      <node id="51" version="1" lon="6.1200" lat="49.0100"/>
      <node id="60" version="1" lon="6.0000" lat="49.0300"/>
      <node id="61" version="1" lon="6.1200" lat="49.0350"/>
      <node id="70" version="1" lon="6.0000" lat="49.0500"/>
      <node id="71" version="1" lon="6.0600" lat="49.0520"/>
      <node id="72" version="1" lon="6.1200" lat="49.0550"/>

      <node id="80" version="1" lon="6.0700" lat="49.0200"/>
      <node id="81" version="1" lon="6.0800" lat="49.0200"/>
      <node id="82" version="1" lon="6.0800" lat="49.0280"/>
      <node id="83" version="1" lon="6.0700" lat="49.0280"/>
      <node id="84" version="1" lon="6.0740" lat="49.0230"/>
      <node id="85" version="1" lon="6.0760" lat="49.0230"/>
      <node id="86" version="1" lon="6.0760" lat="49.0250"/>
      <node id="87" version="1" lon="6.0740" lat="49.0250"/>

      <node id="90" version="1" lon="6.0050" lat="49.0550"/>
      <node id="91" version="1" lon="6.0200" lat="49.0550"/>
      <node id="92" version="1" lon="6.0200" lat="49.0700"/>
      <node id="93" version="1" lon="6.0050" lat="49.0700"/>

      <node id="100" version="1" lon="6.0350" lat="49.0300"/>
      <node id="101" version="1" lon="6.0700" lat="49.0300"/>
      <node id="102" version="1" lon="6.0700" lat="49.0500"/>
      <node id="103" version="1" lon="6.0350" lat="49.0500"/>

      <way id="200" version="1">
        <nd ref="10"/><nd ref="11"/><nd ref="12"/>
        <tag k="highway" v="residential"/>
        <tag k="name" v="Rue de l'Essai"/>
      </way>
      <way id="201" version="1">
        <nd ref="12"/><nd ref="13"/>
        <tag k="highway" v="residential"/>
        <tag k="name" v="Rue de l'Essai"/>
      </way>
      <way id="202" version="1">
        <nd ref="20"/><nd ref="21"/><nd ref="22"/>
        <tag k="highway" v="primary"/>
        <tag k="name" v="Grand-Rue"/>
        <tag k="ref" v="N 1"/>
      </way>
      <way id="203" version="1">
        <nd ref="30"/><nd ref="31"/>
        <tag k="highway" v="track"/>
        <tag k="name" v="Chemin des Vignes"/>
      </way>
      <way id="204" version="1">
        <nd ref="40"/><nd ref="41"/>
        <tag k="highway" v="secondary"/>
        <tag k="name" v="Route de Petit-Essai"/>
      </way>
      <way id="205" version="1">
        <nd ref="50"/><nd ref="51"/>
        <tag k="highway" v="motorway"/>
        <tag k="ref" v="A 99"/>
      </way>
      <way id="206" version="1">
        <nd ref="60"/><nd ref="61"/>
        <tag k="railway" v="rail"/>
      </way>
      <way id="207" version="1">
        <nd ref="70"/><nd ref="71"/><nd ref="72"/>
        <tag k="waterway" v="river"/>
        <tag k="name" v="La Testue"/>
      </way>
      <way id="208" version="1">
        <nd ref="80"/><nd ref="81"/><nd ref="82"/><nd ref="83"/><nd ref="80"/>
      </way>
      <way id="209" version="1">
        <nd ref="84"/><nd ref="85"/><nd ref="86"/><nd ref="87"/><nd ref="84"/>
      </way>
      <way id="210" version="1">
        <nd ref="90"/><nd ref="91"/><nd ref="92"/><nd ref="93"/><nd ref="90"/>
        <tag k="landuse" v="forest"/>
        <tag k="name" v="Bois de Test"/>
      </way>
      <way id="211" version="1">
        <nd ref="100"/><nd ref="101"/><nd ref="102"/><nd ref="103"/><nd ref="100"/>
      </way>

      <relation id="300" version="1">
        <member type="way" ref="208" role="outer"/>
        <member type="way" ref="209" role="inner"/>
        <tag k="type" v="multipolygon"/>
        <tag k="natural" v="water"/>
        <tag k="name" v="Lac de l'Essai"/>
      </relation>
      <relation id="301" version="1">
        <member type="way" ref="211" role="outer"/>
        <member type="node" ref="1" role="admin_centre"/>
        <tag k="type" v="boundary"/>
        <tag k="boundary" v="administrative"/>
        <tag k="admin_level" v="8"/>
        <tag k="name" v="Testville"/>
      </relation>
    </osm>
  '';

  # Coverage: lon 6.00-6.10, lat 49.00-49.07 (Osmosis polygon format).
  poly = writeText "testland.poly" ''
    testland
    1
       6.0000E+00   4.9000E+01
       6.1000E+00   4.9000E+01
       6.1000E+00   4.9070E+01
       6.0000E+00   4.9070E+01
       6.0000E+00   4.9000E+01
    END
    END
  '';
}
