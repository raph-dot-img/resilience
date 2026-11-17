//! `resilience-mapmaker natural-earth`: Natural Earth GeoJSON files in,
//! the same two streams as `classify` out (features for tippecanoe, rows
//! for `finish`). It makes the built-in world overview (ADR 0016 §2).
//!
//! | Natural Earth file (10m)            | layer, class             | rows        |
//! |-------------------------------------|--------------------------|-------------|
//! | `land`                              | `land`, `landmass`       |             |
//! | `admin_0_boundary_lines_land`       | `boundaries`, level 2    |             |
//! | `admin_1_states_provinces_lines`    | `boundaries`, level 4    |             |
//! | `lakes`                             | `water`, `lake`          | named lakes |
//! | `rivers_lake_centerlines`           | `water`, `river`         | named rivers|
//! | `populated_places_simple`           | `places`, `city`/`town`  | all         |
//!
//! Every boundary line of the first file is drawn alike, disputed or not:
//! the map takes no position.
//!
//! The minimum zoom of a feature is the later of Natural Earth's own
//! `min_zoom` (its editors' choice of what matters at each scale) and the
//! zoom from which the style draws the class ([`tags::min_zoom`] for the
//! whole world). A line or an area that would only appear after
//! [`MAX_ZOOM`] is left out; a place is kept at [`MAX_ZOOM`] (and is
//! always in the search rows).
//!
//! The files are pinned by hash in Nix, so they are not hostile, but they
//! are parsed with the same care: sizes are bounded, what does not parse
//! is skipped and counted, names are cleaned. Each file is one JSON
//! document and is held in memory (the largest is 21 MB). Output order is
//! the order of [`Kind::ALL`], then the file's order.

use std::io::{self, Write};

use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Map, Value};

use resilience_core::maps::style::{AdminLevel, Class, LandClass, PlaceClass, WaterClass};

use super::classify::{OutFeature, Tippecanoe, bbox_array};
use super::geom::Geometry;
use super::rows::{self, Row};
use super::tags;

/// Highest zoom of the overview.
pub const MAX_ZOOM: u8 = 7;
/// Largest Natural Earth file read.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// A place with at least this many inhabitants is a city (as in
/// OpenStreetMap's convention); so is every national capital.
pub const CITY_POPULATION: u64 = 100_000;
/// The whole world, for the style's minimum zooms.
const WORLD_MAX_LAT: f64 = 85.0;

/// What a Natural Earth file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Land,
    Countries,
    Regions,
    Lakes,
    Rivers,
    Places,
}

impl Kind {
    /// In output order.
    pub const ALL: [Kind; 6] = [
        Kind::Land,
        Kind::Countries,
        Kind::Regions,
        Kind::Lakes,
        Kind::Rivers,
        Kind::Places,
    ];

    /// The command-line option naming the file.
    pub const fn option(self) -> &'static str {
        match self {
            Kind::Land => "land",
            Kind::Countries => "countries",
            Kind::Regions => "regions",
            Kind::Lakes => "lakes",
            Kind::Rivers => "rivers",
            Kind::Places => "places",
        }
    }

    fn index(self) -> usize {
        Kind::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stats {
    /// Features read, per kind.
    pub read: [u64; 6],
    /// Features written, per kind.
    pub written: [u64; 6],
    /// No geometry, a geometry of the wrong type, or bad coordinates.
    pub bad_geometry: u64,
    /// Would only appear after [`MAX_ZOOM`].
    pub too_detailed: u64,
    /// A place without a usable name.
    pub unnamed_places: u64,
    pub rows: u64,
}

impl Stats {
    pub fn report(&self) -> String {
        let mut s = String::from("natural-earth: features read/written:");
        for k in Kind::ALL {
            let i = k.index();
            s.push_str(&format!(
                " {}={}/{}",
                k.option(),
                self.read[i],
                self.written[i]
            ));
        }
        s.push_str(&format!(
            "\nnatural-earth: {} bad geometry, {} beyond zoom {MAX_ZOOM}, {} unnamed places, {} rows\n",
            self.bad_geometry, self.too_detailed, self.unnamed_places, self.rows
        ));
        s
    }
}

#[derive(Deserialize)]
struct Collection<'a> {
    #[serde(borrow)]
    features: Vec<InFeature<'a>>,
}

#[derive(Deserialize)]
struct InFeature<'a> {
    #[serde(borrow, default)]
    geometry: Option<&'a RawValue>,
    #[serde(default)]
    properties: Option<Map<String, Value>>,
}

/// A property under its lower-case or upper-case name (Natural Earth's
/// files differ).
fn prop<'a>(p: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    p.get(key).or_else(|| p.get(&key.to_ascii_uppercase()))
}

/// Natural Earth's `min_zoom` (a number such as `6.7`), rounded down;
/// 0 when absent or not a sensible number.
fn ne_zoom(p: &Map<String, Value>) -> u8 {
    match prop(p, "min_zoom").and_then(Value::as_f64) {
        Some(z) if z.is_finite() && z >= 1.0 => z.min(255.0).floor() as u8,
        _ => 0,
    }
}

fn name_of(p: &Map<String, Value>) -> Option<String> {
    let raw = prop(p, "name").and_then(Value::as_str)?;
    (raw.len() <= tags::MAX_TAG_BYTES).then_some(())?;
    tags::clean_name(raw, tags::MAX_NAME_CHARS)
}

fn population(p: &Map<String, Value>) -> Option<u64> {
    let v = prop(p, "pop_max")?;
    v.as_u64().or_else(|| {
        v.as_f64()
            .filter(|f| f.is_finite() && *f >= 1.0 && *f < 1e11)
            .map(|f| f as u64)
    })
}

fn is_capital(p: &Map<String, Value>) -> bool {
    prop(p, "adm0cap")
        .and_then(Value::as_f64)
        .is_some_and(|v| v == 1.0)
}

/// The class of a file's features (places: by population and capital).
fn class_of(kind: Kind, p: &Map<String, Value>) -> Class {
    match kind {
        Kind::Land => Class::Land(LandClass::Landmass),
        Kind::Countries => Class::Boundary(AdminLevel::Country),
        Kind::Regions => Class::Boundary(AdminLevel::Region),
        Kind::Lakes => Class::Water(WaterClass::Lake),
        Kind::Rivers => Class::Water(WaterClass::River),
        Kind::Places => {
            if is_capital(p) || population(p).is_some_and(|n| n >= CITY_POPULATION) {
                Class::Place(PlaceClass::City)
            } else {
                Class::Place(PlaceClass::Town)
            }
        }
    }
}

fn shape_fits(kind: Kind, g: &Geometry) -> bool {
    matches!(
        (kind, g),
        (
            Kind::Land | Kind::Lakes,
            Geometry::Polygon(_) | Geometry::MultiPolygon(_)
        ) | (
            Kind::Countries | Kind::Regions | Kind::Rivers,
            Geometry::LineString(_) | Geometry::MultiLineString(_)
        ) | (Kind::Places, Geometry::Point(_))
    )
}

/// Convert one Natural Earth file (`text`, a GeoJSON FeatureCollection).
pub fn convert(
    kind: Kind,
    text: &str,
    mut features: impl Write,
    mut rows: impl Write,
    stats: &mut Stats,
) -> io::Result<()> {
    let coll: Collection<'_> = serde_json::from_str(text)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    let i = kind.index();
    for f in coll.features {
        stats.read[i] += 1;
        let props = f.properties.unwrap_or_default();
        let Some(raw) = f.geometry else {
            stats.bad_geometry += 1;
            continue;
        };
        let Some(geom) = serde_json::from_str::<Geometry>(raw.get())
            .ok()
            .filter(|g| g.is_valid() && shape_fits(kind, g))
        else {
            stats.bad_geometry += 1;
            continue;
        };
        let class = class_of(kind, &props);
        let Some(style_zoom) = tags::min_zoom(class, WORLD_MAX_LAT) else {
            continue;
        };
        let mut minzoom = style_zoom.max(ne_zoom(&props));
        let name = name_of(&props);
        if kind == Kind::Places {
            if name.is_none() {
                stats.unnamed_places += 1;
                continue;
            }
            minzoom = minzoom.min(MAX_ZOOM);
        } else if minzoom > MAX_ZOOM {
            stats.too_detailed += 1;
            continue;
        }

        let layer = class.layer();
        let mut out = Map::new();
        let attr = Class::attribute(layer);
        match class {
            Class::Boundary(level) => out.insert(attr.into(), Value::from(level.level())),
            c => out.insert(attr.into(), Value::from(c.as_str())),
        };
        let mut row: Option<Row> = None;
        match (class, &name) {
            (Class::Place(pc), Some(n)) => {
                let idx = tags::settlement_class_index(pc.as_str()).unwrap_or(9);
                let rank = tags::settlement_rank(idx, population(&props));
                out.insert("rank".into(), Value::from(rank));
                out.insert("name".into(), Value::from(n.as_str()));
                if let Some(at) = geom.area_point() {
                    row = Some(Row {
                        kind: rows::Kind::Settlement,
                        merge: false,
                        class: pc.as_str().to_string(),
                        name: n.clone(),
                        lon: at.lon,
                        lat: at.lat,
                        rank,
                        bbox: None,
                    });
                }
            }
            (Class::Water(wc), Some(n)) => {
                let river = wc == WaterClass::River;
                let at = if river {
                    geom.line_point()
                } else {
                    geom.area_point()
                };
                if let (Some(at), Some(b)) = (at, geom.bbox()) {
                    row = Some(Row {
                        kind: rows::Kind::Natural,
                        merge: river,
                        class: wc.as_str().to_string(),
                        name: n.clone(),
                        lon: at.lon,
                        lat: at.lat,
                        rank: tags::natural_rank(wc.as_str()),
                        bbox: Some(bbox_array(b)),
                    });
                }
            }
            _ => {}
        }
        serde_json::to_writer(
            &mut features,
            &OutFeature {
                kind: "Feature",
                tippecanoe: Tippecanoe {
                    layer: layer.as_str(),
                    minzoom,
                },
                properties: &out,
                geometry: raw,
            },
        )?;
        features.write_all(b"\n")?;
        stats.written[i] += 1;
        if let Some(r) = row {
            serde_json::to_writer(&mut rows, &r)?;
            rows.write_all(b"\n")?;
            stats.rows += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collection(features: &[(&str, &str)]) -> String {
        let fs: Vec<String> = features
            .iter()
            .map(|(g, p)| format!("{{\"type\":\"Feature\",\"properties\":{p},\"geometry\":{g}}}"))
            .collect();
        format!(
            "{{\"type\":\"FeatureCollection\",\"features\":[{}]}}",
            fs.join(",")
        )
    }

    fn run(kind: Kind, features: &[(&str, &str)]) -> (Vec<Value>, Vec<Row>, Stats) {
        let (mut f, mut r) = (Vec::new(), Vec::new());
        let mut stats = Stats::default();
        convert(kind, &collection(features), &mut f, &mut r, &mut stats).expect("convert");
        let feats = String::from_utf8(f)
            .expect("utf8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("feature"))
            .collect();
        let rows = String::from_utf8(r)
            .expect("utf8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("row"))
            .collect();
        (feats, rows, stats)
    }

    const SQUARE: &str = "{\"type\":\"Polygon\",\"coordinates\":[[[0,0],[2,0],[2,2],[0,2],[0,0]]]}";
    const LINE: &str = "{\"type\":\"LineString\",\"coordinates\":[[0,0],[2,0]]}";
    const MULTILINE: &str = "{\"type\":\"MultiLineString\",\"coordinates\":[[[0,0],[2,0]]]}";

    #[test]
    fn land_is_landmass_from_natural_earths_zoom() {
        let (f, r, s) = run(
            Kind::Land,
            &[
                (SQUARE, "{\"featurecla\":\"Land\",\"min_zoom\":2}"),
                (SQUARE, "{}"),
                (SQUARE, "{\"min_zoom\":9.5}"),
                (LINE, "{}"),
                ("null", "{}"),
            ],
        );
        assert_eq!(f.len(), 2);
        assert_eq!(f[0]["tippecanoe"]["layer"], "land");
        assert_eq!(f[0]["tippecanoe"]["minzoom"], 2);
        assert_eq!(f[0]["properties"]["class"], "landmass");
        assert_eq!(f[0]["geometry"]["type"], "Polygon");
        assert_eq!(f[1]["tippecanoe"]["minzoom"], 0);
        assert!(r.is_empty());
        assert_eq!((s.read[0], s.written[0]), (5, 2));
        assert_eq!((s.too_detailed, s.bad_geometry), (1, 2));
    }

    #[test]
    fn borders_have_levels_and_upper_case_properties_are_read() {
        let (f, _, _) = run(Kind::Countries, &[(LINE, "{\"MIN_ZOOM\":0}")]);
        assert_eq!(f[0]["tippecanoe"]["layer"], "boundaries");
        assert_eq!(f[0]["properties"]["admin_level"], 2);
        assert_eq!(f[0]["tippecanoe"]["minzoom"], 0);

        let (f, _, s) = run(
            Kind::Regions,
            &[
                (MULTILINE, "{\"MIN_ZOOM\":6.6}"),
                (MULTILINE, "{\"MIN_ZOOM\":0}"),
                (MULTILINE, "{\"MIN_ZOOM\":10}"),
            ],
        );
        assert_eq!(f.len(), 2);
        assert_eq!(f[0]["properties"]["admin_level"], 4);
        assert_eq!(f[0]["tippecanoe"]["minzoom"], 6);
        // Never before the style draws region borders.
        let style = tags::min_zoom(Class::Boundary(AdminLevel::Region), 85.0).expect("drawn");
        assert_eq!(f[1]["tippecanoe"]["minzoom"], style);
        assert_eq!(s.too_detailed, 1);
    }

    #[test]
    fn named_lakes_and_rivers_are_searchable() {
        let (f, r, _) = run(
            Kind::Lakes,
            &[
                (SQUARE, "{\"name\":\"Lake Test\",\"min_zoom\":3}"),
                (SQUARE, "{\"name\":null}"),
            ],
        );
        assert_eq!(f.len(), 2);
        assert_eq!(f[0]["properties"]["class"], "lake");
        assert!(f[0]["properties"].get("name").is_none());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].kind, rows::Kind::Natural);
        assert_eq!((r[0].lon, r[0].lat), (1_000_000, 1_000_000));
        assert_eq!(r[0].bbox, Some([0, 0, 2_000_000, 2_000_000]));
        assert!(!r[0].merge);
        r[0].check().expect("valid row");

        let (f, r, _) = run(Kind::Rivers, &[(MULTILINE, "{\"name\":\"Essai\"}")]);
        assert_eq!(f[0]["properties"]["class"], "river");
        assert_eq!(r[0].class, "river");
        assert!(r[0].merge);
        assert_eq!((r[0].lon, r[0].lat), (1_000_000, 0));
        r[0].check().expect("valid row");
    }

    #[test]
    fn places_are_cities_or_towns_and_always_kept() {
        let at = "{\"type\":\"Point\",\"coordinates\":[6.13,49.61]}";
        let (f, r, s) = run(
            Kind::Places,
            &[
                (
                    at,
                    "{\"name\":\"Luxembourg\",\"adm0cap\":1,\"pop_max\":76684,\"min_zoom\":4.0}",
                ),
                (
                    at,
                    "{\"name\":\"Big\",\"adm0cap\":0,\"pop_max\":237042,\"min_zoom\":6.7}",
                ),
                (
                    at,
                    "{\"name\":\"Small\\u0007 Place\",\"pop_max\":900,\"min_zoom\":9}",
                ),
                (at, "{\"name\":null,\"pop_max\":5}"),
            ],
        );
        assert_eq!(f.len(), 3);
        assert_eq!(f[0]["properties"]["class"], "city");
        assert_eq!(f[0]["properties"]["name"], "Luxembourg");
        assert_eq!(f[0]["tippecanoe"]["minzoom"], 4);
        assert_eq!(f[1]["properties"]["class"], "city");
        assert_eq!(f[1]["tippecanoe"]["minzoom"], 6);
        assert_eq!(f[2]["properties"]["class"], "town");
        assert_eq!(f[2]["properties"]["name"], "Small Place");
        assert_eq!(f[2]["tippecanoe"]["minzoom"], MAX_ZOOM);
        assert_eq!(s.unnamed_places, 1);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].kind, rows::Kind::Settlement);
        assert_eq!((r[0].lon, r[0].lat), (6_130_000, 49_610_000));
        // The larger city ranks first; any city before any town.
        assert!(r[1].rank < r[0].rank && r[0].rank < r[2].rank);
        assert_eq!(f[0]["properties"]["rank"], r[0].rank);
        for row in &r {
            row.check().expect("valid row");
        }
    }

    #[test]
    fn bad_input_is_an_error_or_skipped_never_a_panic() {
        let (mut f, mut r) = (Vec::new(), Vec::new());
        let mut s = Stats::default();
        assert!(convert(Kind::Land, "not json", &mut f, &mut r, &mut s).is_err());
        assert!(convert(Kind::Land, "{\"features\":3}", &mut f, &mut r, &mut s).is_err());
        let (f, _, s) = run(
            Kind::Places,
            &[
                (
                    "{\"type\":\"Point\",\"coordinates\":[600,49]}",
                    "{\"name\":\"X\"}",
                ),
                (
                    "{\"type\":\"Point\",\"coordinates\":[6,49,100]}",
                    "{\"name\":\"X\"}",
                ),
                (
                    "{\"type\":\"Point\",\"coordinates\":[6,49]}",
                    "{\"name\":\"X\",\"min_zoom\":\"high\",\"pop_max\":-4}",
                ),
            ],
        );
        assert_eq!(f.len(), 1);
        assert_eq!(s.bad_geometry, 2);
        assert!(s.report().contains("places=3/1"));
    }
}
