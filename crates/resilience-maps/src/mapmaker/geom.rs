//! The geometry the builder needs: GeoJSON geometries as osmium writes
//! them, a representative point and bounding box for search rows, the
//! centre of an area for points of interest, and the rings of a boundary
//! area as lines. Planar calculations in degrees are good enough for
//! these uses (a point to centre the map on, a box around a street).

use serde::{Deserialize, Serialize};

use crate::profile::{Micro, MicroBox};

pub type Position = [f64; 2];

/// A GeoJSON geometry (RFC 7946 §3.1) with two-number positions. A
/// position with an altitude, or any other shape, fails to parse and the
/// feature is skipped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "coordinates")]
pub enum Geometry {
    Point(Position),
    MultiPoint(Vec<Position>),
    LineString(Vec<Position>),
    MultiLineString(Vec<Vec<Position>>),
    Polygon(Vec<Vec<Position>>),
    MultiPolygon(Vec<Vec<Vec<Position>>>),
}

/// Only the `type` of a geometry, read without parsing its coordinates.
#[derive(Debug, Deserialize)]
pub struct GeometryType<'a> {
    #[serde(rename = "type", borrow)]
    pub kind: &'a str,
}

fn valid(p: &Position) -> bool {
    p[0].is_finite() && p[1].is_finite() && p[0].abs() <= 180.0 && p[1].abs() <= 90.0
}

impl Geometry {
    /// Every position, in order.
    pub fn positions(&self) -> Box<dyn Iterator<Item = &Position> + '_> {
        match self {
            Geometry::Point(p) => Box::new(std::iter::once(p)),
            Geometry::MultiPoint(v) | Geometry::LineString(v) => Box::new(v.iter()),
            Geometry::MultiLineString(v) | Geometry::Polygon(v) => Box::new(v.iter().flatten()),
            Geometry::MultiPolygon(v) => Box::new(v.iter().flatten().flatten()),
        }
    }

    /// Whether every position is a finite point of the world.
    pub fn is_valid(&self) -> bool {
        let mut any = false;
        for p in self.positions() {
            if !valid(p) {
                return false;
            }
            any = true;
        }
        any
    }

    /// The bounding box in microdegrees (`None` if empty or invalid).
    pub fn bbox(&self) -> Option<MicroBox> {
        let mut b: Option<(f64, f64, f64, f64)> = None;
        for p in self.positions() {
            if !valid(p) {
                return None;
            }
            b = Some(match b {
                None => (p[0], p[1], p[0], p[1]),
                Some((w, s, e, n)) => (w.min(p[0]), s.min(p[1]), e.max(p[0]), n.max(p[1])),
            });
        }
        let (w, s, e, n) = b?;
        Some(MicroBox {
            south_west: Micro::from_degrees(w, s)?,
            north_east: Micro::from_degrees(e, n)?,
        })
    }

    /// The point used for a line in search: half-way along its longest
    /// part (the first such part). Lengths are planar, with longitude
    /// scaled by the cosine of the line's first latitude.
    pub fn line_point(&self) -> Option<Micro> {
        let parts: Vec<&Vec<Position>> = match self {
            Geometry::LineString(v) => vec![v],
            Geometry::MultiLineString(v) => v.iter().collect(),
            _ => return None,
        };
        let mut best: Option<(&Vec<Position>, f64)> = None;
        for p in parts {
            if p.is_empty() || p.iter().any(|q| !valid(q)) {
                return None;
            }
            let len = line_length(p);
            if best.is_none_or(|(_, b)| len > b) {
                best = Some((p, len));
            }
        }
        let (line, total) = best?;
        let k = line[0][1].to_radians().cos();
        let mut left = total / 2.0;
        for w in line.windows(2) {
            let d = seg_length(&w[0], &w[1], k);
            if d > 0.0 && left <= d {
                let t = left / d;
                let x = w[0][0] + (w[1][0] - w[0][0]) * t;
                let y = w[0][1] + (w[1][1] - w[0][1]) * t;
                return Micro::from_degrees(x, y);
            }
            left -= d;
        }
        // A single point, or a line of zero length.
        Micro::from_degrees(line[0][0], line[0][1])
    }

    /// The centre of an area: the area-weighted centroid of its largest
    /// outer ring (the vertex average for a degenerate ring). For a
    /// point, the point.
    pub fn area_point(&self) -> Option<Micro> {
        let outers: Vec<&Vec<Position>> = match self {
            Geometry::Point(p) => {
                valid(p).then_some(())?;
                return Micro::from_degrees(p[0], p[1]);
            }
            Geometry::Polygon(rings) => rings.first().into_iter().collect(),
            Geometry::MultiPolygon(polys) => polys.iter().filter_map(|r| r.first()).collect(),
            _ => return None,
        };
        let mut best: Option<(&Vec<Position>, f64)> = None;
        for ring in outers {
            if ring.iter().any(|p| !valid(p)) {
                return None;
            }
            let a = ring_area(ring).abs();
            if best.is_none_or(|(_, b)| a > b) {
                best = Some((ring, a));
            }
        }
        let (ring, _) = best?;
        let (x, y) = centroid(ring)?;
        Micro::from_degrees(x, y)
    }

    /// The rings of an area as lines (for boundaries, drawn as lines).
    pub fn rings_as_lines(&self) -> Option<Vec<Vec<Position>>> {
        let lines: Vec<Vec<Position>> = match self {
            Geometry::Polygon(rings) => rings.clone(),
            Geometry::MultiPolygon(polys) => polys.iter().flatten().cloned().collect(),
            _ => return None,
        };
        // A ring has at least 4 positions (the first repeated last).
        let lines: Vec<_> = lines.into_iter().filter(|l| l.len() >= 4).collect();
        (!lines.is_empty()).then_some(lines)
    }
}

fn seg_length(a: &Position, b: &Position, k: f64) -> f64 {
    ((b[0] - a[0]) * k).hypot(b[1] - a[1])
}

fn line_length(line: &[Position]) -> f64 {
    let Some(first) = line.first() else {
        return 0.0;
    };
    let k = first[1].to_radians().cos();
    line.windows(2).map(|w| seg_length(&w[0], &w[1], k)).sum()
}

/// Signed area (shoelace) of a ring, in square degrees. The ring may or
/// may not repeat its first vertex.
pub fn ring_area(ring: &[Position]) -> f64 {
    if ring.len() < 3 {
        return 0.0;
    }
    let (x0, y0) = (ring[0][0], ring[0][1]);
    let mut a = 0.0;
    for w in ring.windows(2) {
        a += (w[0][0] - x0) * (w[1][1] - y0) - (w[1][0] - x0) * (w[0][1] - y0);
    }
    a / 2.0
}

/// Area-weighted centroid of a ring (coordinates relative to the first
/// vertex, for precision); the vertex average when the area is ~0.
pub fn centroid(ring: &[Position]) -> Option<(f64, f64)> {
    let first = ring.first()?;
    let (x0, y0) = (first[0], first[1]);
    let a = ring_area(ring);
    if a.abs() > 1e-14 {
        let (mut cx, mut cy) = (0.0, 0.0);
        for w in ring.windows(2) {
            let (ax, ay) = (w[0][0] - x0, w[0][1] - y0);
            let (bx, by) = (w[1][0] - x0, w[1][1] - y0);
            let cross = ax * by - bx * ay;
            cx += (ax + bx) * cross;
            cy += (ay + by) * cross;
        }
        let (x, y) = (x0 + cx / (6.0 * a), y0 + cy / (6.0 * a));
        if x.is_finite() && y.is_finite() {
            return Some((x, y));
        }
    }
    let n = ring.len() as f64;
    let (sx, sy) = ring
        .iter()
        .fold((0.0, 0.0), |(sx, sy), p| (sx + p[0] - x0, sy + p[1] - y0));
    Some((x0 + sx / n, y0 + sy / n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Geometry {
        serde_json::from_str(s).expect("geometry")
    }

    #[test]
    fn geojson_geometries_parse_and_round_trip() {
        let g = parse(r#"{"type":"LineString","coordinates":[[6.1,49.6],[6.2,49.7],[6.3,49.65]]}"#);
        let b = g.bbox().expect("bbox");
        assert_eq!(b.south_west, Micro::from_degrees(6.1, 49.6).expect("sw"));
        assert_eq!(b.north_east, Micro::from_degrees(6.3, 49.7).expect("ne"));
        // Half-way along the line, not its middle vertex.
        let two = parse(r#"{"type":"LineString","coordinates":[[6.0,49.0],[6.2,49.0]]}"#);
        assert_eq!(two.line_point(), Micro::from_degrees(6.1, 49.0));
        let bent = parse(r#"{"type":"LineString","coordinates":[[0,0],[0,1],[0,4]]}"#);
        assert_eq!(bent.line_point(), Micro::from_degrees(0.0, 2.0));
        // The longest part of a multi-line.
        let multi = parse(
            r#"{"type":"MultiLineString","coordinates":[[[0,0],[0,1]],[[5,0],[5,3]],[[9,9],[9,9.5],[9,10]]]}"#,
        );
        assert_eq!(multi.line_point(), Micro::from_degrees(5.0, 1.5));
        let dot = parse(r#"{"type":"LineString","coordinates":[[3,4],[3,4]]}"#);
        assert_eq!(dot.line_point(), Micro::from_degrees(3.0, 4.0));
        // Serialising gives the same numbers back (shortest round trip).
        let s = serde_json::to_string(&g).expect("json");
        assert_eq!(parse(&s), g);
        assert!(s.contains("6.1,49.6"));
        // Altitudes, unknown types, bad numbers: refused.
        for bad in [
            r#"{"type":"Point","coordinates":[1,2,3]}"#,
            r#"{"type":"GeometryCollection","geometries":[]}"#,
            r#"{"type":"Point","coordinates":["1","2"]}"#,
            r#"{"type":"Point"}"#,
        ] {
            assert!(serde_json::from_str::<Geometry>(bad).is_err(), "{bad}");
        }
        assert!(!parse(r#"{"type":"Point","coordinates":[181,0]}"#).is_valid());
        assert!(!parse(r#"{"type":"LineString","coordinates":[]}"#).is_valid());
        assert_eq!(
            parse(r#"{"type":"Point","coordinates":[0,95]}"#).bbox(),
            None
        );
    }

    #[test]
    fn area_points_and_rings() {
        // A 2x2 square with a far smaller second polygon: the square's
        // centre wins.
        let g = parse(
            r#"{"type":"MultiPolygon","coordinates":[
                [[[10,10],[10.1,10],[10.1,10.1],[10,10]]],
                [[[0,0],[2,0],[2,2],[0,2],[0,0]],[[0.5,0.5],[0.5,1],[1,1],[0.5,0.5]]]]}"#,
        );
        assert_eq!(g.area_point(), Micro::from_degrees(1.0, 1.0));
        let lines = g.rings_as_lines().expect("rings");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].len(), 5);
        // A degenerate ring falls back to the vertex average.
        let flat = parse(r#"{"type":"Polygon","coordinates":[[[0,0],[2,0],[4,0],[0,0]]]}"#);
        assert_eq!(flat.area_point(), Micro::from_degrees(1.5, 0.0));
        let p = parse(r#"{"type":"Point","coordinates":[6.5,49.5]}"#);
        assert_eq!(p.area_point(), Micro::from_degrees(6.5, 49.5));
        assert_eq!(p.rings_as_lines(), None);
        assert_eq!(p.line_point(), None);
        // The ring's orientation does not matter.
        let cw = parse(r#"{"type":"Polygon","coordinates":[[[0,0],[0,2],[2,2],[2,0],[0,0]]]}"#);
        assert_eq!(cw.area_point(), Micro::from_degrees(1.0, 1.0));
        assert!((ring_area(&[[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]) - 4.0).abs() < 1e-12);
    }
}
