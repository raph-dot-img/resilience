//! `resilience-mapmaker sea`: the sea of a region (ADR 0016, decided
//! 2026-11-21), from OpenStreetMap's water polygons.
//!
//! A regional extract has coastline lines but no sea: the coastline is cut
//! where the extract ends, so it cannot be closed into areas without the
//! rest of the world. osmdata.openstreetmap.de publishes the sea made from
//! the whole planet's coastline (osmcoastline): `water-polygons-split-4326`,
//! a shapefile of polygons split into squares of at most 1 degree. The pack
//! build pins one copy of it by hash, cuts it to the rectangle around the
//! region with `ogr2ogr -spat ... -clipsrc spat_extent -f GeoJSONSeq`, and
//! this step turns each polygon into a feature of layer `water`, class
//! `sea`, for tippecanoe. The viewer clips a pack to its coverage polygon,
//! so the rectangle is enough.
//!
//! The input is pinned, so it is not hostile, but it is read with the same
//! care as osmium's output: one line at a time (at most [`MAX_LINE`]
//! bytes), only polygons with valid positions kept, everything else
//! skipped and counted, properties dropped. Output order is input order.

use std::io::{self, BufRead, Write};

use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Map, Value};

use resilience_core::maps::style::{Class, WaterClass};

use super::classify::{OutFeature, Tippecanoe, read_line_bounded};
use super::geom::Geometry;
use super::tags;

/// Longest input line (one polygon of a 1-degree square).
pub const MAX_LINE: usize = 64 * 1024 * 1024;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stats {
    pub read: u64,
    pub written: u64,
    /// Not a polygon, or positions outside the world, or not JSON.
    pub bad: u64,
    pub too_long: u64,
}

impl Stats {
    pub fn report(&self) -> String {
        format!(
            "sea: {} polygons read, {} written, {} bad, {} too long",
            self.read, self.written, self.bad, self.too_long
        )
    }
}

#[derive(Deserialize)]
struct InFeature<'a> {
    #[serde(borrow)]
    geometry: Option<&'a RawValue>,
}

/// Convert ogr2ogr's GeoJSON sequence to sea features. `max_abs_lat` is
/// the region's largest absolute latitude (for the style's minimum zoom,
/// as in `classify`).
pub fn convert(
    mut input: impl BufRead,
    mut out: impl Write,
    max_abs_lat: f64,
    stats: &mut Stats,
) -> io::Result<()> {
    let class = Class::Water(WaterClass::Sea);
    let minzoom = tags::min_zoom(class, max_abs_lat).unwrap_or(0);
    let mut props = Map::new();
    props.insert("class".into(), Value::from(WaterClass::Sea.as_str()));
    let mut buf = Vec::new();
    while let Some(fits) = read_line_bounded(&mut input, &mut buf, MAX_LINE)? {
        // ogr2ogr may start each record with RS (0x1e, RFC 8142).
        let line = trim(&buf);
        if line.is_empty() && fits {
            continue;
        }
        stats.read += 1;
        if !fits {
            stats.too_long += 1;
            continue;
        }
        let Some(raw) = serde_json::from_slice::<InFeature<'_>>(line)
            .ok()
            .and_then(|f| f.geometry)
        else {
            stats.bad += 1;
            continue;
        };
        let ok = serde_json::from_str::<Geometry>(raw.get()).is_ok_and(|g| {
            g.is_valid() && matches!(g, Geometry::Polygon(_) | Geometry::MultiPolygon(_))
        });
        if !ok {
            stats.bad += 1;
            continue;
        }
        serde_json::to_writer(
            &mut out,
            &OutFeature {
                kind: "Feature",
                tippecanoe: Tippecanoe {
                    layer: "water",
                    minzoom,
                },
                properties: &props,
                geometry: raw,
            },
        )?;
        out.write_all(b"\n")?;
        stats.written += 1;
    }
    // A write error in the last buffer must fail the step, not vanish
    // when the writer is dropped.
    out.flush()
}

fn trim(mut b: &[u8]) -> &[u8] {
    while let [0x1e | b' ' | b'\t' | b'\r', rest @ ..] = b {
        b = rest;
    }
    while let [rest @ .., b' ' | b'\t' | b'\r'] = b {
        b = rest;
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &str) -> (Vec<Value>, Stats) {
        let mut out = Vec::new();
        let mut st = Stats::default();
        convert(input.as_bytes(), &mut out, 48.9, &mut st).expect("convert");
        let v = out
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_slice(l).expect("json"))
            .collect();
        (v, st)
    }

    const SQUARE: &str =
        r#"{"type":"Polygon","coordinates":[[[-5,48],[-4,48],[-4,49],[-5,49],[-5,48]]]}"#;

    #[test]
    fn polygons_become_sea_features() {
        let input = format!(
            "{{\"type\":\"Feature\",\"properties\":{{\"FID\":7}},\"geometry\":{SQUARE}}}\n\
             \x1e{{\"type\":\"Feature\",\"properties\":{{}},\"geometry\":{{\"type\":\"MultiPolygon\",\"coordinates\":[[[[-5,48],[-4,48],[-4,49],[-5,48]]]]}}}}\n\n"
        );
        let (f, st) = run(&input);
        assert_eq!(st.read, 2);
        assert_eq!(st.written, 2);
        assert_eq!(f[0]["tippecanoe"]["layer"], "water");
        assert_eq!(f[0]["tippecanoe"]["minzoom"], 0);
        assert_eq!(f[0]["properties"], serde_json::json!({"class": "sea"}));
        assert_eq!(f[0]["geometry"]["type"], "Polygon");
        assert_eq!(f[1]["geometry"]["type"], "MultiPolygon");
    }

    #[test]
    fn anything_else_is_skipped_and_counted() {
        let input = "not json\n\
             {\"type\":\"Feature\",\"geometry\":null}\n\
             {\"type\":\"Feature\",\"geometry\":{\"type\":\"LineString\",\"coordinates\":[[0,0],[1,1]]}}\n\
             {\"type\":\"Feature\",\"geometry\":{\"type\":\"Polygon\",\"coordinates\":[[[0,0],[200,0],[0,1],[0,0]]]}}\n\
             {\"type\":\"Feature\",\"geometry\":{\"type\":\"Point\",\"coordinates\":[0,0]}}\n";
        let (f, st) = run(input);
        assert!(f.is_empty());
        assert_eq!((st.read, st.bad, st.written), (5, 5, 0));
        assert!(st.report().contains("5 bad"));
    }
}
