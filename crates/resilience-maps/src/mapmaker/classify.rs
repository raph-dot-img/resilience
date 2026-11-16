//! `resilience-mapmaker classify`: osmium's GeoJSON sequence in, two
//! streams out:
//!
//! - **features** for tippecanoe, one GeoJSON feature per line, with the
//!   tippecanoe extension `{"layer": ..., "minzoom": ...}` and only the
//!   attributes of `resilience-1` (ADR 0016 §1);
//! - **rows** for `finish` (see [`super::rows`]): settlements, street and
//!   river segments, POIs, peaks, lakes.
//!
//! It streams: one input line is held at a time (at most [`MAX_LINE`]
//! bytes; a longer one is skipped and counted), so memory does not grow
//! with the extract. Output order is input order, and osmium's export is
//! deterministic, so the output is too.
//!
//! Geometry is passed through verbatim (serde_json `RawValue`) except
//! where something must be computed: the centre of an area that becomes
//! a point (a POI or a place mapped as an area), the rings of a boundary
//! area written as lines, and the positions and boxes of rows.

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{Map, Value};

use resilience_core::maps::style::{Class, Layer, WaterClass};

use super::geom::{Geometry, GeometryType};
use super::rows::{Kind, Row};
use super::tags::{self, Geom, Tags};
use crate::profile::{Micro, MicroBox};

/// Longest input line (one GeoJSON feature). A country's boundary with
/// its sea limits is a few tens of MB; nothing real comes near this.
pub const MAX_LINE: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// The largest absolute latitude of the pack's coverage, for the
    /// minimum zooms ([`tags::min_zoom`]).
    pub max_abs_lat: f64,
}

/// What happened to the input, for the build log.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stats {
    pub lines: u64,
    pub too_long: u64,
    /// Not a GeoJSON feature as osmium writes them.
    pub unparsed: u64,
    /// A geometry type we do not use, or coordinates that do not parse.
    pub bad_geometry: u64,
    /// Parsed, but no layer wants it.
    pub unused: u64,
    pub features: [u64; 8],
    pub rows: [u64; 4],
}

impl Stats {
    pub fn report(&self) -> String {
        let mut s = format!(
            "classify: {} input features, {} too long, {} unparsed, {} bad geometry, {} unused\n",
            self.lines, self.too_long, self.unparsed, self.bad_geometry, self.unused
        );
        s.push_str("classify: features by layer:");
        for (l, n) in Layer::ALL.iter().zip(self.features) {
            s.push_str(&format!(" {}={n}", l.as_str()));
        }
        s.push_str("\nclassify: rows by kind:");
        for (k, n) in [Kind::Settlement, Kind::Street, Kind::Poi, Kind::Natural]
            .iter()
            .zip(self.rows)
        {
            s.push_str(&format!(" {}={n}", k.as_str()));
        }
        s.push('\n');
        s
    }
}

#[derive(Deserialize)]
struct InFeature<'a> {
    #[serde(borrow)]
    geometry: &'a RawValue,
    #[serde(default)]
    properties: Option<Map<String, Value>>,
}

#[derive(Serialize)]
pub(super) struct Tippecanoe {
    pub(super) layer: &'static str,
    pub(super) minzoom: u8,
}

/// One feature for tippecanoe (also written by [`super::overview`]).
#[derive(Serialize)]
pub(super) struct OutFeature<'a> {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) tippecanoe: Tippecanoe,
    pub(super) properties: &'a Map<String, Value>,
    pub(super) geometry: &'a RawValue,
}

/// Read one line of at most `max` bytes into `buf` (without the line
/// end). `Ok(None)` at the end of input, `Ok(Some(false))` when the line
/// was longer than `max` (the rest of it is skipped and `buf` is empty).
pub fn read_line_bounded(
    r: &mut impl BufRead,
    buf: &mut Vec<u8>,
    max: usize,
) -> io::Result<Option<bool>> {
    buf.clear();
    let mut any = false;
    let mut fits = true;
    loop {
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            return Ok(any.then_some(fits));
        }
        any = true;
        let (part, done) = match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => (&chunk[..i], Some(i + 1)),
            None => (chunk, None),
        };
        if fits && buf.len() + part.len() <= max {
            buf.extend_from_slice(part);
        } else {
            fits = false;
            buf.clear();
        }
        let used = done.unwrap_or(chunk.len());
        r.consume(used);
        if done.is_some() {
            return Ok(Some(fits));
        }
    }
}

fn trim(mut b: &[u8]) -> &[u8] {
    // RFC 8142 record separator, then white space.
    while let [first, rest @ ..] = b {
        if *first == 0x1e || first.is_ascii_whitespace() {
            b = rest;
        } else {
            break;
        }
    }
    while let [rest @ .., last] = b {
        if last.is_ascii_whitespace() {
            b = rest;
        } else {
            break;
        }
    }
    b
}

pub(super) fn bbox_array(b: MicroBox) -> [i32; 4] {
    [
        b.south_west.lon,
        b.south_west.lat,
        b.north_east.lon,
        b.north_east.lat,
    ]
}

/// The full geometry, parsed once when first needed.
struct Lazy<'a> {
    raw: &'a RawValue,
    parsed: Option<Option<Geometry>>,
}

impl Lazy<'_> {
    fn get(&mut self) -> Option<&Geometry> {
        if self.parsed.is_none() {
            let g = serde_json::from_str::<Geometry>(self.raw.get())
                .ok()
                .filter(Geometry::is_valid);
            self.parsed = Some(g);
        }
        self.parsed.as_ref().and_then(Option::as_ref)
    }
}

fn point_json(p: Micro) -> Option<Box<RawValue>> {
    let g = Geometry::Point([p.lon_degrees(), p.lat_degrees()]);
    RawValue::from_string(serde_json::to_string(&g).ok()?).ok()
}

/// Run `classify` over `input`.
pub fn run(
    mut input: impl BufRead,
    mut features: impl Write,
    mut rows: impl Write,
    opts: &Options,
) -> io::Result<Stats> {
    let mut stats = Stats::default();
    let mut buf = Vec::new();
    while let Some(fits) = read_line_bounded(&mut input, &mut buf, MAX_LINE)? {
        if !fits {
            stats.lines += 1;
            stats.too_long += 1;
            continue;
        }
        let line = trim(&buf);
        if line.is_empty() {
            continue;
        }
        stats.lines += 1;
        let Ok(f) = serde_json::from_slice::<InFeature<'_>>(line) else {
            stats.unparsed += 1;
            continue;
        };
        let props = f.properties.unwrap_or_default();
        let geom = match serde_json::from_str::<GeometryType<'_>>(f.geometry.get()) {
            Ok(GeometryType { kind: "Point" }) => Geom::Point,
            Ok(GeometryType {
                kind: "LineString" | "MultiLineString",
            }) => Geom::Line,
            Ok(GeometryType {
                kind: "Polygon" | "MultiPolygon",
            }) => Geom::Area,
            _ => {
                stats.bad_geometry += 1;
                continue;
            }
        };
        let name = tags::pick_name(&props);
        let items = tags::classify(&props, geom, name.is_some());
        let search_only = tags::place_search_only(&props, geom).filter(|_| name.is_some());
        if items.is_empty() && search_only.is_none() {
            stats.unused += 1;
            continue;
        }
        let mut lazy = Lazy {
            raw: f.geometry,
            parsed: None,
        };
        let mut wrote = false;
        let bad_before = stats.bad_geometry;
        let mut emit_row = |row: Row, stats: &mut Stats| -> io::Result<()> {
            serde_json::to_writer(&mut rows, &row)?;
            rows.write_all(b"\n")?;
            stats.rows[usize::try_from(row.kind.order()).unwrap_or(0)] += 1;
            Ok(())
        };
        if let Some(class) = search_only
            && let Some(at) = lazy.get().and_then(Geometry::area_point)
        {
            let idx = tags::settlement_class_index(class).unwrap_or(9);
            let pop = props.tag("population").and_then(tags::parse_population);
            emit_row(
                Row {
                    kind: Kind::Settlement,
                    merge: false,
                    class: class.to_string(),
                    name: name.clone().unwrap_or_default(),
                    lon: at.lon,
                    lat: at.lat,
                    rank: tags::settlement_rank(idx, pop),
                    bbox: None,
                },
                &mut stats,
            )?;
            wrote = true;
        }
        for item in &items {
            let Some(minzoom) = tags::min_zoom(item.class, opts.max_abs_lat) else {
                continue;
            };
            let layer = item.class.layer();
            let mut out = Map::new();
            let attr = Class::attribute(layer);
            match item.class {
                Class::Boundary(level) => {
                    out.insert(attr.into(), Value::from(level.level()));
                }
                c => {
                    out.insert(attr.into(), Value::from(c.as_str()));
                }
            }
            let mut row: Option<Row> = None;
            // The geometry to write: verbatim, or computed.
            let computed: Option<Box<RawValue>> = match item.class {
                Class::Boundary(_) => {
                    let Some(lines) = lazy.get().and_then(Geometry::rings_as_lines) else {
                        stats.bad_geometry += 1;
                        continue;
                    };
                    let g = Geometry::MultiLineString(lines);
                    match serde_json::to_string(&g)
                        .ok()
                        .and_then(|s| RawValue::from_string(s).ok())
                    {
                        Some(r) => Some(r),
                        None => {
                            stats.bad_geometry += 1;
                            continue;
                        }
                    }
                }
                Class::Place(_) | Class::Poi(_) | Class::Peak(_) => {
                    let Some(at) = lazy.get().and_then(Geometry::area_point) else {
                        stats.bad_geometry += 1;
                        continue;
                    };
                    let rank;
                    let row_kind;
                    let row_class: String;
                    let row_name = name.clone().unwrap_or_default();
                    match item.class {
                        Class::Place(_) => {
                            let idx = tags::settlement_class_index(&item.osm_class).unwrap_or(9);
                            let pop = props.tag("population").and_then(tags::parse_population);
                            rank = tags::settlement_rank(idx, pop);
                            out.insert("rank".into(), Value::from(rank));
                            row_kind = Some(Kind::Settlement);
                            row_class = item.osm_class.to_string();
                        }
                        Class::Poi(cat) => {
                            rank = tags::poi_rank(cat);
                            row_kind = Some(Kind::Poi);
                            row_class = item.osm_class.to_string();
                        }
                        _ => {
                            let ele = props.tag("ele").and_then(tags::parse_ele);
                            if let Some(e) = ele {
                                out.insert("ele".into(), Value::from(e));
                            }
                            row_class = item.class.as_str().to_string();
                            rank = tags::natural_rank(&row_class);
                            row_kind = (name.is_some() || ele.is_some()).then_some(Kind::Natural);
                        }
                    }
                    if let Some(kind) = row_kind {
                        row = Some(Row {
                            kind,
                            merge: false,
                            class: row_class,
                            name: row_name,
                            lon: at.lon,
                            lat: at.lat,
                            rank,
                            bbox: None,
                        });
                    }
                    if geom == Geom::Point {
                        None
                    } else {
                        match point_json(at) {
                            Some(p) => Some(p),
                            None => {
                                stats.bad_geometry += 1;
                                continue;
                            }
                        }
                    }
                }
                Class::Road(rc) => {
                    if let Some(r) = tags::pick_ref(&props) {
                        out.insert("ref".into(), Value::from(r));
                    }
                    if let Some(n) = &name
                        && let Some(g) = lazy.get()
                        && let (Some(at), Some(b)) = (g.line_point(), g.bbox())
                    {
                        row = Some(Row {
                            kind: Kind::Street,
                            merge: true,
                            class: rc.as_str().to_string(),
                            name: n.clone(),
                            lon: at.lon,
                            lat: at.lat,
                            rank: tags::road_rank(rc),
                            bbox: Some(bbox_array(b)),
                        });
                    }
                    None
                }
                Class::Water(wc) => {
                    if let Some(n) = &name
                        && let Some(g) = lazy.get()
                    {
                        let at = match wc {
                            WaterClass::River => g.line_point(),
                            WaterClass::Lake | WaterClass::Reservoir => g.area_point(),
                            _ => None,
                        };
                        if let (Some(at), Some(b)) = (at, g.bbox()) {
                            row = Some(Row {
                                kind: Kind::Natural,
                                merge: wc == WaterClass::River,
                                class: wc.as_str().to_string(),
                                name: n.clone(),
                                lon: at.lon,
                                lat: at.lat,
                                rank: tags::natural_rank(wc.as_str()),
                                bbox: Some(bbox_array(b)),
                            });
                        }
                    }
                    None
                }
                Class::Land(_) | Class::Rail(_) => None,
            };
            if let Some(n) = &name
                && matches!(
                    layer,
                    Layer::Roads | Layer::Places | Layer::Pois | Layer::Peaks
                )
            {
                out.insert("name".into(), Value::from(n.as_str()));
            }
            let geometry: &RawValue = computed.as_deref().unwrap_or(f.geometry);
            serde_json::to_writer(
                &mut features,
                &OutFeature {
                    kind: "Feature",
                    tippecanoe: Tippecanoe {
                        layer: layer.as_str(),
                        minzoom,
                    },
                    properties: &out,
                    geometry,
                },
            )?;
            features.write_all(b"\n")?;
            let li = Layer::ALL.iter().position(|l| *l == layer).unwrap_or(0);
            stats.features[li] += 1;
            wrote = true;
            if let Some(r) = row {
                emit_row(r, &mut stats)?;
            }
        }
        if !wrote && stats.bad_geometry == bad_before {
            stats.unused += 1;
        }
    }
    features.flush()?;
    rows.flush()?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(geometry: &str, props: &str) -> String {
        format!("\x1e{{\"type\":\"Feature\",\"geometry\":{geometry},\"properties\":{props}}}\n")
    }

    fn run_on(input: &str, lat: f64) -> (Vec<Value>, Vec<Row>, Stats) {
        let (mut f, mut r) = (Vec::new(), Vec::new());
        let stats = run(
            input.as_bytes(),
            &mut f,
            &mut r,
            &Options { max_abs_lat: lat },
        )
        .expect("classify");
        let feats = String::from_utf8(f)
            .expect("utf8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("feature json"))
            .collect();
        let rows = String::from_utf8(r)
            .expect("utf8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("row json"))
            .collect();
        (feats, rows, stats)
    }

    const STREET: &str =
        r#"{"type":"LineString","coordinates":[[6.0,49.0],[6.001,49.001],[6.002,49.002]]}"#;

    #[test]
    fn streets_go_to_roads_and_rows() {
        let input = feature(
            STREET,
            r#"{"highway":"residential","name":"Rue de l'Essai","ref":"C 1","surface":"asphalt"}"#,
        );
        let (f, r, s) = run_on(&input, 45.0);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0]["tippecanoe"]["layer"], "roads");
        assert_eq!(f[0]["tippecanoe"]["minzoom"], 12);
        assert_eq!(
            f[0]["properties"],
            serde_json::json!({"class":"residential","name":"Rue de l'Essai","ref":"C 1"})
        );
        // The geometry is passed through verbatim.
        let raw: Value = serde_json::from_str(STREET).expect("json");
        assert_eq!(f[0]["geometry"], raw);
        assert_eq!(r.len(), 1);
        assert_eq!(
            r[0],
            Row {
                kind: Kind::Street,
                merge: true,
                class: "residential".into(),
                name: "Rue de l'Essai".into(),
                lon: 6_001_000,
                lat: 49_001_000,
                rank: 6,
                bbox: Some([6_000_000, 49_000_000, 6_002_000, 49_002_000]),
            }
        );
        assert_eq!(s.features[2], 1);
        assert_eq!(s.rows[1], 1);
        // Further north, the minimum zoom is lower.
        let (f, _, _) = run_on(&input, 70.0);
        assert_eq!(f[0]["tippecanoe"]["minzoom"], 11);
    }

    #[test]
    fn points_areas_and_boundaries() {
        let square = r#"{"type":"MultiPolygon","coordinates":[[[[6.0,49.0],[6.02,49.0],[6.02,49.02],[6.0,49.02],[6.0,49.0]]]]}"#;
        let input = [
            feature(
                r#"{"type":"Point","coordinates":[6.01,49.01]}"#,
                r#"{"place":"town","name":"Testville","population":"12 000"}"#,
            ),
            feature(square, r#"{"amenity":"pharmacy"}"#),
            feature(
                r#"{"type":"Point","coordinates":[6.03,49.03]}"#,
                r#"{"natural":"peak","name":"Mont Test","ele":"1 234 m"}"#,
            ),
            feature(
                r#"{"type":"Point","coordinates":[6.04,49.04]}"#,
                r#"{"natural":"peak"}"#,
            ),
            feature(
                square,
                r#"{"boundary":"administrative","admin_level":"8","name":"Testville"}"#,
            ),
            feature(square, r#"{"natural":"water","name":"Lac Test"}"#),
            feature(square, r#"{"place":"suburb","name":"Quartier"}"#),
        ]
        .concat();
        let (f, r, s) = run_on(&input, 49.0);
        let layers: Vec<&str> = f
            .iter()
            .map(|x| x["tippecanoe"]["layer"].as_str().expect("layer"))
            .collect();
        assert_eq!(
            layers,
            ["places", "pois", "peaks", "peaks", "boundaries", "water"]
        );
        assert_eq!(f[0]["properties"]["rank"], 159);
        // The pharmacy area became its centre; no name, no name attribute.
        assert_eq!(
            f[1]["geometry"],
            serde_json::json!({"type":"Point","coordinates":[6.01,49.01]})
        );
        assert_eq!(f[1]["properties"], serde_json::json!({"category":"health"}));
        assert_eq!(f[2]["properties"]["ele"], 1234);
        // The boundary is written as lines, with the level as a number.
        assert_eq!(f[4]["geometry"]["type"], "MultiLineString");
        assert_eq!(f[4]["properties"], serde_json::json!({"admin_level": 8}));
        assert_eq!(f[5]["properties"], serde_json::json!({"class":"lake"}));
        let kinds: Vec<(Kind, &str, &str)> = r
            .iter()
            .map(|x| (x.kind, x.class.as_str(), x.name.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                (Kind::Settlement, "town", "Testville"),
                (Kind::Poi, "pharmacy", ""),
                (Kind::Natural, "peak", "Mont Test"),
                (Kind::Natural, "lake", "Lac Test"),
                (Kind::Settlement, "suburb", "Quartier"),
            ]
        );
        // The unnamed peak without `ele` has no row; the lake has a box.
        assert_eq!(
            r[3].bbox,
            Some([6_000_000, 49_000_000, 6_020_000, 49_020_000])
        );
        assert_eq!((r[3].lon, r[3].lat), (6_010_000, 49_010_000));
        assert_eq!(s.unused, 0);
    }

    #[test]
    fn long_lines_are_skipped_whole() {
        let input = format!("short\n{}\nnext", "x".repeat(300));
        let mut cursor = input.as_bytes();
        let mut buf = Vec::new();
        let mut got = Vec::new();
        while let Some(fits) = read_line_bounded(&mut cursor, &mut buf, 200).expect("read") {
            got.push((fits, String::from_utf8(buf.clone()).expect("utf8")));
        }
        assert_eq!(
            got,
            [
                (true, "short".to_string()),
                (false, String::new()),
                (true, "next".to_string())
            ]
        );
    }

    #[test]
    fn messy_input_is_skipped_and_counted() {
        let p = r#"{"type":"Point","coordinates":[1,2]}"#;
        let input = [
            "\n\x1e\n".to_string(),
            "not json\n".into(),
            "{\"type\":\"Feature\"}\n".into(),
            feature(
                r#"{"type":"Point","coordinates":[1,2,3]}"#,
                r#"{"amenity":"toilets"}"#,
            ),
            feature(
                r#"{"type":"GeometryCollection","geometries":[]}"#,
                r#"{"amenity":"toilets"}"#,
            ),
            feature(p, r#"{"amenity":17}"#),
            feature(p, r#"{"highway":"bus_stop"}"#),
            feature(
                r#"{"type":"Polygon","coordinates":[[[0,0],[1,0]]]}"#,
                r#"{"boundary":"administrative","admin_level":"2"}"#,
            ),
            feature(p, r#"{"amenity":"toilets"}"#),
        ]
        .concat();
        let (f, r, s) = run_on(&input, 45.0);
        assert_eq!(s.lines, 8, "{s:?}");
        assert_eq!(s.unparsed, 2, "{s:?}");
        assert_eq!(s.bad_geometry, 3, "{s:?}");
        assert_eq!(s.unused, 2, "{s:?}");
        // Only the last toilets made it.
        assert_eq!((f.len(), r.len()), (1, 1));
    }

    #[test]
    fn the_same_input_gives_the_same_output() {
        let input = [
            feature(STREET, r#"{"highway":"primary","name":"Grand-Rue"}"#),
            feature(
                r#"{"type":"Point","coordinates":[6.123456789,49.987654321]}"#,
                r#"{"amenity":"cafe","name":"Café du Test","opening_hours":"24/7"}"#,
            ),
        ]
        .concat();
        let once = || {
            let (mut f, mut r) = (Vec::new(), Vec::new());
            run(
                input.as_bytes(),
                &mut f,
                &mut r,
                &Options { max_abs_lat: 50.0 },
            )
            .expect("run");
            (f, r)
        };
        let a = once();
        assert_eq!(a, once());
        let text = String::from_utf8(a.0).expect("utf8");
        assert!(
            text.contains(r#""coordinates":[6.123456789,49.987654321]"#),
            "{text}"
        );
        assert!(
            text.contains(r#""properties":{"category":"other","name":"Café du Test"}"#),
            "{text}"
        );
    }
}
